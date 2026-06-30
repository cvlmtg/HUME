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

use similar::{Algorithm, DiffOp, capture_diff_slices, capture_diff_slices_deadline};
use unicode_segmentation::UnicodeSegmentation;

/// Wall-clock budget for the histogram pass. If histogram cannot finish within
/// this duration, the line diff falls back to Myers with a fresh budget of the
/// same size.
///
/// Configurable later: replace with a setting threaded through from the editor.
const DIFF_DEADLINE: Duration = Duration::from_millis(250);

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

/// The kind of a [`LineHunk`]. `Equal` carries no payload — callers that need
/// the matching text can fetch it from their input by the hunk's line ranges.
/// The other variants own the changed text so that consumers without access to
/// the original inputs (e.g. plugins, the inline-diff UI) still have a
/// self-contained result.
#[derive(Debug, Clone, PartialEq, Eq)]
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
pub struct LineHunk {
    /// Line indices in the old input covered by this hunk.
    pub old: Range<usize>,
    /// Line indices in the new input covered by this hunk.
    pub new: Range<usize>,
    /// What kind of change this is, plus any changed-line payloads.
    pub kind: LineHunkKind,
}

/// Result of a line-level diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineDiff {
    /// Which algorithm ran. `Myers` implies [`Self::deadline_hit`] is `true`.
    pub algo_used: AlgoUsed,
    /// `true` when the histogram pass could not finish in time and Myers was
    /// used as a fallback. Always `false` when `algo_used == Histogram`.
    pub deadline_hit: bool,
    /// The captured change hunks, in order.
    pub hunks: Vec<LineHunk>,
}

/// Line-level diff with an explicit deadline, for testing.
///
/// The public [`diff_lines`] uses [`DIFF_DEADLINE`]; tests call this with a
/// shorter budget to exercise the Myers fallback path quickly.
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
            deadline_hit: false,
            hunks: ops_to_line_hunks(&ops, old, new),
        };
    }

    // Histogram bailed — Myers gets a fresh budget. Myers' divide-and-conquer
    // recursion still produces a usable (if coarser) result when it too hits
    // the deadline, so we keep whatever it returns.
    let myers_start = Instant::now();
    let myers_deadline = myers_start + deadline;
    let myers_ops =
        capture_diff_slices_deadline(Algorithm::Myers, old, new, Some(myers_deadline));
    LineDiff {
        algo_used: AlgoUsed::Myers,
        deadline_hit: true,
        hunks: ops_to_line_hunks(&myers_ops, old, new),
    }
}

/// Line-level diff with the default deadline.
///
/// `old` and `new` are line slices — the caller decides how to tokenize
/// (rope lines, file lines, etc.). This keeps `diff.rs` independent of
/// `ropey` and lets the rope-vs-plain-text choice live with the caller.
pub fn diff_lines(old: &[&str], new: &[&str]) -> LineDiff {
    diff_lines_with_deadline(old, new, DIFF_DEADLINE)
}

