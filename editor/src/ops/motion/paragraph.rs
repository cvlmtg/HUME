use crate::core::text::Text;
use crate::helpers::line_end_exclusive;

// ── Paragraph motion helpers ─────────────────────────────────────────────────

/// Returns `true` if `line` is an empty line — either zero chars or exactly
/// one newline. Whitespace-only lines are NOT empty (matching Helix semantics).
pub(super) fn is_empty_line(buf: &Text, line: usize) -> bool {
    let start = buf.line_to_char(line);
    let end = line_end_exclusive(buf, line);
    // Zero chars (last line of an empty buffer) or exactly one '\n'.
    end == start || (end == start + 1 && buf.char_at(start) == Some('\n'))
}

// ── Paragraph motions (inner) ─────────────────────────────────────────────────

/// Move to the start of the next paragraph (`]p`).
///
/// Two-phase forward scan:
/// 1. Skip non-empty lines (the current paragraph).
/// 2. Skip empty lines (the gap after the paragraph).
///
/// Lands on the first char of the next paragraph, or `len_chars()` if there is
/// no paragraph below (EOF). At EOF already: no-op.
pub(super) fn next_paragraph(buf: &Text, head: usize) -> usize {
    let mut line = buf.char_to_line(head);
    let total = buf.len_lines();

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
pub(super) fn prev_paragraph(buf: &Text, head: usize) -> usize {
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
