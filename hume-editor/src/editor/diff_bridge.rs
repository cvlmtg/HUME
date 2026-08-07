//! Translates `hume_editing::diff::LineHunk` into the Steel-facing
//! [`DiffHunk`] shape (Phase 2a, `docs/GIT-DIFF.md`).
//!
//! Both sides of a diff are normalized through [`Text::from`] before
//! tokenizing — CRLF-normalized and given a trailing newline if missing, the
//! same as loading the text as a HUME buffer. This is deliberate: it matches
//! how a plugin's git-ref text and the live buffer are actually compared (as
//! buffer content, not raw bytes), and it means a ref blob missing its final
//! newline — routine in git — produces no phantom trailing-line hunk against
//! a buffer that (by HUME's own invariant) always has one.

use std::borrow::Cow;
use std::ops::Range;

use hume_editing::diff::{LineHunkKind, diff_lines};
use hume_editing::text::Text;

use hume_scripting::host::DiffHunk;

/// Line-level hunks between `old` and `new`. `Equal` runs are dropped;
/// each [`DiffHunk`]'s line lists are re-sliced from the tokenized input,
/// never split back out of `LineHunkKind`'s payload — that payload joins
/// its covered lines with no separator, so a multi-line change cannot be
/// recovered from it (`hume-editing/src/diff.rs`'s `ops_to_line_hunks`).
pub(crate) fn line_hunks(old: &Text, new: &Text) -> Vec<DiffHunk> {
    let old_tokens = tokenize(old);
    let new_tokens = tokenize(new);
    let old_view: Vec<&str> = old_tokens.iter().map(Cow::as_ref).collect();
    let new_view: Vec<&str> = new_tokens.iter().map(Cow::as_ref).collect();

    diff_lines(&old_view, &new_view)
        .hunks
        .into_iter()
        .filter(|hunk| hunk.kind != LineHunkKind::Equal)
        .map(|hunk| {
            let old_range = hunk.old.clone();
            let new_range = hunk.new.clone();
            DiffHunk {
                old_start: old_range.start,
                old_count: old_range.len(),
                new_start: new_range.start,
                new_count: new_range.len(),
                old_lines: strip_newlines(&old_tokens, old_range),
                new_lines: strip_newlines(&new_tokens, new_range),
            }
        })
        .collect()
}

/// Walks every rope line, trailing `\n` included — matching
/// `changeset::diff_cs::lines_keep_newline`'s token shape so an `Equal`
/// hunk stays byte-comparable across the trailing-empty-line boundary —
/// borrowing via `RopeSlice::as_str()` where the line sits in a single rope
/// chunk (the common case) and falling back to an owned copy only when it
/// straddles a chunk boundary.
fn tokenize(text: &Text) -> Vec<Cow<'_, str>> {
    (0..text.len_lines())
        .map(|i| {
            let line = text.rope().line(i);
            match line.as_str() {
                Some(s) => Cow::Borrowed(s),
                None => Cow::Owned(line.to_string()),
            }
        })
        .collect()
}

/// Slices `tokens[range]` into owned lines with each token's trailing `\n`
/// stripped — a [`DiffHunk`]'s line payloads never carry a newline, since a
/// plugin may feed one straight into `set-virtual-lines!`'s row text.
fn strip_newlines(tokens: &[Cow<'_, str>], range: Range<usize>) -> Vec<String> {
    tokens[range]
        .iter()
        .map(|line| line.strip_suffix('\n').unwrap_or(line).to_string())
        .collect()
}

#[cfg(test)]
mod tests;
