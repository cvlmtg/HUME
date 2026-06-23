use super::MotionMode;
use hume_editing::lines::{is_line_start, line_end_exclusive};
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::Text;

// ── Line selection motions ────────────────────────────────────────────────────

/// Select or extend to the full line (`x` / `x` in extend mode): branches on `mode`.
///
/// `Move` — re-anchors: selects from line start to the trailing `\n`. If the
/// selection already ends on a `\n`, jumps to the next line. Always produces a
/// forward selection.
///
/// `Extend` — grows the selection to cover the current line. If the selection
/// already ends on a `\n`, accumulates the next line instead. Always produces a
/// forward selection (anchor=start, head=`\n`).
pub(crate) fn cmd_select_line(buf: &Text, sels: SelectionSet, mode: MotionMode) -> SelectionSet {
    let result = sels.map(|sel| {
        match mode {
            MotionMode::Move => {
                let bottom_line = buf.char_to_line(sel.end());
                let end_excl = line_end_exclusive(buf, bottom_line);
                // If selection already ends on the trailing `\n`, jump to the next line.
                let target_line = if sel.ends_on_newline(buf) && end_excl < buf.len_chars() {
                    bottom_line + 1
                } else {
                    buf.char_to_line(sel.start())
                };
                let start = buf.line_to_char(target_line);
                let end = line_end_exclusive(buf, target_line) - 1; // inclusive `\n`
                Selection::new(start, end)
            }
            MotionMode::Extend => {
                let bottom_line = buf.char_to_line(sel.end());
                let end_excl = line_end_exclusive(buf, bottom_line);
                if sel.ends_on_newline(buf) && end_excl >= buf.len_chars() {
                    return sel; // already at last line — clamp
                }
                let (tgt_start, tgt_end) = if sel.ends_on_newline(buf) {
                    // Already ends on `\n` — target next line.
                    let next_line = bottom_line + 1;
                    (
                        buf.line_to_char(next_line),
                        line_end_exclusive(buf, next_line) - 1,
                    )
                } else {
                    // Expand to cover full lines.
                    let top_line = buf.char_to_line(sel.start());
                    (buf.line_to_char(top_line), end_excl - 1)
                };
                let new_start = sel.start().min(tgt_start);
                let new_end = sel.end().max(tgt_end);
                Selection::new(new_start, new_end) // always forward
            }
        }
    });
    result.debug_assert_valid(buf);
    result
}

/// Select or extend to the full line backward (`X` / `X` in extend mode): branches on `mode`.
///
/// `Move` — re-anchors: anchor on the trailing `\n`, head on line start. If the
/// selection already starts at a line boundary, jumps to the previous line.
///
/// `Extend` — grows the selection upward to cover the current line. If the selection
/// already starts at a line boundary, accumulates the previous line. Always produces
/// a backward selection (anchor=bottom `\n`, head=top start).
pub(crate) fn cmd_select_line_backward(
    buf: &Text,
    sels: SelectionSet,
    mode: MotionMode,
) -> SelectionSet {
    let result = sels.map(|sel| {
        match mode {
            MotionMode::Move => {
                let top_line = buf.char_to_line(sel.start());
                // If selection already starts at line start, jump to previous line.
                let target_line = if is_line_start(buf, &sel) && top_line > 0 {
                    top_line - 1
                } else {
                    top_line
                };
                let start = buf.line_to_char(target_line);
                let end = line_end_exclusive(buf, target_line) - 1; // inclusive `\n`
                Selection::new(end, start) // backward: anchor=`\n`, head=line_start
            }
            MotionMode::Extend => {
                let top_line = buf.char_to_line(sel.start());
                if is_line_start(buf, &sel) && top_line == 0 {
                    return sel; // already at first line — clamp
                }
                let (tgt_start, tgt_end) = if is_line_start(buf, &sel) {
                    // Already starts at line boundary — target previous line.
                    let prev_line = top_line - 1;
                    (
                        buf.line_to_char(prev_line),
                        line_end_exclusive(buf, prev_line) - 1,
                    )
                } else {
                    // Expand to cover full lines.
                    let bottom_line = buf.char_to_line(sel.end());
                    (
                        buf.line_to_char(top_line),
                        line_end_exclusive(buf, bottom_line) - 1,
                    )
                };
                let new_start = sel.start().min(tgt_start);
                let new_end = sel.end().max(tgt_end);
                Selection::new(new_end, new_start) // backward: anchor=bottom, head=top
            }
        }
    });
    result.debug_assert_valid(buf);
    result
}
