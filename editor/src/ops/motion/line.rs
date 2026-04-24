use crate::core::grapheme::next_grapheme_boundary;
use crate::core::text::Text;
use crate::helpers::{line_content_end, line_end_exclusive, snap_to_grapheme_boundary};

// ── Line motions (inner) ──────────────────────────────────────────────────────

/// Jump to the first character on the current line.
pub(super) fn goto_line_start(buf: &Text, head: usize) -> usize {
    buf.line_to_char(buf.char_to_line(head))
}

/// Jump to the last non-newline grapheme cluster on the current line.
///
/// On an empty line (containing only `\n`), the cursor stays on the newline —
/// there is no other character to land on.
pub(super) fn goto_line_end(buf: &Text, head: usize) -> usize {
    // The core logic lives in helpers::line_content_end, which is also used by
    // selection_cmd.rs — one implementation, two callers.
    line_content_end(buf, buf.char_to_line(head))
}

/// Jump to the first non-blank character on the current line.
///
/// "Blank" means ASCII space or tab. If no non-blank character exists on the
/// line (e.g. a line of only spaces), the motion is a no-op and the cursor
/// stays at its current position.
pub(super) fn goto_first_nonblank(buf: &Text, head: usize) -> usize {
    let line = buf.char_to_line(head);
    let line_start = buf.line_to_char(line);
    let end_excl = line_end_exclusive(buf, line);

    let mut pos = line_start;
    while pos < end_excl {
        match buf.char_at(pos) {
            // Step by grapheme boundary to respect the project invariant even
            // for space/tab (both are always single-codepoint, but be consistent).
            Some(' ') | Some('\t') => pos = next_grapheme_boundary(buf, pos),
            Some('\n') | None => break, // end of line content without finding non-blank
            Some(_) => return pos,      // found a non-blank char
        }
    }
    head // no non-blank found — no-op, matching Helix
}

/// Move the cursor down one line, preserving the char-offset column.
///
/// `preferred_col` overrides the column computed from the current position.
/// Pass `None` to use the current column. A `Some` value supports sticky-column
/// behaviour once the editor layer tracks it.
///
/// **Column model (current simplification):** column is a char offset from line
/// start, not a display column. This is correct for ASCII. When the renderer
/// adds tab/wide-char support, vertical motions will switch to display columns.
pub(super) fn move_down_inner(buf: &Text, head: usize, preferred_col: Option<usize>) -> usize {
    let line = buf.char_to_line(head);
    if line + 1 >= buf.len_lines() {
        return head; // already on the last line
    }

    let col = preferred_col.unwrap_or_else(|| head - buf.line_to_char(line));
    let target_start = buf.line_to_char(line + 1);

    // The phantom trailing line (produced by the structural trailing \n) has
    // target_start == len_chars(). Moving into it would place the cursor past
    // all characters — stay put instead.
    if target_start >= buf.len_chars() {
        return head;
    }

    let target_end = line_end_exclusive(buf, line + 1);
    let target = target_start + col;

    if target >= target_end {
        // Column overshoots the target line — clamp to last char.
        goto_line_end(buf, target_start)
    } else {
        snap_to_grapheme_boundary(buf, target_start, target)
    }
}

/// Move the cursor up one line, preserving the char-offset column.
///
/// See `move_down_inner` for the column model and `preferred_col` semantics.
pub(super) fn move_up_inner(buf: &Text, head: usize, preferred_col: Option<usize>) -> usize {
    let line = buf.char_to_line(head);
    if line == 0 {
        return head; // already on the first line
    }

    let col = preferred_col.unwrap_or_else(|| head - buf.line_to_char(line));
    let target_start = buf.line_to_char(line - 1);
    let target_end = line_end_exclusive(buf, line - 1);
    let target = target_start + col;

    if target >= target_end {
        goto_line_end(buf, target_start)
    } else {
        snap_to_grapheme_boundary(buf, target_start, target)
    }
}
