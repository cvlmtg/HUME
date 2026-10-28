//! BufferText diffing, through `imara-diff`.
//!
//! - [`diff_lines`]: line-level diff. Histogram, which anchors on lines
//!   occurring once on both sides and so reads better around moved or
//!   repeated lines, runs while its estimated cost fits
//!   `HISTOGRAM_WORK_LIMIT`, and Myers otherwise. The estimate counts the
//!   Myers diff's hunks, unless inputs this short fit even at the most hunks
//!   a diff of them can have. [`LineHunk`] ranges are line indices.
//! - [`diff_words`]: word-level Myers diff, meant for refining a replaced
//!   line. [`WordHunk`] ranges are char offsets.
//!
//! `imara-diff` stays an implementation detail: consumers depend only on the
//! hunk types defined here.

use std::hash::Hash;
use std::ops::Range;

use imara_diff::{Algorithm, Diff, InternedInput, Interner};
use unicode_segmentation::UnicodeSegmentation;

/// The most `lines × hunks` the histogram pass may cost, `lines` counting
/// both inputs and `hunks` the Myers diff's change hunks. Histogram's time
/// grows with that product, at about 0.85 ns a unit in a release build on an
/// M4 Pro from 5,000 to 100,000 lines, so this keeps it near 14 ms there on
/// top of a per-line cost Myers pays too.
const HISTOGRAM_WORK_LIMIT: usize = 16_000_000;

// ── Line-level types ──────────────────────────────────────────────────────────

/// The kind of a [`LineHunk`]. No variant carries a payload: every consumer
/// re-slices the changed lines from its own input via the hunk's `old`/`new`
/// ranges instead (a payload would join those lines with no separator,
/// making a multi-line hunk unrecoverable from it anyway).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum LineHunkKind {
    /// Lines are identical on both sides.
    Equal,
    /// Lines were removed from the old side.
    Delete,
    /// Lines were added on the new side.
    Insert,
    /// A contiguous block of old lines was replaced by a contiguous block of
    /// new lines.
    Replace,
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
    /// What kind of change this is.
    pub kind: LineHunkKind,
}

/// Line-level diff: the hunks partition both inputs, in order, unchanged runs
/// included as [`LineHunkKind::Equal`].
///
/// `old` and `new` are line slices; the caller decides how to tokenize
/// (rope lines, file lines, etc.). This keeps `diff.rs` independent of
/// `ropey` and lets the rope-vs-plain-text choice live with the caller.
pub fn diff_lines<T: Eq + Hash + Clone>(old: &[T], new: &[T]) -> Vec<LineHunk> {
    diff_lines_within(old, new, HISTOGRAM_WORK_LIMIT)
}

/// [`diff_lines`] with `limit` in place of [`HISTOGRAM_WORK_LIMIT`].
fn diff_lines_within<T: Eq + Hash + Clone>(old: &[T], new: &[T], limit: usize) -> Vec<LineHunk> {
    let input = interned(old, new);
    let lines = old.len() + new.len();
    // Unchanged lines separate change hunks, and the shorter input holds
    // every unchanged line, so a diff has at most `most_hunks` hunks.
    let most_hunks = old.len().min(new.len()) + 1;
    let myers =
        (!histogram_fits(lines, most_hunks, limit)).then(|| computed(Algorithm::Myers, &input));
    let diff = match myers {
        Some(myers) if !histogram_fits(lines, myers.hunks().count(), limit) => myers,
        _ => computed(Algorithm::Histogram, &input),
    };
    partition(&diff, old.len(), new.len())
}

/// Whether a histogram pass over `lines` lines with at most `hunks` change
/// hunks stays within `limit` (see [`HISTOGRAM_WORK_LIMIT`]).
fn histogram_fits(lines: usize, hunks: usize, limit: usize) -> bool {
    lines.saturating_mul(hunks) <= limit
}

/// `old` and `new` interned for `imara-diff`.
fn interned<T: Eq + Hash + Clone>(old: &[T], new: &[T]) -> InternedInput<T> {
    let mut input = InternedInput {
        before: Vec::with_capacity(old.len()),
        after: Vec::with_capacity(new.len()),
        interner: Interner::new(old.len() + new.len()),
    };
    input.update_before(old.iter().cloned());
    input.update_after(new.iter().cloned());
    input
}

/// `algorithm`'s diff of `input`, its hunks merged and slid down where their
/// placement is ambiguous.
fn computed<T>(algorithm: Algorithm, input: &InternedInput<T>) -> Diff {
    let mut diff = Diff::compute(algorithm, input);
    diff.postprocess_no_heuristic(input);
    diff
}

/// `diff`'s change hunks over inputs of `old_len` and `new_len` tokens, with
/// the unchanged runs between them, as token-index ranges in order.
fn partition(diff: &Diff, old_len: usize, new_len: usize) -> Vec<LineHunk> {
    let mut hunks = Vec::new();
    let (mut old_at, mut new_at) = (0, 0);
    for (old, new) in token_ranges(diff) {
        if old.start > old_at {
            hunks.push(LineHunk {
                old: old_at..old.start,
                new: new_at..new.start,
                kind: LineHunkKind::Equal,
            });
        }
        (old_at, new_at) = (old.end, new.end);
        let kind = match (old.is_empty(), new.is_empty()) {
            (true, _) => LineHunkKind::Insert,
            (_, true) => LineHunkKind::Delete,
            _ => LineHunkKind::Replace,
        };
        hunks.push(LineHunk { old, new, kind });
    }
    if old_at < old_len {
        hunks.push(LineHunk {
            old: old_at..old_len,
            new: new_at..new_len,
            kind: LineHunkKind::Equal,
        });
    }
    hunks
}

