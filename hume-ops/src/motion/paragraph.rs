use hume_editing::lines::is_empty_line;
use hume_editing::text::BufferText;

// ── Paragraph motions (inner) ─────────────────────────────────────────────────

/// Move to the start of the next paragraph (`]p`).
///
/// Two-phase forward scan:
/// 1. Skip non-empty lines (the current paragraph).
/// 2. Skip empty lines (the gap after the paragraph).
///
/// Lands on the first char of the next paragraph, or the buffer's last valid
/// position (the structural trailing `\n`) if there is no paragraph below
/// (EOF). At EOF already: no-op.
pub(super) fn next_paragraph(buf: &BufferText, head: usize) -> usize {
    let mut line = buf.char_to_line(head);
    // Deliberately the ropey count, not content_line_count(): the phantom
    // trailing line is empty like any other (is_empty_line agrees), so the
    // scan is allowed to walk onto it and Phase 2 swallows it as part of the
    // gap — `line >= total` below then lands on the trailing \n uniformly,
    // with no separate EOF branch needed.
    let total = buf.ropey_line_count();

    // Phase 1: skip the current paragraph (non-empty lines).
    while line < total && !is_empty_line(buf, line) {
        line += 1;
    }
    // Phase 2: skip the gap (empty lines).
    while line < total && is_empty_line(buf, line) {
        line += 1;
    }

    if line >= total {
        // No paragraph below — land on the trailing \n (last valid position).
        // len_chars() - 1 is safe: every buffer has at least one char.
        buf.len_chars() - 1
    } else {
        buf.line_to_char(line)
    }
}

/// Move to the first empty line above the current paragraph (`[p`).
///
/// Three-phase backward scan:
/// 1. Skip empty lines backward (if already in a gap — jump over it).
/// 2. Skip non-empty lines backward (the current paragraph).
/// 3. Scan to the TOP of the gap above (in case there are multiple empty lines).
///
/// Lands on the first (topmost) empty line of the gap above, or line 0 if
/// there is no paragraph above. At line 0 already: no-op.
pub(super) fn prev_paragraph(buf: &BufferText, head: usize) -> usize {
    let mut line = buf.char_to_line(head);

    // Phase 1: skip empty lines backward (handles starting inside a gap).
    while line > 0 && is_empty_line(buf, line) {
        line -= 1;
    }
    // Phase 2: skip non-empty lines backward (current paragraph).
    while line > 0 && !is_empty_line(buf, line) {
        line -= 1;
    }
    // Phase 3: scan to the top of the gap — there may be multiple empty lines.
    while line > 0 && is_empty_line(buf, line - 1) {
        line -= 1;
    }

    buf.line_to_char(line)
}
