//! `&BufferText`-ergonomic wrappers over `hume_rope::lines`'s `&Rope`-based line
//! helpers. See `hume_rope::lines` for the implementations and detailed doc
//! comments.

use hume_rope::cluster::{ClusterRange, ClusterStart};
use hume_rope::column::{BufferLineCol, ByteCol, CharCol, GraphemeCol};
use hume_rope::line::{ContentLine, RopeyLine};
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::text::BufferText;

/// See [`hume_rope::lines::line_start`].
pub fn line_start(text: &BufferText, line: ContentLine) -> ClusterStart {
    hume_rope::lines::line_start(text.rope(), line)
}

/// See [`hume_rope::lines::line_break`].
pub fn line_break(text: &BufferText, line: ContentLine) -> ClusterStart {
    hume_rope::lines::line_break(text.rope(), line)
}

/// See [`hume_rope::lines::line_range`].
pub fn line_range(text: &BufferText, line: ContentLine) -> ClusterRange {
    hume_rope::lines::line_range(text.rope(), line)
}

/// See [`hume_rope::lines::lines_range`].
pub fn lines_range(text: &BufferText, first: ContentLine, last: ContentLine) -> ClusterRange {
    hume_rope::lines::lines_range(text.rope(), first, last)
}

/// See [`hume_rope::lines::line_content_range`].
pub fn line_content_range(text: &BufferText, line: ContentLine) -> Option<ClusterRange> {
    hume_rope::lines::line_content_range(text.rope(), line)
}

/// See [`hume_rope::lines::next_line_start`].
pub fn next_line_start(text: &BufferText, line: RopeyLine) -> CharOffset {
    hume_rope::lines::next_line_start(text.rope(), line)
}

/// See [`hume_rope::lines::leading_whitespace_end`].
pub fn leading_whitespace_end(text: &BufferText, line: ContentLine) -> ClusterStart {
    hume_rope::lines::leading_whitespace_end(text.rope(), line)
}

/// See [`hume_rope::lines::leading_indent`].
pub fn leading_indent(
    text: &BufferText,
    line: ContentLine,
    tab_width: u8,
) -> (ClusterStart, BufferLineCol) {
    hume_rope::lines::leading_indent(text.rope(), line, tab_width)
}

/// See [`hume_rope::lines::line_content_end`].
pub fn line_content_end(text: &BufferText, line: ContentLine) -> ClusterStart {
    hume_rope::lines::line_content_end(text.rope(), line)
}

/// See [`hume_rope::lines::char_col_in_line`].
pub fn char_col_in_line(text: &BufferText, line: ContentLine, char_pos: CharOffset) -> CharCol {
    hume_rope::lines::char_col_in_line(text.rope(), line, char_pos)
}

/// See [`hume_rope::lines::place_char_column`].
pub fn place_char_column(text: &BufferText, line: RopeyLine, char_col: CharCol) -> ClusterStart {
    hume_rope::lines::place_char_column(text.rope(), line, char_col)
}

/// See [`hume_rope::lines::place_grapheme_column`].
pub fn place_grapheme_column(
    text: &BufferText,
    line: RopeyLine,
    grapheme_col: GraphemeCol,
) -> ClusterStart {
    hume_rope::lines::place_grapheme_column(text.rope(), line, grapheme_col)
}

/// See [`hume_rope::lines::char_to_line_byte`].
pub fn char_to_line_byte(text: &BufferText, char_pos: CharOffset) -> (RopeyLine, ByteCol) {
    hume_rope::lines::char_to_line_byte(text.rope(), char_pos)
}

/// See [`hume_rope::lines::line_segments`].
pub fn line_segments(
    text: &BufferText,
    range: ExclusiveRange<CharOffset>,
) -> impl Iterator<Item = (ContentLine, ByteCol, ByteCol)> + '_ {
    hume_rope::lines::line_segments(text.rope(), range)
}
