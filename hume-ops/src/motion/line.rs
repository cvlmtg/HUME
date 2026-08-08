use hume_editing::grapheme::next_grapheme_boundary;
use hume_editing::lines::{line_content_end, line_end_exclusive, place_column};
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
/// returns the `\n` position. The buffer invariant guarantees every line —
/// including the last — ends with `\n`, so `line_end_exclusive - 1` is always
/// valid. Used by `cmd_open_line_below` to make the insertion point uniform
/// across empty and non-empty lines.
pub(super) fn goto_line_newline(buf: &Text, head: usize) -> usize {
    let line = buf.char_to_line(head);
    line_end_exclusive(buf, line) - 1
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
/// Pass `None` to use the current column.
///
/// **Column model:** column is a char offset from line start, not a display
/// column — correct for ASCII, wrong for tabs/wide chars. Interactive `j`/`k`
/// (and page/half-page scroll, the mouse wheel) never reach this: they go
/// through `editor::visual_move::move_vertical`'s display-column model
/// instead. This function is now reached only by an explicit numeric prefix
/// (`9j`), which counts buffer lines to match relative-line-number gutters,
/// and by direct/proptest callers of the pure `cmd_move_down` op.
pub(super) fn move_down_inner(buf: &Text, head: usize, preferred_col: Option<usize>) -> usize {
    let line = buf.char_to_line(head);
    // On the last content line, line + 1 is the phantom trailing line (the
    // structural \n) — nothing to land on there, so stay put.
    if line >= buf.last_content_line() {
        return head;
    }

    let col = preferred_col.unwrap_or_else(|| head - buf.line_to_char(line));
    place_column(buf, line + 1, col)
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
    place_column(buf, line - 1, col)
}
