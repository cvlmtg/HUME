use hume_editing::grapheme::{display_col_in_line, next_grapheme_boundary};
use hume_editing::lines::{
    line_break_char, line_content_end, line_end_exclusive, place_display_column,
};
use hume_editing::text::Text;

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
    // The core logic lives in hume_editing::lines::line_content_end, which is
    // also used by selection_cmd.rs — one implementation, two callers.
    line_content_end(buf, buf.char_to_line(head))
}

/// Jump to the `\n` that terminates the current line.
///
/// Unlike `goto_line_end` (which stops at the last non-newline grapheme and
/// therefore lands on the `\n` itself only on empty lines), this always
/// returns the `\n` position. Used by `cmd_open_line_below` to make the
/// insertion point uniform across empty and non-empty lines.
pub(super) fn goto_line_newline(buf: &Text, head: usize) -> usize {
    let line = buf.char_to_line(head);
    line_break_char(buf, line)
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

/// Place the cursor on `target_line` at the display column `head` occupies on
/// `line` — the shared tail of `move_down_inner`/`move_up_inner`, which differ
/// only in which neighboring line they target and its own boundary check.
/// `line` is `head`'s own line, passed in rather than re-derived since both
/// callers already have it for that check.
///
/// **Column model:** display column — tab-aware and unicode-width-aware, via
/// `place_display_column`/`display_col_in_line`. Matches
/// `editor::visual_move::move_vertical`'s model, which bare `j`/`k` (and
/// page/half-page scroll, the mouse wheel) use.
///
/// The column is re-derived from `head` on every step rather than carried
/// across a repeat count, so `9j` through a short line resumes from that
/// line's own column. Bare `j` pressed nine times keeps the original column
/// instead, because the sticky column lives on the `Selection`
/// (`sticky_display_col`) where a multi-step motion can't see it.
fn to_line_keeping_display_col(
    buf: &Text,
    head: usize,
    line: usize,
    target_line: usize,
    tab_width: u8,
) -> usize {
    let display_col = display_col_in_line(buf, line, head, tab_width);
    place_display_column(buf, target_line, display_col, tab_width)
}

/// Move the cursor down one line, preserving the display column.
///
/// This function is reached only by an explicit numeric prefix (`9j`), which
/// counts buffer lines to match relative-line-number gutters even while
/// wrapping, and by direct/proptest callers of the pure `cmd_move_down` op.
pub(super) fn move_down_inner(buf: &Text, head: usize, tab_width: u8) -> usize {
    let line = buf.char_to_line(head);
    // On the last content line, line + 1 is the phantom trailing line (the
    // structural \n) — nothing to land on there, so stay put.
    if line >= buf.last_content_line() {
        return head;
    }
    to_line_keeping_display_col(buf, head, line, line + 1, tab_width)
}

/// Move the cursor up one line, preserving the display column.
///
/// See `move_down_inner` for the column model.
pub(super) fn move_up_inner(buf: &Text, head: usize, tab_width: u8) -> usize {
    let line = buf.char_to_line(head);
    if line == 0 {
        return head; // already on the first line
    }
    to_line_keeping_display_col(buf, head, line, line - 1, tab_width)
}
