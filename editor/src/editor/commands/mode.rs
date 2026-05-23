use crate::core::grapheme::next_grapheme_boundary;
use crate::core::selection::Selection;
use crate::ops::MotionMode;
use crate::ops::edit::insert_char;
use crate::ops::motion::{
    cmd_goto_first_nonblank, cmd_goto_line_newline, cmd_goto_line_start, cmd_goto_line_end,
    cmd_move_left, cmd_move_right,
};
use crate::ops::selection_cmd::cmd_collapse_selection;

use engine::types::EditorMode;
use super::super::{doc_ops, MiniBuffer, Mode};
use super::super::Editor;
use crate::core::error::CommandError;

// ── Mode transitions ──────────────────────────────────────────────────────────

pub fn cmd_insert_before(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |_b, sels| {
        sels.map(|s| Selection::collapsed(s.start()))
    });
    ed.begin_insert_session();
    Ok(())
}

pub fn cmd_insert_after(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        cmd_move_right(b, s, 1, MotionMode::Move)
    });
    ed.begin_insert_session();
    Ok(())
}

pub fn cmd_insert_at_line_start(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        cmd_goto_first_nonblank(b, s, 1, MotionMode::Move)
    });
    ed.begin_insert_session();
    Ok(())
}

pub fn cmd_insert_at_line_end(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        cmd_goto_line_end(b, s, 1, MotionMode::Move)
    });
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        cmd_move_right(b, s, 1, MotionMode::Move)
    });
    ed.begin_insert_session();
    Ok(())
}

/// Enter insert mode at the start of each selection (min of anchor and head).
/// For a collapsed cursor this is identical to `i`.
pub fn cmd_insert_at_selection_start(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |_b, sels| {
        sels.map(|sel| Selection::collapsed(sel.start()))
    });
    ed.begin_insert_session();
    Ok(())
}

/// Enter insert mode after the end of each selection (one past max of anchor and head).
/// For a collapsed cursor this is identical to `a`.
///
/// Clamps to `len_chars() - 1` so pressing `a` on the structural trailing `\n`
/// (the last char in the buffer) does not place the cursor out of bounds.
pub fn cmd_insert_at_selection_end(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, sels| {
        // len_chars() - 1 is safe: the buffer invariant guarantees at least one char.
        let max = b.len_chars() - 1;
        sels.map(|sel| Selection::collapsed(next_grapheme_boundary(b, sel.end()).min(max)))
    });
    ed.begin_insert_session();
    Ok(())
}

/// Open a new line below the cursor and enter insert mode.
///
/// `begin_insert_session` opens the edit group so the structural `\n` and
/// everything typed before Esc form one undo step — the same pattern as
/// `cmd_change`.
pub fn cmd_open_line_below(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    ed.begin_insert_session();
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        cmd_goto_line_newline(b, s, 1, MotionMode::Move)
    });
    doc_ops::apply_doc_edit_grouped(&mut ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        insert_char(b, s, '\n')
    });
    Ok(())
}

/// Open a new line above the cursor and enter insert mode.
pub fn cmd_open_line_above(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    ed.begin_insert_session();
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        cmd_goto_line_start(b, s, 1, MotionMode::Move)
    });
    doc_ops::apply_doc_edit_grouped(&mut ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        insert_char(b, s, '\n')
    });
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        cmd_move_left(b, s, 1, MotionMode::Move)
    });
    Ok(())
}

pub fn cmd_command_mode(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.history.begin_session_all();
    ed.set_mode(Mode::Command);
    ed.minibuf = Some(MiniBuffer {
        prompt: ':',
        input: String::new(),
        cursor: 0,
    });
    Ok(())
}

pub fn cmd_exit_insert(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.end_insert_session();
    Ok(())
}

// ── Extend mode ───────────────────────────────────────────────────────────────

pub fn cmd_toggle_extend(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.mode = if ed.mode == EditorMode::Extend {
        EditorMode::Normal
    } else {
        EditorMode::Extend
    };
    Ok(())
}

/// Collapse each selection to its cursor AND exit extend mode.
///
/// Collapsing is a "done selecting" signal, so extend mode is always cleared.
pub fn cmd_collapse_and_exit_extend(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    // Mode is SSOT for extend state; setting Normal implicitly clears Extend.
    ed.mode = EditorMode::Normal;
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        cmd_collapse_selection(b, s, MotionMode::Move)
    });
    Ok(())
}

// ── Dot repeat ───────────────────────────────────────────────────────────────

/// Replay the last repeatable editing action.
///
/// Count semantics: if the user typed an explicit count before `.`, that count
/// overrides the original; otherwise the original count is reused. This mirrors
/// Vim's behaviour (`3.` → repeat with 3; `.` alone → repeat with original count).
pub fn cmd_repeat(
    ed: &mut Editor,
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let Some(action) = ed.last_repeatable_action.take() else {
        return Ok(());
    };

    // Prefer an explicit user count; fall back to the count from the original action.
    let effective_count = if ed.explicit_count {
        count
    } else {
        action.count
    };

    // Restore the char arg so wait-char commands (replace, find/till) work.
    ed.pending_char = action.char_arg;

    // Pre-open the edit group before re-executing. This is the replay signal:
    // `begin_insert_session` checks `is_group_open()` and suppresses both the
    // redundant `begin_edit_group` call and keystroke recording when the group
    // is already open. For non-insert commands the group stays empty and the
    // commit below is a no-op.
    ed.begin_edit_group_current();

    // Re-execute the original command through the normal dispatch path.
    // extend=false because the replayed command was already resolved to its
    // final form (the resolved name is what gets stored in RepeatableAction).
    // Clone the name while `action` is locally owned (moved out via `.take()`).
    ed.execute_keymap_command(action.command.clone(), effective_count, false, vec![]);

    // Feed recorded insert keystrokes through the normal insert handler.
    // `KeyEvent` is `Copy`, so iterate by reference and dereference each key.
    for key in &action.insert_keys {
        ed.handle_insert(*key);
    }

    // Close the insert session / edit group:
    // - For insert commands: `end_insert_session` commits the group (delete +
    //   typed text as one undo step). `insert_session` is `None` here (replay
    //   suppressed it), so no keystrokes are moved into `last_repeatable_action`.
    // - For non-insert commands: the group is empty (no `apply_edit_grouped`
    //   calls), so `commit_edit_group` is a no-op and the command's own
    //   `apply_edit` revision stands alone in history.
    if ed.mode == EditorMode::Insert {
        ed.end_insert_session();
    } else {
        ed.commit_edit_group_current();
    }

    // Restore the original action so `.` can be pressed again.
    // `execute_keymap_command` may have overwritten `last_repeatable_action` during
    // replay; this final assignment ensures the stored action is always the
    // one the user actually performed.
    ed.last_repeatable_action = Some(action);
    Ok(())
}
