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
pub fn is_line_start(buf: &BufferText, sel: &Selection) -> bool {
    let pos = sel.start();
    let line = buf.char_to_line(pos);
    pos == buf.line_to_char(line)
}

/// See [`hume_rope::lines::line_end_exclusive`].
pub fn line_end_exclusive(buf: &BufferText, line: usize) -> usize {
    hume_rope::lines::line_end_exclusive(buf.rope(), line)
}

/// See [`hume_rope::lines::line_break_char`].
pub fn line_break_char(buf: &BufferText, line: usize) -> usize {
    hume_rope::lines::line_break_char(buf.rope(), line)
}

/// See [`hume_rope::lines::leading_whitespace_end`].
pub fn leading_whitespace_end(buf: &BufferText, line: usize) -> usize {
    hume_rope::lines::leading_whitespace_end(buf.rope(), line)
}

/// See [`hume_rope::lines::is_empty_line`].
pub fn is_empty_line(buf: &BufferText, line: usize) -> bool {
    hume_rope::lines::is_empty_line(buf.rope(), line)
}

/// See [`hume_rope::lines::line_content_end`].
pub fn line_content_end(buf: &BufferText, line: usize) -> usize {
    hume_rope::lines::line_content_end(buf.rope(), line)
}

/// See [`hume_rope::lines::char_col_in_line`].
pub fn char_col_in_line(buf: &BufferText, line: usize, char_pos: usize) -> usize {
    hume_rope::lines::char_col_in_line(buf.rope(), line, char_pos)
}

/// See [`hume_rope::lines::place_char_column`].
pub fn place_char_column(buf: &BufferText, line: usize, char_col: usize) -> usize {
    hume_rope::lines::place_char_column(buf.rope(), line, char_col)
}

/// See [`hume_rope::lines::char_to_line_byte`].
pub fn char_to_line_byte(buf: &BufferText, char_pos: usize) -> (usize, usize) {
    hume_rope::lines::char_to_line_byte(buf.rope(), char_pos)
}

/// See [`hume_rope::lines::line_segments`].
pub fn line_segments(
    buf: &BufferText,
    start: usize,
    end_char_excl: usize,
) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
    hume_rope::lines::line_segments(buf.rope(), start, end_char_excl)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
