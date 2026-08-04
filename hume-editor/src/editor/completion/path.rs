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

        // On error (dir doesn't exist or no permission), return no
        // candidates — not a hard error.
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => return CompletionResult::sorted(arg_start, vec![]),
        };

        let candidates: Vec<Completion> = rd
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

        CompletionResult::sorted(arg_start, candidates)
    }
}

impl Completer for PathCompleter {
    fn complete(&self, input: &str, cursor: usize, ctx: &CompletionCtx<'_>) -> CompletionResult {
        self.complete_with_expand(input, cursor, ctx, hume_platform::path::expand)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
