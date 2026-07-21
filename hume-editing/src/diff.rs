//! Text diffing.
//!
//! Two composable entry points:
//!
//! - [`diff_lines`] — line-level diff. Uses the **histogram** algorithm by
//!   default and falls back to **Myers** if the histogram pass cannot finish
//!   within a deadline. Deadlines keep diffing responsive on large files or
//!   files with very long lines (e.g. minified code), where histogram's
//!   anchor search can blow up. Myers handles those inputs more gracefully
//!   because of its divide-and-conquer "middle snake" recursion.
//! - [`diff_words`] — word-level diff using **Myers**. Intended as an optional
//!   refinement pass on top of a line-level diff (e.g. to highlight the exact
//!   words that changed inside a replaced line), or for any other use case
//!   where a caller wants a fine-grained comparison of two short strings.
//!
//! `similar` is an implementation detail of this module — the public types
//! (`LineHunk`, `WordHunk`, …) are owned by `hume-editing` so that consumers
//! (engine, UI, scripting) never depend on the diff backend directly. This
//! mirrors how `unicode-segmentation` is hidden behind `grapheme.rs`.
//!
//! ## Position units
//!
//! - [`LineHunk`] ranges are **line indices** into the caller-supplied
//!   `&[&str]` slices.
//! - [`WordHunk`] ranges are **char offsets** into the caller-supplied
//!   `&str` inputs (consistent with `text::Text`'s char-offset invariant).

use std::ops::Range;
use std::time::{Duration, Instant};

use similar::{Algorithm, DiffOp, capture_diff_slices_deadline};
use unicode_segmentation::UnicodeSegmentation;

/// Wall-clock budget for the line-level histogram pass. If histogram cannot
/// finish within this duration, the line diff falls back to Myers with a
/// fresh budget of the same size.
///
/// Configurable later: replace with a setting threaded through from the editor.
pub(crate) const DIFF_LINE_DEADLINE: Duration = Duration::from_millis(250);

/// Wall-clock budget for the word-level Myers pass. Word diffs are meant as a
/// refinement pass on single replaced lines, so the budget is tighter than the
/// line-level one; on timeout Myers returns a coarse (Replace-all) result and
/// [`WordDiff::deadline_hit`] reports it. The guard is a safety net — callers
/// are still expected to pass short strings.
const DIFF_WORD_DEADLINE: Duration = Duration::from_millis(50);

// ── Line-level types ──────────────────────────────────────────────────────────

/// Which algorithm produced a [`LineDiff`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlgoUsed {
    /// Histogram diff completed within the deadline.
    Histogram,
    /// Myers diff — used as a fallback because the histogram pass hit the
    /// deadline.
    Myers,
}

/// The kind of a [`LineHunk`]. `Equal` carries no payload — unchanged text is
/// the common case and is never materialized; callers that need it can fetch
/// it from their input by the hunk's line ranges. The change variants own the
/// changed text so the interesting parts of the diff are self-contained
/// without a slice lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum LineHunkKind {
    /// Lines are identical on both sides. No payload — fetch via the hunk's
    /// ranges if context is needed.
    Equal,
    /// Lines were removed from the old side.
    Delete(String),
    /// Lines were added on the new side.
    Insert(String),
    /// A contiguous block of old lines was replaced by a contiguous block of
    /// new lines.
    Replace { old: String, new: String },
}

/// A single line-level change. `old` and `new` are line-index ranges into the
/// inputs passed to [`diff_lines`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct LineHunk {
    /// Line indices in the old input covered by this hunk.
    pub old: Range<usize>,
    /// Line indices in the new input covered by this hunk.
    pub new: Range<usize>,
    /// What kind of change this is, plus any changed-line payloads.
    pub kind: LineHunkKind,
}

/// Result of a line-level diff.
///
/// `deadline_hit()` is derived from `algo_used` — Myers only ever runs as a
/// histogram fallback, so the two are never out of step (single source of
/// truth on which algorithm won).
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct LineDiff {
    /// Which algorithm ran.
    pub algo_used: AlgoUsed,
    /// The captured change hunks, in order.
    pub hunks: Vec<LineHunk>,
}

impl LineDiff {
    /// `true` when the histogram pass could not finish in time and Myers was
    /// used as a fallback. Always `false` when `algo_used == Histogram`.
    pub fn deadline_hit(&self) -> bool {
        self.algo_used == AlgoUsed::Myers
    }
}

