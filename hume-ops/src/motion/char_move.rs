use hume_editing::grapheme::{next_grapheme_boundary, prev_grapheme_boundary};
use hume_editing::text::BufferText;
use hume_rope::offset::CharOffset;

// ── Character motions (inner) ─────────────────────────────────────────────────

/// Move one grapheme cluster to the right.
///
/// Clamps to `text.last_char()` so the cursor never moves past the
/// trailing `\n` (which is always the last character in the buffer).
pub(super) fn move_right(text: &BufferText, head: CharOffset) -> CharOffset {
    let next = next_grapheme_boundary(text, head);
    next.min(text.last_char())
}

/// Move one grapheme cluster to the left.
///
/// Returns `0` when already at the start of the buffer.
pub(super) fn move_left(text: &BufferText, head: CharOffset) -> CharOffset {
    prev_grapheme_boundary(text, head)
}

// ── BufferText-level goto motions (inner) ────────────────────────────────────────

/// Jump to the first character of the buffer.
pub(super) fn goto_first_line(_buf: &BufferText, _head: CharOffset) -> CharOffset {
    CharOffset::new(0)
}

/// Jump to the first character of the last (real) line of the buffer.
pub(super) fn goto_last_line(text: &BufferText, _head: CharOffset) -> CharOffset {
    text.line_to_char(text.last_content_line().into())
}
