use hume_editing::grapheme::{first_cluster, next_cluster, prev_cluster};
use hume_editing::lines::line_start;
use hume_editing::text::BufferText;
use hume_rope::cluster::ClusterStart;

// ── Character motions (inner) ─────────────────────────────────────────────────

/// Move one grapheme cluster to the right; the structural `\n` is as far as
/// it goes.
pub(super) fn move_right(text: &BufferText, head: ClusterStart) -> ClusterStart {
    next_cluster(text, head).unwrap_or(head)
}

/// Move one grapheme cluster to the left; the buffer start is as far as it
/// goes.
pub(super) fn move_left(text: &BufferText, head: ClusterStart) -> ClusterStart {
    prev_cluster(text, head.into()).unwrap_or(head)
}

// ── BufferText-level goto motions (inner) ────────────────────────────────────────

/// Jump to the first character of the buffer.
pub(super) fn goto_first_line(text: &BufferText, _head: ClusterStart) -> ClusterStart {
    first_cluster(text)
}

/// Jump to the first character of the last (real) line of the buffer.
pub(super) fn goto_last_line(text: &BufferText, _head: ClusterStart) -> ClusterStart {
    line_start(text, text.last_content_line())
}
