//! Minibuffer tab-completion — completers, types, and dispatch helpers.
//!
//! Design contract:
//! - `Completer::complete` is a pure function: given `(input, cursor, ctx)` it
//!   returns a sorted `Vec<Completion>` and the byte offset in `input` at which
//!   the completed token starts (`span_start`).  No &mut access, no I/O side
//!   effects visible to the caller.
//! - `CompletionState` on `Editor` is the SSOT.  It is cleared whenever the
//!   minibuffer closes or the user edits the input by any key other than Tab /
//!   Shift-Tab.

use std::path::{Path, PathBuf};

use crate::editor::buffer_store::BufferStore;
use crate::editor::registry::CommandRegistry;

// ── Public types ──────────────────────────────────────────────────────────────

/// A single completion candidate.
///
/// `display` is shown in the popup (may include decorators like trailing `/`
/// for directories). `replacement` is the text written into the minibuffer.
/// The two fields are often identical; they differ for e.g. buffer names where
/// the display is the basename but the replacement is the full path.
#[derive(Debug, Clone)]
pub(crate) struct Completion {
    /// Text to insert at the span location in the minibuffer input.
    pub replacement: String,
    /// Text shown in the completion popup row.
    #[allow(dead_code)] // used by the popup renderer (Phase C)
    pub display: String,
}

/// Completion session state, stored on `Editor` while a popup is open.
///
/// Invariant: `selected < candidates.len()`. Created only when there are ≥2
/// candidates (single-candidate completion is applied silently without state).
pub(crate) struct CompletionState {
    pub candidates: Vec<Completion>,
    /// Index of the currently-displayed candidate.
    pub selected: usize,
    /// Byte offset in the minibuffer input where the completed token starts.
    /// Constant across the session (the span start never shifts while cycling).
    pub span_start: usize,
    /// Current byte end of the replacement in the minibuffer input.
    /// Equals `span_start + candidates[selected].replacement.len()` after
    /// every apply.  Used to know what to replace on the next Tab.
    pub current_end: usize,
}

impl CompletionState {
    /// The byte range that the current replacement occupies in the input.
    pub(crate) fn current_span(&self) -> std::ops::Range<usize> {
        self.span_start..self.current_end
    }
}

/// Context supplied to every `Completer::complete` call.
///
/// Bundles read-only references to the editor state that completers need
/// (command registry, buffer list, working directory) without exposing a full
/// `&Editor`.  This makes unit-testing completers straightforward — no Editor
/// construction required.
pub(crate) struct CompletionCtx<'a> {
    pub registry: &'a CommandRegistry,
    /// Used by `BufferNameCompleter`; wired into command dispatch when `:b` is added (M7).
    #[allow(dead_code)]
    pub buffers:  &'a BufferStore,
    pub cwd:      &'a Path,
}

/// Result of a single `Completer::complete` call.
///
/// `span_start` is the byte offset in `input` where the completed token
/// begins.  All candidates are replacements for `input[span_start..cursor]`.
pub(crate) struct CompletionResult {
    pub span_start: usize,
    pub candidates:  Vec<Completion>,
}

