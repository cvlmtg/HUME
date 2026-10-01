use hume_editing::text::BufferText;
use hume_rope::cluster::ClusterStart;

// ── Line motions (inner) ──────────────────────────────────────────────────────

/// Jump to the first character on the current line.
pub(super) fn goto_line_start(text: &BufferText, head: ClusterStart) -> ClusterStart {
    text.lines().start(text.char_to_line(head.offset()))
}

/// Jump to the last non-newline grapheme cluster on the current line.
///
/// On an empty line (containing only `\n`), the cursor stays on the newline:
/// there is no other character to land on.
pub(super) fn goto_line_end(text: &BufferText, head: ClusterStart) -> ClusterStart {
    text.lines().content_end(text.char_to_line(head.offset()))
}

/// Jump to the `\n` that terminates the current line.
///
/// Unlike `goto_line_end` (which stops at the last non-newline grapheme and
/// therefore lands on the `\n` itself only on empty lines), this always
/// returns the `\n` position.
pub(super) fn goto_line_newline(text: &BufferText, head: ClusterStart) -> ClusterStart {
    text.lines().newline(text.char_to_line(head.offset()))
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
