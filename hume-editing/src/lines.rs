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

/// See [`hume_rope::line_end_exclusive`].
pub fn line_end_exclusive(buf: &Text, line: usize) -> usize {
    hume_rope::line_end_exclusive(buf.rope(), line)
}

/// See [`hume_rope::leading_whitespace_end`].
pub fn leading_whitespace_end(buf: &Text, line: usize) -> usize {
    hume_rope::leading_whitespace_end(buf.rope(), line)
}

/// See [`hume_rope::snap_to_grapheme_boundary`].
pub fn snap_to_grapheme_boundary(buf: &Text, line_start: usize, target: usize) -> usize {
    hume_rope::snap_to_grapheme_boundary(buf.rope(), line_start, target)
}

/// See [`hume_rope::is_empty_line`].
pub fn is_empty_line(buf: &Text, line: usize) -> bool {
    hume_rope::is_empty_line(buf.rope(), line)
}

/// See [`hume_rope::line_content_end`].
pub fn line_content_end(buf: &Text, line: usize) -> usize {
    hume_rope::line_content_end(buf.rope(), line)
}

/// See [`hume_rope::place_column`].
pub fn place_column(buf: &Text, line: usize, col: usize) -> usize {
    hume_rope::place_column(buf.rope(), line, col)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
