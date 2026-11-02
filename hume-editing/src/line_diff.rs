//! The line diff of two whole texts, shared by [`crate::hunk::text_hunks`]
//! and [`crate::changeset::changesets_from_line_diff`].

use std::borrow::Cow;

use hume_rope::lines::{SharedLineEnds, line_token_strs, shared_line_ends};
use hume_rope::offset::{CharOffset, ExclusiveRange};
use ropey::RopeSlice;

use crate::diff::{LineHunk, diff_lines};
use crate::text::BufferContent;

/// The line diff of two texts, split and diffed only between the lines both
/// texts start and end with.
pub(crate) struct TextLineDiff<'a> {
    pub(crate) ends: SharedLineEnds,
    /// The line tokens of each text's middle.
    pub(crate) old: Vec<Cow<'a, str>>,
    pub(crate) new: Vec<Cow<'a, str>>,
    /// The diff of `old` against `new`, `Equal` runs included: its ranges
    /// index the middles' tokens, so line `i` of a range is line
    /// `ends.first_middle_line.advance(i)` of its text.
    pub(crate) hunks: Vec<LineHunk>,
}

pub(crate) fn text_line_diff<'a>(
    old: &'a BufferContent<'_>,
    new: &'a BufferContent<'_>,
) -> TextLineDiff<'a> {
    let (old, new) = (old.slice(), new.slice());
    let ends = shared_line_ends(old, new);
    let middle = |text: RopeSlice<'a>, chars: ExclusiveRange<CharOffset>| {
        line_token_strs(text.slice(chars.start.index()..chars.end.index())).collect()
    };
    let old_tokens: Vec<Cow<'a, str>> = middle(old, ends.old_middle);
    let new_tokens: Vec<Cow<'a, str>> = middle(new, ends.new_middle);
    let hunks = diff_lines(&old_tokens, &new_tokens);
    TextLineDiff {
        ends,
        old: old_tokens,
        new: new_tokens,
        hunks,
    }
}

/// The char offset in its text where each of a middle's `tokens` starts, then
/// where the middle ends: `offsets[i]` is where token `i` starts and
/// `offsets[tokens.len()]` is `middle.end`, so a hunk's token range maps to
/// a char range of its text.
pub(crate) fn token_offsets(
    tokens: &[Cow<'_, str>],
    middle: ExclusiveRange<CharOffset>,
) -> Vec<CharOffset> {
    let mut offsets = Vec::with_capacity(tokens.len() + 1);
    offsets.push(middle.start);
    // Raw `usize` accumulator, minted to `CharOffset` only once each running
    // total is complete, the same "trusted mint" shape as an ASCII delimiter
    // scan's own loop variable: `char_acc` never appears anywhere as a
    // position in its own right until it's pushed.
    let mut char_acc = middle.start.index();
    for token in tokens {
        char_acc += token.chars().count();
        offsets.push(CharOffset::new(char_acc));
    }
    debug_assert_eq!(
        offsets.last(),
        Some(&middle.end),
        "a middle's tokens cover it"
    );
    offsets
}
