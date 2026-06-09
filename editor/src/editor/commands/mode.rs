use editing::grapheme::next_grapheme_boundary;
use editing::selection::Selection;
use engine::pipeline::EngineView;
use engine::types::EditorMode;

use crate::ops::MotionMode;
use crate::ops::edit::insert_char;
use crate::ops::motion::{
    cmd_goto_first_nonblank, cmd_goto_line_newline, cmd_goto_line_start, cmd_goto_line_end,
    cmd_move_left, cmd_move_right,
};
use crate::ops::selection_cmd::cmd_collapse_selection;

use super::super::{doc_ops, EditorState, MiniBuffer, Mode};
use super::super::Editor;
use crate::editor::error::CommandError;
use crate::editor::SideEffects;
use super::{
    begin_insert_session, end_insert_session, enqueue_mode_change,
    focused_buffer_id,
};

// ── Mode transitions ──────────────────────────────────────────────────────────

pub fn cmd_insert_before(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<SideEffects, CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.pane_state, focused, buf, |_b, sels| {
        sels.map(|s| Selection::collapsed(s.start()))
    });
    begin_insert_session(state, view);
    Ok(SideEffects::none())
}

pub fn cmd_insert_after(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<SideEffects, CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.pane_state, focused, buf, |b, s| {
        cmd_move_right(b, s, 1, MotionMode::Move)
    });
    begin_insert_session(state, view);
    Ok(SideEffects::none())
}

pub fn cmd_insert_at_line_start(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<SideEffects, CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.pane_state, focused, buf, |b, s| {
        cmd_goto_first_nonblank(b, s, 1, MotionMode::Move)
    });
    begin_insert_session(state, view);
    Ok(SideEffects::none())
}

pub fn cmd_insert_at_line_end(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<SideEffects, CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.pane_state, focused, buf, |b, s| {
        cmd_goto_line_end(b, s, 1, MotionMode::Move)
    });
    doc_ops::apply_doc_motion(&state.buffers, &mut state.pane_state, focused, buf, |b, s| {
        cmd_move_right(b, s, 1, MotionMode::Move)
    });
    begin_insert_session(state, view);
    state.mark_insert_step_back();
    Ok(SideEffects::none())
}

/// Enter insert mode at the start of each selection (min of anchor and head).
/// For a collapsed cursor this is identical to `i`.
pub fn cmd_insert_at_selection_start(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<SideEffects, CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.pane_state, focused, buf, |_b, sels| {
        sels.map(|sel| Selection::collapsed(sel.start()))
    });
    begin_insert_session(state, view);
    Ok(SideEffects::none())
}

/// Enter insert mode after the end of each selection (one past max of anchor and head).
/// For a collapsed cursor this is identical to `a`.
///
/// On Esc, the cursor steps back one grapheme (`mark_insert_step_back`) so that
/// pressing `a` again re-enters Insert at the same spot rather than advancing forward.
/// Clamps to `len_chars() - 1` so `a` on the buffer-final `\n` stays in bounds.
pub fn cmd_insert_at_selection_end(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<SideEffects, CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.pane_state, focused, buf, |b, sels| {
        // len_chars() - 1 is safe: the buffer invariant guarantees at least one char.
        let max = b.len_chars() - 1;
        sels.map(|sel| Selection::collapsed(next_grapheme_boundary(b, sel.end()).min(max)))
    });
    begin_insert_session(state, view);
    state.mark_insert_step_back();
    Ok(SideEffects::none())
}

/// Open a new line below the cursor and enter insert mode.
///
/// `begin_insert_session` opens the edit group so the structural `\n` and
/// everything typed before Esc form one undo step — the same pattern as
/// `cmd_change`.
pub fn cmd_open_line_below(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<SideEffects, CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    begin_insert_session(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.pane_state, focused, buf, |b, s| {
        cmd_goto_line_newline(b, s, 1, MotionMode::Move)
    });
    doc_ops::apply_doc_edit_grouped(&mut state.buffers, &mut state.pane_state, focused, buf, |b, s| {
        insert_char(b, s, '\n')
    });
    Ok(SideEffects::none())
}

/// Open a new line above the cursor and enter insert mode.
pub fn cmd_open_line_above(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<SideEffects, CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    begin_insert_session(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.pane_state, focused, buf, |b, s| {
        cmd_goto_line_start(b, s, 1, MotionMode::Move)
    });
    doc_ops::apply_doc_edit_grouped(&mut state.buffers, &mut state.pane_state, focused, buf, |b, s| {
        insert_char(b, s, '\n')
    });
    doc_ops::apply_doc_motion(&state.buffers, &mut state.pane_state, focused, buf, |b, s| {
        cmd_move_left(b, s, 1, MotionMode::Move)
    });
    Ok(SideEffects::none())
}

pub fn cmd_command_mode(
    state: &mut EditorState,
    _view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<SideEffects, CommandError> {
    let old_mode = state.mode;
    state.history.begin_session_all();
    state.mode = Mode::Command;
    state.minibuf = Some(MiniBuffer {
        prompt: ':',
        input: String::new(),
        cursor: 0,
    });
    enqueue_mode_change(state, old_mode, Mode::Command);
    Ok(SideEffects::none())
}

pub fn cmd_exit_insert(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<SideEffects, CommandError> {
    end_insert_session(state, view);
    Ok(SideEffects::none())
}

// ── Extend mode ───────────────────────────────────────────────────────────────

pub fn cmd_toggle_extend(
    state: &mut EditorState,
    _view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<SideEffects, CommandError> {
    state.mode = if state.mode == EditorMode::Extend {
        EditorMode::Normal
    } else {
        EditorMode::Extend
    };
    Ok(SideEffects::none())
}

/// Collapse each selection to its cursor AND exit extend mode.
///
/// Collapsing is a "done selecting" signal, so extend mode is always cleared.
pub fn cmd_collapse_and_exit_extend(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<SideEffects, CommandError> {
    // Mode is SSOT for extend state; setting Normal implicitly clears Extend.
    state.mode = EditorMode::Normal;
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.pane_state, focused, buf, |b, s| {
        cmd_collapse_selection(b, s, MotionMode::Move)
    });
    Ok(SideEffects::none())
}

// ── Dot repeat ───────────────────────────────────────────────────────────────

/// Replay the last repeatable editing action.
///
/// Count semantics: if the user typed an explicit count before `.`, that count
/// overrides the original; otherwise the original count is reused. This mirrors
/// Vim's behaviour (`3.` → repeat with 3; `.` alone → repeat with original count).
///
/// Kept as `Legacy` variant: it calls `execute_keymap_command` and
/// `handle_insert`, which require `&mut Editor`.
pub fn cmd_repeat(
    ed: &mut Editor,
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let Some(action) = ed.state.last_repeatable_action.take() else {
        return Ok(());
    };

    // Prefer an explicit user count; fall back to the count from the original action.
    let effective_count = if ed.state.explicit_count {
        count
    } else {
        action.count
    };

    // Restore the char arg so wait-char commands (replace, find/till) work.
    ed.state.pending_char = action.char_arg;

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
    if ed.state.mode == EditorMode::Insert {
        ed.end_insert_session();
    } else {
        ed.commit_edit_group_current();
    }

    // Restore the original action so `.` can be pressed again.
    // `execute_keymap_command` may have overwritten `last_repeatable_action` during
    // replay; this final assignment ensures the stored action is always the
    // one the user actually performed.
    ed.state.last_repeatable_action = Some(action);
    Ok(())
}
