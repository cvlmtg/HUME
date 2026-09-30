//! `&BufferText`-ergonomic wrappers over `hume_rope::grapheme`'s `RopeSlice`-based
//! grapheme-cluster algorithms. See that module for the implementations and
//! detailed doc comments.

use hume_rope::cluster::{ClusterBound, ClusterStart};
use hume_rope::column::{BufferLineCol, GraphemeCol};
use hume_rope::line::ContentLine;
use hume_rope::offset::CharOffset;

use crate::text::BufferText;

/// See [`hume_rope::grapheme::graphemes_at`].
pub fn graphemes_at(text: &BufferText, from: ClusterBound) -> hume_rope::grapheme::Graphemes<'_> {
    hume_rope::grapheme::graphemes_at(text.full_slice(), from)
}

/// See [`hume_rope::grapheme::clusters_before`].
pub fn clusters_before(
    text: &BufferText,
    bound: ClusterBound,
) -> hume_rope::grapheme::ClustersBefore<'_> {
    hume_rope::grapheme::clusters_before(text.full_slice(), bound)
}

/// See [`hume_rope::grapheme::cluster_end`].
pub fn cluster_end(text: &BufferText, start: ClusterStart) -> ClusterBound {
    hume_rope::grapheme::cluster_end(text.full_slice(), start)
}

/// See [`hume_rope::grapheme::next_cluster`].
pub fn next_cluster(text: &BufferText, start: ClusterStart) -> Option<ClusterStart> {
    hume_rope::grapheme::next_cluster(text.full_slice(), start)
}

/// The text's first cluster.
pub fn first_cluster(text: &BufferText) -> ClusterStart {
    hume_rope::grapheme::first_cluster(text.full_slice()).expect("a buffer is never empty")
}

/// The text's last cluster: its structural `\n`.
pub fn last_cluster(text: &BufferText) -> ClusterStart {
    hume_rope::grapheme::last_cluster(text.full_slice()).expect("a buffer is never empty")
}

/// See [`hume_rope::grapheme::prev_cluster`].
pub fn prev_cluster(text: &BufferText, bound: ClusterBound) -> Option<ClusterStart> {
    hume_rope::grapheme::prev_cluster(text.full_slice(), bound)
}

/// See [`hume_rope::grapheme::grapheme_col_in_line`].
pub fn grapheme_col_in_line(
    text: &BufferText,
    line_idx: ContentLine,
    char_pos: CharOffset,
) -> GraphemeCol {
    hume_rope::grapheme::grapheme_col_in_line(text.full_slice(), line_idx, char_pos)
}

/// See [`hume_rope::grapheme::display_col_in_line`].
pub fn display_col_in_line(
    text: &BufferText,
    line_idx: ContentLine,
    char_pos: CharOffset,
    tab_width: u8,
) -> BufferLineCol {
    hume_rope::grapheme::display_col_in_line(text.full_slice(), line_idx, char_pos, tab_width)
}

/// See [`hume_rope::grapheme::char_pos_at_display_col`].
pub fn char_pos_at_display_col(
    text: &BufferText,
    line_idx: ContentLine,
    target_display_col: BufferLineCol,
    tab_width: u8,
) -> ClusterStart {
    hume_rope::grapheme::char_pos_at_display_col(
        text.full_slice(),
        line_idx,
        target_display_col,
        tab_width,
    )
}