/// A completion source for a specific context (command name, path, buffer name).
pub(crate) trait Completer {
    /// Return sorted candidates for the token at `cursor` in `input`.
    ///
    /// Returns `span_start` (the byte offset where the completed token begins)
    /// alongside the candidates.  Returns an empty `Vec` when there are no
    /// matches.
    fn complete(&self, input: &str, cursor: usize, ctx: &CompletionCtx<'_>) -> CompletionResult;
}

// ── CommandCompleter ──────────────────────────────────────────────────────────

/// Completes command names + aliases from the registry.
///
/// The completed token is the command name prefix `input[0..cursor]`.
/// Both canonical names and aliases are offered as candidates so the user
/// can discover either form.
pub(crate) struct CommandCompleter;

impl Completer for CommandCompleter {
    fn complete(&self, input: &str, cursor: usize, ctx: &CompletionCtx<'_>) -> CompletionResult {
        let prefix = &input[..cursor.min(input.len())];
        let mut candidates: Vec<Completion> = ctx.registry
            .iter_names_and_aliases()
            .filter(|name| name.starts_with(prefix) && *name != prefix)
            .map(|name| Completion { replacement: name.to_owned(), display: name.to_owned() })
            .collect();
        // Exact prefix match goes first only if the prefix itself is a valid command;
        // otherwise sort everything alphabetically for predictability.
        candidates.sort_unstable_by(|a, b| a.display.cmp(&b.display));
        candidates.dedup_by(|a, b| a.replacement == b.replacement);
        CompletionResult { span_start: 0, candidates }
    }
}

// ── BufferNameCompleter ───────────────────────────────────────────────────────

/// Completes open buffer names.
///
/// Matches on the file basename (or `*scratch*` for unnamed buffers).
/// The `replacement` is the full canonical path so the command receives an
/// unambiguous target.
///
/// Wired into command dispatch when `:b` is added (M7 next item).
#[allow(dead_code)]
pub(crate) struct BufferNameCompleter;

impl Completer for BufferNameCompleter {
    fn complete(&self, input: &str, cursor: usize, ctx: &CompletionCtx<'_>) -> CompletionResult {
        // arg_start: byte offset after the command + space
        let (arg_start, prefix) = arg_prefix(input, cursor);

        let mut candidates: Vec<Completion> = ctx.buffers
            .iter()
            .filter_map(|(_, buf)| {
                let (display, replacement) = if let Some(path) = &buf.path {
                    let name = path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    let repl = path.display().to_string();
                    (name, repl)
                } else {
                    let s = crate::editor::buffer::Buffer::SCRATCH_BUFFER_NAME.to_owned();
                    (s.clone(), s)
                };
                if display.starts_with(prefix) {
                    Some(Completion { display, replacement })
                } else {
                    None
                }
            })
            .collect();

        candidates.sort_unstable_by(|a, b| a.display.cmp(&b.display));
        CompletionResult { span_start: arg_start, candidates }
    }
}

// ── PathCompleter ─────────────────────────────────────────────────────────────

/// Completes filesystem paths for `:e` / `:w`.
///
/// Splits the arg into a directory prefix and a filename prefix.  Reads the
/// directory and filters by the filename prefix.  Directory entries get a
/// trailing `/` in both `display` and `replacement`.  Hidden files (leading
/// `.`) are excluded unless the filename prefix itself starts with `.`.
pub(crate) struct PathCompleter;

impl Completer for PathCompleter {
    fn complete(&self, input: &str, cursor: usize, ctx: &CompletionCtx<'_>) -> CompletionResult {
        let (arg_start, prefix) = arg_prefix(input, cursor);

        // Split prefix into (dir_str, file_prefix).
        let (dir_str, file_prefix) = match prefix.rfind('/') {
            Some(idx) => (&prefix[..=idx], &prefix[idx + 1..]),
            None      => ("", prefix),
        };

        // Resolve the directory: absolute if it starts with '/', else relative to cwd.
        let dir: PathBuf = if dir_str.is_empty() {
            ctx.cwd.to_owned()
        } else if Path::new(dir_str).is_absolute() {
            PathBuf::from(dir_str)
        } else {
            ctx.cwd.join(dir_str)
        };

        let include_hidden = file_prefix.starts_with('.');

        // `crate::os::fs::read_dir` wraps std::fs::read_dir.  On error (dir
        // doesn't exist or no permission), return no candidates — not a hard error.
        let rd = match crate::os::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => return CompletionResult { span_start: arg_start, candidates: vec![] },
        };

