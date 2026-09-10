//! `&BufferText`-ergonomic wrappers over `hume_rope::grapheme`'s `RopeSlice`-based
//! grapheme-cluster algorithms. See that module for the implementations and
//! detailed doc comments.

use hume_rope::line::ContentLine;
use hume_rope::offset::CharOffset;

use crate::text::BufferText;

/// See [`hume_rope::grapheme::next_grapheme_boundary`].
pub fn next_grapheme_boundary(text: &BufferText, char_offset: CharOffset) -> CharOffset {
    hume_rope::grapheme::next_grapheme_boundary(text.full_slice(), char_offset)
}

/// See [`hume_rope::grapheme::prev_grapheme_boundary`].
pub fn prev_grapheme_boundary(text: &BufferText, char_offset: CharOffset) -> CharOffset {
    hume_rope::grapheme::prev_grapheme_boundary(text.full_slice(), char_offset)
}

/// See [`hume_rope::grapheme::snap_to_cluster_start`].
pub fn snap_to_cluster_start(text: &BufferText, char_offset: CharOffset) -> CharOffset {
    hume_rope::grapheme::snap_to_cluster_start(text.full_slice(), char_offset)
}

/// See [`hume_rope::grapheme::cluster_last_char`].
pub fn cluster_last_char(text: &BufferText, cluster_start: CharOffset) -> CharOffset {
    hume_rope::grapheme::cluster_last_char(text.full_slice(), cluster_start)
}

/// See [`hume_rope::grapheme::grapheme_col_in_line`].
pub fn grapheme_col_in_line(
    text: &BufferText,
    line_idx: ContentLine,
    char_pos: CharOffset,
) -> usize {
    hume_rope::grapheme::grapheme_col_in_line(text.full_slice(), line_idx.index(), char_pos)
}

/// See [`hume_rope::grapheme::display_col_in_line`].
pub fn display_col_in_line(
    text: &BufferText,
    line_idx: ContentLine,
    char_pos: CharOffset,
    tab_width: u8,
) -> usize {
    hume_rope::grapheme::display_col_in_line(
        text.full_slice(),
        line_idx.index(),
        char_pos,
        tab_width,
    )
}

/// See [`hume_rope::grapheme::char_pos_at_display_col`].
pub fn char_pos_at_display_col(
    text: &BufferText,
    line_idx: ContentLine,
    target_display_col: usize,
    tab_width: u8,
) -> CharOffset {
    hume_rope::grapheme::char_pos_at_display_col(
        text.full_slice(),
        line_idx.index(),
        target_display_col,
        tab_width,
    )
}