/// Line-level diff with an explicit deadline, for callers that want a tighter
/// budget than the public default (e.g. scripting, tests). The public
/// [`diff_lines`] uses [`DIFF_LINE_DEADLINE`].
pub fn diff_lines_with_deadline(old: &[&str], new: &[&str], deadline: Duration) -> LineDiff {
    let start = Instant::now();
    let deadline_instant = start + deadline;
    let ops = capture_diff_slices_deadline(Algorithm::Histogram, old, new, Some(deadline_instant));
    // Wall-clock check: if we used the full budget, treat the histogram pass
    // as having bailed out and retry with Myers. The slop is microseconds and
    // the budget is hundreds of milliseconds, so this is robust in practice.
    if start.elapsed() < deadline {
        return LineDiff {
            algo_used: AlgoUsed::Histogram,
            hunks: ops_to_line_hunks(&ops, old, new),
        };
    }

    // Histogram bailed — Myers gets a fresh budget. Myers' divide-and-conquer
    // recursion still produces a usable (if coarser) result when it too hits
    // the deadline, so we keep whatever it returns.
    let myers_start = Instant::now();
    let myers_deadline = myers_start + deadline;
    let myers_ops = capture_diff_slices_deadline(Algorithm::Myers, old, new, Some(myers_deadline));
    LineDiff {
        algo_used: AlgoUsed::Myers,
        hunks: ops_to_line_hunks(&myers_ops, old, new),
    }
}

/// Line-level diff with the default deadline.
///
/// `old` and `new` are line slices — the caller decides how to tokenize
/// (rope lines, file lines, etc.). This keeps `diff.rs` independent of
/// `ropey` and lets the rope-vs-plain-text choice live with the caller.
pub fn diff_lines(old: &[&str], new: &[&str]) -> LineDiff {
    diff_lines_with_deadline(old, new, DIFF_LINE_DEADLINE)
}

fn ops_to_line_hunks(ops: &[DiffOp], old: &[&str], new: &[&str]) -> Vec<LineHunk> {
    // Payloads join the covered lines with no separator — the caller already
    // knows the line granularity, and Equal hunks carry no payload at all.
    ops.iter()
        .map(|op| {
            let old_range = op.old_range();
            let new_range = op.new_range();
            let kind = match op {
                DiffOp::Equal { .. } => LineHunkKind::Equal,
                DiffOp::Delete { .. } => LineHunkKind::Delete(old[old_range.clone()].join("")),
                DiffOp::Insert { .. } => LineHunkKind::Insert(new[new_range.clone()].join("")),
                DiffOp::Replace { .. } => LineHunkKind::Replace {
                    old: old[old_range.clone()].join(""),
                    new: new[new_range.clone()].join(""),
                },
            };
            LineHunk {
                old: old_range,
                new: new_range,
                kind,
            }
        })
        .collect()
}

// ── Word-level types ──────────────────────────────────────────────────────────

/// The kind of a [`WordHunk`]. Mirrors [`LineHunkKind`] but for word-level
/// changes. `Equal` carries no payload — see [`LineHunkKind`] for the
/// rationale.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum WordHunkKind {
    /// Words are identical on both sides.
    Equal,
    /// Words were removed from the old side.
    Delete(String),
    /// Words were added on the new side.
    Insert(String),
    /// A contiguous block of old words was replaced by a contiguous block of
    /// new words.
    Replace { old: String, new: String },
}

/// A single word-level change. `old` and `new` are **char-offset** ranges into
/// the inputs passed to [`diff_words`]. Note: `&str` indexing is byte-based, so
/// slicing `&old[hunk.old]` panics on non-ASCII inputs — convert char offsets
/// to byte offsets (e.g. via `char_indices`) before slicing. This matches
/// [`crate::text::Text`]'s char-offset invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct WordHunk {
    /// Char offsets in the old input covered by this hunk.
    pub old: Range<usize>,
    /// Char offsets in the new input covered by this hunk.
    pub new: Range<usize>,
    /// What kind of change this is, plus any changed-word payloads.
    pub kind: WordHunkKind,
}

/// Result of a word-level diff.
///
/// `deadline_hit()` reports whether Myers could not finish within
/// [`DIFF_WORD_DEADLINE`] and returned a coarse (Replace-all) result. Unlike
/// [`LineDiff`], word-level has a single algorithm, so the deadline hit is
/// independent state with nothing to derive from — hence the private field
/// behind the accessor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct WordDiff {
    /// The captured change hunks, in order.
    pub hunks: Vec<WordHunk>,
    deadline_hit: bool,
}