/// `diff`'s change hunks as old-side and new-side token-index ranges.
fn token_ranges(diff: &Diff) -> impl Iterator<Item = (Range<usize>, Range<usize>)> + '_ {
    diff.hunks().map(|hunk| {
        (
            hunk.before.start as usize..hunk.before.end as usize,
            hunk.after.start as usize..hunk.after.end as usize,
        )
    })
}

// ── Word-level types ──────────────────────────────────────────────────────────

/// The kind of a [`WordHunk`]. Unlike [`LineHunkKind`], the change variants
/// here do own their text: a word hunk is the fine-grained refinement of a
/// single already-short line, so a consumer can move the payload directly
/// into its own shape rather than re-slicing.
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
/// slicing `&old[hunk.old]` panics on non-ASCII inputs. Convert char offsets
/// to byte offsets (e.g. via `char_indices`) before slicing. This matches
/// [`crate::text::BufferText`]'s char-offset invariant.
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

/// Word-level Myers diff: the hunks partition both inputs, in order.
///
/// Meant for short strings (e.g. the contents of a single replaced line, or
/// two short snippets for plugin use). For large inputs, diff at the line
/// level first and refine per-line with this.
///
/// Tokenization uses Unicode word boundaries (UAX #29) via
/// `unicode-segmentation`, so token boundaries are grapheme-safe: a combining
/// sequence or ZWJ emoji is never split across two tokens. Char offsets in the
/// returned hunks are into the `old` / `new` strings passed here.
///
/// Note: this UAX #29 notion of "word" is linguistic and distinct from the
/// vim `w`/`W` semantics in [`crate::word`]; the two intentionally do not
/// agree.
pub fn diff_words(old: &str, new: &str) -> Vec<WordHunk> {
    let tokens = WordTokens::new(old, new);
    let input = interned(&tokens.old, &tokens.new);
    partition(
        &computed(Algorithm::Myers, &input),
        tokens.old.len(),
        tokens.new.len(),
    )
    .into_iter()
    .map(|LineHunk { old, new, kind }| {
        let joined = |side: &[&str], range: &Range<usize>| side[range.clone()].join("");
        let kind = match kind {
            LineHunkKind::Equal => WordHunkKind::Equal,
            LineHunkKind::Delete => WordHunkKind::Delete(joined(&tokens.old, &old)),
            LineHunkKind::Insert => WordHunkKind::Insert(joined(&tokens.new, &new)),
            LineHunkKind::Replace => WordHunkKind::Replace {
                old: joined(&tokens.old, &old),
                new: joined(&tokens.new, &new),
            },
        };
        WordHunk {
            old: char_range(&tokens.old_offsets, &old),
            new: char_range(&tokens.new_offsets, &new),
            kind,
        }
    })
    .collect()
}

/// Changed char ranges of the old side, then of the new side.
pub(crate) type ChangedRanges = (Vec<Range<usize>>, Vec<Range<usize>>);

/// The char ranges of `old` and of `new` that a word diff marks as changed,
/// with no text payloads. The two sides' word tokens come off `budget`, or
/// the result is `None` and `budget` is untouched when they outnumber it.
/// Tokenizing stops at the budget, so the cost of an input far over it is
/// the budget's. Myers keeps its cost near-linear in the token count. A pure
/// insertion or deletion contributes a zero-width range to the side it
/// leaves untouched.
pub(crate) fn word_change_ranges(
    old: &str,
    new: &str,
    budget: &mut usize,
) -> Option<ChangedRanges> {
    let tokens = WordTokens::within(old, new, *budget)?;
    *budget -= tokens.old.len() + tokens.new.len();
    let input = interned(&tokens.old, &tokens.new);
    Some(
        token_ranges(&computed(Algorithm::Myers, &input))
            .map(|(old, new)| {
                (
                    char_range(&tokens.old_offsets, &old),
                    char_range(&tokens.new_offsets, &new),
                )
            })
            .unzip(),
    )
}

/// The word tokens of both inputs, with each side's offset table.
struct WordTokens<'a> {
    old: Vec<&'a str>,
    old_offsets: Vec<usize>,
    new: Vec<&'a str>,
    new_offsets: Vec<usize>,
}

impl<'a> WordTokens<'a> {
    /// Every token of both inputs.
    fn new(old: &'a str, new: &'a str) -> Self {
        // Whitespace runs are tokens of their own, so the ops cover all of
        // both inputs.
        let (old, old_offsets) = tokenize_with_offsets(old, usize::MAX);
        let (new, new_offsets) = tokenize_with_offsets(new, usize::MAX);
        Self {
            old,
            old_offsets,
            new,
            new_offsets,
        }
    }

    /// The tokens of both inputs, or `None` when together they outnumber
    /// `limit`.
    fn within(old: &'a str, new: &'a str, limit: usize) -> Option<Self> {
        let (old, old_offsets) = tokenize_with_offsets(old, limit);
        let left = limit.checked_sub(old.len())?;
        let (new, new_offsets) = tokenize_with_offsets(new, left);
        if new.len() > left {
            return None;
        }
        Some(Self {
            old,
            old_offsets,
            new,
            new_offsets,
        })
    }
}

/// Split `s` into Unicode word-boundary tokens, returning the tokens and the
/// char offset of each token plus a trailing sentinel (`offsets.len() ==
/// tokens.len() + 1`). Stops after `limit + 1` tokens, so a result longer
/// than `limit` means the input has more.
fn tokenize_with_offsets(s: &str, limit: usize) -> (Vec<&str>, Vec<usize>) {
    let mut tokens = Vec::new();
    let mut offsets = Vec::new();
    let mut char_pos = 0usize;
    for token in s.split_word_bounds().take(limit.saturating_add(1)) {
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

#[cfg(test)]
mod tests;
