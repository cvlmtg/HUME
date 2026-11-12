//! Completion: both of the editor's completion targets (Insert mode and
//! the minibuffer's `:` and `|` lines), on one model:
//!
//! - `registry.rs`: every *source*, native or Steel-registered, keyed by
//!   name: what it targets, how its items score.
//! - `session.rs`: the one open *session*: each participating source's
//!   latest invocation (the document it saw, the span it answered for, its
//!   items), ranked per keystroke against each source's own token;
//!   `session/accept.rs` applies the accepted item as a buffer edit.
//! - `orchestrate.rs`: the one *driver*: triggers, invokes sources, lands
//!   answers, reacts to edits, applies the minibuffer's eager policy.
//! - `item.rs`: the item type both targets share; `simple.rs`/`path.rs`/
//!   `set.rs`: the native minibuffer sources.

use std::ops::Range;

use hume_platform::path::AbsolutePath;

use crate::editor::buffer::store::BufferStore;
use crate::editor::registry::CommandRegistry;
use crate::editor::settings::ArgSplit;
use hume_treesitter::registry::LanguageRegistry;

mod item;
mod orchestrate;
mod path;
mod registry;
mod session;
mod set;
mod simple;

pub(in crate::editor) use item::{CompletionItem, Documentation};
pub(in crate::editor) use orchestrate::Trigger;
pub(in crate::editor) use path::{PATH_DIRS_ONLY_SOURCE, PATH_SOURCE};
pub(in crate::editor) use registry::{
    BufferSourceEntry, BufferSourceId, MinibufBody, MinibufSourceEntry, RegisterOutcome,
    SourceRegistry,
};
pub(in crate::editor) use session::{BufferSession, ItemKey, MinibufSession};
pub(in crate::editor) use set::{GET_SOURCE, SET_SOURCE};
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
    pub cwd: &'a AbsolutePath,
    pub languages: &'a LanguageRegistry,
    pub dirs: &'a hume_platform::dirs::Dirs,
}

/// The argument a minibuffer source completes: the line, the cursor, and
/// where the argument starts, which is the floor past its leading
/// whitespace. The floor is where the command name ends (0 on a line that
/// is all argument), so `:e!/tmp/x` and `:e /tmp/x` read the same
/// argument, as `parse_typed_command` does on Enter. Narrowing moves the
/// start forward, never past the cursor, so no span it gives starts before
/// the argument.
#[derive(Clone, Copy)]
pub(in crate::editor) struct MinibufArg<'a> {
    input: &'a str,
    start: usize,
    cursor: usize,
}

impl<'a> MinibufArg<'a> {
    /// Panics unless `floor <= cursor <= input.len()`.
    fn new(input: &'a str, floor: usize, cursor: usize) -> Self {
        assert!(
            floor <= cursor && cursor <= input.len(),
            "argument floor {floor} and cursor {cursor} out of order in {input:?}"
        );
        Self {
            input,
            start: floor,
            cursor,
        }
        .trim_start()
    }

    /// `self` narrowed to `tail`, a suffix of its prefix.
    fn narrowed_to(self, tail: &str) -> Self {
        Self {
            start: self.cursor - tail.len(),
            ..self
        }
    }

    pub(in crate::editor) fn input(&self) -> &'a str {
        self.input
    }

    pub(in crate::editor) fn cursor(&self) -> usize {
        self.cursor
    }

    /// The argument up to the cursor.
    pub(in crate::editor) fn prefix(&self) -> &'a str {
        &self.input[self.start..self.cursor]
    }

    /// The span a completion of the argument replaces: from its start, the
    /// text the completer filtered on, to the first of `stops` at or after
    /// the cursor, or the line's end. Its end runs past the cursor, so a
    /// candidate applied mid-token doesn't duplicate the token's tail
    /// (`:e src/ma|in.rs` + Tab must not produce `src/main.rsin.rs`).
    pub(in crate::editor) fn token_span(&self, stops: &[char]) -> Range<usize> {
        let end = self.input[self.cursor..]
            .find(stops)
            .map_or(self.input.len(), |i| self.cursor + i);
        self.start..end
    }

    /// The span of the space-delimited word the cursor is in: the span a
    /// source that does not compute its own replaces.
    pub(in crate::editor) fn last_word_span(&self) -> Range<usize> {
        let word = self
            .prefix()
            .rsplit_once(' ')
            .map_or(*self, |(_, word)| self.narrowed_to(word));
        word.token_span(&[' '])
    }
}

