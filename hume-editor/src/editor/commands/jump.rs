use hume_engine::pipeline::EngineView;
use crate::ops::MotionMode;

use super::super::{EditorState, Severity};
use crate::editor::error::CommandError;
use super::{
    alternate_buffer, current_jump_entry, focused_buffer_id,
    set_current_selections, switch_to_buffer_without_jump,
};

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

// ── Pane focus stubs (M9+) ────────────────────────────────────────────────────

pub fn cmd_pane_focus_next(
    _state: &mut EditorState,
    _view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    Err(CommandError::new(":split not yet implemented"))
}

pub fn cmd_pane_focus_left(
    _state: &mut EditorState,
    _view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    Err(CommandError::new(":split not yet implemented"))
}

pub fn cmd_pane_focus_right(
    _state: &mut EditorState,
    _view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    Err(CommandError::new(":split not yet implemented"))
}

pub fn cmd_pane_focus_up(
    _state: &mut EditorState,
    _view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    Err(CommandError::new(":split not yet implemented"))
}

pub fn cmd_pane_focus_down(
    _state: &mut EditorState,
    _view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    Err(CommandError::new(":split not yet implemented"))
}
