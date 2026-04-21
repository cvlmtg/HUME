use crate::core::text::Text;
use crate::core::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};

// ── Character motions (inner) ─────────────────────────────────────────────────

/// Move one grapheme cluster to the right.
///
/// Clamps to `buf.len_chars() - 1` so the cursor never moves past the
/// trailing `\n` (which is always the last character in the buffer).
pub(super) fn move_right(buf: &Text, head: usize) -> usize {
    let next = next_grapheme_boundary(buf, head);
    // len_chars() - 1 is safe: the buffer always has at least one char (\n).
    next.min(buf.len_chars() - 1)
}

/// Move one grapheme cluster to the left.
///
/// Returns `0` when already at the start of the buffer.
pub(super) fn move_left(buf: &Text, head: usize) -> usize {
    prev_grapheme_boundary(buf, head)
}

// ── Text-level goto motions (inner) ────────────────────────────────────────

/// Jump to the first character of the buffer.
pub(super) fn goto_first_line(_buf: &Text, _head: usize) -> usize {
    0
}

/// Jump to the first character of the last (real) line of the buffer.
///
/// `ropey`'s `len_lines()` counts the empty "ghost" line that follows every
/// trailing `\n`, so the last content line is always at index `len_lines() - 2`.
/// For the minimal buffer (`"\n"`) that yields index 0, which is correct.
pub(super) fn goto_last_line(buf: &Text, _head: usize) -> usize {
    let last_line = buf.len_lines().saturating_sub(2);
    buf.line_to_char(last_line)
}
