use std::ffi::OsString;
use std::ops::Range;

use hume_platform::sh;

use super::path::path_candidates;
use super::{CompletionCtx, CompletionItem};

// ── Shell command line ────────────────────────────────────────────────────────

/// The name this source registers under for the `|` line.
pub(in crate::editor) const SHELL_SOURCE: &str = "shell";

/// Completes the `|` line's word at the cursor: a program on `PATH` where
/// the word names one, a file path anywhere else. A word that names a
/// program but holds a path separator (`./run`, `/bin/ba`) is a path.
/// Names are inserted backslash-escaped for POSIX `sh`.
pub(super) fn complete_shell(
    input: &str,
    cursor: usize,
    ctx: &CompletionCtx<'_>,
) -> (Range<usize>, Vec<CompletionItem>) {
    complete_shell_with(input, cursor, ctx, || std::env::var_os("PATH"))
}

/// Testable core of [`complete_shell`]. `search_path` is called only for a
/// word that names a program, so completing a path never reads `PATH`.
fn complete_shell_with(
    input: &str,
    cursor: usize,
    ctx: &CompletionCtx<'_>,
    search_path: impl FnOnce() -> Option<OsString>,
) -> (Range<usize>, Vec<CompletionItem>) {
    let word = sh::word_at(input, cursor);
    let typed = &input[word.span.start..cursor.min(input.len())];
    let items = if word.command_position && !typed.contains(sh::PATH_SEP) {
        search_path().map_or_else(Vec::new, |path| {
            hume_platform::path::executables_in(&path, &sh::prefix_value(typed), &[".exe"])
                .into_iter()
                .map(|name| {
                    let insert = sh::escape(&name).into_owned();
                    CompletionItem::plain(name, insert)
                })
                .collect()
        })
    } else {
        let (typed_dir, typed_file) = sh::split_at_sep(typed);
        path_candidates(
            &sh::prefix_value(typed_dir),
            &sh::prefix_value(typed_file),
            ctx.cwd,
            false,
            |name| format!("{typed_dir}{}", sh::escape(name)),
        )
    };
    (word.span, items)
}

#[cfg(test)]
mod tests;