impl WordDiff {
    /// `true` when Myers could not finish within [`DIFF_WORD_DEADLINE`] and
    /// returned a coarse (Replace-all) result.
    pub fn deadline_hit(&self) -> bool {
        self.deadline_hit
    }
}

/// Word-level diff using Myers, protected by [`DIFF_WORD_DEADLINE`].
///
/// # Contract
///
/// Callers **should** pass short strings (e.g. the contents of a single
/// replaced line, or two short snippets for plugin use). The deadline is a
/// safety net, not a license to feed huge inputs: on timeout Myers returns a
/// coarse (Replace-all) result and [`WordDiff::deadline_hit`] reports it. For
/// large inputs, diff at the line level first and refine per-line with this.
///
/// Tokenization uses Unicode word boundaries (UAX #29) via
/// `unicode-segmentation`, so token boundaries are grapheme-safe: a combining
/// sequence or ZWJ emoji is never split across two tokens. Char offsets in the
/// returned hunks are into the `old` / `new` strings passed here.
///
/// Note: this UAX #29 notion of "word" is linguistic and distinct from the
/// vim `w`/`W` semantics in [`crate::word`]; the two intentionally do not
/// agree.
pub fn diff_words(old: &str, new: &str) -> WordDiff {
    diff_words_with_deadline(old, new, DIFF_WORD_DEADLINE)
}

/// Word-level diff with an explicit deadline, for callers that want a tighter
/// budget than the public default (e.g. scripting, tests). The public
/// [`diff_words`] uses [`DIFF_WORD_DEADLINE`].
///
/// Myers' divide-and-conquer recursion still produces a coherent (if coarser)
/// result when it hits the deadline — the ranges always partition the input —
/// so we keep whatever it returns and report the timeout via
/// [`WordDiff::deadline_hit`].
pub fn diff_words_with_deadline(old: &str, new: &str, deadline: Duration) -> WordDiff {
    // Tokenize into words (including whitespace runs as separate tokens, so
    // the diff reconstructs the full input). Track each token's char offset;
    // a trailing sentinel offset makes range ends O(1).
    let (old_tokens, old_offsets) = tokenize_with_offsets(old);
    let (new_tokens, new_offsets) = tokenize_with_offsets(new);

    let start = Instant::now();
    let deadline_instant = start + deadline;
    let ops = capture_diff_slices_deadline(
        Algorithm::Myers,
        &old_tokens,
        &new_tokens,
        Some(deadline_instant),
    );
    let deadline_hit = start.elapsed() >= deadline;
    let hunks = ops
        .iter()
        .map(|op| {
            let old_range = op.old_range();
            let new_range = op.new_range();
            let kind = match op {
                DiffOp::Equal { .. } => WordHunkKind::Equal,
                DiffOp::Delete { .. } => {
                    WordHunkKind::Delete(old_tokens[old_range.clone()].join(""))
                }
                DiffOp::Insert { .. } => {
                    WordHunkKind::Insert(new_tokens[new_range.clone()].join(""))
                }
                DiffOp::Replace { .. } => WordHunkKind::Replace {
                    old: old_tokens[old_range.clone()].join(""),
                    new: new_tokens[new_range.clone()].join(""),
                },
            };
            WordHunk {
                old: char_range(&old_offsets, &old_range),
                new: char_range(&new_offsets, &new_range),
                kind,
            }
        })
        .collect();
    WordDiff {
        hunks,
        deadline_hit,
    }
}

/// Split `s` into Unicode word-boundary tokens, returning the tokens and the
/// char offset of each token plus a trailing sentinel (`offsets.len() ==
/// tokens.len() + 1`).
fn tokenize_with_offsets(s: &str) -> (Vec<&str>, Vec<usize>) {
    let mut tokens = Vec::new();
    let mut offsets = Vec::new();
    let mut char_pos = 0usize;
    for token in s.split_word_bounds() {
        offsets.push(char_pos);
        char_pos += token.chars().count();
        tokens.push(token);
    }
    offsets.push(char_pos);
    (tokens, offsets)
}

/// Map a token-index range to a char-offset range using the offset table
/// (with its trailing sentinel).
fn char_range(offsets: &[usize], token_range: &Range<usize>) -> Range<usize> {
    offsets[token_range.start]..offsets[token_range.end]
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
