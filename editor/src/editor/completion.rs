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
}

impl CompletionState {
    /// The byte range that the current replacement occupies in the input.
    pub(crate) fn current_span(&self) -> std::ops::Range<usize> {
        debug_assert!(
            self.selected < self.candidates.len(),
            "CompletionState invariant violated: selected {} >= len {}",
            self.selected,
            self.candidates.len(),
        );
        let end = self.span_start + self.candidates[self.selected].replacement.len();
        self.span_start..end
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
    pub buffers: &'a BufferStore,
    pub cwd: &'a Path,
}

/// Result of a single `Completer::complete` call.
///
/// `span_start` is the byte offset in `input` where the completed token
/// begins.  All candidates are replacements for `input[span_start..cursor]`.
pub(crate) struct CompletionResult {
    pub span_start: usize,
    pub candidates: Vec<Completion>,
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
        let mut candidates: Vec<Completion> = ctx
            .registry
            .iter_names_and_aliases()
            .filter(|name| name.starts_with(prefix) && *name != prefix)
            .map(|name| Completion {
                replacement: name.to_owned(),
                display: name.to_owned(),
            })
            .collect();
        candidates.sort_unstable_by(|a, b| a.display.cmp(&b.display));
        candidates.dedup_by(|a, b| a.replacement == b.replacement);
        CompletionResult {
            span_start: 0,
            candidates,
        }
    }
}

// ── BufferNameCompleter ───────────────────────────────────────────────────────

/// Completes open buffer names for `:b`.
///
/// Matches on the file basename (or `*scratch*` for unnamed buffers).
/// The `replacement` is the full canonical path so the command receives an
/// unambiguous target.
///
/// When two open buffers share the same basename, a shortened parent-directory
/// suffix is appended to `display` (e.g. `foo.rs  (~/a/)`) so the user can
/// distinguish them in the popup without accepting the wrong one.
pub(crate) struct BufferNameCompleter;

impl Completer for BufferNameCompleter {
    fn complete(&self, input: &str, cursor: usize, ctx: &CompletionCtx<'_>) -> CompletionResult {
        let (arg_start, prefix) = arg_prefix(input, cursor);

        // (display-basename, full-path replacement for the command).
        let entry_for = |buf: &crate::editor::buffer::Buffer| -> (String, String) {
            let base = buf.display_name();
            let replacement = buf
                .path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| base.clone());
            (base, replacement)
        };

        // Count how many open buffers share each basename (for disambiguation).
        let mut name_count: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for (_, buf) in ctx.buffers.iter() {
            let (base, _) = entry_for(buf);
            *name_count.entry(base).or_insert(0) += 1;
        }

        let mut candidates: Vec<Completion> = ctx
            .buffers
            .iter()
            .filter_map(|(_, buf)| {
                let (base, replacement) = entry_for(buf);
                if !base.starts_with(prefix) {
                    return None;
                }
                let display = if *name_count.get(&base).expect("base was counted above") >= 2 {
                    // Two or more buffers share this basename — show parent dir.
                    if let Some(parent) = buf.path().and_then(|p| p.parent()) {
                        format!("{base}  ({}/)", crate::os::path::shorten_home(parent))
                    } else {
                        base // scratch can't collide
                    }
                } else {
                    base
                };
                Some(Completion {
                    display,
                    replacement,
                })
            })
            .collect();

        candidates.sort_unstable_by(|a, b| a.display.cmp(&b.display));
        CompletionResult {
            span_start: arg_start,
            candidates,
        }
    }
}

// ── PathCompleter ─────────────────────────────────────────────────────────────

/// Completes filesystem paths for `:e` / `:w` / `:cd`.
///
/// Splits the arg into a directory prefix and a filename prefix.  Reads the
/// directory and filters by the filename prefix.  Directory entries get a
/// trailing `/` in both `display` and `replacement`.  Hidden files (leading
/// `.`) are excluded unless the filename prefix itself starts with `.`.
///
/// When `dirs_only` is `true` (used by `:cd`), non-directory entries are
/// filtered out.
pub(crate) struct PathCompleter {
    pub(crate) dirs_only: bool,
}

