use hume_engine::pipeline::EngineView;
use hume_ops::MotionMode;

use super::super::EditorState;
use super::super::visual_move::{VerticalUnit, apply_visual_vertical};
use super::{current_selections, focused_buffer_id, pane_row_map, viewport};
use crate::editor::error::CommandError;

// ── Page / half-page scroll ───────────────────────────────────────────────────
//
// Uses `view.height` (or half of it) as the move count rather than the user's
// numeric prefix. Calls `apply_visual_vertical` directly (not the registry, to
// avoid a runtime string lookup; not the `cmd_visual_move_*` wrappers, since a
// scroll count is always a display-row count, never "N buffer lines").

fn scroll_page(
    state: &mut EditorState,
    view: &mut EngineView,
    mode: MotionMode,
    half: bool,
    down: bool,
) -> Result<(), CommandError> {
    let height = viewport(state, view).height as usize;
    let count = if half { (height / 2).max(1) } else { height };
    apply_visual_vertical(state, view, count, down, mode, VerticalUnit::ScreenRow);
    Ok(())
}

pub(crate) fn cmd_page_down(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    scroll_page(state, view, mode, false, true)
}
pub(crate) fn cmd_page_up(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    scroll_page(state, view, mode, false, false)
}
pub(crate) fn cmd_half_page_down(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    scroll_page(state, view, mode, true, true)
}
pub(crate) fn cmd_half_page_up(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    scroll_page(state, view, mode, true, false)
}

// ── View-trie scroll (z z / z k / z j) ────────────────────────────────────────

fn cmd_view_scroll_to_row(state: &mut EditorState, view: &mut EngineView, target_row: usize) {
    let cursor_char = current_selections(state, view).primary().head();
    let pid = state.focused_pane_id;
    let buf_id = focused_buffer_id(state, view);
    let key = state.format_key(&view.panes[pid]);
    let (mut rm, viewport) = pane_row_map(state.buffers.get(buf_id), &mut view.panes[pid], key);
    super::super::scroll::scroll_cursor_to_row(viewport, &mut rm, cursor_char, target_row);
}

pub(crate) fn cmd_view_center(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let target = (viewport(state, view).height as usize) / 2;
    cmd_view_scroll_to_row(state, view, target);
    Ok(())
}

pub(crate) fn cmd_view_top(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    cmd_view_scroll_to_row(state, view, 0);
    Ok(())
}

pub(crate) fn cmd_view_bottom(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let target = (viewport(state, view).height as usize).saturating_sub(1);
    cmd_view_scroll_to_row(state, view, target);
    Ok(())
}
