use std::borrow::Cow;
use std::ffi::OsString;
use std::ops::Range;

use super::path::path_candidates;
use super::{CompletionCtx, CompletionItem};

// ── Shell command line ────────────────────────────────────────────────────────

/// The name this source registers under for the `|` line.
pub(in crate::editor) const SHELL_SOURCE: &str = "shell";

/// The operator characters, which end a word as whitespace does.
const OPERATORS: &[char] = &['|', ';', '&', '(', ')', '<', '>'];

/// The operators after which the next word names a program.
const COMMAND_STARTERS: &[char] = &['|', ';', '&', '('];

/// The characters POSIX `sh` reads specially inside a word.
const SH_SPECIAL: &[char] = &[
    ' ', '\t', '\n', '\\', '\'', '"', '`', '$', '&', ';', '|', '(', ')', '<', '>', '*', '?', '[',
    ']', '{', '}', '!', '#', '~',
];

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
    let items = if word.command_position && !typed.contains(hume_platform::path::is_path_sep) {
        search_path().map_or_else(Vec::new, |path| {
            hume_platform::path::executables_in(&path, &sh_unescape(typed))
                .into_iter()
                .map(|name| {
                    let insert = sh_escape(&name).into_owned();
                    CompletionItem::plain(name, insert)
                })
                .collect()
        })
    } else {
        let (typed_dir, typed_file) = hume_platform::path::split_path_at_sep(typed);
        path_candidates(
            &sh_unescape(typed_dir),
            &sh_unescape(typed_file),
            ctx.cwd,
            false,
            hume_platform::path::expand,
            |name| format!("{typed_dir}{}", sh_escape(name)),
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

/// The word around `cursor`: words end at unescaped whitespace and at
/// [`OPERATORS`], and a backslash keeps the character after it in the
/// word. Quotes are not read, so a quoted space still ends a word.
fn shell_word_at(input: &str, cursor: usize) -> ShellWord {
    let cursor = cursor.min(input.len());
    let mut start = 0;
    // The last non-whitespace character before `start`.
    let mut before_word = None;
    let mut last = None;
    let mut head = input[..cursor].char_indices();
    while let Some((i, c)) = head.next() {
        if c == '\\' {
            head.next();
            last = Some(c);
        } else if c.is_whitespace() {
            start = i + c.len_utf8();
            before_word = last;
        } else if OPERATORS.contains(&c) {
            start = i + c.len_utf8();
            last = Some(c);
            before_word = last;
        } else {
            last = Some(c);
        }
    }
    let mut end = input.len();
    let mut tail = input[cursor..].char_indices();
    while let Some((i, c)) = tail.next() {
        if c == '\\' {
            tail.next();
        } else if c.is_whitespace() || OPERATORS.contains(&c) {
            end = cursor + i;
            break;
        }
    }
    ShellWord {
        span: start..end,
        command_position: before_word.is_none_or(|c| COMMAND_STARTERS.contains(&c)),
    }
}

/// `name` with a backslash before each of [`SH_SPECIAL`].
fn sh_escape(name: &str) -> Cow<'_, str> {
    if !name.contains(SH_SPECIAL) {
        return Cow::Borrowed(name);
    }
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if SH_SPECIAL.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
    Cow::Owned(out)
}

/// `word` with each backslash pair read as the character it escapes; a
/// trailing lone backslash escapes nothing and is dropped.
fn sh_unescape(word: &str) -> Cow<'_, str> {
    if !word.contains('\\') {
        return Cow::Borrowed(word);
    }
    let mut out = String::with_capacity(word.len());
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend(chars.next()),
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests;
