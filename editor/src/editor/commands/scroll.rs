use engine::pipeline::EngineView;
use crate::ops::MotionMode;

use super::super::visual_move::{cmd_visual_move_down, cmd_visual_move_up};
use super::super::EditorState;
use crate::editor::error::CommandError;
use super::{current_selections, focused_format_context, viewport};

// ── Page / half-page scroll ───────────────────────────────────────────────────
//
// Uses `view.height` (or half of it) as the move count rather than the user's
// numeric prefix. Calls the visual-move commands directly instead of going
// through the registry to avoid a runtime string lookup.

pub fn cmd_page_down(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let count = viewport(state, view).height as usize;
    cmd_visual_move_down(state, view, count, mode)
}
pub fn cmd_page_up(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let count = viewport(state, view).height as usize;
    cmd_visual_move_up(state, view, count, mode)
}
pub fn cmd_half_page_down(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let count = (viewport(state, view).height as usize / 2).max(1);
    cmd_visual_move_down(state, view, count, mode)
}
pub fn cmd_half_page_up(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let count = (viewport(state, view).height as usize / 2).max(1);
    cmd_visual_move_up(state, view, count, mode)
}

// ── View-trie scroll (zz / zt / zb) ───────────────────────────────────────────

fn cmd_view_scroll_to_row(state: &mut EditorState, view: &mut EngineView, target_row: usize) {
    let cursor_char = current_selections(state, view).primary().head();
    let (wrap_mode, tab_width, whitespace) = focused_format_context(state, view);
    let rope = state.buffers.get(view.panes[state.focused_pane_id].buffer_id).text().rope().clone();
    let pane = &mut view.panes[state.focused_pane_id];
    super::super::scroll::scroll_cursor_to_row(
        &mut pane.viewport,
        &rope,
        cursor_char,
        &wrap_mode,
        tab_width,
        &whitespace,
        &mut state.motion_format_scratch,
        target_row,
    );
}

pub fn cmd_view_center(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let target = (viewport(state, view).height as usize) / 2;
    cmd_view_scroll_to_row(state, view, target);
    Ok(())
}

pub fn cmd_view_top(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    cmd_view_scroll_to_row(state, view, 0);
    Ok(())
}

pub fn cmd_view_bottom(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let target = (viewport(state, view).height as usize).saturating_sub(1);
    cmd_view_scroll_to_row(state, view, target);
    Ok(())
}
