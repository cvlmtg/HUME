use crate::ops::MotionMode;
use hume_engine::pipeline::EngineView;

use super::super::{EditorState, Severity};
use super::{
    alternate_buffer, current_jump_entry, focused_buffer_id, set_current_selections,
    switch_to_buffer_without_jump,
};
use crate::editor::error::CommandError;

// ── Misc ──────────────────────────────────────────────────────────────────────

pub fn cmd_quit(
    state: &mut EditorState,
    _view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    state.should_quit = true;
    Ok(())
}

// ── Jump list navigation ─────────────────────────────────────────────────────

pub fn cmd_jump_backward(
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
    if let Some((target_buf, sels)) = nav {
        if target_buf != focused_buffer_id(state, view) {
            switch_to_buffer_without_jump(state, view, target_buf);
        }
        set_current_selections(state, view, sels);
    }
    Ok(())
}

pub fn cmd_jump_forward(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let pid = state.focused_pane_id;
    let nav = state.panes.jumps[pid]
        .forward()
        .map(|e| (e.buffer_id, e.selections.clone()));
    if let Some((target_buf, sels)) = nav {
        if target_buf != focused_buffer_id(state, view) {
            switch_to_buffer_without_jump(state, view, target_buf);
        }
        set_current_selections(state, view, sels);
    }
    Ok(())
}

// ── Alternate buffer ─────────────────────────────────────────────────────────

/// `Ctrl+6` / `goto-alternate-file` — switch to the most-recently-focused
/// other buffer.
///
/// Uses `switch_to_buffer_without_jump` because `execute_keymap_command` already
/// records the pre-switch state for all `is_jump=true` commands. Using the
/// `_with_jump` variant here would push twice, corrupting the jump list on the
/// second Ctrl+O.
pub fn cmd_goto_alternate_file(
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

// ── Pane focus (M10/T4) ──────────────────────────────────────────────────────

/// Directional neighbour selection for pane focus.
enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// Move focus to the nearest pane in `dir`, reading geometry from the cache
/// populated by `prepare_frame`. Silent no-op (`Ok`) when no pane lies in that
/// direction. Focus switch is a single field write — `open_pane` already
/// seeded per-pane maps for every existing pane.
fn focus_in_direction(
    state: &mut EditorState,
    view: &EngineView,
    dir: Dir,
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let Some(&(_, cur)) = view.pane_rects.iter().find(|(p, _)| *p == focused) else {
        return Ok(());
    };
    // Nearest by primary-axis gap; tie-break on perpendicular center distance.
    // Pack gap into the high 16 bits and perp into the low 16 bits so a single
    // `min_by_key` orders by gap then perp — both are u16 so neither can
    // contaminate the other.
    let target = view
        .pane_rects
        .iter()
        .copied()
        .filter(|(p, _)| p != &focused)
        .filter_map(|(pid, r)| {
            let (gap, perp): (u16, u16) = match dir {
                Dir::Left if r.x + r.width <= cur.x => {
                    (cur.x - (r.x + r.width), cur.y.abs_diff(r.y))
                }
                Dir::Right if r.x >= cur.x + cur.width => {
                    (r.x - (cur.x + cur.width), cur.y.abs_diff(r.y))
                }
                Dir::Up if r.y + r.height <= cur.y => {
                    (cur.y - (r.y + r.height), cur.x.abs_diff(r.x))
                }
                Dir::Down if r.y >= cur.y + cur.height => {
                    (r.y - (cur.y + cur.height), cur.x.abs_diff(r.x))
                }
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

pub fn cmd_pane_focus_next(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let Some(idx) = view.pane_rects.iter().position(|(p, _)| *p == focused) else {
        return Ok(());
    };
    let n = view.pane_rects.len();
    if n > 1 {
        state.focused_pane_id = view.pane_rects[(idx + 1) % n].0;
    }
    Ok(())
}

pub fn cmd_pane_focus_left(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    focus_in_direction(state, view, Dir::Left)
}

pub fn cmd_pane_focus_right(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    focus_in_direction(state, view, Dir::Right)
}

pub fn cmd_pane_focus_up(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    focus_in_direction(state, view, Dir::Up)
}

pub fn cmd_pane_focus_down(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    focus_in_direction(state, view, Dir::Down)
}
