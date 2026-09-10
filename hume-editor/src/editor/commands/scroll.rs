use hume_engine::pipeline::EngineView;
use hume_ops::MotionMode;

use super::super::EditorState;
use super::super::visual_move::{VerticalUnit, apply_visual_vertical};
use super::{current_selections, focused_buffer_id, pane_display_lines, viewport};
use crate::editor::error::CommandError;

// ── Page / half-page scroll ───────────────────────────────────────────────────
//
// Uses `view.height` (or half of it) as the move count rather than the user's
// numeric prefix. Calls `apply_visual_vertical` directly (not the registry, to
// avoid a runtime string lookup; not the `cmd_visual_move_*` wrappers, since a
// scroll count is always a display-line count, never "N buffer lines").

fn scroll_page(
    state: &mut EditorState,
    view: &mut EngineView,
    mode: MotionMode,
    half: bool,
    down: bool,
) -> Result<(), CommandError> {
    let height = viewport(state, view).height as usize;
    let count = if half { (height / 2).max(1) } else { height };
    apply_visual_vertical(state, view, count, down, mode, VerticalUnit::AnyDisplayLine);
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

fn cmd_view_scroll_to_display_line(state: &mut EditorState, view: &mut EngineView, target_display_line: usize) {
    let cursor_char = current_selections(state, view).primary().head();
    let pid = state.focused_pane_id;
    let buf_id = focused_buffer_id(state, view);
    let key = state.format_key(&view.panes[pid]);
    let (mut dlm, viewport) =
        pane_display_lines(state.buffers.get(buf_id), &mut view.panes[pid], key);
    super::super::scroll::scroll_cursor_to_display_line(viewport, &mut dlm, cursor_char, target_display_line);
}

/// Center the head in the viewport, like `z z`. Infallible core shared by
/// [`cmd_view_center`] (the registered `z z` command) and any other caller
/// that wants the same effect without going through an `EditorCmdFn`'s
/// `Result` — `lifecycle.rs`'s post-file-load placement, LSP goto-definition
/// (`lsp/edits.rs`), and `step_align_view`'s `Center` arm.
pub(crate) fn view_center(state: &mut EditorState, view: &mut EngineView) {
    let target = (viewport(state, view).height as usize) / 2;
    cmd_view_scroll_to_display_line(state, view, target);
}

/// Pin the head at the viewport's top display line, like `z k`. Infallible core
/// shared by [`cmd_view_top`] and `step_align_view`'s `Top` arm.
pub(crate) fn view_top(state: &mut EditorState, view: &mut EngineView) {
    cmd_view_scroll_to_display_line(state, view, 0);
}

pub(crate) fn cmd_view_center(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    view_center(state, view);
    Ok(())
}

pub(crate) fn cmd_view_top(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    view_top(state, view);
    Ok(())
}

pub(crate) fn cmd_view_bottom(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let target = (viewport(state, view).height as usize).saturating_sub(1);
    cmd_view_scroll_to_display_line(state, view, target);
    Ok(())
}