impl PathCompleter {
    /// Testable core of [`Completer::complete`].
    ///
    /// `expand_fn` mirrors `crate::os::path::expand`: given a raw path string it
    /// returns the tilde / env-var expanded form.  Tests pass a stub closure;
    /// production calls this with the real `expand`.
    fn complete_with_expand<F>(
        &self,
        input: &str,
        cursor: usize,
        ctx: &CompletionCtx<'_>,
        expand_fn: F,
    ) -> CompletionResult
    where
        F: for<'a> Fn(&'a str) -> std::borrow::Cow<'a, str>,
    {
        let (arg_start, prefix) = arg_prefix(input, cursor);

        // Split prefix into (dir_str, file_prefix).
        let (dir_str, file_prefix) = crate::os::path::split_path_at_sep(prefix);

        // Expand `~` and env vars for the directory lookup only; the literal
        // `dir_str` is still used in `replacement` below so `~/` is preserved
        // in the minibuffer exactly as the user typed it.
        let expanded_dir = expand_fn(dir_str);

        // Resolve the directory: absolute if it starts with '/', else relative to cwd.
        let dir: PathBuf = if expanded_dir.is_empty() {
            ctx.cwd.to_owned()
        } else if Path::new(expanded_dir.as_ref()).is_absolute() {
            PathBuf::from(expanded_dir.as_ref())
        } else {
            ctx.cwd.join(expanded_dir.as_ref())
        };

        let include_hidden = file_prefix.starts_with('.');

        // `crate::os::fs::read_dir` wraps std::fs::read_dir.  On error (dir
        // doesn't exist or no permission), return no candidates — not a hard error.
        let rd = match crate::os::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => {
                return CompletionResult {
                    span_start: arg_start,
                    candidates: vec![],
                };
            }
        };

        let mut candidates: Vec<Completion> = rd
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.starts_with(file_prefix) {
                    return None;
                }
                if !include_hidden && name.starts_with('.') {
                    return None;
                }
                let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                if self.dirs_only && !is_dir {
                    return None;
                }
                let suffix = if is_dir { "/" } else { "" };
                let display = format!("{name}{suffix}");
                // Build the full replacement: dir_str + name + suffix.
                let replacement = format!("{dir_str}{name}{suffix}");
                Some(Completion {
                    display,
                    replacement,
                })
            })
            .collect();

        candidates.sort_unstable_by(|a, b| a.display.cmp(&b.display));
        CompletionResult {
            span_start: arg_start,
            candidates,
        }
    }
}

