//! Translates `hume_editing::diff::{LineHunk, WordHunk}` into their
//! Steel-facing [`DiffHunk`]/[`WordDiffHunk`] shapes (Phase 2a/2b,
//! `docs/GIT-DIFF.md`).
//!
//! **Line diff** normalizes both sides through [`Text::from`] before
//! tokenizing — CRLF-normalized and given a trailing newline if missing, the
//! same as loading the text as a HUME buffer. This is deliberate: it matches
//! how a plugin's git-ref text and the live buffer are actually compared (as
//! buffer content, not raw bytes), and it means a ref blob missing its final
//! newline — routine in git — produces no phantom trailing-line hunk against
//! a buffer that (by HUME's own invariant) always has one.
//!
//! **Word diff** does no such normalization and no re-slicing: its inputs
//! are always two already-extracted line strings (typically one side of a
//! line-diff `Replace` hunk), and `diff_words`' tokens abut with no
//! separator, so its hunk payload strings are already the exact substring
//! at that char range — unlike line hunks, which join covered lines with no
//! `\n` separator and must be re-sliced from the tokenized input.

use std::borrow::Cow;
use std::ops::Range;

use hume_editing::diff::{LineHunkKind, WordDiff, WordHunkKind, diff_lines, diff_words};
use hume_editing::text::Text;

use hume_scripting::host::{DiffHunk, WordDiffHunk};

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

/// Word-level hunks between `old` and `new`, `Equal` runs dropped. No
/// `Text` normalization and no re-slicing — see the module doc's rationale.
pub(crate) fn word_hunks(old: &str, new: &str) -> (Vec<WordDiffHunk>, bool) {
    convert_word_diff(diff_words(old, new))
}

/// As [`word_hunks`], with an explicit deadline — exists so tests can force
/// the Myers timeout path (`Duration::ZERO`) without waiting on a
/// pathological input, mirroring `diff_lines_with_deadline`'s own test use.
#[cfg(test)]
fn word_hunks_with_deadline(
    old: &str,
    new: &str,
    deadline: std::time::Duration,
) -> (Vec<WordDiffHunk>, bool) {
    convert_word_diff(hume_editing::diff::diff_words_with_deadline(
        old, new, deadline,
    ))
}

/// Shared `WordDiff` → Steel-facing shape mapping for [`word_hunks`] and
/// [`word_hunks_with_deadline`].
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
