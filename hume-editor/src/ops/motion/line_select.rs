use super::MotionMode;
use hume_editing::lines::{is_line_start, line_end_exclusive};
use hume_editing::selection::{Selection, SelectionSet, is_selection_linewise};
use hume_editing::text::Text;

// ── Line selection motions ────────────────────────────────────────────────────

/// Extend a linewise selection by one line in extend mode: branches on
/// whether `sel` already covers whole lines.
///
/// If `sel` is not yet linewise, the first press only aligns it to the full
/// lines it touches — the direction is fixed by `delta`'s sign (`+1` → `x` →
/// forward, `-1` → `X` → backward), matching the `Move`-mode identity of
/// each command, regardless of `sel`'s own anchor/head direction.
///
/// Once aligned, each press moves the **head**'s line by `delta` and rebuilds
/// the span between the (unmoved) anchor's line and the new head line. This
/// is what lets a press in the opposite direction shrink the selection back
/// down rather than only ever growing it: the anchor's line is always kept in
/// the span, but the far edge tracks the head. Clamps at the buffer's first
/// or last line are head-relative (checked against the line the head is
/// about to leave), not selection-end-relative — a backward selection whose
/// far edge sits on the last line must still be able to shrink via `x`.
fn extend_line_span(buf: &Text, sel: Selection, delta: isize) -> Selection {
    if !is_selection_linewise(buf, &sel) {
        let top_line = buf.char_to_line(sel.start());
        let bottom_line = buf.char_to_line(sel.end());
        let end = line_end_exclusive(buf, bottom_line) - 1;
        return Selection::directed(buf.line_to_char(top_line), end, delta > 0);
    }

    let anchor_line = buf.char_to_line(sel.anchor());
    let head_line = buf.char_to_line(sel.head());
    if delta > 0 {
        if line_end_exclusive(buf, head_line) >= buf.len_chars() {
            return sel; // head already on the last line — clamp
        }
    } else if head_line == 0 {
        return sel; // head already on the first line — clamp
    }
    let new_head_line = (head_line as isize + delta) as usize;

    let lo = anchor_line.min(new_head_line);
    let hi = anchor_line.max(new_head_line);
    let end = line_end_exclusive(buf, hi) - 1;
    Selection::directed(buf.line_to_char(lo), end, anchor_line <= new_head_line)
}

/// Select or extend to the full line (`x` / `x` in extend mode): branches on `mode`.
///
/// `Move` — re-anchors: selects from line start to the trailing `\n`. If the
/// selection already ends on a `\n`, jumps to the next line. Always produces a
/// forward selection.
///
/// `Extend` — grows or shrinks toward covering one more line downward; see
/// [`extend_line_span`].
pub(crate) fn cmd_select_line(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    let result = sels.map(|sel| match mode {
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
        MotionMode::Extend => extend_line_span(buf, sel, 1),
    });
    result.debug_assert_valid(buf);
    result
}

/// Select or extend to the full line backward (`X` / `X` in extend mode): branches on `mode`.
///
/// `Move` — re-anchors: anchor on the trailing `\n`, head on line start. If the
/// selection already starts at a line boundary, jumps to the previous line.
///
/// `Extend` — grows or shrinks toward covering one more line upward; see
/// [`extend_line_span`].
pub(crate) fn cmd_select_line_backward(
    buf: &Text,
    sels: SelectionSet,
    _count: usize,
    mode: MotionMode,
) -> SelectionSet {
    let result = sels.map(|sel| match mode {
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
        MotionMode::Extend => extend_line_span(buf, sel, -1),
    });
    result.debug_assert_valid(buf);
    result
}
