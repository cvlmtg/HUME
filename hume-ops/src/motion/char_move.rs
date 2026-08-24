use hume_editing::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use hume_editing::text::BufferText;

// ── Character motions (inner) ─────────────────────────────────────────────────

/// Move one grapheme cluster to the right.
///
/// Clamps to `buf.len_chars() - 1` so the cursor never moves past the
/// trailing `\n` (which is always the last character in the buffer).
pub(super) fn move_right(buf: &BufferText, head: usize) -> usize {
    let next = next_grapheme_boundary(buf, head);
    // len_chars() - 1 is safe: the buffer always has at least one char (\n).
    next.min(buf.len_chars() - 1)
}

/// Move one grapheme cluster to the left.
///
/// Returns `0` when already at the start of the buffer.
pub(super) fn move_left(buf: &BufferText, head: usize) -> usize {
    prev_grapheme_boundary(buf, head)
}

// ── BufferText-level goto motions (inner) ────────────────────────────────────────

/// Jump to the first character of the buffer.
pub(super) fn goto_first_line(_buf: &BufferText, _head: usize) -> usize {
    0
}

/// Jump to the first character of the last (real) line of the buffer.
pub(super) fn goto_last_line(buf: &BufferText, _head: usize) -> usize {
    buf.line_to_char(buf.last_content_line())
}
