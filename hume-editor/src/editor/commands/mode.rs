use hume_editing::grapheme::next_grapheme_boundary;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::Text;
use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;

use crate::ops::MotionMode;
use crate::ops::edit::insert_char;
use crate::ops::motion::{
    cmd_goto_first_nonblank, cmd_goto_line_end, cmd_goto_line_newline, cmd_goto_line_start,
    cmd_move_left, cmd_move_right,
};
use crate::ops::selection_cmd::{cmd_collapse_selection_to_anchor, cmd_collapse_selection_to_head};

use super::super::{EditorState, MiniBuffer, Mode, PendingRepeat};
use super::{
    apply_focused_edit_grouped, apply_focused_motion, begin_insert_session, end_insert_session,
};
use crate::editor::error::CommandError;

// ── Mode transitions ──────────────────────────────────────────────────────────

pub fn cmd_insert_before(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    apply_focused_motion(state, view, |_b, sels| {
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
    apply_focused_motion(state, view, |b, s| {
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
    apply_focused_motion(state, view, |b, s| {
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
    apply_focused_motion(state, view, |b, s| {
        // Move to line content-end, then step right onto the \n slot — unless the
        // line is empty, in which case line-end is already the \n and stepping past
        // it would land on the next line.
        let max = b.len_chars() - 1;
        let at_end = cmd_goto_line_end(b, s, 1, MotionMode::Move);
        at_end.map(|sel| {
            let pos = if sel.ends_on_newline(b) {
                // Empty line — cursor is on the \n; inserting here equals `i`.
                sel.head()
            } else {
                // Non-empty line — advance one grapheme onto the trailing \n slot.
                next_grapheme_boundary(b, sel.head()).min(max)
            };
            Selection::collapsed(pos)
        })
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
    apply_focused_motion(state, view, |_b, sels| {
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
///
/// If the selection ends on a `\n` (e.g. after `select-line` / `x`, or on an empty
/// line), the cursor stays on that `\n` slot rather than stepping past it — `a` on
/// an empty line is identical to `i`.
pub fn cmd_insert_at_selection_end(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    apply_focused_motion(state, view, |b, sels| {
        // len_chars() - 1 is safe: the buffer invariant guarantees at least one char.
        let max = b.len_chars() - 1;
        sels.map(|sel| {
            let pos = if sel.ends_on_newline(b) {
                sel.end() // selection ends on '\n' — insert before it, not past it
            } else {
                next_grapheme_boundary(b, sel.end())
            };
            Selection::collapsed(pos.min(max))
        })
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
    begin_insert_session(state, view);
    state.mark_insert_step_back();
    apply_focused_motion(state, view, |b, s| {
        cmd_goto_line_newline(b, s, 1, MotionMode::Move)
    });
    apply_focused_edit_grouped(state, view, |b, s| insert_char(b, s, '\n'));
    Ok(())
}

/// Open a new line above the cursor and enter insert mode.
pub fn cmd_open_line_above(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    begin_insert_session(state, view);
    state.mark_insert_step_back();
    apply_focused_motion(state, view, |b, s| {
        cmd_goto_line_start(b, s, 1, MotionMode::Move)
    });
    apply_focused_edit_grouped(state, view, |b, s| insert_char(b, s, '\n'));
    apply_focused_motion(state, view, |b, s| cmd_move_left(b, s, 1, MotionMode::Move));
    Ok(())
}

pub fn cmd_command_mode(
    state: &mut EditorState,
    _view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    state.history.begin_session_all();
    state.minibuf = Some(MiniBuffer {
        prompt: ":".to_string(),
        input: String::new(),
        cursor: 0,
    });
    state.set_mode(Mode::Command);
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
    let target = if state.mode() == EditorMode::Extend {
        EditorMode::Normal
    } else {
        EditorMode::Extend
    };
    state.set_mode(target);
    Ok(())
}

// Shared body for the two collapse-and-exit-extend handlers. Sets Normal mode
// (clearing Extend) then applies `collapse` to every selection on the focused
// buffer. Extracted to avoid duplicating the focused-pane + apply_doc_motion
// boilerplate.
fn do_collapse_and_exit_extend(
    state: &mut EditorState,
    view: &mut EngineView,
    collapse: impl FnOnce(&Text, SelectionSet) -> SelectionSet,
) {
    state.set_mode(EditorMode::Normal);
    apply_focused_motion(state, view, collapse);
}

/// Collapse each selection to its cursor (head) and exit extend mode.
///
/// Collapsing is a "done selecting" signal, so extend mode is always cleared.
pub fn cmd_collapse_to_head_and_exit_extend(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_collapse_and_exit_extend(state, view, |b, s| {
        cmd_collapse_selection_to_head(b, s, 0, MotionMode::Move)
    });
    Ok(())
}

/// Collapse each selection to its anchor and exit extend mode.
///
/// Mirror of [`cmd_collapse_to_head_and_exit_extend`] — the cursor lands on the
/// stationary (anchor) end. For a forward word selection this puts the cursor
/// on the first character of the word. Only reachable via the kitty keyboard
/// protocol (`Ctrl+;`); harmless no-op on legacy terminals.
pub fn cmd_collapse_to_anchor_and_exit_extend(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_collapse_and_exit_extend(state, view, |b, s| {
        cmd_collapse_selection_to_anchor(b, s, 0, MotionMode::Move)
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
/// `replay_dot` at the tail of `handle_key`, where `&mut Editor` is available
/// for `run_native_body`/`run_steel_command` and `handle_insert`. This satisfies
/// the D7 invariant: no EditorCmd handler takes `&mut Editor`.
pub fn cmd_repeat(
    state: &mut EditorState,
    _view: &mut EngineView,
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    // Peek without taking — replay_dot owns the take so it can
    // restore the action after replay.
    let Some(orig_count) = state.last_repeatable_action.as_ref().map(|a| a.count) else {
        return Ok(());
    };
    // Prefer an explicit user count; fall back to the count from the original action.
    let effective = if state.explicit_count {
        count
    } else {
        orig_count
    };
    state.pending_repeat = Some(PendingRepeat { count: effective });
    Ok(())
}
