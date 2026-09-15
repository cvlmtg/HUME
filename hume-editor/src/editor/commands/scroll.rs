use hume_editing::selection::Selection;
use hume_engine::display_lines::{DisplayColTarget, carry};
use hume_engine::pipeline::{EngineView, PaneId};
use hume_ops::MotionMode;

use super::super::EditorState;
use super::super::doc_ops;
use super::{current_selections, focused_buffer_id, pane_display_lines, viewport};
use crate::editor::error::CommandError;

// ── Page / half-page scroll ───────────────────────────────────────────────────
//
// Uses `view.height` (or half of it) as the move count rather than the user's
// numeric prefix.

/// Scroll pane `pid` `count` display lines and carry every cursor the
/// same distance, so the cursor keeps its screen row.
///
/// The single implementation behind the wheel, `Ctrl+D`/`Ctrl+U` and
/// `PageDown`/`PageUp` — they differ only in `count` and which pane they pass
/// (always `state.focus.id()`, except the wheel, which hit-tests the pointer
/// against `pane_at_screen_pos` — see `editor/mouse.rs`). `Viewport::scroll_by`
/// is what makes a scroll move the view even while the cursor is still
/// inside it (a `Ctrl+D` from the top of a file); `carry`, called per
/// selection below with the same requested `delta` (not `scroll_by`'s
/// return — see `hume_engine::display_lines::scroll`'s module doc for why
/// the two walk independently), is what keeps the cursor at the same
/// relative position instead of being snapped back into the margin next
/// frame — see `carry`'s own doc for why a selection it can't place stays
/// untouched rather than collapsing.
pub(in crate::editor) fn scroll_view(
    state: &mut EditorState,
    view: &mut EngineView,
    pid: PaneId,
    count: usize,
    down: bool,
    mode: MotionMode,
) {
    let buf_id = view.panes[pid].buffer_id;
    let key = state.format_key(&view.panes[pid]);
    let scrolloff = state.settings.scrolloff;
    let (mut dlm, viewport) =
        pane_display_lines(state.buffers.get(buf_id), &mut view.panes[pid], key);
    let Some(geo) = viewport.geometry(scrolloff) else {
        return; // a collapsed pane has nothing to scroll and nowhere to carry a cursor
    };
    let delta = if down {
        count as isize
    } else {
        -(count as isize)
    };
    viewport.scroll_by(&mut dlm, geo, delta);

    // Not the same predicate as `apply_visual_vertical`'s own multi-selection
    // guards: `frame.rs`'s scroll step only ever compares the primary head,
    // so the pin is decided from the primary alone here.
    let head_before = state.panes.state[pid][buf_id].selections.primary().head();
    doc_ops::apply_doc_motion(
        &state.buffers,
        &mut state.panes.state,
        pid,
        buf_id,
        |_text, sels| {
            sels.map(|sel| {
                let head_pos = dlm.locate_display_line(sel.head());
                let Some(landed) = carry(&mut dlm, head_pos, delta) else {
                    return sel; // parked behind a virtual block or at a document edge
                };
                let target_col = dlm.locate(sel.head()).1;
                let new_head = dlm.char_at(landed, target_col, DisplayColTarget::NearestContent);
                let anchor = if mode == MotionMode::Extend {
                    sel.anchor()
                } else {
                    new_head
                };
                Selection::new(anchor, new_head)
            })
        },
    );
    let head_after = state.panes.state[pid][buf_id].selections.primary().head();
    // Written unconditionally — see `PaneBufferState::scroll_pin`.
    state.panes.state[pid][buf_id].scroll_pin = (head_before == head_after).then_some(head_after);
}

fn scroll_page(
    state: &mut EditorState,
    view: &mut EngineView,
    mode: MotionMode,
    half: bool,
    down: bool,
) -> Result<(), CommandError> {
    let pid = state.focus.id();
    let height = viewport(state, view).height as usize;
    let count = if half { (height / 2).max(1) } else { height };
    scroll_view(state, view, pid, count, down, mode);
    Ok(())
}

pub(in crate::editor) fn cmd_page_down(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    scroll_page(state, view, mode, false, true)
}
pub(in crate::editor) fn cmd_page_up(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    scroll_page(state, view, mode, false, false)
}
pub(in crate::editor) fn cmd_half_page_down(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    scroll_page(state, view, mode, true, true)
}
pub(in crate::editor) fn cmd_half_page_up(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    scroll_page(state, view, mode, true, false)
}

// ── View-trie scroll (z z / z k / z j) ────────────────────────────────────────

fn cmd_view_scroll_to_display_line(
    state: &mut EditorState,
    view: &mut EngineView,
    target_display_line: usize,
) {
    let cursor_char = current_selections(state, view).primary().head();
    let pid = state.focus.id();
    let buf_id = focused_buffer_id(state, view);
    let key = state.format_key(&view.panes[pid]);
    let scrolloff = state.settings.scrolloff;
    let (mut dlm, viewport) =
        pane_display_lines(state.buffers.get(buf_id), &mut view.panes[pid], key);
    // A collapsed pane has no geometry, and nowhere to align a cursor to.
    let Some(geo) = viewport.geometry(scrolloff) else {
        return;
    };
    super::super::scroll::scroll_cursor_to_display_line(
        viewport,
        &mut dlm,
        geo,
        cursor_char,
        target_display_line,
    );
}

/// Center the head in the viewport, like `z z`. Infallible core shared by
/// [`cmd_view_center`] (the registered `z z` command) and any other caller
/// that wants the same effect without going through an `EditorCmdFn`'s
/// `Result` — `lifecycle.rs`'s post-file-load placement, LSP goto-definition
/// (`lsp/edits.rs`), and `step_align_view`'s `Center` arm.
pub(in crate::editor) fn view_center(state: &mut EditorState, view: &mut EngineView) {
    let target = (viewport(state, view).height as usize) / 2;
    cmd_view_scroll_to_display_line(state, view, target);
}

/// Pin the head at the viewport's top display line, like `z k`. Infallible core
/// shared by [`cmd_view_top`] and `step_align_view`'s `Top` arm.
pub(in crate::editor::commands) fn view_top(state: &mut EditorState, view: &mut EngineView) {
    cmd_view_scroll_to_display_line(state, view, 0);
}

pub(in crate::editor) fn cmd_view_center(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    view_center(state, view);
    Ok(())
}

pub(in crate::editor) fn cmd_view_top(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    view_top(state, view);
    Ok(())
}

pub(in crate::editor) fn cmd_view_bottom(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let target = (viewport(state, view).height as usize).saturating_sub(1);
    cmd_view_scroll_to_display_line(state, view, target);
    Ok(())
}
