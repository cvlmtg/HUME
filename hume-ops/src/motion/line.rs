use hume_editing::grapheme::{display_col_in_line, next_grapheme_boundary};
use hume_editing::lines::{
    line_break_char, line_content_end, line_end_exclusive, place_display_column,
};
use hume_editing::selection::{DisplayColOrigin, Selection, SelectionSet, StickyDisplayCol};
use hume_editing::text::Text;

use super::MotionMode;

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

/// Move or extend every selection by `count` buffer lines, preserving the
/// display column across the whole hop — the buffer-line counterpart of
/// `editor::visual_move::move_vertical`, which does the same for display
/// rows. This function is reached only by an explicit numeric prefix (`9j`),
/// which counts buffer lines to match relative-line-number gutters even
/// while wrapping, and by direct/proptest callers of `cmd_move_down`/
/// `cmd_move_up`.
///
/// Hand-written rather than folded through `apply_motion`: it needs the whole
/// `Selection` (to read and write `sticky_display_col`), not just a head, and
/// it computes `target_line` directly instead of hopping one line at a time —
/// `999999999j` would otherwise call `place_display_column` a billion times
/// for a motion that provably lands on the buffer's first or last line.
///
/// **Column model:** display column, tab-aware and unicode-width-aware via
/// `place_display_column`/`display_col_in_line`. Reuses a `BufferLine`-tagged
/// latch from the incoming selection when present, so a run of `9j`/`9k` (or
/// `j` then `2j` with wrapping off, where a `BufferLine` latch already *is*
/// the row latch — see [`DisplayColOrigin`]) doesn't lose the column to a
/// short line partway through; a `DisplayRow`-tagged latch is ignored and the
/// column re-derived from `head`, since it's a different quantity under wrap.
/// The result always latches `BufferLine`.
pub(super) fn move_vertical_buffer_line(
    buf: &Text,
    sels: SelectionSet,
    down: bool,
    count: usize,
    mode: MotionMode,
    tab_width: u8,
) -> SelectionSet {
    let last_line = buf.last_content_line();
    let result = sels.map(|sel| {
        let head = sel.head();
        let line = buf.char_to_line(head);
        let display_col = match sel.sticky_display_col() {
            Some(StickyDisplayCol {
                display_col,
                origin: DisplayColOrigin::BufferLine,
            }) => display_col as usize,
            _ => display_col_in_line(buf, line, head, tab_width),
        };
        let target_line = if down {
            // On the last content line, line + 1 is the phantom trailing
            // line (the structural \n) — clamp there is nothing past it.
            line.saturating_add(count).min(last_line)
        } else {
            line.saturating_sub(count)
        };
        let new_head = if target_line == line {
            head // already at the document's first/last content line
        } else {
            place_display_column(buf, target_line, display_col, tab_width)
        };
        let anchor = match mode {
            MotionMode::Move => new_head,
            MotionMode::Extend => sel.anchor(),
        };
        // Saturate rather than `as`: a silent wrap-around past u32::MAX
        // display columns would misreport the latch, where clamping to it
        // only makes an already-absurd column landing sticky at the cap.
        let latch_display_col = u32::try_from(display_col).unwrap_or(u32::MAX);
        Selection::with_sticky_display_col(
            anchor,
            new_head,
            StickyDisplayCol {
                display_col: latch_display_col,
                origin: DisplayColOrigin::BufferLine,
            },
        )
    });
    result.debug_assert_valid(buf);
    result
}
