use hume_editing::selection::Selection;
use hume_engine::display_lines::{DisplayColTarget, carry};
use hume_engine::pipeline::{EngineView, PaneId};
use hume_ops::MotionMode;

use super::super::EditorState;
use super::super::doc_ops;
use super::{pane_display_lines, viewport};
use crate::editor::error::CommandError;

// ── Page / half-page scroll ───────────────────────────────────────────────────
//
// Uses `view.height` (or half of it) as the move count rather than the user's
// numeric prefix.

/// Scroll pane `pid` `count` display lines and carry every cursor the
/// same distance, so the cursor keeps its screen row.
///
/// The single implementation behind the wheel, `Ctrl-d`/`Ctrl-u` and
/// `PageDown`/`PageUp` — they differ only in `count` and which pane they pass
/// (always `state.focus.id()`, except the wheel, which hit-tests the pointer
/// against `pane_at_screen_pos` — see `editor/mouse.rs`). `Viewport::scroll_by`
/// is what makes a scroll move the view even while the cursor is still
/// inside it (a `Ctrl-d` from the top of a file); `carry`, called per
/// selection below with the same requested `delta` and the viewport's own
/// post-scroll `top` — walking its own bound independently of `scroll_by`'s,
/// see `hume_engine::display_lines::scroll`'s module doc for why — is what
/// keeps the cursor at the same relative position (or, failing that, lands
/// it back inside the scrolloff band) instead of being snapped back next
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
    let top = viewport.top();

    // `apply_doc_motion`'s own head-before/after comparison is what raises
    // `PaneBufferState::reveal_pending` here — no separate pin to track: a
    // scroll that couldn't carry a selection anywhere (parked behind a
    // virtual block, or already at a document edge) leaves that selection's
    // head unchanged, which is exactly the case the funnel already treats
    // as "nothing to reveal".
    doc_ops::apply_doc_motion(
        &state.buffers,
        &mut state.panes.state,
        pid,
        buf_id,
        |_text, sels| {
            sels.map(|sel| {
                let (head_pos, target_col) = dlm.locate(sel.head());
                let Some(landed) = carry(&mut dlm, geo, top, head_pos, delta) else {
                    return sel; // parked behind a virtual block or at a document edge
                };
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
}

fn scroll_page(
    state: &mut EditorState,
    view: &mut EngineView,
    pid: PaneId,
    mode: MotionMode,
    half: bool,
    down: bool,
) -> Result<(), CommandError> {
    let height = viewport(view, pid).height as usize;
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
    let pid = state.focus.id();
    scroll_page(state, view, pid, mode, false, true)
}
pub(in crate::editor) fn cmd_page_up(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let pid = state.focus.id();
    scroll_page(state, view, pid, mode, false, false)
}
pub(in crate::editor) fn cmd_half_page_down(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let pid = state.focus.id();
    scroll_page(state, view, pid, mode, true, true)
}
pub(in crate::editor) fn cmd_half_page_up(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let pid = state.focus.id();
    scroll_page(state, view, pid, mode, true, false)
}

// ── View-trie scroll (z z / z k / z j) ────────────────────────────────────────

fn cmd_view_scroll_to_display_line(
    state: &mut EditorState,
    view: &mut EngineView,
    pid: PaneId,
    target_display_line: usize,
) {
    let buf_id = view.panes[pid].buffer_id;
    let cursor_char = state
        .panes
        .buffer_state(pid, buf_id)
        .expect(
            "pane has no seeded state for the buffer it is showing — \
             pane.buffer_id and panes.state are out of sync",
        )
        .selections()
        .primary()
        .head();
    let key = state.format_key(&view.panes[pid]);
    let scrolloff = state.settings.scrolloff;
    let (mut dlm, viewport) =
        pane_display_lines(state.buffers.get(buf_id), &mut view.panes[pid], key);
    // A collapsed pane has no geometry, and nowhere to align a cursor to.
    let Some(geo) = viewport.geometry(scrolloff) else {
        return;
    };
    let cursor_pos = dlm.locate_display_line(cursor_char);
    viewport.align(&mut dlm, geo, cursor_pos, target_display_line);
}

/// Center the head in the viewport, like `z z`. Infallible core shared by
/// [`cmd_view_center`] (the registered `z z` command) and any other caller
/// that wants the same effect without going through an `EditorCmdFn`'s
/// `Result` — `lifecycle.rs`'s post-file-load placement, LSP goto-definition
/// (`lsp/edits.rs`), and `step_align_view`'s `Center` arm.
pub(in crate::editor) fn view_center(state: &mut EditorState, view: &mut EngineView, pid: PaneId) {
    let target = (viewport(view, pid).height as usize) / 2;
    cmd_view_scroll_to_display_line(state, view, pid, target);
}

/// Pin the head at the viewport's top display line, like `z k`. Infallible core
/// shared by [`cmd_view_top`] and `step_align_view`'s `Top` arm.
pub(in crate::editor::commands) fn view_top(
    state: &mut EditorState,
    view: &mut EngineView,
    pid: PaneId,
) {
    cmd_view_scroll_to_display_line(state, view, pid, 0);
}

pub(in crate::editor) fn cmd_view_center(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let pid = state.focus.id();
    view_center(state, view, pid);
    Ok(())
}

pub(in crate::editor) fn cmd_view_top(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let pid = state.focus.id();
    view_top(state, view, pid);
    Ok(())
}

pub(in crate::editor) fn cmd_view_bottom(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let pid = state.focus.id();
    let target = (viewport(view, pid).height as usize).saturating_sub(1);
    cmd_view_scroll_to_display_line(state, view, pid, target);
    Ok(())
}
