use std::path::{Path, PathBuf};

use super::{Completer, Completion, CompletionCtx, CompletionResult, arg_prefix};

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
    /// `expand_fn` mirrors `hume_platform::path::expand`: given a raw path string it
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
        let (dir_str, file_prefix) = hume_platform::path::split_path_at_sep(prefix);

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

        // `hume_platform::fs::read_dir` wraps std::fs::read_dir.  On error (dir
        // doesn't exist or no permission), return no candidates — not a hard error.
        let rd = match hume_platform::fs::read_dir(&dir) {
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
        self.complete_with_expand(input, cursor, ctx, hume_platform::path::expand)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    use crate::editor::buffer::store::BufferStore;
    use crate::editor::registry::CommandRegistry;
    use hume_treesitter::registry::LanguageRegistry;

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
        let langs = LanguageRegistry::new();
        let ctx = CompletionCtx {
            registry: &reg,
            buffers: &store,
            cwd,
            languages: &langs,
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
        let langs = LanguageRegistry::new();
        let ctx = CompletionCtx {
            registry: &reg,
            buffers: &store,
            cwd,
            languages: &langs,
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
        let langs = LanguageRegistry::new();
        let ctx = CompletionCtx {
            registry: &reg,
            buffers: &store,
            cwd,
            languages: &langs,
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
