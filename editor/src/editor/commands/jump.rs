use crate::ops::MotionMode;

use super::super::{Severity};
use super::super::Editor;
use crate::editor::error::CommandError;

// ── Misc ──────────────────────────────────────────────────────────────────────

pub fn cmd_quit(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.state.should_quit = true;
    Ok(())
}

// ── Jump list navigation ─────────────────────────────────────────────────────

pub fn cmd_jump_backward(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let pid = ed.state.focused_pane_id;
    let current = ed.current_jump_entry();
    let nav = ed.state.pane_jumps[pid]
        .backward(current)
        .map(|e| (e.buffer_id, e.selections.clone()));
    if let Some((target_buf, sels)) = nav {
        if target_buf != ed.focused_buffer_id() {
            ed.switch_to_buffer_without_jump(target_buf);
        }
        ed.set_current_selections(sels);
    }
    Ok(())
}

pub fn cmd_jump_forward(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let pid = ed.state.focused_pane_id;
    let nav = ed.state.pane_jumps[pid]
        .forward()
        .map(|e| (e.buffer_id, e.selections.clone()));
    if let Some((target_buf, sels)) = nav {
        if target_buf != ed.focused_buffer_id() {
            ed.switch_to_buffer_without_jump(target_buf);
        }
        ed.set_current_selections(sels);
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
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    match ed.alternate_buffer() {
        Some(id) => ed.switch_to_buffer_without_jump(id),
        None => ed.report(Severity::Warning, "No alternate buffer".to_string()),
    }
    Ok(())
}

// ── Pane focus stubs (M9+) ────────────────────────────────────────────────────

pub fn cmd_pane_focus_next(
    _ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    Err(CommandError::new(":split not yet implemented"))
}

pub fn cmd_pane_focus_left(
    _ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    Err(CommandError::new(":split not yet implemented"))
}

pub fn cmd_pane_focus_right(
    _ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    Err(CommandError::new(":split not yet implemented"))
}

pub fn cmd_pane_focus_up(
    _ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    Err(CommandError::new(":split not yet implemented"))
}

pub fn cmd_pane_focus_down(
    _ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    Err(CommandError::new(":split not yet implemented"))
}
