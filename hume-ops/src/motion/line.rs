use hume_editing::text::BufferText;
use hume_rope::cluster::ClusterStart;

use crate::LineEnd;

// ── Line motions (inner) ──────────────────────────────────────────────────────

/// Jump to the first character on the current line.
pub(super) fn goto_line_start(text: &BufferText, head: ClusterStart) -> ClusterStart {
    text.lines().start(text.char_to_line(head.offset()))
}

/// Jump to the end of the current line, where `end` says.
pub(super) fn goto_line_end(text: &BufferText, head: ClusterStart, end: LineEnd) -> ClusterStart {
    let line = text.char_to_line(head.offset());
    match end {
        LineEnd::LastCluster => text.lines().content_end(line),
        LineEnd::LineBreak => text.lines().newline(line),
    }
}

/// Jump to the first non-blank character on the current line.
///
/// If no non-blank character exists on the line (e.g. a line of only
/// spaces), the motion is a no-op and the cursor stays at its current
/// position.
pub(super) fn goto_first_nonblank(text: &BufferText, head: ClusterStart) -> ClusterStart {
    let line = text.char_to_line(head.offset());
    let first = text.lines().indent_end(line);
    if first == text.lines().newline(line) {
        head
    } else {
        first
    }
}
