//! `&BufferText`-ergonomic wrappers over `hume_rope::lines`'s `&Rope`-based line
//! helpers, plus [`is_line_start`] (needs a [`Selection`], so it stays here).
//! See `hume_rope::lines` for the implementations and detailed doc comments.

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
    pos == text.line_to_char(line)
}

/// See [`hume_rope::lines::line_end_exclusive`].
pub fn line_end_exclusive(text: &BufferText, line: usize) -> usize {
    hume_rope::lines::line_end_exclusive(text.rope(), line)
}

/// See [`hume_rope::lines::line_break_char`].
pub fn line_break_char(text: &BufferText, line: usize) -> usize {
    hume_rope::lines::line_break_char(text.rope(), line)
}

/// See [`hume_rope::lines::leading_whitespace_end`].
pub fn leading_whitespace_end(text: &BufferText, line: usize) -> usize {
    hume_rope::lines::leading_whitespace_end(text.rope(), line)
}

/// See [`hume_rope::lines::leading_indent`].
pub fn leading_indent(text: &BufferText, line: usize, tab_width: u8) -> (usize, usize) {
    hume_rope::lines::leading_indent(text.rope(), line, tab_width)
}

/// See [`hume_rope::lines::is_empty_line`].
pub fn is_empty_line(text: &BufferText, line: usize) -> bool {
    hume_rope::lines::is_empty_line(text.rope(), line)
}

/// See [`hume_rope::lines::line_content_end`].
pub fn line_content_end(text: &BufferText, line: usize) -> usize {
    hume_rope::lines::line_content_end(text.rope(), line)
}

/// See [`hume_rope::lines::char_col_in_line`].
pub fn char_col_in_line(text: &BufferText, line: usize, char_pos: usize) -> usize {
    hume_rope::lines::char_col_in_line(text.rope(), line, char_pos)
}

/// See [`hume_rope::lines::place_char_column`].
pub fn place_char_column(text: &BufferText, line: usize, char_col: usize) -> usize {
    hume_rope::lines::place_char_column(text.rope(), line, char_col)
}

/// See [`hume_rope::lines::place_grapheme_column`].
pub fn place_grapheme_column(text: &BufferText, line: usize, grapheme_col: usize) -> usize {
    hume_rope::lines::place_grapheme_column(text.rope(), line, grapheme_col)
}

/// See [`hume_rope::lines::char_to_line_byte`].
pub fn char_to_line_byte(text: &BufferText, char_pos: usize) -> (usize, usize) {
    hume_rope::lines::char_to_line_byte(text.rope(), char_pos)
}

/// See [`hume_rope::lines::line_segments`].
pub fn line_segments(
    text: &BufferText,
    start: usize,
    end_char_excl: usize,
) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
    hume_rope::lines::line_segments(text.rope(), start, end_char_excl)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
