//! Completion: both of the editor's completion targets (Insert mode and
//! the `:` command line), on one model:
//!
//! - `registry.rs`: every *source*, native or Steel-registered, keyed by
//!   name: what it targets, how its items score.
//! - `session.rs`: the one open *session*: each participating source's
//!   latest invocation (the document it saw, the span it answered for, its
//!   items), ranked per keystroke against each source's own token;
//!   `session/accept.rs` applies the accepted item as a buffer edit.
//! - `orchestrate.rs`: the one *driver*: triggers, invokes sources, lands
//!   answers, reacts to edits, applies the `:` line's eager policy.
//! - `item.rs`: the item type both targets share; `simple.rs`/`path.rs`/
//!   `set.rs`: the native minibuffer sources.

use std::path::Path;

use crate::editor::buffer::store::BufferStore;
use crate::editor::registry::CommandRegistry;
use hume_treesitter::registry::LanguageRegistry;

mod item;
mod orchestrate;
mod path;
mod registry;
mod session;
mod set;
mod simple;

pub(in crate::editor) use item::CompletionItem;
pub(in crate::editor) use orchestrate::Trigger;
pub(in crate::editor) use path::{PATH_DIRS_ONLY_SOURCE, PATH_SOURCE};
pub(in crate::editor) use registry::{
    BufferSourceEntry, MinibufBody, MinibufSourceEntry, RegisterOutcome, SourceRegistry,
};
pub(in crate::editor) use session::{BufferSession, MatchKind, MinibufSession};
pub(in crate::editor) use set::SET_SOURCE;
pub(in crate::editor) use simple::{BUFFER_NAME_SOURCE, COMMAND_SOURCE, THEME_SOURCE};

// ── Context ──────────────────────────────────────────────────────────────────

/// Context supplied to every native completer function.
///
/// Bundles read-only references to the editor state that completers need
/// (command registry, buffer list, working directory) without exposing a full
/// `&Editor`.  This makes unit-testing completers straightforward: no Editor
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
/// Correct only for a *single*-argument command, where "everything after
/// the command name" genuinely is the one argument (`path.rs`'s own
/// `:e`/`:w`/`:cd` callers). A multi-argument command's own argument span
/// is [`arg_span`], which finds the *last* stop char before the cursor
/// instead.
pub(in crate::editor) fn arg_prefix(input: &str, cursor: usize) -> (usize, &str) {
    let up_to_cursor = &input[..cursor.min(input.len())];
    match up_to_cursor.find(' ') {
        Some(space_idx) => (space_idx + 1, &up_to_cursor[space_idx + 1..]),
        None => (0, up_to_cursor),
    }
}

/// Forward counterpart of [`arg_prefix`]'s backward scan: the byte offset,
/// at or after `cursor`, of the first char in `stops`, or `input.len()` if
/// none appears before the end. The token a completion *replaces* extends
/// past the cursor to wherever it actually ends (`arg_prefix` alone only
/// ever looks at `input[..cursor]`), so a candidate applied with the cursor
/// mid-token doesn't duplicate the token's own tail (`:e src/ma|in.rs` +
/// Tab must not produce `src/main.rsin.rs`).
///
/// `stops` names the token's own separator alphabet: a `:set` key stops at
/// `'='` too (so completing `:set global th|eme=x` doesn't swallow the
/// `=x`), where a path or command-name token stops at whitespace alone.
pub(in crate::editor) fn token_end_at(input: &str, cursor: usize, stops: &[char]) -> usize {
    let from = cursor.min(input.len());
    input[from..]
        .find(|c: char| stops.contains(&c))
        .map_or(input.len(), |i| from + i)
}

/// The `[start, end)` span of the token at `cursor`: `start` is one past
/// the last `back_stop` at or before `cursor` (0 if none), `end` is
/// [`token_end_at`]'s forward scan for the first of `fwd_stops`. `back_stop`
/// is a single char, not a slice like `fwd_stops`: every caller's forward
/// and backward separator alphabets already differ (a `:set` key stops
/// backward at `' '` but forward at `[' ', '=']`, so a completed key's span
/// doesn't swallow a following `=value`; a `:set` value stops backward at
/// `'='` and forward at `[' ']` only, since the value itself may contain
/// `=`), and every one of them backs up to exactly one separator, never a
/// choice of several. Shared by [`super::orchestrate`]'s whitespace-
/// delimited `'arg` span (`back_stop`/`fwd_stops` both `' '`) and `set.rs`'s
/// three phases.
pub(in crate::editor) fn arg_span(
    input: &str,
    cursor: usize,
    back_stop: char,
    fwd_stops: &[char],
) -> std::ops::Range<usize> {
    let up_to = &input[..cursor.min(input.len())];
    let start = up_to
        .rfind(back_stop)
        .map_or(0, |i| i + back_stop.len_utf8());
    start..token_end_at(input, cursor, fwd_stops)
}

/// Every installed theme's name (file stem), directory-scanned across
/// `theme_search_paths()` once each; a stem in an earlier search path
/// shadows a same-named one in a later path. Shared by `:theme`
/// ([`simple::complete_theme`], a `String`-kind source's full universe,
/// no prefix filtering here, since that source's own token-vs-item scoring
/// happens later, at rank time) and `:set global theme=`'s value phase
/// ([`set::complete_set_value`], `Delegated`, filtered by the typed prefix
/// through [`set::prefix_completions`]): the one place both need to agree
/// on which theme names exist and which one wins under a shared stem.
fn theme_name_candidates() -> Vec<String> {
    let mut seen: rustc_hash::FxHashSet<String> = rustc_hash::FxHashSet::default();
    let mut stems = Vec::new();

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
            if seen.insert(stem.to_owned()) {
                stems.push(stem.to_owned());
            }
        }
    }

    stems
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
        let dir = crate::editor::tests::safe_tempdir();
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

    /// Shared empty registry for tests that don't register languages. It avoids
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
