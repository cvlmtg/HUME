//! `&BufferText`-ergonomic wrappers over `hume_rope::lines`'s `&Rope`-based line
//! helpers, plus [`is_line_start`] (needs a [`Selection`], so it stays here).
//! See `hume_rope::lines` for the implementations and detailed doc comments.

use hume_rope::column::{BufferLineCol, ByteCol, CharCol, GraphemeCol};
use hume_rope::line::{ContentLine, RopeyLine};
use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::selection::Selection;
use crate::text::BufferText;

/// Returns `true` if the start of `sel` is the first char of its line (or the
/// buffer start).
///
/// Equivalent to "the char before `sel.start()` is a `\n`, or `sel.start()` is
/// 0", but expressed via line arithmetic — no grapheme-stepping needed.
pub fn is_line_start(text: &BufferText, sel: &Selection) -> bool {
    let pos = sel.start();
    let line = text.char_to_line(pos);
    pos == text.line_to_char(line.into())
}

/// See [`hume_rope::lines::next_line_start`].
pub fn next_line_start(text: &BufferText, line: RopeyLine) -> CharOffset {
    hume_rope::lines::next_line_start(text.rope(), line)
}

/// See [`hume_rope::lines::line_break_char`].
pub fn line_break_char(text: &BufferText, line: ContentLine) -> CharOffset {
    hume_rope::lines::line_break_char(text.rope(), line)
}

/// See [`hume_rope::lines::leading_whitespace_end`].
pub fn leading_whitespace_end(text: &BufferText, line: ContentLine) -> CharOffset {
    hume_rope::lines::leading_whitespace_end(text.rope(), line)
}

/// See [`hume_rope::lines::leading_indent`].
pub fn leading_indent(
    text: &BufferText,
    line: ContentLine,
    tab_width: u8,
) -> (CharOffset, BufferLineCol) {
    hume_rope::lines::leading_indent(text.rope(), line, tab_width)
}

/// See [`hume_rope::lines::is_empty_line`].
pub fn is_empty_line(text: &BufferText, line: RopeyLine) -> bool {
    hume_rope::lines::is_empty_line(text.rope(), line)
}

/// See [`hume_rope::lines::line_content_end`].
pub fn line_content_end(text: &BufferText, line: ContentLine) -> CharOffset {
    hume_rope::lines::line_content_end(text.rope(), line)
}

/// See [`hume_rope::lines::line_last_char`].
pub fn line_last_char(text: &BufferText, line: ContentLine) -> CharOffset {
    hume_rope::lines::line_last_char(text.rope(), line)
}

/// See [`hume_rope::lines::char_col_in_line`].
pub fn char_col_in_line(text: &BufferText, line: ContentLine, char_pos: CharOffset) -> CharCol {
    hume_rope::lines::char_col_in_line(text.rope(), line, char_pos)
}

/// See [`hume_rope::lines::place_char_column`].
pub fn place_char_column(text: &BufferText, line: RopeyLine, char_col: CharCol) -> CharOffset {
    hume_rope::lines::place_char_column(text.rope(), line, char_col)
}

/// See [`hume_rope::lines::place_grapheme_column`].
pub fn place_grapheme_column(
    text: &BufferText,
    line: RopeyLine,
    grapheme_col: GraphemeCol,
) -> CharOffset {
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
