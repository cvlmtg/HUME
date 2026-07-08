use crate::ops::MotionMode;
use hume_engine::pipeline::EngineView;

use super::super::EditorState;
use super::super::visual_move::apply_visual_vertical;
use super::{current_selections, focused_buffer_id, focused_format_context, viewport};
use crate::editor::error::CommandError;

// ── Page / half-page scroll ───────────────────────────────────────────────────
//
// Uses `view.height` (or half of it) as the move count rather than the user's
// numeric prefix. Calls `apply_visual_vertical` directly (not the registry, to
// avoid a runtime string lookup; not the `cmd_visual_move_*` wrappers, since a
// scroll count is always a display-row count, never "N buffer lines").

pub fn cmd_page_down(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let count = viewport(state, view).height as usize;
    apply_visual_vertical(state, view, count, true, mode, false);
    Ok(())
}
pub fn cmd_page_up(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let count = viewport(state, view).height as usize;
    apply_visual_vertical(state, view, count, false, mode, false);
    Ok(())
}
pub fn cmd_half_page_down(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let count = (viewport(state, view).height as usize / 2).max(1);
    apply_visual_vertical(state, view, count, true, mode, false);
    Ok(())
}
pub fn cmd_half_page_up(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let count = (viewport(state, view).height as usize / 2).max(1);
    apply_visual_vertical(state, view, count, false, mode, false);
    Ok(())
}

// ── View-trie scroll (zz / zt / zb) ───────────────────────────────────────────

fn cmd_view_scroll_to_row(state: &mut EditorState, view: &mut EngineView, target_row: usize) {
    let cursor_char = current_selections(state, view).primary().head();
    let (wrap_mode, tab_width, whitespace) = focused_format_context(state, view);
    let buf_id = focused_buffer_id(state, view);
    let content_width = view.panes[state.focused_pane_id]
        .content_width(state.buffers.get(buf_id).text().len_lines());
    let pane = &mut view.panes[state.focused_pane_id];
    // `buffers` and `motion_format_scratch` are disjoint fields of `state`, so
    // the rope can be borrowed alongside the scratch — no clone needed.
    super::super::scroll::scroll_cursor_to_row(
        &mut pane.viewport,
        state.buffers.get(buf_id).text().rope(),
        cursor_char,
        &wrap_mode,
        tab_width,
        &whitespace,
        &mut state.motion_format_scratch,
        target_row,
        &pane.providers,
        content_width,
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