impl Completer for PathCompleter {
    fn complete(&self, input: &str, cursor: usize, ctx: &CompletionCtx<'_>) -> CompletionResult {
        self.complete_with_expand(input, cursor, ctx, crate::os::path::expand)
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
        buffers: &'a BufferStore,
        cwd: &'a Path,
    ) -> CompletionCtx<'a> {
        CompletionCtx {
            registry,
            buffers,
            cwd,
        }
    }

    fn ev() -> EngineView {
        EngineView::new(Theme::default())
    }

    fn make_id(ev: &mut EngineView) -> BufferId {
        ev.buffers.insert(SharedBuffer::new())
    }

    fn make_buf() -> Buffer {
        Buffer::new(Text::from("a\n"), SelectionSet::default())
    }

    fn buf_with_path(path: &str) -> Buffer {
        let mut b = make_buf();
        b.set_path(Some(PathBuf::from(path)));
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
        assert!(
            result
                .candidates
                .iter()
                .all(|c| c.replacement.starts_with('q'))
        );
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
        let names: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.display.as_str())
            .collect();
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
        let names: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.replacement.as_str())
            .collect();
        assert!(names.contains(&"write"), "canonical 'write' should appear");
        assert!(
            names.contains(&"write-quit"),
            "canonical 'write-quit' should appear"
        );
        // Verify aliases also surface: "wq" is an alias, starts with "w" not "wr".
        let result2 = CommandCompleter.complete("w", 1, &ctx);
        let names2: Vec<&str> = result2
            .candidates
            .iter()
            .map(|c| c.replacement.as_str())
            .collect();
        assert!(
            names2.contains(&"write"),
            "canonical 'write' should appear with prefix 'w'"
        );
        assert!(
            names2.contains(&"wq"),
            "'wq' alias should appear with prefix 'w'"
        );
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
        assert!(
            result
                .candidates
                .iter()
                .any(|c| c.replacement == "/tmp/foo.txt")
        );
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
        assert!(
            result
                .candidates
                .iter()
                .any(|c| c.replacement == "*scratch*")
        );
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

    #[test]
    fn buffer_name_completer_duplicate_basename_adds_parent_suffix() {
        let mut ev = ev();
        let (reg, mut store, dir) = make_ctx_parts();
        let id1 = make_id(&mut ev);
        let id2 = make_id(&mut ev);
        let id3 = make_id(&mut ev);
        store.open(id1, buf_with_path("/a/foo.txt"));
        store.open(id2, buf_with_path("/b/foo.txt"));
        store.open(id3, buf_with_path("/tmp/bar.txt"));
        let ctx = ctx(&reg, &store, dir.path());

        let result = BufferNameCompleter.complete("b ", 2, &ctx);
        // All three buffers should appear (prefix "" matches all).
        assert_eq!(result.candidates.len(), 3);

        // The two foo.txt entries must have parent-dir suffixes in their display.
        let foo_entries: Vec<&str> = result
            .candidates
            .iter()
            .filter(|c| c.display.contains("foo.txt"))
            .map(|c| c.display.as_str())
            .collect();
        assert_eq!(foo_entries.len(), 2, "both foo.txt entries must appear");
        assert!(
            foo_entries.iter().all(|d| d.contains('(')),
            "duplicate basenames must include a parent-dir suffix: {foo_entries:?}"
        );

        // The unique bar.txt entry must NOT have a suffix.
        let bar_entry = result
            .candidates
            .iter()
            .find(|c| c.display.contains("bar.txt"))
            .expect("bar.txt must appear");
        assert!(
            !bar_entry.display.contains('('),
            "unique basename must not have a suffix: {}",
            bar_entry.display
        );

        // Replacements are always the full paths.
        assert!(
            result
                .candidates
                .iter()
                .any(|c| c.replacement == "/a/foo.txt")
        );
        assert!(
            result
                .candidates
                .iter()
                .any(|c| c.replacement == "/b/foo.txt")
        );
        assert!(
            result
                .candidates
                .iter()
                .any(|c| c.replacement == "/tmp/bar.txt")
        );
    }

    // ── PathCompleter ─────────────────────────────────────────────────────────

    #[test]
    fn path_completer_lists_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alpha.txt"), b"").unwrap();
        std::fs::write(dir.path().join("beta.txt"), b"").unwrap();
        std::fs::create_dir(dir.path().join("gamma")).unwrap();

        let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
        let ctx = ctx(&reg, &store, dir.path());
        let input = "e ";
        let result = PathCompleter { dirs_only: false }.complete(input, input.len(), &ctx);

        let names: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.display.as_str())
            .collect();
        assert!(names.contains(&"alpha.txt"), "alpha.txt should appear");
        assert!(names.contains(&"beta.txt"), "beta.txt should appear");
        assert!(names.contains(&"gamma/"), "directory gets trailing /");
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
        let result = PathCompleter { dirs_only: false }.complete(input, input.len(), &ctx);

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
        let result = PathCompleter { dirs_only: false }.complete("e ", 2, &ctx);
        assert!(!result.candidates.iter().any(|c| c.display.starts_with('.')));
        assert!(result.candidates.iter().any(|c| c.display == "visible"));

        // With dot prefix: hidden included.
        let input = "e .";
        let result = PathCompleter { dirs_only: false }.complete(input, input.len(), &ctx);
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
        let result = PathCompleter { dirs_only: false }.complete(input, input.len(), &ctx);
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].replacement, "sub/file.rs");
    }

    #[test]
    fn path_completer_missing_dir_returns_empty() {
        let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
        let cwd = Path::new("/nonexistent/path/that/does/not/exist");
        let ctx = CompletionCtx {
            registry: &reg,
            buffers: &store,
            cwd,
        };
        let result = PathCompleter { dirs_only: false }.complete("e foo", 5, &ctx);
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
        let result = PathCompleter { dirs_only: false }.complete("e ", 2, &ctx);
        let names: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.display.as_str())
            .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "results must be sorted alphabetically");
    }

    #[test]
    #[cfg(not(windows))]
    fn path_completer_tilde_expands_for_lookup_keeps_literal_replacement() {
        use std::borrow::Cow;

        let home_dir = tempfile::tempdir().unwrap();
        std::fs::write(home_dir.path().join("notes.md"), b"").unwrap();
        std::fs::create_dir(home_dir.path().join("code")).unwrap();

        let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
        let cwd = Path::new("/tmp");
        let ctx = CompletionCtx {
            registry: &reg,
            buffers: &store,
            cwd,
        };

        let home = home_dir.path().to_path_buf();
        let input = "e ~/";
        let result = PathCompleter { dirs_only: false }.complete_with_expand(
            input,
            input.len(),
            &ctx,
            |s: &str| {
                if let Some(tail) = s.strip_prefix('~')
                    && (tail.is_empty() || tail.starts_with('/'))
                {
                    return Cow::Owned(format!("{}{tail}", home.display()));
                }
                Cow::Borrowed(s)
            },
        );

        // Candidates must be present (the temp home has files).
        assert!(
            !result.candidates.is_empty(),
            "tilde should resolve to home and list entries"
        );
        // Replacements must keep the literal `~/` prefix, not expand to the absolute path.
        assert!(
            result
                .candidates
                .iter()
                .all(|c| c.replacement.starts_with("~/")),
            "replacements must preserve the `~/` prefix"
        );
        let names: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.display.as_str())
            .collect();
        assert!(names.contains(&"notes.md"), "notes.md should appear");
        assert!(
            names.contains(&"code/"),
            "code/ directory should appear with trailing /"
        );
    }

    #[test]
    #[cfg(not(windows))]
    fn path_completer_dollar_var_expands_for_lookup() {
        use std::borrow::Cow;

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.rs"), b"").unwrap();

        let (reg, store) = (CommandRegistry::with_defaults(), BufferStore::new());
        let cwd = Path::new("/tmp");
        let ctx = CompletionCtx {
            registry: &reg,
            buffers: &store,
            cwd,
        };

        let expanded = dir.path().to_string_lossy().into_owned();
        let input = "e $MYDIR/";
        let result = PathCompleter { dirs_only: false }.complete_with_expand(
            input,
            input.len(),
            &ctx,
            |s: &str| {
                if let Some(rest) = s.strip_prefix("$MYDIR") {
                    Cow::Owned(format!("{expanded}{rest}"))
                } else {
                    Cow::Borrowed(s)
                }
            },
        );

        assert!(
            !result.candidates.is_empty(),
            "$MYDIR should expand and list entries"
        );
        assert!(
            result
                .candidates
                .iter()
                .all(|c| c.replacement.starts_with("$MYDIR/"))
        );
        let names: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.display.as_str())
            .collect();
        assert!(names.contains(&"main.rs"));
    }
}
