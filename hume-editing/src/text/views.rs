//! Borrowed views of a [`BufferText`] that answer cluster, line and column
//! questions. Each method delegates to the `hume_rope` function of the same
//! job, which carries the detailed contract.

use hume_rope::cluster::{ClusterBound, ClusterRange, ClusterStart};
use hume_rope::column::{BufferLineCol, ByteCol, CharCol, GraphemeCol};
use hume_rope::grapheme::{ClustersBefore, Graphemes};
use hume_rope::line::{ContentLine, RopeyLine};
use hume_rope::offset::{CharOffset, ExclusiveRange};

use super::BufferText;

/// Grapheme-cluster queries over a text, from [`BufferText::clusters`].
#[derive(Clone, Copy)]
pub struct ClusterView<'a>(pub(super) &'a BufferText);

/// Line queries over a text that need only the text and a position, from
/// [`BufferText::lines`].
#[derive(Clone, Copy)]
pub struct LineView<'a>(pub(super) &'a BufferText);

/// Column conversions over a text: whatever needs a tab width or names a
/// column type, from [`BufferText::columns`].
#[derive(Clone, Copy)]
pub struct ColumnView<'a>(pub(super) &'a BufferText);

impl<'a> ClusterView<'a> {
    /// See [`hume_rope::grapheme::graphemes_at`].
    pub fn graphemes_at(self, from: ClusterBound) -> Graphemes<'a> {
        hume_rope::grapheme::graphemes_at(self.0.full_slice(), from)
    }

    /// See [`hume_rope::grapheme::clusters_before`].
    pub fn before(self, bound: ClusterBound) -> ClustersBefore<'a> {
        hume_rope::grapheme::clusters_before(self.0.full_slice(), bound)
    }

    /// See [`hume_rope::grapheme::cluster_end`].
    pub fn end_of(self, start: ClusterStart) -> ClusterBound {
        hume_rope::grapheme::cluster_end(self.0.full_slice(), start)
    }

    /// See [`hume_rope::grapheme::next_cluster`].
    pub fn next(self, start: ClusterStart) -> Option<ClusterStart> {
        hume_rope::grapheme::next_cluster(self.0.full_slice(), start)
    }

    /// See [`hume_rope::grapheme::prev_cluster`].
    pub fn prev(self, bound: ClusterBound) -> Option<ClusterStart> {
        hume_rope::grapheme::prev_cluster(self.0.full_slice(), bound)
    }

    /// The text's first cluster.
    pub fn first(self) -> ClusterStart {
        hume_rope::grapheme::first_cluster(self.0.full_slice()).expect("a buffer is never empty")
    }

    /// The text's last cluster: its structural `\n`.
    pub fn last(self) -> ClusterStart {
        hume_rope::grapheme::last_cluster(self.0.full_slice()).expect("a buffer is never empty")
    }

    /// The boundary at the text's end.
    pub fn text_end(self) -> ClusterBound {
        hume_rope::grapheme::text_end(self.0.full_slice())
    }
}

impl<'a> LineView<'a> {
    /// See [`hume_rope::lines::line_start`].
    pub fn start(self, line: ContentLine) -> ClusterStart {
        hume_rope::lines::line_start(self.0.rope(), line)
    }

    /// See [`hume_rope::lines::line_break`].
    pub fn newline(self, line: ContentLine) -> ClusterStart {
        hume_rope::lines::line_break(self.0.rope(), line)
    }

    /// See [`hume_rope::lines::line_range`].
    pub fn range(self, line: ContentLine) -> ClusterRange {
        hume_rope::lines::line_range(self.0.rope(), line)
    }

    /// See [`hume_rope::lines::lines_range`].
    pub fn span(self, first: ContentLine, last: ContentLine) -> ClusterRange {
        hume_rope::lines::lines_range(self.0.rope(), first, last)
    }

    /// See [`hume_rope::lines::line_content_range`].
    pub fn content_range(self, line: ContentLine) -> Option<ClusterRange> {
        hume_rope::lines::line_content_range(self.0.rope(), line)
    }

    /// See [`hume_rope::lines::line_content_end`].
    pub fn content_end(self, line: ContentLine) -> ClusterStart {
        hume_rope::lines::line_content_end(self.0.rope(), line)
    }

    /// See [`hume_rope::lines::next_line_start`].
    pub fn next_start(self, line: RopeyLine) -> CharOffset {
        hume_rope::lines::next_line_start(self.0.rope(), line)
    }

    /// See [`hume_rope::lines::leading_whitespace_end`].
    pub fn indent_end(self, line: ContentLine) -> ClusterStart {
        hume_rope::lines::leading_whitespace_end(self.0.rope(), line)
    }
}

impl<'a> ColumnView<'a> {
    /// See [`hume_rope::lines::leading_indent`].
    pub fn indent(self, line: ContentLine, tab_width: u8) -> (ClusterStart, BufferLineCol) {
        hume_rope::lines::leading_indent(self.0.rope(), line, tab_width)
    }

    /// See [`hume_rope::grapheme::grapheme_col_in_line`].
    pub fn grapheme_col(self, line: ContentLine, char_pos: CharOffset) -> GraphemeCol {
        hume_rope::grapheme::grapheme_col_in_line(self.0.full_slice(), line, char_pos)
    }

    /// See [`hume_rope::lines::char_col_in_line`].
    pub fn char_col(self, line: ContentLine, char_pos: CharOffset) -> CharCol {
        hume_rope::lines::char_col_in_line(self.0.rope(), line, char_pos)
    }

    /// See [`hume_rope::grapheme::display_col_in_line`].
    pub fn display_col(
        self,
        line: ContentLine,
        char_pos: CharOffset,
        tab_width: u8,
    ) -> BufferLineCol {
        hume_rope::grapheme::display_col_in_line(self.0.full_slice(), line, char_pos, tab_width)
    }

    /// See [`hume_rope::grapheme::char_pos_at_display_col`].
    pub fn pos_at_display_col(
        self,
        line: ContentLine,
        target: BufferLineCol,
        tab_width: u8,
    ) -> ClusterStart {
        hume_rope::grapheme::char_pos_at_display_col(self.0.full_slice(), line, target, tab_width)
    }

    /// See [`hume_rope::lines::place_char_column`].
    pub fn place_char(self, line: RopeyLine, col: CharCol) -> ClusterStart {
        hume_rope::lines::place_char_column(self.0.rope(), line, col)
    }

    /// See [`hume_rope::lines::place_grapheme_column`].
    pub fn place_grapheme(self, line: RopeyLine, col: GraphemeCol) -> ClusterStart {
        hume_rope::lines::place_grapheme_column(self.0.rope(), line, col)
    }

    /// See [`hume_rope::lines::char_to_line_byte`].
    pub fn byte_position(self, char_pos: CharOffset) -> (RopeyLine, ByteCol) {
        hume_rope::lines::char_to_line_byte(self.0.rope(), char_pos)
    }

    /// See [`hume_rope::lines::line_segments`].
    pub fn segments(
        self,
        range: ExclusiveRange<CharOffset>,
    ) -> impl Iterator<Item = (ContentLine, ByteCol, ByteCol)> + 'a {
        hume_rope::lines::line_segments(self.0.rope(), range)
    }
}
