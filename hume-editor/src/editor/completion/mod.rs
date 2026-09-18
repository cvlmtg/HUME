//! Completion — both of the editor's completion systems, on one shared
//! item/session model.
//!
//! - Insert-mode: `CompletionSession` (`session.rs`) holds every
//!   contributing source's items and does the per-keystroke filter/rank;
//!   `CompletionItem` (`item.rs`) is its item type; `session/accept.rs`
//!   applies the accepted item as a buffer edit.
//! - Minibuffer (`:` command line): the same `CompletionSession`/
//!   `CompletionItem`, targeted at the minibuffer's own input instead of a
//!   buffer (`CompletionTarget::Minibuf`) — see `session.rs`'s module doc.
//!
//! What still differs per source is *how* candidates are gathered — see
//! `MatchKind`'s own doc for the three kinds and why `Delegated` sources
//! (`complete_path`/`complete_set`) keep taking the live input while
//! `Fuzzy`/`String` sources don't.

use std::path::Path;

use crate::editor::buffer::store::BufferStore;
use crate::editor::registry::CommandRegistry;
use hume_treesitter::registry::LanguageRegistry;

mod item;
mod path;
mod registry;
mod session;
mod set;
mod simple;

pub(in crate::editor) use item::CompletionItem;
pub(in crate::editor) use path::{PATH_DIRS_ONLY_SOURCE, PATH_SOURCE};
pub(in crate::editor) use registry::{CompletionSourceRegistry, SourceResult};
pub(in crate::editor) use session::{CompletionMenuUi, CompletionSession, Interaction, MatchKind};
pub(in crate::editor) use set::SET_SOURCE;
pub(in crate::editor) use simple::{BUFFER_NAME_SOURCE, COMMAND_SOURCE, THEME_SOURCE};

// ── Context ──────────────────────────────────────────────────────────────────

/// Context supplied to every completer function — Insert-mode's native
/// sources (none exist yet) and the minibuffer's five.
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

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Extract the argument prefix for commands that take a single argument.
///
/// Splits `input[..cursor]` on the first space.  Returns `(arg_start, prefix)`
/// where `arg_start` is the byte offset of the argument in `input` and
/// `prefix` is the unfinished argument text up to the cursor.
///
/// If there is no space (command-only input), returns `(0, input[..cursor])`.
/// `Delegated` sources use this for their own filtering; `complete_minibuf`
/// (`input_stack/command.rs`) uses it too, for a `String`/`Fuzzy` argument
/// source's span — that source's own function doesn't see the input at all,
/// so nothing else computes this for it.
pub(in crate::editor) fn arg_prefix(input: &str, cursor: usize) -> (usize, &str) {
    let up_to_cursor = &input[..cursor.min(input.len())];
    match up_to_cursor.find(' ') {
        Some(space_idx) => (space_idx + 1, &up_to_cursor[space_idx + 1..]),
        None => (0, up_to_cursor),
    }
}

/// Scan `themes/*.toml` in every search path and return the stems that start
/// with `prefix` (excluding an exact match, so a fully-typed theme name
/// isn't re-offered). User themes (earlier in the search path list) shadow
/// bundled themes with the same stem.
///
/// Shared by `:theme` ([`complete_theme`], called with an empty prefix — a
/// `String`-kind source's full universe) and `:set global theme=`'s value
/// phase ([`set::complete_set`], `Delegated`, called with the real typed
/// prefix) so the candidate set stays in sync between the two — but the two
/// callers' `MatchKind`s disagree on who does the sorting, so `delegated`
/// picks the right one: `false` (`complete_theme`) sets `sort_text = stem`
/// and leaves the list in scan order, so the session's own `String`-kind
/// tiebreak (`sort_text` ascending) alphabetizes it; `true` (`complete_set`)
/// sorts alphabetically here and gives every item an empty `sort_text`, so
/// the rank key's index-ascending tiebreak preserves *this* order — the
/// contract every other `Delegated` source's own item constructor follows
/// (see `MatchKind::Delegated`'s doc). Leaving `sort_text = stem` for the
/// `Delegated` caller would violate that contract while still looking
/// alphabetized by accident (`sort_text` ascending happens to equal what an
/// explicit sort produces) — this makes the alphabetizing the source's own
/// doing instead of a side effect nothing enforces.
fn theme_name_candidates(prefix: &str, delegated: bool) -> Vec<CompletionItem> {
    let mut seen: rustc_hash::FxHashSet<String> = rustc_hash::FxHashSet::default();
    let mut candidates = Vec::new();

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
                let sort_text = if delegated {
                    String::new()
                } else {
                    stem.to_owned()
                };
                candidates.push(CompletionItem::plain(
                    stem.to_owned(),
                    stem.to_owned(),
                    sort_text,
                ));
            }
        }
    }

    if delegated {
        candidates.sort_unstable_by(|a, b| a.label.cmp(&b.label));
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

    pub(in crate::editor::completion) fn ctx<'a>(
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

    pub(in crate::editor::completion) fn ev() -> EngineView {
        EngineView::new(Theme::default())
    }

    pub(in crate::editor::completion) fn make_id(ev: &mut EngineView) -> BufferId {
        ev.buffers.insert(())
    }

    pub(in crate::editor::completion) fn make_buf() -> Buffer {
        Buffer::new(BufferText::from("a\n"), SelectionSet::default())
    }

    pub(in crate::editor::completion) fn buf_with_path(path: &str) -> Buffer {
        let mut b = make_buf();
        b.set_path(Some(PathBuf::from(path)));
        b
    }
}
