use crate::ops::MotionMode;

use super::super::visual_move::{cmd_visual_move_down, cmd_visual_move_up};
use super::super::Editor;
use crate::editor::error::CommandError;

// ── Page / half-page scroll ───────────────────────────────────────────────────
//
// Uses `view.height` (or half of it) as the move count rather than the user's
// numeric prefix. Calls the visual-move commands directly instead of going
// through the registry to avoid a runtime string lookup.

pub fn cmd_page_down(
    ed: &mut Editor,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let count = ed.viewport().height as usize;
    cmd_visual_move_down(ed, count, mode)
}
pub fn cmd_page_up(
    ed: &mut Editor,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let count = ed.viewport().height as usize;
    cmd_visual_move_up(ed, count, mode)
}
pub fn cmd_half_page_down(
    ed: &mut Editor,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let count = (ed.viewport().height as usize / 2).max(1);
    cmd_visual_move_down(ed, count, mode)
}
pub fn cmd_half_page_up(
    ed: &mut Editor,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let count = (ed.viewport().height as usize / 2).max(1);
    cmd_visual_move_up(ed, count, mode)
}

// ── View-trie scroll (zz / zt / zb) ───────────────────────────────────────────
//
// These commands scroll the viewport so the primary selection head lands at a
// chosen display row. Cursor position is unchanged — only `viewport.top_line`
// (and `top_row_offset` in wrap mode) move. On the next frame
// `scroll_into_view` in `prepare_frame` re-applies `scrolloff`: `zz` lands
// inside `[margin, height - margin)` and survives untouched, while `zt`/`zb`
// target row 0 / `height-1` and get trimmed inward by `margin` rows. The
// trim is vim's "smart scrolloff" behaviour and is intentional.

fn cmd_view_scroll_to_row(ed: &mut Editor, target_row: usize) {
    let cursor_char = ed.current_selections().primary().head();
    let (wrap_mode, tab_width, whitespace) = ed.focused_format_context();
    let rope = ed.doc().text().rope().clone();
    let pane = &mut ed.engine_view.panes[ed.focused_pane_id];
    super::super::scroll::scroll_cursor_to_row(
        &mut pane.viewport,
        &rope,
        cursor_char,
        &wrap_mode,
        tab_width,
        &whitespace,
        &mut ed.motion_format_scratch,
        target_row,
    );
}

pub fn cmd_view_center(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let target = (ed.viewport().height as usize) / 2;
    cmd_view_scroll_to_row(ed, target);
    Ok(())
}

pub fn cmd_view_top(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    cmd_view_scroll_to_row(ed, 0);
    Ok(())
}

pub fn cmd_view_bottom(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let target = (ed.viewport().height as usize).saturating_sub(1);
    cmd_view_scroll_to_row(ed, target);
    Ok(())
}
