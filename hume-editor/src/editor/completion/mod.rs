//! Completion — both of the editor's completion systems.
//!
//! - Insert-mode: `CompletionSession` (`session.rs`) holds every
//!   contributing source's items and does the per-keystroke filter/rank;
//!   `CompletionItem` (`item.rs`) is its item type; `session/accept.rs`
//!   applies the accepted item as a buffer edit.
//! - Minibuffer (`:` command line): the completer functions below,
//!   prefix-matched; accept splices the minibuffer input.
//!
//! One module because today they share only `hume_ui::popup`'s menu
//! renderer, and the shared half has to have somewhere to land.
//!
//! Minibuffer design contract:
//! - Each completer is a pure function: given `(input, cursor, ctx)` it
//!   returns a sorted `Vec<Completion>` and the byte offset in `input` at which
//!   the completed token starts (`span_start`).  No &mut access, no I/O side
//!   effects visible to the caller.
//! - `MinibufCompletionState` lives on `CommandLayer`
//!   (`input_stack/command.rs`) — cleared whenever the minibuffer closes or
//!   the user edits the input by any key other than Tab / Shift-Tab.

use std::path::Path;

use crate::editor::buffer::store::BufferStore;
use crate::editor::registry::CommandRegistry;
use hume_treesitter::registry::LanguageRegistry;

mod item;
mod path;
mod session;
mod set;
mod simple;

pub(in crate::editor) use item::CompletionItem;
pub(in crate::editor) use path::complete_path;
pub(in crate::editor) use session::{CompletionMenuUi, CompletionSession};
pub(in crate::editor) use set::complete_set;
pub(in crate::editor) use simple::{complete_buffer_name, complete_command, complete_theme};

// ── Public types ──────────────────────────────────────────────────────────────

/// A single completion candidate.
///
/// `display` is shown in the popup (may include decorators like trailing `/`
/// for directories). `replacement` is the text written into the minibuffer.
/// The two fields are often identical; they differ for e.g. buffer names where
/// the display is the basename but the replacement is the full path.
#[derive(Debug, Clone)]
pub(in crate::editor) struct Completion {
    /// BufferText to insert at the span location in the minibuffer input.
    pub replacement: String,
    /// BufferText shown in the completion popup row.
    pub display: String,
}

/// Completion session state, stored on `Editor` while a popup is open.
///
/// Invariant: `selected < candidates.len()`. Created only when there are ≥2
/// candidates (single-candidate completion is applied silently without state).
pub(in crate::editor) struct MinibufCompletionState {
    pub candidates: Vec<Completion>,
    /// Index of the currently-displayed candidate.
    pub selected: usize,
    /// Byte offset in the minibuffer input where the completed token starts.
    /// Constant across the session (the span start never shifts while cycling).
    pub span_start: usize,
    /// `candidates`' display strings, pre-measured once here at construction
    /// — candidates never change during a session's lifetime (only
    /// `selected` does), so `Editor::sync_minibuf_completion_view` clones
    /// this (an `Arc` bump) each frame instead of re-collecting and
    /// re-measuring every candidate every frame the popup stays open.
    pub rows: hume_ui::popup::MenuRows,
}

impl MinibufCompletionState {
    /// The byte range that the current replacement occupies in the input.
    pub(in crate::editor) fn current_span(&self) -> std::ops::Range<usize> {
        debug_assert!(
            self.selected < self.candidates.len(),
            "MinibufCompletionState invariant violated: selected {} >= len {}",
            self.selected,
            self.candidates.len(),
        );
        let end = self.span_start + self.candidates[self.selected].replacement.len();
        self.span_start..end
    }
}

/// Context supplied to every minibuffer completer function.
///
/// Bundles read-only references to the editor state that completers need
/// (command registry, buffer list, working directory) without exposing a full
/// `&Editor`.  This makes unit-testing completers straightforward — no Editor
/// construction required.
pub(in crate::editor) struct CompletionCtx<'a> {
    pub registry: &'a CommandRegistry,
    pub buffers: &'a BufferStore,
    pub cwd: &'a Path,
    pub languages: &'a LanguageRegistry,
}

/// Result of a single completer function call.
///
/// `span_start` is the byte offset in `input` where the completed token
/// begins.  All candidates are replacements for `input[span_start..cursor]`.
pub(in crate::editor) struct CompletionResult {
    pub span_start: usize,
    pub candidates: Vec<Completion>,
}

