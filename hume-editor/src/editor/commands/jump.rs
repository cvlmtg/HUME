use hume_core::Count;
use hume_engine::pipeline::{Cycle, Direction, EngineView, Resize, ResizeRefused, Rotation, Side};
use hume_ops::MotionMode;

use super::super::EditorState;
use super::{CommandPane, FocusedPane, set_pane_selections};
use crate::editor::buffer::lifecycle::open_scratch_on;
use crate::editor::error::CommandError;
use crate::editor::focus::focus_pane;
use crate::editor::focus::switch_pane_to_buffer;
use crate::editor::jump_list::current_jump_entry;

// ── Jump list navigation ─────────────────────────────────────────────────────

/// Shared tail for `cmd_jump_backward`/`cmd_jump_forward`: switch buffer (if
/// the jump target lives elsewhere) and restore its selections. No-op if the
/// jump list had nothing in that direction.
fn apply_jump_nav(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    nav: Option<(hume_core::BufferId, hume_editing::selection::SelectionSet)>,
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
    let nav = state.panes.entries[t.pid()]
        .jumps
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
    let nav = state.panes.entries[t.pid()]
        .jumps
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

/// The one place an open-order buffer step is taken, shared by the mappable
/// `goto-next-buffer`/`goto-prev-buffer` and their typed `:bnext`/`:bprev`
/// spellings. Needs no same-buffer guard: with one buffer open `step`
/// returns it unchanged and the switch is inert, and `with_jump` records
/// only a move.
pub(super) fn goto_buffer_in_order(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    cycle: Cycle,
) {
    let target = state.buffers.step(t.bid(view), cycle);
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
    goto_buffer_in_order(state, view, t, Cycle::Next);
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
    goto_buffer_in_order(state, view, t, Cycle::Prev);
    Ok(())
}

// ── New scratch buffer ───────────────────────────────────────────────────────

/// `buffer-new`: open a new empty scratch buffer in the pane.
pub(in crate::editor) fn cmd_buffer_new(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    open_scratch_on(state, view, t);
    Ok(())
}

// ── Pane focus ───────────────────────────────────────────────────────────────

/// Move focus to the nearest pane on `side` on screen, a no-op when
/// none lies that way.
fn focus_toward(state: &mut EditorState, view: &mut EngineView, side: Side) {
    let neighbor = state
        .screen_layout(view)
        .focused_neighbor(view.focused_layout(), side);
    if let Some(pid) = neighbor {
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
    focus_cycled(state, view, Cycle::Next);
    Ok(())
}

pub(in crate::editor) fn cmd_pane_focus_prev(
    state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    focus_cycled(state, view, Cycle::Prev);
    Ok(())
}

/// Focus the pane one step from the focused one in layout order, if there is
/// another pane.
fn focus_cycled(state: &mut EditorState, view: &mut EngineView, cycle: Cycle) {
    if let Some(pid) = view.pane_cycled_from_focused(cycle) {
        focus_pane(state, view, pid);
    }
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
    super::split_pane_onto(state, view, fp, Direction::Vertical, |_, view| {
        Ok(fp.bid(view))
    })
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
    super::split_pane_onto(state, view, fp, Direction::Horizontal, |_, view| {
        Ok(fp.bid(view))
    })
}

/// `Ctrl-p c`: close the focused pane, collapsing the split onto its sibling.
/// Refuses when it's the tab's only pane (`:q` owns closing the tab in that
/// case; see `typed_quit`), and while the pane's pipe run is in flight,
/// naming `:q!`, the typed close that stops it.
pub(in crate::editor) fn cmd_close_pane(
    state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    super::close_focused_pane(state, view).map_err(|refused| match refused {
        super::CloseRefused::Last => CommandError::transient("cannot close last pane"),
        super::CloseRefused::PipeRunning(running) => {
            super::pipe_running_refusal(state, view, running, ":q!")
        }
    })
}

/// `Ctrl-p r`: rotate the split the focused pane sits in forward by
/// `count` places, focus staying on the pane it was on.
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

/// `Ctrl-p >`: widen the focused pane by `count` columns.
pub(in crate::editor) fn cmd_pane_grow_width(
    state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    resize_focused(state, view, Direction::Horizontal, Resize::Grow, count)
}

/// `Ctrl-p <`: narrow the focused pane by `count` columns.
pub(in crate::editor) fn cmd_pane_shrink_width(
    state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    resize_focused(state, view, Direction::Horizontal, Resize::Shrink, count)
}

/// `Ctrl-p +`: make the focused pane `count` rows taller.
pub(in crate::editor) fn cmd_pane_grow_height(
    state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    resize_focused(state, view, Direction::Vertical, Resize::Grow, count)
}

/// `Ctrl-p -`: make the focused pane `count` rows shorter.
pub(in crate::editor) fn cmd_pane_shrink_height(
    state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    resize_focused(state, view, Direction::Vertical, Resize::Shrink, count)
}

fn resize_focused(
    state: &EditorState,
    view: &mut EngineView,
    axis: Direction,
    resize: Resize,
    count: Count,
) -> Result<(), CommandError> {
    let cells = u16::try_from(count.get()).expect("a count is at most 10 000");
    let screen = state.screen_layout(view);
    view.resize_focused(&screen, axis, resize, cells)
        .map_err(|refused| {
            CommandError::transient(match (refused, axis, resize) {
                (ResizeRefused::NoNeighbor, Direction::Horizontal, _) => {
                    "no pane to the left or right"
                }
                (ResizeRefused::NoNeighbor, Direction::Vertical, _) => "no pane above or below",
                (ResizeRefused::AtLimit, _, Resize::Grow) => "no room to grow the pane",
                (ResizeRefused::AtLimit, _, Resize::Shrink) => "no room to shrink the pane",
            })
        })
}

/// `Ctrl-p =`: give every pane of the tab an equal share of its split
/// again.
pub(in crate::editor) fn cmd_pane_equalize(
    _state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    view.equalize();
    Ok(())
}
