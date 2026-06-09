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

use super::super::{doc_ops, EditorState, MiniBuffer, Mode, PendingRepeat};
use crate::editor::error::CommandError;
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
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf, |_b, sels| {
        sels.map(|s| Selection::collapsed(s.start()))
    });
    begin_insert_session(state, view);
    Ok(())
}

pub fn cmd_insert_after(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf, |b, s| {
        cmd_move_right(b, s, 1, MotionMode::Move)
    });
    begin_insert_session(state, view);
    Ok(())
}

pub fn cmd_insert_at_line_start(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf, |b, s| {
        cmd_goto_first_nonblank(b, s, 1, MotionMode::Move)
    });
    begin_insert_session(state, view);
    Ok(())
}

pub fn cmd_insert_at_line_end(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf, |b, s| {
        cmd_goto_line_end(b, s, 1, MotionMode::Move)
    });
    doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf, |b, s| {
        cmd_move_right(b, s, 1, MotionMode::Move)
    });
    begin_insert_session(state, view);
    state.mark_insert_step_back();
    Ok(())
}

/// Enter insert mode at the start of each selection (min of anchor and head).
/// For a collapsed cursor this is identical to `i`.
pub fn cmd_insert_at_selection_start(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf, |_b, sels| {
        sels.map(|sel| Selection::collapsed(sel.start()))
    });
    begin_insert_session(state, view);
    Ok(())
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
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf, |b, sels| {
        // len_chars() - 1 is safe: the buffer invariant guarantees at least one char.
        let max = b.len_chars() - 1;
        sels.map(|sel| Selection::collapsed(next_grapheme_boundary(b, sel.end()).min(max)))
    });
    begin_insert_session(state, view);
    state.mark_insert_step_back();
    Ok(())
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
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    begin_insert_session(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf, |b, s| {
        cmd_goto_line_newline(b, s, 1, MotionMode::Move)
    });
    doc_ops::apply_doc_edit_grouped(&mut state.buffers, &mut state.panes.state, focused, buf, |b, s| {
        insert_char(b, s, '\n')
    });
    Ok(())
}

/// Open a new line above the cursor and enter insert mode.
pub fn cmd_open_line_above(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    begin_insert_session(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf, |b, s| {
        cmd_goto_line_start(b, s, 1, MotionMode::Move)
    });
    doc_ops::apply_doc_edit_grouped(&mut state.buffers, &mut state.panes.state, focused, buf, |b, s| {
        insert_char(b, s, '\n')
    });
    doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf, |b, s| {
        cmd_move_left(b, s, 1, MotionMode::Move)
    });
    Ok(())
}

pub fn cmd_command_mode(
    state: &mut EditorState,
    _view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let old_mode = state.mode;
    state.history.begin_session_all();
    state.mode = Mode::Command;
    state.minibuf = Some(MiniBuffer {
        prompt: ':',
        input: String::new(),
        cursor: 0,
    });
    enqueue_mode_change(state, old_mode, Mode::Command);
    Ok(())
}

pub fn cmd_exit_insert(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    end_insert_session(state, view);
    Ok(())
}

// ── Extend mode ───────────────────────────────────────────────────────────────

pub fn cmd_toggle_extend(
    state: &mut EditorState,
    _view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    state.mode = if state.mode == EditorMode::Extend {
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
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    // Mode is SSOT for extend state; setting Normal implicitly clears Extend.
    state.mode = EditorMode::Normal;
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf, |b, s| {
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
///
/// The handler only enqueues a `PendingRepeat` marker; the actual replay
/// (edit-group bracketing, re-dispatch, insert-key replay) runs in
/// `drain_pending_repeat` at the tail of `handle_key`, where `&mut Editor`
/// is available for `execute_keymap_command` and `handle_insert`. This satisfies
/// the D7 invariant: no EditorCmd handler takes `&mut Editor`.
pub fn cmd_repeat(
    state: &mut EditorState,
    _view: &mut EngineView,
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    // Peek without taking — drain_pending_repeat owns the take so it can
    // restore the action after replay.
    let Some(orig_count) = state.last_repeatable_action.as_ref().map(|a| a.count) else {
        return Ok(());
    };
    // Prefer an explicit user count; fall back to the count from the original action.
    let effective = if state.explicit_count { count } else { orig_count };
    state.pending_repeat = Some(PendingRepeat { count: effective });
    Ok(())
}