impl<'a> ArgSplit<'a> for MinibufArg<'a> {
    /// The prefix before the first `sep`, and the argument after it.
    fn split_once(self, sep: char) -> Option<(&'a str, Self)> {
        let (head, tail) = self.prefix().split_once(sep)?;
        Some((head, self.narrowed_to(tail)))
    }

    fn trim_start(self) -> Self {
        self.narrowed_to(self.prefix().trim_start())
    }
}

/// Every installed theme's name (file stem), directory-scanned across
/// `theme_search_paths()` once each; a stem in an earlier search path
/// shadows a same-named one in a later path. No prefix filtering here
/// (callers score/filter later): the one place every caller needs to
/// agree on which theme names exist and which one wins under a shared
/// stem.
fn theme_name_candidates(dirs: &hume_platform::dirs::Dirs) -> Vec<String> {
    let mut seen: rustc_hash::FxHashSet<String> = rustc_hash::FxHashSet::default();
    let mut stems = Vec::new();

    for dir in &super::theme_search_paths(dirs) {
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

#[cfg(test)]
mod tests;

// ── Shared test support ───────────────────────────────────────────────────────

#[cfg(test)]
mod testing {
    use tempfile::TempDir;

    use super::*;
    use crate::editor::buffer::NewBuffer;
    use hume_core::BufferId;
    use hume_editing::text::BufferText;
    use hume_engine::pipeline::EngineView;
    use hume_engine::theme::Theme;

    /// `input`'s argument with the cursor at its end, floored where the
    /// command name ends, as the orchestrator floors a `:` line.
    pub(in crate::editor::completion) fn command_arg(input: &str) -> MinibufArg<'_> {
        let floor = crate::editor::input_stack::command::scan_command_name(input).2;
        MinibufArg::new(input, floor, input.len())
    }

    pub(in crate::editor::completion) fn make_ctx_parts() -> (CommandRegistry, BufferStore, TempDir)
    {
        let reg = CommandRegistry::with_defaults();
        let store = BufferStore::new(crate::editor::clock::Clock::default());
        let dir = tempfile::tempdir().unwrap();
        (reg, store, dir)
    }

    pub(in crate::editor::completion) fn ctx<'a>(
        registry: &'a CommandRegistry,
        buffers: &'a BufferStore,
        cwd: &'a AbsolutePath,
    ) -> CompletionCtx<'a> {
        ctx_with(registry, buffers, cwd, empty_langs())
    }

    pub(in crate::editor::completion) fn ctx_with<'a>(
        registry: &'a CommandRegistry,
        buffers: &'a BufferStore,
        cwd: &'a AbsolutePath,
        languages: &'a LanguageRegistry,
    ) -> CompletionCtx<'a> {
        static NO_DIRS: hume_platform::dirs::Dirs = hume_platform::dirs::Dirs::none();
        CompletionCtx {
            registry,
            buffers,
            cwd,
            languages,
            dirs: &NO_DIRS,
        }
    }

    /// Shared empty registry for tests that don't register languages. It avoids
    /// re-allocating one per `ctx()` call and sidesteps the borrow-lifetime
    /// issue of constructing it inline.
    pub(in crate::editor::completion) fn empty_langs() -> &'static LanguageRegistry {
        use std::sync::OnceLock;
        static EMPTY: OnceLock<LanguageRegistry> = OnceLock::new();
        EMPTY.get_or_init(LanguageRegistry::for_testing)
    }

    pub(in crate::editor::completion) fn ev() -> EngineView {
        EngineView::new(Theme::default(), hume_grid::Rect::default()).0
    }

    pub(in crate::editor::completion) fn make_id(ev: &mut EngineView) -> BufferId {
        ev.buffers.insert(())
    }

    pub(in crate::editor::completion) fn make_buf() -> NewBuffer {
        NewBuffer::Scratch(BufferText::from("a\n"))
    }

    pub(in crate::editor::completion) fn buf_with_path(path: &str) -> NewBuffer {
        NewBuffer::File {
            text: BufferText::from("a\n"),
            file: crate::editor::buffer::BufferFile::at(path),
            pending_dump: None,
        }
    }
}
