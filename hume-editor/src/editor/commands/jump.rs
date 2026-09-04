use hume_engine::pipeline::{Direction, EngineView};
use hume_ops::MotionMode;

use super::super::{EditorState, Severity};
use super::{
    alternate_buffer, current_jump_entry, focused_buffer_id, set_current_selections,
    switch_to_buffer_without_jump,
};
use crate::editor::error::CommandError;

// ── Jump list navigation ─────────────────────────────────────────────────────

/// Shared tail for `cmd_jump_backward`/`cmd_jump_forward`: switch buffer (if
/// the jump target lives elsewhere) and restore its selections. No-op if the
/// jump list had nothing in that direction.
fn apply_jump_nav(
    state: &mut EditorState,
    view: &mut EngineView,
    nav: Option<(
        hume_engine::pipeline::BufferId,
        hume_editing::selection::SelectionSet,
    )>,
) {
    if let Some((target_buf, sels)) = nav {
        if target_buf != focused_buffer_id(state, view) {
            switch_to_buffer_without_jump(state, view, target_buf);
        }
        set_current_selections(state, view, sels);
    }
}

pub(crate) fn cmd_jump_backward(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let pid = state.focused_pane_id;
    let current = current_jump_entry(state, view);
    let nav = state.panes.jumps[pid]
        .backward(current)
        .map(|e| (e.buffer_id, e.selections.clone()));
    apply_jump_nav(state, view, nav);
    Ok(())
}

pub(crate) fn cmd_jump_forward(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let pid = state.focused_pane_id;
    let nav = state.panes.jumps[pid]
        .forward()
        .map(|e| (e.buffer_id, e.selections.clone()));
    apply_jump_nav(state, view, nav);
    Ok(())
}

// ── Alternate buffer ─────────────────────────────────────────────────────────

/// `Ctrl+6` / `goto-alternate-buffer` — switch to the most-recently-focused
/// other buffer.
///
/// Uses `switch_to_buffer_without_jump` because `execute_keymap_command` already
/// records the pre-switch state for all `is_jump=true` commands. Using the
/// `_with_jump` variant here would push twice, corrupting the jump list on the
/// second Ctrl+O.
pub(crate) fn cmd_goto_alternate_buffer(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    match alternate_buffer(state, view) {
        Some(id) => switch_to_buffer_without_jump(state, view, id),
        None => state.report(Severity::Warning, "No alternate buffer".to_string()),
    }
    Ok(())
}

// ── Open-order buffer cycling ────────────────────────────────────────────────

/// Which way [`goto_buffer_in_order`] steps through the open-order buffer list.
pub(super) enum BufferStep {
    Next,
    Prev,
}

/// The one place an open-order buffer step is taken — shared by the mappable
/// `goto-next-buffer`/`goto-prev-buffer` and their typed `:bnext`/`:bprev`
/// spellings (`typed_buffer::typed_buffer_step`).
///
/// Uses `switch_to_buffer_without_jump` for the same reason as
/// `cmd_goto_alternate_buffer` above: the mappable half carries `.jump()`, so
/// `step_record_jump` already snapshots the outgoing position; the typed half
/// has no `CmdMeta` to read and pushes its own entry instead. Needs no
/// same-buffer guard — with one buffer open `next`/`prev` return it
/// unchanged and the switch is inert, and both jump-recording paths gate on
/// the cursor having actually moved.
pub(super) fn goto_buffer_in_order(
    state: &mut EditorState,
    view: &mut EngineView,
    step: BufferStep,
) {
    let current = focused_buffer_id(state, view);
    let target = match step {
        BufferStep::Next => state.buffers.next(current),
        BufferStep::Prev => state.buffers.prev(current),
    };
    switch_to_buffer_without_jump(state, view, target);
}

/// `goto-next-buffer` — switch to the next buffer in open-order.
pub(crate) fn cmd_goto_next_buffer(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    goto_buffer_in_order(state, view, BufferStep::Next);
    Ok(())
}

/// `goto-prev-buffer` — switch to the previous buffer in open-order.
pub(crate) fn cmd_goto_prev_buffer(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    goto_buffer_in_order(state, view, BufferStep::Prev);
    Ok(())
}

// ── Pane focus ───────────────────────────────────────────────────────────────

