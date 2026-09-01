//! Translates `hume_editing::diff::{LineHunk, WordHunk}` into their
//! Steel-facing [`DiffHunk`]/[`WordDiffHunk`] shapes.
//!
//! **Line diff** normalizes both sides through [`BufferText::from`] before
//! tokenizing — every line ending becomes LF and a trailing newline is added
//! if missing, the same as loading the text as a HUME buffer. This is
//! deliberate: it matches how a plugin's git-ref text and the live buffer are
//! actually compared (as buffer content, not raw bytes), and it means a ref
//! blob missing its final newline — routine in git — produces no phantom
//! trailing-line hunk against a buffer that (by HUME's own invariant) always
//! has one.
//!
//! **Word diff** does no such normalization and no re-slicing — see
//! [`word_hunks`]'s doc.

use std::borrow::Cow;
use std::ops::Range;

use hume_editing::diff::{LineHunk, LineHunkKind, WordDiff, WordHunkKind, diff_lines, diff_words};
use hume_editing::text::BufferText;
use hume_rope::lines::strip_line_break;

use hume_scripting::host::{DiffHunk, WordDiffHunk};

/// Line-level hunks between two texts, neither yet loaded as a HUME buffer —
/// both sides go through [`BufferText::from`]'s normalization (see the module doc).
pub(crate) fn line_hunks(old: &str, new: &str) -> Vec<DiffHunk> {
    hunks(&BufferText::from(old), &BufferText::from(new))
}

/// As [`line_hunks`], diffing `ref_text` (normalized here) against `buffer`
/// — already a live, normalized `BufferText`, so it needs no second pass.
pub(crate) fn line_hunks_against_buffer(ref_text: &str, buffer: &BufferText) -> Vec<DiffHunk> {
    hunks(&BufferText::from(ref_text), buffer)
}

/// `Equal` runs are dropped; each [`DiffHunk`]'s line lists are re-sliced
/// from the tokenized input — `LineHunkKind` carries no payload to split
/// (`hume-editing/src/diff.rs`).
fn hunks(old: &BufferText, new: &BufferText) -> Vec<DiffHunk> {
    let old_tokens: Vec<Cow<'_, str>> = old.line_tokens().collect();
    let new_tokens: Vec<Cow<'_, str>> = new.line_tokens().collect();

    diff_lines(&old_tokens, &new_tokens)
        .hunks
        .into_iter()
        .filter(|hunk| hunk.kind != LineHunkKind::Equal)
        .map(|hunk| {
            let LineHunk { old, new, .. } = hunk;
            DiffHunk {
                old_start: old.start,
                new_start: new.start,
                old_lines: strip_newlines(&old_tokens, old),
                new_lines: strip_newlines(&new_tokens, new),
            }
        })
        .collect()
}

/// Slices `tokens[range]` into owned lines with each token's trailing line
/// break stripped (via [`strip_line_break`]) — a [`DiffHunk`]'s line
/// payloads never carry one, since a plugin may feed one straight into
/// `set-virtual-lines!`'s row text.
fn strip_newlines(tokens: &[Cow<'_, str>], range: Range<usize>) -> Vec<String> {
    tokens[range]
        .iter()
        .map(|line| strip_line_break(line).to_string())
        .collect()
}

/// Word-level hunks between `old` and `new`, `Equal` runs dropped. No
/// `BufferText` normalization and no re-slicing: its inputs are always two
/// already-extracted line strings (typically one side of a line-diff
/// `Replace` hunk), and `diff_words`' tokens abut with no separator, so its
/// hunk payload strings are already the exact substring at that char range.
pub(crate) fn word_hunks(old: &str, new: &str) -> (Vec<WordDiffHunk>, bool) {
    convert_word_diff(diff_words(old, new))
}

/// Shared `WordDiff` → Steel-facing shape mapping for [`word_hunks`] (and,
/// under `#[cfg(test)]`, tests that force the Myers timeout path directly
/// via `hume_editing::diff::diff_words_with_deadline`).
fn convert_word_diff(diff: WordDiff) -> (Vec<WordDiffHunk>, bool) {
    let deadline_hit = diff.deadline_hit();
    let hunks = diff
        .hunks
        .into_iter()
        .filter_map(|h| {
            let (old_text, new_text) = match h.kind {
                WordHunkKind::Equal => return None,
                WordHunkKind::Delete(s) => (s, String::new()),
                WordHunkKind::Insert(s) => (String::new(), s),
                WordHunkKind::Replace { old, new } => (old, new),
            };
            Some(WordDiffHunk {
                old_start: h.old.start,
                old_end: h.old.end,
                new_start: h.new.start,
                new_end: h.new.end,
                old_text,
                new_text,
            })
        })
        .collect();
    (hunks, deadline_hit)
}

#[cfg(test)]
mod tests;