impl CompletionResult {
    /// Sort `candidates` by display text and wrap for `span_start` — every
    /// completer does this immediately before returning.
    fn sorted(span_start: usize, mut candidates: Vec<Completion>) -> Self {
        candidates.sort_unstable_by(|a, b| a.display.cmp(&b.display));
        Self {
            span_start,
            candidates,
        }
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Extract the argument prefix for commands that take a single argument.
///
/// Splits `input[..cursor]` on the first space.  Returns `(arg_start, prefix)`
/// where `arg_start` is the byte offset of the argument in `input` and
/// `prefix` is the unfinished argument text up to the cursor.
///
/// If there is no space (command-only input), returns `(0, input[..cursor])`.
fn arg_prefix(input: &str, cursor: usize) -> (usize, &str) {
    let up_to_cursor = &input[..cursor.min(input.len())];
    match up_to_cursor.find(' ') {
        Some(space_idx) => (space_idx + 1, &up_to_cursor[space_idx + 1..]),
        None => (0, up_to_cursor),
    }
}

/// Scan `themes/*.toml` in every search path and return the stems that start
/// with `prefix` (excluding an exact match, so Tab on a fully-typed theme name
/// is a no-op rather than re-offering it). User themes (earlier in the search
/// path list) shadow bundled themes with the same stem.
///
/// Shared by `:theme` (via [`complete_theme`](simple::complete_theme)) and
/// `:set global theme=` (via [`complete_set`](set::complete_set)) so the
/// candidate set stays in sync.
fn theme_name_candidates(prefix: &str) -> Vec<Completion> {
    let mut seen: rustc_hash::FxHashSet<String> = rustc_hash::FxHashSet::default();
    let mut candidates: Vec<Completion> = Vec::new();

    for dir in &super::theme_search_paths() {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            // User themes (earlier in search_dirs) shadow bundled themes.
            if !seen.insert(stem.to_owned()) {
                continue;
            }
            if stem.starts_with(prefix) && stem != prefix {
                candidates.push(Completion {
                    replacement: stem.to_owned(),
                    display: stem.to_owned(),
                });
            }
        }
    }

    candidates
}

// ── Shared test support ───────────────────────────────────────────────────────

#[cfg(test)]
mod testing {
    use tempfile::TempDir;

    use super::*;
    use crate::editor::buffer::Buffer;
    use hume_editing::selection::SelectionSet;
    use hume_editing::text::BufferText;
    use hume_engine::pipeline::{BufferId, EngineView};
    use hume_engine::theme::Theme;
    use std::path::PathBuf;

    pub(in crate::editor::completion) fn make_ctx_parts() -> (CommandRegistry, BufferStore, TempDir)
    {
        let reg = CommandRegistry::with_defaults();
        let store = BufferStore::new();
        let dir = tempfile::tempdir().unwrap();
        (reg, store, dir)
    }

    pub(in crate::editor) fn ctx<'a>(
        registry: &'a CommandRegistry,
        buffers: &'a BufferStore,
        cwd: &'a Path,
    ) -> CompletionCtx<'a> {
        ctx_with(registry, buffers, cwd, empty_langs())
    }

    pub(in crate::editor::completion) fn ctx_with<'a>(
        registry: &'a CommandRegistry,
        buffers: &'a BufferStore,
        cwd: &'a Path,
        languages: &'a LanguageRegistry,
    ) -> CompletionCtx<'a> {
        CompletionCtx {
            registry,
            buffers,
            cwd,
            languages,
        }
    }

    /// Shared empty registry for tests that don't register languages — avoids
    /// re-allocating one per `ctx()` call and sidesteps the borrow-lifetime
    /// issue of constructing it inline.
    pub(in crate::editor::completion) fn empty_langs() -> &'static LanguageRegistry {
        use std::sync::OnceLock;
        static EMPTY: OnceLock<LanguageRegistry> = OnceLock::new();
        EMPTY.get_or_init(LanguageRegistry::new)
    }

    pub(in crate::editor) fn ev() -> EngineView {
        EngineView::new(Theme::default())
    }

    pub(in crate::editor) fn make_id(ev: &mut EngineView) -> BufferId {
        ev.buffers.insert(())
    }

    pub(in crate::editor) fn make_buf() -> Buffer {
        Buffer::new(BufferText::from("a\n"), SelectionSet::default())
    }

    pub(in crate::editor::completion) fn buf_with_path(path: &str) -> Buffer {
        let mut b = make_buf();
        b.set_path(Some(PathBuf::from(path)));
        b
    }
}
