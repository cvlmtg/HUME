//! `&Text`-ergonomic wrappers over `hume_rope::lines`'s `&Rope`-based line
//! helpers, plus [`is_line_start`] (needs a [`Selection`], so it stays here).
//! See `hume_rope::lines` for the implementations and detailed doc comments.

use crate::selection::Selection;
use crate::text::Text;

/// Returns `true` if the start of `sel` is the first char of its line (or the
/// buffer start).
///
/// Equivalent to "the char before `sel.start()` is a `\n`, or `sel.start()` is
/// 0", but expressed via line arithmetic — no grapheme-stepping needed.
pub fn is_line_start(buf: &Text, sel: &Selection) -> bool {
    let pos = sel.start();
    let line = buf.char_to_line(pos);
    pos == buf.line_to_char(line)
}

/// See [`hume_rope::lines::line_end_exclusive`].
pub fn line_end_exclusive(buf: &Text, line: usize) -> usize {
    hume_rope::lines::line_end_exclusive(buf.rope(), line)
}

/// See [`hume_rope::lines::line_break_char`].
pub fn line_break_char(buf: &Text, line: usize) -> usize {
    hume_rope::lines::line_break_char(buf.rope(), line)
}

/// See [`hume_rope::lines::leading_whitespace_end`].
pub fn leading_whitespace_end(buf: &Text, line: usize) -> usize {
    hume_rope::lines::leading_whitespace_end(buf.rope(), line)
}

/// See [`hume_rope::lines::snap_to_grapheme_boundary`].
pub fn snap_to_grapheme_boundary(buf: &Text, line_start: usize, target: usize) -> usize {
    hume_rope::lines::snap_to_grapheme_boundary(buf.rope(), line_start, target)
}

/// See [`hume_rope::lines::is_empty_line`].
pub fn is_empty_line(buf: &Text, line: usize) -> bool {
    hume_rope::lines::is_empty_line(buf.rope(), line)
}

/// See [`hume_rope::lines::line_content_end`].
pub fn line_content_end(buf: &Text, line: usize) -> usize {
    hume_rope::lines::line_content_end(buf.rope(), line)
}

/// See [`hume_rope::lines::place_char_column`].
pub fn place_char_column(buf: &Text, line: usize, char_col: usize) -> usize {
    hume_rope::lines::place_char_column(buf.rope(), line, char_col)
}

/// See [`hume_rope::lines::place_display_column`].
pub fn place_display_column(
    buf: &Text,
    line: usize,
    target_display_col: usize,
    tab_width: u8,
) -> usize {
    hume_rope::lines::place_display_column(buf.rope(), line, target_display_col, tab_width)
}

/// See [`hume_rope::lines::char_to_line_byte`].
pub fn char_to_line_byte(buf: &Text, char_pos: usize) -> (usize, usize) {
    hume_rope::lines::char_to_line_byte(buf.rope(), char_pos)
}

/// See [`hume_rope::lines::line_segments`].
pub fn line_segments(
    buf: &Text,
    start: usize,
    end_char_excl: usize,
) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
    hume_rope::lines::line_segments(buf.rope(), start, end_char_excl)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
