use super::MotionMode;
use hume_editing::lines::{line_range, lines_range, next_line_start};
use hume_editing::selection::{Facing, Selection, SelectionView};
use hume_editing::state::EditState;

// ── Line selection motions ────────────────────────────────────────────────────

/// Apply `step` up to `count` times, stopping early at a fixed point: every
/// step function here is idempotent once clamped at a buffer edge, so a huge
/// count prefix (e.g. `999999999x`) does O(lines moved) work, not O(count).
fn repeat_motion<'a>(
    sel: SelectionView<'a>,
    count: usize,
    step: impl Fn(SelectionView<'a>) -> Selection,
) -> Selection {
    let mut s = sel;
    for _ in 0..count {
        let next = step(s);
        if next == s.selection() {
            break;
        }
        s = s.with_selection(next);
    }
    s.selection()
}

fn facing(forward: bool) -> Facing {
    if forward {
        Facing::Forward
    } else {
        Facing::Backward
    }
}

/// Extend a linewise selection by one line in extend mode: branches on
/// whether `sel` already covers whole lines.
///
/// If `sel` is not yet linewise, the first press only aligns it to the full
/// lines it touches. The direction is fixed by `forward` (`true` → `x` →
/// forward, `false` → `X` → backward), matching the `Move`-mode identity of
/// each command, regardless of `sel`'s own anchor/head direction.
///
/// Once aligned, each press moves the **head**'s line one line in `forward`'s
/// direction and rebuilds the span between the (unmoved) anchor's line and
/// the new head line. This is what lets a press in the opposite direction
/// shrink the selection back down rather than only ever growing it: the
/// anchor's line is always kept in the span, but the far edge tracks the
/// head. Clamps at the buffer's first or last line are head-relative
/// (checked against the line the head is about to leave), not
/// selection-end-relative: a backward selection whose far edge sits on the
/// last line must still be able to shrink via `x`.
fn extend_line_span(sel: SelectionView<'_>, forward: bool) -> Selection {
    let text = sel.text();
    if !sel.is_linewise() {
        let lines = sel.lines();
        return Selection::covering(lines_range(text, lines.start, lines.end), facing(forward));
    }

    let anchor_line = text.char_to_line(sel.anchor().offset());
    let head_line = sel.head_line();
    let new_head_line = if forward {
        if next_line_start(text, head_line.into()) >= text.end() {
            return sel.selection(); // head already on the last line: clamp
        }
        head_line.advance(1)
    } else {
        if head_line.index() == 0 {
            return sel.selection(); // head already on the first line: clamp
        }
        head_line.retreat_saturating(1)
    };
    Selection::covering(
        lines_range(text, anchor_line, new_head_line),
        facing(anchor_line <= new_head_line),
    )
}

/// One `x` press (`Move` mode): re-anchors to select the full current line,
/// or, if `sel` already covers whole lines, jumps to the next line.
/// Always produces a forward selection. `count` replays this exactly as if
/// `x` were pressed `count` times in a row: it moves, landing on a single
/// line, rather than growing a span (that's `Ctrl-x` / [`extend_line_span`]).
fn move_select_line(sel: SelectionView<'_>) -> Selection {
    let text = sel.text();
    let lines = sel.lines();
    let has_next = next_line_start(text, lines.end.into()) < text.end();
    let target = if sel.is_linewise() && has_next {
        lines.end.advance(1)
    } else {
        lines.start
    };
    Selection::covering(line_range(text, target), Facing::Forward)
}

/// Select or extend to the full line (`x` / `x` in extend mode): branches on `mode`.
///
/// `Move`: replays `move_select_line` `count` times, so `3x` moves to the
/// 3rd line the same way pressing `x` three times would, ending on a single
/// line (not growing to span all of them).
///
/// `Extend`: grows or shrinks toward covering one more line downward, `count`
/// times; see `extend_line_span`.
pub fn cmd_select_line(state: EditState, count: usize, mode: MotionMode) -> EditState {
    state.map(|sel| match mode {
        MotionMode::Move => repeat_motion(sel, count, move_select_line),
        MotionMode::Extend => repeat_motion(sel, count, |s| extend_line_span(s, true)),
    })
}

/// One `X` press (`Move` mode): re-anchors to select the full current line
/// backward (anchor on the trailing `\n`, head on line start), or, if `sel`
/// already covers whole lines, jumps to the previous line. `count`
/// replays this exactly as if `X` were pressed `count` times in a row: it
/// moves, landing on a single line, rather than growing a span (that's
/// `Ctrl-X` / [`extend_line_span`]).
fn move_select_line_backward(sel: SelectionView<'_>) -> Selection {
    let top = sel.lines().start;
    let target = if sel.is_linewise() && top.index() > 0 {
        top.retreat_saturating(1)
    } else {
        top
    };
    Selection::covering(line_range(sel.text(), target), Facing::Backward)
}

/// Select or extend to the full line backward (`X` / `X` in extend mode): branches on `mode`.
///
/// `Move`: replays `move_select_line_backward` `count` times, so `3X`
/// moves to the 3rd line up the same way pressing `X` three times would,
/// ending on a single line (not growing to span all of them).
///
/// `Extend`: grows or shrinks toward covering one more line upward, `count`
/// times; see `extend_line_span`.
pub fn cmd_select_line_backward(state: EditState, count: usize, mode: MotionMode) -> EditState {
    state.map(|sel| match mode {
        MotionMode::Move => repeat_motion(sel, count, move_select_line_backward),
        MotionMode::Extend => repeat_motion(sel, count, |s| extend_line_span(s, false)),
    })
}