        let mut candidates: Vec<Completion> = rd
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.starts_with(file_prefix) { return None; }
                if !include_hidden && name.starts_with('.') { return None; }
                let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                let suffix = if is_dir { "/" } else { "" };
                let display = format!("{name}{suffix}");
                // Build the full replacement: dir_str + name + suffix.
                let replacement = format!("{dir_str}{name}{suffix}");
                Some(Completion { display, replacement })
            })
            .collect();

        candidates.sort_unstable_by(|a, b| a.display.cmp(&b.display));
        CompletionResult { span_start: arg_start, candidates }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use tempfile::TempDir;

    use crate::core::selection::SelectionSet;
    use crate::core::text::Text;
    use crate::editor::buffer::Buffer;
    use crate::editor::buffer_store::BufferStore;
    use crate::editor::registry::CommandRegistry;
    use engine::pipeline::{BufferId, EngineView, SharedBuffer};
    use engine::theme::Theme;

    use super::*;

    fn make_ctx_parts() -> (CommandRegistry, BufferStore, TempDir) {
        let reg = CommandRegistry::with_defaults();
        let store = BufferStore::new();
        let dir = tempfile::tempdir().unwrap();
        (reg, store, dir)
    }

    fn ctx<'a>(
        registry: &'a CommandRegistry,
        buffers:  &'a BufferStore,
        cwd:      &'a Path,
    ) -> CompletionCtx<'a> {
        CompletionCtx { registry, buffers, cwd }
    }

    fn ev() -> EngineView { EngineView::new(Theme::default()) }

    fn make_id(ev: &mut EngineView) -> BufferId {
        ev.buffers.insert(SharedBuffer::new())
    }

    fn make_buf() -> Buffer {
        Buffer::new(Text::from("a\n"), SelectionSet::default())
    }

    fn buf_with_path(path: &str) -> Buffer {
        let mut b = make_buf();
        b.path = Some(Arc::new(PathBuf::from(path)));
        b
    }

    // ── CommandCompleter ──────────────────────────────────────────────────────

    #[test]
    fn command_completer_empty_prefix_returns_all() {
        let (reg, store, dir) = make_ctx_parts();
        let ctx = ctx(&reg, &store, dir.path());
        let result = CommandCompleter.complete("", 0, &ctx);
        // All registered names (canonicals + aliases) minus empty prefix match all.
        assert!(!result.candidates.is_empty());
        assert_eq!(result.span_start, 0);
    }

    #[test]
    fn command_completer_prefix_filters() {
        let (reg, store, dir) = make_ctx_parts();
        let ctx = ctx(&reg, &store, dir.path());
        let result = CommandCompleter.complete("q", 1, &ctx);
        assert!(result.candidates.iter().all(|c| c.replacement.starts_with('q')));
        assert!(result.candidates.iter().any(|c| c.replacement == "quit"));
    }

    #[test]
    fn command_completer_no_match_returns_empty() {
        let (reg, store, dir) = make_ctx_parts();
        let ctx = ctx(&reg, &store, dir.path());
        let result = CommandCompleter.complete("zzz", 3, &ctx);
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn command_completer_sorted_ascending() {
        let (reg, store, dir) = make_ctx_parts();
        let ctx = ctx(&reg, &store, dir.path());
        let result = CommandCompleter.complete("w", 1, &ctx);
        let names: Vec<&str> = result.candidates.iter().map(|c| c.display.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }

    #[test]
    fn command_completer_alias_and_canonical_both_appear() {
        let (reg, store, dir) = make_ctx_parts();
        let ctx = ctx(&reg, &store, dir.path());
        // Typing "wr" matches "write" (canonical) and "write-quit" (canonical);
        // "w" (alias) is excluded because it doesn't start with "wr".
        // This verifies that both alias forms and canonical forms of other commands
        // starting with the same prefix are surfaced.
        let result = CommandCompleter.complete("wr", 2, &ctx);
        let names: Vec<&str> = result.candidates.iter().map(|c| c.replacement.as_str()).collect();
        assert!(names.contains(&"write"), "canonical 'write' should appear");
        assert!(names.contains(&"write-quit"), "canonical 'write-quit' should appear");
        // Verify aliases also surface: "wq" is an alias, starts with "w" not "wr".
        let result2 = CommandCompleter.complete("w", 1, &ctx);
        let names2: Vec<&str> = result2.candidates.iter().map(|c| c.replacement.as_str()).collect();
        assert!(names2.contains(&"write"), "canonical 'write' should appear with prefix 'w'");
        assert!(names2.contains(&"wq"), "'wq' alias should appear with prefix 'w'");
    }

    #[test]
    fn command_completer_exact_prefix_not_included() {
        // Typing the exact name should not complete to itself.
        let (reg, store, dir) = make_ctx_parts();
        let ctx = ctx(&reg, &store, dir.path());
        let result = CommandCompleter.complete("quit", 4, &ctx);
        assert!(!result.candidates.iter().any(|c| c.replacement == "quit"));
    }

    // ── BufferNameCompleter ───────────────────────────────────────────────────

    #[test]
    fn buffer_name_completer_matches_basename() {
        let mut ev = ev();
        let (reg, mut store, dir) = make_ctx_parts();
        let id = make_id(&mut ev);
        store.open(id, buf_with_path("/tmp/foo.txt"));
        let ctx = ctx(&reg, &store, dir.path());
        let result = BufferNameCompleter.complete("bd f", 4, &ctx);
        assert_eq!(result.span_start, 3);
        assert!(result.candidates.iter().any(|c| c.replacement == "/tmp/foo.txt"));
    }

    #[test]
    fn buffer_name_completer_scratch_buffer() {
        let mut ev = ev();
        let (reg, mut store, dir) = make_ctx_parts();
        let id = make_id(&mut ev);
        store.open(id, make_buf()); // no path → scratch
        let ctx = ctx(&reg, &store, dir.path());
        let result = BufferNameCompleter.complete("bd *", 4, &ctx);
        assert_eq!(result.span_start, 3);
        assert!(result.candidates.iter().any(|c| c.replacement == "*scratch*"));
    }

    #[test]
    fn buffer_name_completer_no_match() {
        let mut ev = ev();
        let (reg, mut store, dir) = make_ctx_parts();
        let id = make_id(&mut ev);
        store.open(id, buf_with_path("/tmp/foo.txt"));
        let ctx = ctx(&reg, &store, dir.path());
        let result = BufferNameCompleter.complete("bd z", 4, &ctx);
        assert!(result.candidates.is_empty());
    }

    // ── PathCompleter ─────────────────────────────────────────────────────────

    #[test]
    fn path_completer_lists_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alpha.txt"), b"").unwrap();
        std::fs::write(dir.path().join("beta.txt"),  b"").unwrap();
        std::fs::create_dir(dir.path().join("gamma")).unwrap();

        let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
        let ctx = ctx(&reg, &store, dir.path());
        let input = "e ";
        let result = PathCompleter.complete(input, input.len(), &ctx);

        let names: Vec<&str> = result.candidates.iter().map(|c| c.display.as_str()).collect();
        assert!(names.contains(&"alpha.txt"), "alpha.txt should appear");
        assert!(names.contains(&"beta.txt"),  "beta.txt should appear");
        assert!(names.contains(&"gamma/"),    "directory gets trailing /");
        assert_eq!(result.span_start, 2);
    }

    #[test]
    fn path_completer_filters_by_prefix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("foo.txt"), b"").unwrap();
        std::fs::write(dir.path().join("bar.txt"), b"").unwrap();

        let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
        let ctx = ctx(&reg, &store, dir.path());
        let input = "e foo";
        let result = PathCompleter.complete(input, input.len(), &ctx);

        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].replacement, "foo.txt");
    }

    #[test]
    fn path_completer_excludes_hidden_unless_dot_prefix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".hidden"), b"").unwrap();
        std::fs::write(dir.path().join("visible"), b"").unwrap();

        let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
        let ctx = ctx(&reg, &store, dir.path());

        // Without dot prefix: hidden excluded.
        let result = PathCompleter.complete("e ", 2, &ctx);
        assert!(!result.candidates.iter().any(|c| c.display.starts_with('.')));
        assert!(result.candidates.iter().any(|c| c.display == "visible"));

        // With dot prefix: hidden included.
        let input = "e .";
        let result = PathCompleter.complete(input, input.len(), &ctx);
        assert!(result.candidates.iter().any(|c| c.display == ".hidden"));
    }

    #[test]
    fn path_completer_multi_segment() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("file.rs"), b"").unwrap();

        let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
        let ctx = ctx(&reg, &store, dir.path());

        // Completing "sub/f" — should find "sub/file.rs".
        let input = "e sub/f";
        let result = PathCompleter.complete(input, input.len(), &ctx);
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].replacement, "sub/file.rs");
    }

    #[test]
    fn path_completer_missing_dir_returns_empty() {
        let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
        let cwd = Path::new("/nonexistent/path/that/does/not/exist");
        let ctx = CompletionCtx { registry: &reg, buffers: &store, cwd };
        let result = PathCompleter.complete("e foo", 5, &ctx);
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn path_completer_sorted_ascending() {
        let dir = tempfile::tempdir().unwrap();
        for name in &["zz.txt", "aa.txt", "mm.txt"] {
            std::fs::write(dir.path().join(name), b"").unwrap();
        }
        let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
        let ctx = ctx(&reg, &store, dir.path());
        let result = PathCompleter.complete("e ", 2, &ctx);
        let names: Vec<&str> = result.candidates.iter().map(|c| c.display.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "results must be sorted alphabetically");
    }
}
