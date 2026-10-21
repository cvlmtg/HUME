use std::borrow::Cow;
use std::ffi::OsString;
use std::ops::Range;

use hume_platform::sh;

use super::path::path_candidates;
use super::{CompletionCtx, CompletionItem};

// ── Shell command line ────────────────────────────────────────────────────────

/// The name this source registers under for the `|` line.
pub(in crate::editor) const SHELL_SOURCE: &str = "shell";

/// The operator characters, which end a word as a blank does.
const OPERATORS: &[char] = &['|', ';', '&', '(', ')', '<', '>'];

/// The operators after which the next word names a program.
const COMMAND_STARTERS: &[char] = &['|', ';', '&', '('];

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
    let word = shell_word_at(input, cursor);
    let typed = &input[word.span.start..cursor.min(input.len())];
    let items = if word.command_position && !typed.contains(sh::PATH_SEP) {
        search_path().map_or_else(Vec::new, |path| {
            hume_platform::path::executables_in(&path, &sh::prefix_value(typed))
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
            |dir| Cow::Borrowed(dir),
            |name| format!("{typed_dir}{}", sh::escape(name)),
        )
    };
    (word.span, items)
}

/// The word of a shell line a cursor sits in.
struct ShellWord {
    /// From the word's start to its end, past the cursor.
    span: Range<usize>,
    /// The word stands where `sh` reads a program name: first on the line
    /// or right after one of [`COMMAND_STARTERS`].
    command_position: bool,
}

/// The word around `cursor`: words end at unescaped [`sh::is_blank`]
/// characters and at [`OPERATORS`], and a backslash keeps the character
/// after it in the word, even with the cursor between the two. Quotes are
/// not read, so a quoted space still ends a word.
fn shell_word_at(input: &str, cursor: usize) -> ShellWord {
    let mut start = 0;
    let mut end = input.len();
    // The last non-blank character before `start`.
    let mut before_word = None;
    let mut last = None;
    let mut chars = input.char_indices();
    while let Some((i, c)) = chars.next() {
        if c == '\\' {
            chars.next();
            last = Some(c);
            continue;
        }
        let operator = OPERATORS.contains(&c);
        if !operator && !sh::is_blank(c) {
            last = Some(c);
            continue;
        }
        if i >= cursor {
            end = i;
            break;
        }
        start = i + c.len_utf8();
        if operator {
            last = Some(c);
        }
        before_word = last;
    }
    ShellWord {
        span: start..end,
        command_position: before_word.is_none_or(|c| COMMAND_STARTERS.contains(&c)),
    }
}

#[cfg(test)]
mod tests;
