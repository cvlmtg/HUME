//! `&BufferText`-ergonomic wrappers over `hume_rope::grapheme`'s `RopeSlice`-based
//! grapheme-cluster algorithms. See that module for the implementations and
//! detailed doc comments.

use crate::text::BufferText;

/// See [`hume_rope::grapheme::next_grapheme_boundary`].
pub fn next_grapheme_boundary(text: &BufferText, char_offset: usize) -> usize {
    hume_rope::grapheme::next_grapheme_boundary(text.full_slice(), char_offset)
}

/// See [`hume_rope::grapheme::prev_grapheme_boundary`].
pub fn prev_grapheme_boundary(text: &BufferText, char_offset: usize) -> usize {
    hume_rope::grapheme::prev_grapheme_boundary(text.full_slice(), char_offset)
}

/// See [`hume_rope::grapheme::snap_to_cluster_start`].
pub fn snap_to_cluster_start(text: &BufferText, char_offset: usize) -> usize {
    hume_rope::grapheme::snap_to_cluster_start(text.full_slice(), char_offset)
}

/// See [`hume_rope::grapheme::grapheme_col_in_line`].
pub fn grapheme_col_in_line(text: &BufferText, line_idx: usize, char_pos: usize) -> usize {
    hume_rope::grapheme::grapheme_col_in_line(text.full_slice(), line_idx, char_pos)
}

/// See [`hume_rope::grapheme::display_col_in_line`].
pub fn display_col_in_line(
    text: &BufferText,
    line_idx: usize,
    char_pos: usize,
    tab_width: u8,
) -> usize {
    hume_rope::grapheme::display_col_in_line(text.full_slice(), line_idx, char_pos, tab_width)
}

/// See [`hume_rope::grapheme::char_pos_at_display_col`].
pub fn char_pos_at_display_col(
    text: &BufferText,
    line_idx: usize,
    target_display_col: usize,
    tab_width: u8,
) -> usize {
    hume_rope::grapheme::char_pos_at_display_col(
        text.full_slice(),
        line_idx,
        target_display_col,
        tab_width,
    )
}