/// Directional neighbour selection for pane focus.
enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// Move focus to the nearest pane in `dir`, reading geometry recomputed from
/// the layout tree and the terminal area cached by `prepare_frame` (see
/// `EngineView::pane_rects`). Silent no-op (`Ok`) when no pane lies in that
/// direction. Focus switch is a single field write — `open_pane` already
/// seeded per-pane maps for every existing pane.
fn focus_in_direction(
    state: &mut EditorState,
    view: &EngineView,
    dir: Dir,
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let rects = view.pane_rects();
    let Some(&(_, cur)) = rects.iter().find(|(p, _)| *p == focused) else {
        return Ok(());
    };
    let cur_cx = cur.x + cur.width / 2;
    let cur_cy = cur.y + cur.height / 2;
    // Nearest by primary-axis gap among panes that overlap on the perpendicular
    // axis (excludes purely-diagonal neighbours); tie-break on perpendicular
    // center distance. Pack gap into the high 16 bits and perp into the low 16
    // bits so a single `min_by_key` orders by gap then perp — both are u16 so
    // neither can contaminate the other.
    let target = rects
        .iter()
        .copied()
        .filter(|(p, _)| p != &focused)
        .filter_map(|(pid, r)| {
            let overlaps_v = r.y < cur.y + cur.height && cur.y < r.y + r.height;
            let overlaps_h = r.x < cur.x + cur.width && cur.x < r.x + r.width;
            let (gap, perp): (u16, u16) = match dir {
                Dir::Left if overlaps_v && r.x + r.width <= cur.x => {
                    (cur.x - (r.x + r.width), cur_cy.abs_diff(r.y + r.height / 2))
                }
                Dir::Right if overlaps_v && r.x >= cur.x + cur.width => (
                    r.x - (cur.x + cur.width),
                    cur_cy.abs_diff(r.y + r.height / 2),
                ),
                Dir::Up if overlaps_h && r.y + r.height <= cur.y => {
                    (cur.y - (r.y + r.height), cur_cx.abs_diff(r.x + r.width / 2))
                }
                Dir::Down if overlaps_h && r.y >= cur.y + cur.height => (
                    r.y - (cur.y + cur.height),
                    cur_cx.abs_diff(r.x + r.width / 2),
                ),
                _ => return None,
            };
            Some(((gap as u32) << 16 | (perp as u32), pid))
        })
        .min_by_key(|(score, _)| *score)
        .map(|(_, pid)| pid);
    if let Some(pid) = target {
        state.focused_pane_id = pid;
    }
    Ok(())
}

pub(crate) fn cmd_pane_focus_next(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let rects = view.pane_rects();
    let Some(idx) = rects.iter().position(|(p, _)| *p == focused) else {
        return Ok(());
    };
    let n = rects.len();
    if n > 1 {
        state.focused_pane_id = rects[(idx + 1) % n].0;
    }
    Ok(())
}

pub(crate) fn cmd_pane_focus_left(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    focus_in_direction(state, view, Dir::Left)
}

pub(crate) fn cmd_pane_focus_right(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    focus_in_direction(state, view, Dir::Right)
}

pub(crate) fn cmd_pane_focus_up(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    focus_in_direction(state, view, Dir::Up)
}

pub(crate) fn cmd_pane_focus_down(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    focus_in_direction(state, view, Dir::Down)
}

// ── Pane split (keymap-bound, no path argument) ─────────────────────────────

/// `Ctrl+p s` — split the focused pane, stacking the new pane below it, onto
/// the same buffer. Keymap-bound sibling of the typed `:split` (which also
/// accepts an optional path argument); shares its core via `split_pane_onto`.
pub(crate) fn cmd_split_pane(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let bid = focused_buffer_id(state, view);
    super::split_pane_onto(state, view, bid, Direction::Vertical)
}

/// `Ctrl+p v` — split the focused pane side by side, onto the same buffer.
/// Keymap-bound sibling of the typed `:vsplit`.
pub(crate) fn cmd_vsplit_pane(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let bid = focused_buffer_id(state, view);
    super::split_pane_onto(state, view, bid, Direction::Horizontal)
}

/// `Ctrl+p c` — close the focused pane, collapsing the split onto its sibling.
/// No-ops with a warning when only one pane remains (`:q` owns quitting).
pub(crate) fn cmd_close_pane(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if view.panes.len() > 1 {
        super::close_focused_pane(state, view);
    } else {
        state.report(Severity::Warning, "cannot close last pane".to_string());
    }
    Ok(())
}
