use crate::editor::registry::Count;
use hume_engine::pipeline::{Direction, EngineView, Rotation, Side};
use hume_ops::MotionMode;

use super::super::EditorState;
use super::{CommandPane, FocusedPane, set_pane_selections};
use crate::editor::buffer::lifecycle::switch_pane_to_buffer;
use crate::editor::error::CommandError;
use crate::editor::focus::focus_pane;
use crate::editor::jump_list::current_jump_entry;

// ── Jump list navigation ─────────────────────────────────────────────────────

/// Shared tail for `cmd_jump_backward`/`cmd_jump_forward`: switch buffer (if
/// the jump target lives elsewhere) and restore its selections. No-op if the
/// jump list had nothing in that direction.
fn apply_jump_nav(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    nav: Option<(
        hume_engine::pipeline::BufferId,
        hume_editing::selection::SelectionSet,
    )>,
) {
    if let Some((target_buf, sels)) = nav {
        if target_buf != t.bid(view) {
            switch_pane_to_buffer(state, view, t.pid(), target_buf);
        }
        set_pane_selections(state, view, t, sels);
    }
}

pub(in crate::editor) fn cmd_jump_backward(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let current = current_jump_entry(state, view, t);
    let nav = state.panes.jumps[t.pid()]
        .backward(current)
        .map(|e| (e.buffer_id, e.selections.clone()));
    apply_jump_nav(state, view, t, nav);
    Ok(())
}

pub(in crate::editor) fn cmd_jump_forward(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let nav = state.panes.jumps[t.pid()]
        .forward()
        .map(|e| (e.buffer_id, e.selections.clone()));
    apply_jump_nav(state, view, t, nav);
    Ok(())
}

// ── Alternate buffer ─────────────────────────────────────────────────────────

/// `Ctrl-6` / `goto-alternate-buffer`: switch to the second-most-recently
/// viewed buffer. The history is global (`mru`), not per pane, so
/// [`BufferStore::second_most_recent`](crate::editor::buffer::store::BufferStore::second_most_recent)
/// needs no notion of a current buffer. The target pane's outgoing buffer is
/// touched first, then the target, so a second call from any pane toggles
/// back. Both touches are no-ops when `t` is the focused pane.
///
/// Uses `switch_pane_to_buffer`, not `switch_to_buffer_with_jump`:
/// `execute_keymap_command` already records a jump for `is_jump=true`
/// commands, and a second entry would corrupt the next Ctrl-o.
pub(in crate::editor) fn cmd_goto_alternate_buffer(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    match state.buffers.second_most_recent() {
        Some(target) => {
            state.buffers.touch_mru(t.bid(view));
            switch_pane_to_buffer(state, view, t.pid(), target);
            state.buffers.touch_mru(target);
            Ok(())
        }
        None => Err(CommandError::transient("No alternate buffer")),
    }
}

// ── Open-order buffer cycling ────────────────────────────────────────────────

/// Which way [`goto_buffer_in_order`] steps through the open-order buffer list.
pub(super) enum BufferStep {
    Next,
    Prev,
}

/// The one place an open-order buffer step is taken, shared by the mappable
/// `goto-next-buffer`/`goto-prev-buffer` and their typed `:bnext`/`:bprev`
/// spellings.
///
/// Switches via `switch_pane_to_buffer` directly for the same reason as
/// `cmd_goto_alternate_buffer` above: the mappable half carries `.jump()`, so
/// `with_jump` already snapshots the outgoing position; the typed half
/// has no `CmdMeta` to read and pushes its own entry instead. Needs no
/// same-buffer guard: with one buffer open `next`/`prev` return it
/// unchanged and the switch is inert, and both jump-recording paths gate on
/// the cursor having actually moved.
pub(super) fn goto_buffer_in_order(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    step: BufferStep,
) {
    let current = t.bid(view);
    let target = match step {
        BufferStep::Next => state.buffers.next(current),
        BufferStep::Prev => state.buffers.prev(current),
    };
    switch_pane_to_buffer(state, view, t.pid(), target);
}

/// `goto-next-buffer`: switch to the next buffer in open-order.
pub(in crate::editor) fn cmd_goto_next_buffer(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    goto_buffer_in_order(state, view, t, BufferStep::Next);
    Ok(())
}

/// `goto-prev-buffer`: switch to the previous buffer in open-order.
pub(in crate::editor) fn cmd_goto_prev_buffer(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    goto_buffer_in_order(state, view, t, BufferStep::Prev);
    Ok(())
}

// ── Pane focus ───────────────────────────────────────────────────────────────

/// Move focus to the nearest pane on `side`, a silent no-op when none lies
/// that way. Geometry comes from the terminal area cached by `prepare_frame`
/// (see `EngineView::focused_neighbor`).
fn focus_toward(state: &mut EditorState, view: &mut EngineView, side: Side) {
    if let Some(pid) = view.focused_neighbor(side) {
        focus_pane(state, view, pid);
    }
}

pub(in crate::editor) fn cmd_pane_focus_next(
    state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if let Some(pid) = view.pane_after_focused() {
        focus_pane(state, view, pid);
    }
    Ok(())
}

pub(in crate::editor) fn cmd_pane_focus_left(
    state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    focus_toward(state, view, Side::Left);
    Ok(())
}

pub(in crate::editor) fn cmd_pane_focus_right(
    state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    focus_toward(state, view, Side::Right);
    Ok(())
}

pub(in crate::editor) fn cmd_pane_focus_up(
    state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    focus_toward(state, view, Side::Up);
    Ok(())
}

pub(in crate::editor) fn cmd_pane_focus_down(
    state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    focus_toward(state, view, Side::Down);
    Ok(())
}

// ── Pane split (keymap-bound, no path argument) ─────────────────────────────

/// `Ctrl-p s`: split the focused pane, stacking the new pane below it, onto
/// the same buffer. Keymap-bound sibling of the typed `:split` (which also
/// accepts an optional path argument); shares its core via `split_pane_onto`.
pub(in crate::editor) fn cmd_split_pane(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let bid = fp.bid(view);
    super::split_pane_onto(state, view, fp, bid, Direction::Vertical)
}

/// `Ctrl-p v`: split the focused pane side by side, onto the same buffer.
/// Keymap-bound sibling of the typed `:vsplit`.
pub(in crate::editor) fn cmd_vsplit_pane(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let bid = fp.bid(view);
    super::split_pane_onto(state, view, fp, bid, Direction::Horizontal)
}

/// `Ctrl-p c`: close the focused pane, collapsing the split onto its sibling.
/// Refuses when it's the tab's only pane (`:q` owns closing the tab in that
/// case; see `typed_quit`).
pub(in crate::editor) fn cmd_close_pane(
    state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    super::close_focused_pane(state, view)
        .map_err(|_| CommandError::transient("cannot close last pane"))
}

/// `Ctrl-p r`: rotate the focused pane's row or column forward by `count`
/// places, focus staying on the pane it was on.
pub(in crate::editor) fn cmd_pane_rotate_forward(
    _state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    rotate_focused(view, Rotation::Forward, count)
}

/// `Ctrl-p R`: the reverse of `Ctrl-p r`.
pub(in crate::editor) fn cmd_pane_rotate_backward(
    _state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    rotate_focused(view, Rotation::Backward, count)
}

fn rotate_focused(
    view: &mut EngineView,
    rotation: Rotation,
    count: Count,
) -> Result<(), CommandError> {
    view.rotate_focused(rotation, count.get())
        .map_err(|_| CommandError::transient("no other pane to rotate"))
}