fn ops_to_line_hunks(ops: &[DiffOp], old: &[&str], new: &[&str]) -> Vec<LineHunk> {
    ops.iter()
        .map(|op| {
            let old_range = op.old_range();
            let new_range = op.new_range();
            let kind = match op {
                DiffOp::Equal { .. } => LineHunkKind::Equal,
                DiffOp::Delete { .. } => {
                    LineHunkKind::Delete(old[old_range.clone()].join(""))
                }
                DiffOp::Insert { .. } => {
                    LineHunkKind::Insert(new[new_range.clone()].join(""))
                }
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
/// changes. `Equal` carries no payload.
#[derive(Debug, Clone, PartialEq, Eq)]
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
/// the inputs passed to [`diff_words`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordHunk {
    /// Char offsets in the old input covered by this hunk.
    pub old: Range<usize>,
    /// Char offsets in the new input covered by this hunk.
    pub new: Range<usize>,
    /// What kind of change this is, plus any changed-word payloads.
    pub kind: WordHunkKind,
}

/// Result of a word-level diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordDiff {
    /// The captured change hunks, in order.
    pub hunks: Vec<WordHunk>,
}

/// Word-level diff using Myers (no deadline — word inputs are small by nature).
///
/// Tokenization uses Unicode word boundaries (UAX #29) via
/// `unicode-segmentation`, so token boundaries are grapheme-safe: a combining
/// sequence or ZWJ emoji is never split across two tokens. Char offsets in the
/// returned hunks are into the `old` / `new` strings passed here.
pub fn diff_words(old: &str, new: &str) -> WordDiff {
    // Tokenize into words (including whitespace runs as separate tokens, so
    // the diff reconstructs the full input). Track each token's char offset;
    // a trailing sentinel offset makes range ends O(1).
    let (old_tokens, old_offsets) = tokenize_with_offsets(old);
    let (new_tokens, new_offsets) = tokenize_with_offsets(new);

    let ops = capture_diff_slices(Algorithm::Myers, &old_tokens, &new_tokens);
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
    WordDiff { hunks }
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
mod tests {
    use super::*;

    /// Join lines for test inputs — each line gets a trailing `\n` so the
    /// joined payload reconstructs the visible text.
    fn lines(s: &str) -> Vec<&str> {
        s.split('\n').map(|l| l).collect::<Vec<_>>()
    }

    // … line-level ………………………………………………………………………………………

    #[test]
    fn diff_lines_basic() {
        let old = lines("a\nb\nc\nd");
        let new = lines("a\nB\nc\nd");
        let d = diff_lines(&old, &new);
        assert_eq!(d.algo_used, AlgoUsed::Histogram);
        assert!(!d.deadline_hit);
        // Expect: equal "a", replace "b"→"B", equal "c\nd".
        assert_eq!(d.hunks.len(), 3);
        assert_eq!(d.hunks[0].kind, LineHunkKind::Equal);
        assert_eq!(d.hunks[0].old, 0..1);
        assert_eq!(
            d.hunks[1].kind,
            LineHunkKind::Replace {
                old: "b".into(),
                new: "B".into()
            }
        );
        assert_eq!(d.hunks[1].old, 1..2);
        assert_eq!(d.hunks[1].new, 1..2);
        assert_eq!(d.hunks[2].kind, LineHunkKind::Equal);
        assert_eq!(d.hunks[2].old, 2..4);
        assert_eq!(d.hunks[2].new, 2..4);
    }

    #[test]
    fn diff_lines_all_equal() {
        let old = lines("x\ny\nz");
        let new = lines("x\ny\nz");
        let d = diff_lines(&old, &new);
        assert_eq!(d.algo_used, AlgoUsed::Histogram);
        assert_eq!(d.hunks.len(), 1);
        assert_eq!(d.hunks[0].kind, LineHunkKind::Equal);
        assert_eq!(d.hunks[0].old, 0..3);
        assert_eq!(d.hunks[0].new, 0..3);
    }

    #[test]
    fn diff_lines_pure_insert() {
        let old = lines("a\nc");
        let new = lines("a\nb\nc");
        let d = diff_lines(&old, &new);
        let insert = d
            .hunks
            .iter()
            .find(|h| matches!(h.kind, LineHunkKind::Insert(_)))
            .expect("should have an insert hunk");
        assert_eq!(insert.old, 1..1);
        assert_eq!(insert.new, 1..2);
        assert_eq!(insert.kind, LineHunkKind::Insert("b".into()));
    }

    #[test]
    fn diff_lines_pure_delete() {
        let old = lines("a\nb\nc");
        let new = lines("a\nc");
        let d = diff_lines(&old, &new);
        let delete = d
            .hunks
            .iter()
            .find(|h| matches!(h.kind, LineHunkKind::Delete(_)))
            .expect("should have a delete hunk");
        assert_eq!(delete.old, 1..2);
        assert_eq!(delete.new, 1..1);
        assert_eq!(delete.kind, LineHunkKind::Delete("b".into()));
    }

    #[test]
    fn diff_lines_replace_block() {
        let old = lines("a\n1\n2\n3\nz");
        let new = lines("a\nX\nY\nz");
        let d = diff_lines(&old, &new);
        let replace = d
            .hunks
            .iter()
            .find(|h| matches!(h.kind, LineHunkKind::Replace { .. }))
            .expect("should have a replace hunk");
        assert_eq!(replace.old, 1..4);
        assert_eq!(replace.new, 1..3);
        assert_eq!(
            replace.kind,
            LineHunkKind::Replace {
                old: "123".into(),
                new: "XY".into()
            }
        );
    }

    #[test]
    fn diff_lines_myers_fallback() {
        // A zero deadline means the histogram pass can never finish in time,
        // so the fallback to Myers is deterministic regardless of machine
        // speed. The input just needs to be non-trivial so the result is
        // meaningful.
        let old: Vec<String> = (0..200)
            .map(|i| format!("line-{i}-pad-{}", i % 7))
            .collect();
        let new: Vec<String> = (0..200)
            .map(|i| format!("line-{i}-pad-{}", (i + 3) % 11))
            .collect();
        let old_refs: Vec<&str> = old.iter().map(String::as_str).collect();
        let new_refs: Vec<&str> = new.iter().map(String::as_str).collect();

        let d = diff_lines_with_deadline(&old_refs, &new_refs, Duration::ZERO);
        assert_eq!(d.algo_used, AlgoUsed::Myers);
        assert!(d.deadline_hit);
        // Myers should still produce a non-empty, coherent diff.
        assert!(!d.hunks.is_empty());
    }

    // … word-level ………………………………………………………………………………………

    #[test]
    fn diff_words_basic() {
        let d = diff_words("foo bar baz", "foo baz qux");
        // Tokenization via split_word_bounds yields
        //   old: ["foo", " ", "bar", " ", "baz"]
        //   new: ["foo", " ", "baz", " ", "qux"]
        // Myers aligns the shared " " between "bar" and "baz", producing:
        //   Equal "foo " | Replace "bar"→"baz" | Equal " " | Replace "baz"→"qux"
        assert!(!d.hunks.is_empty());
        // "foo " must be an Equal run at char offsets 0..4 on both sides.
        let prefix = d
            .hunks
            .iter()
            .find(|h| matches!(h.kind, WordHunkKind::Equal))
            .expect("should have an equal prefix");
        assert_eq!(prefix.old, 0..4);
        assert_eq!(prefix.new, 0..4);
        // "bar" (old chars 4..7) should be replaced by "baz".
        let bar_replaced = d.hunks.iter().any(|h| {
            matches!(&h.kind, WordHunkKind::Replace { old, new } if old == "bar" && new == "baz")
                && h.old == (4..7)
                && h.new == (4..7)
        });
        assert!(bar_replaced, "should replace `bar` with `baz` at 4..7");
        // "baz" (old chars 8..11) should be replaced by "qux".
        let qux_added = d.hunks.iter().any(|h| {
            matches!(&h.kind, WordHunkKind::Replace { old, new } if old == "baz" && new == "qux")
                && h.old == (8..11)
                && h.new == (8..11)
        });
        assert!(qux_added, "should replace `baz` with `qux` at 8..11");
    }

    #[test]
    fn diff_words_replace() {
        let d = diff_words("hello world", "hello there");
        let replace = d
            .hunks
            .iter()
            .find(|h| matches!(h.kind, WordHunkKind::Replace { .. }))
            .expect("should have a replace hunk");
        assert_eq!(
            replace.kind,
            WordHunkKind::Replace {
                old: "world".into(),
                new: "there".into()
            }
        );
        // "world" starts at char offset 6 in "hello world".
        assert_eq!(replace.old, 6..11);
        // "there" starts at char offset 6 in "hello there".
        assert_eq!(replace.new, 6..11);
    }

    #[test]
    fn diff_words_grapheme_safe() {
        // `é` as `e` + U+0301 combining accent. Unicode word boundaries treat
        // the combining sequence as part of the same word, so the token split
        // must not land between `e` and U+0301.
        let old = "cafe\u{0301} bar";
        let new = "cafe\u{0301} baz";
        let d = diff_words(old, new);
        // The change should be "bar" → "baz"; the "café" prefix (with its
        // combining sequence) must be an Equal run covering chars 0..6
        // (c, a, f, e, U+0301, space).
        let prefix = d
            .hunks
            .iter()
            .find(|h| matches!(h.kind, WordHunkKind::Equal))
            .expect("should have an equal prefix hunk");
        assert_eq!(prefix.old, 0..6);
        assert_eq!(prefix.new, 0..6);
    }
}
