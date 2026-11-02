//! The line diff of two whole texts, shared by [`crate::hunk::text_hunks`]
//! and [`crate::changeset::changesets_from_line_diff`].

use std::borrow::Cow;

use hume_rope::lines::{SharedLineEnds, line_token_strs, shared_line_ends};
use hume_rope::offset::{CharOffset, ExclusiveRange};
use ropey::RopeSlice;

use crate::diff::{LineHunk, diff_lines};

/// The line diff of two texts of buffer content, split and diffed only
/// between the lines both texts start and end with.
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

pub(crate) fn text_line_diff<'a>(old: RopeSlice<'a>, new: RopeSlice<'a>) -> TextLineDiff<'a> {
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
