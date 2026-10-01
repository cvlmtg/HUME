use hume_editing::text::BufferText;
use hume_rope::cluster::ClusterStart;

// ── Character motions (inner) ─────────────────────────────────────────────────

/// Move one grapheme cluster to the right; the structural `\n` is as far as
/// it goes.
pub(super) fn move_right(text: &BufferText, head: ClusterStart) -> ClusterStart {
    text.clusters().next(head).unwrap_or(head)
}

/// Move one grapheme cluster to the left; the buffer start is as far as it
/// goes.
pub(super) fn move_left(text: &BufferText, head: ClusterStart) -> ClusterStart {
    text.clusters().prev(head.into()).unwrap_or(head)
}

// ── BufferText-level goto motions (inner) ────────────────────────────────────────

/// Jump to the first character of the buffer.
pub(super) fn goto_first_line(text: &BufferText, _head: ClusterStart) -> ClusterStart {
    text.clusters().first()
}

/// Jump to the first character of the last (real) line of the buffer.
pub(super) fn goto_last_line(text: &BufferText, _head: ClusterStart) -> ClusterStart {
    text.lines().start(text.last_content_line())
}
