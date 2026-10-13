use crate::editor::registry::Count;
use hume_editing::selection::Selection;
use hume_editing::state::EditState;
use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;

use hume_ops::MotionMode;
use hume_ops::edit::{insert_newline_indent, open_line_above};
use hume_ops::motion::{
    cmd_goto_first_nonblank, cmd_goto_line_newline, cmd_goto_line_start, cmd_move_right,
};
use hume_ops::selection_cmd::{cmd_collapse_selection_to_anchor, cmd_collapse_selection_to_head};

use super::super::EditorState;
use super::super::input_stack::{BaseLayer, CommandLayer};
use super::super::replay::{InputOpener, PendingRepeat};
use super::{
    ExitCursor, FocusedPane, apply_focused_edit_grouped, apply_pane_motion, arm_autoindent,
    begin_insert_session, begin_typed_run, end_insert_session,
};
use crate::editor::error::CommandError;

// ── Mode transitions ──────────────────────────────────────────────────────────

/// Enter Insert with the cursors placed by `place`, the shared body of the
/// `i`/`a`/`I`/`A` family. `exit` is what Esc does with an empty typed run.
fn enter_insert(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    exit: ExitCursor,
    place: impl FnOnce(EditState) -> EditState,
) -> Result<(), CommandError> {
    begin_insert_session(state, view, fp, place)?;
    begin_typed_run(state, view, fp, exit);
    Ok(())
}

pub(in crate::editor) fn cmd_insert_before(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    enter_insert(state, view, fp, ExitCursor::StayPut, |st| {
        st.map(|s| Selection::cursor(s.start()))
    })
}

pub(in crate::editor) fn cmd_insert_after(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    enter_insert(state, view, fp, ExitCursor::StayPut, |st| {
        cmd_move_right(st, 1, MotionMode::Move)
    })
}

pub(in crate::editor) fn cmd_insert_at_line_start(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    enter_insert(state, view, fp, ExitCursor::StayPut, |st| {
        cmd_goto_first_nonblank(st, 1, MotionMode::Move)
    })
}

pub(in crate::editor) fn cmd_insert_at_line_end(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    // A cursor on the head line's `\n`, where appended text goes. On an
    // empty line that is the line itself, so `A` equals `i` there.
    enter_insert(state, view, fp, ExitCursor::StepBack, |st| {
        st.map(|sel| Selection::cursor(sel.text().lines().newline(sel.head_line())))
    })
}

/// Enter insert mode at the start of each selection (min of anchor and head).
/// For a collapsed cursor this is identical to `i`.
pub(in crate::editor) fn cmd_insert_at_selection_start(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    enter_insert(state, view, fp, ExitCursor::StayPut, |st| {
        st.map(|sel| Selection::cursor(sel.start()))
    })
}

/// Enter insert mode after the end of each selection (one past max of anchor and head).
/// For a collapsed cursor this is identical to `a`.
///
/// `ExitCursor::StepBack` arms a step-back that applies when there is no
/// typed span to fall back on instead; see `InsertState::step_back_on_exit`'s
/// doc for the full rule.
///
/// If the selection ends on a `\n` (e.g. after `select-line` / `x`, or on an empty
/// line), the cursor stays on that `\n` slot rather than stepping past it; `a` on
/// an empty line is identical to `i`.
pub(in crate::editor) fn cmd_insert_at_selection_end(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    enter_insert(state, view, fp, ExitCursor::StepBack, |st| {
        st.map(|sel| Selection::cursor(sel.append_point()))
    })
}

/// Open a new line below the cursor, carrying over its indent, and enter
/// insert mode.
///
/// `begin_insert_session` opens the edit group so the structural `\n` and
/// everything typed before Esc form one undo step, the same pattern as
/// `cmd_change`. Indent is copied verbatim from the current line, same
/// fallback rule as Enter (`insert_newline_indent`'s own doc); no ownership
/// record yet at this call, so a pre-existing blank line's whitespace isn't
/// vacated here. `arm_autoindent` afterward records the copied indent as
/// this session's own, so it gets trimmed on a bare Esc via the same path
/// Enter uses.
pub(in crate::editor) fn cmd_open_line_below(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    begin_insert_session(state, view, fp, std::convert::identity)?;
    apply_pane_motion(state, view, fp.pane(), |st| {
        cmd_goto_line_newline(st, 1, MotionMode::Move)
    });
    apply_focused_edit_grouped(state, view, fp, |s| insert_newline_indent(s, &[]));
    // Pin after the structural newline, not before: the anchor must mark
    // the start of typed content, not the blank line's own `\n`.
    begin_typed_run(state, view, fp, ExitCursor::StepBack);
    arm_autoindent(state, view, fp);
    Ok(())
}

/// Open a new line above the cursor, carrying over its indent, and enter
/// insert mode. See `cmd_open_line_below`'s doc for the indent/autoindent
/// rationale; identical here.
pub(in crate::editor) fn cmd_open_line_above(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    begin_insert_session(state, view, fp, std::convert::identity)?;
    apply_pane_motion(state, view, fp.pane(), |st| {
        cmd_goto_line_start(st, 1, MotionMode::Move)
    });
    apply_focused_edit_grouped(state, view, fp, open_line_above);
    // Pin after the indent + the structural newline `open_line_above` leaves
    // the cursor on, same reasoning as `cmd_open_line_below`.
    begin_typed_run(state, view, fp, ExitCursor::StepBack);
    arm_autoindent(state, view, fp);
    Ok(())
}

pub(in crate::editor) fn cmd_command_mode(
    state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let input = state
        .waiting_input("command-mode", InputOpener::Code)
        .map_err(CommandError::new)?;
    state.history.begin_session_all();
    state.push_mode_layer(view, CommandLayer::new(input));
    Ok(())
}

pub(in crate::editor) fn cmd_exit_insert(
    state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    end_insert_session(state, view);
    Ok(())
}

/// `completion-trigger` (Ctrl-Space in Insert mode): invoke every
/// registered completion source at the cursor. A native command rather
/// than a plugin's, since the orchestration is the editor's own; the
/// sources it invokes are whatever plugins registered.
///
/// An interactive input (see `EditorState::interactive_input`'s own doc):
/// opening the popup writes nothing itself, and `.` replays the edit that
/// eventually resolves it, so `.` reaching this point refuses rather than
/// firing a fresh completion request on every repeat.
pub(in crate::editor) fn cmd_completion_trigger(
    state: &mut EditorState,
    view: &mut EngineView,
    _fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let input = state
        .interactive_input("completion-trigger")
        .map_err(CommandError::new)?;
    state.trigger_buffer_completion(view, super::super::completion::Trigger::Explicit(input));
    Ok(())
}

// ── Extend mode ───────────────────────────────────────────────────────────────

/// No-op unless the mode layer is `Base`: `EditorMode::Extend` is only ever
/// reported by `BaseLayer::mode()` (`input_stack/base.rs`), so
/// `state.mode() != EditorMode::Extend` reads `true` from *any* other mode
/// (Insert, a minibuf mode). `set_extend` itself "does not gate on the
/// current mode layer" (its own doc): it always writes `Base`'s flag
/// directly. So without this guard, toggling from Insert would arm Extend
/// on `Base` invisibly, with nothing to clear it before the next Esc lands
/// there.
pub(in crate::editor) fn cmd_toggle_extend(
    state: &mut EditorState,
    _view: &mut EngineView,
    _fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if state.input.is::<BaseLayer>(state.input.mode_layer()) {
        state.input.set_extend(state.mode() != EditorMode::Extend);
    }
    Ok(())
}

// Shared body for the two collapse-and-exit-extend handlers. Sets Normal mode
// (clearing Extend) then applies `collapse` to every selection on the focused
// buffer. Extracted to avoid duplicating the focused-pane + apply_doc_motion
// boilerplate.
fn do_collapse_and_exit_extend(
    state: &mut EditorState,
    view: &mut EngineView,
    t: super::CommandPane,
    collapse: impl FnOnce(hume_editing::state::EditState) -> hume_editing::state::EditState,
) {
    state.input.set_extend(false);
    apply_pane_motion(state, view, t, collapse);
}

/// Collapse each selection to its cursor (head) and exit extend mode.
///
/// Collapsing is a "done selecting" signal, so extend mode is always cleared.
pub(in crate::editor) fn cmd_collapse_to_head_and_exit_extend(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_collapse_and_exit_extend(state, view, fp.pane(), |st| {
        cmd_collapse_selection_to_head(st, 0, MotionMode::Move)
    });
    Ok(())
}

/// Collapse each selection to its anchor and exit extend mode.
///
/// Mirror of [`cmd_collapse_to_head_and_exit_extend`]: the cursor lands on the
/// stationary (anchor) end. For a forward word selection this puts the cursor
/// on the first character of the word. Only reachable via the kitty keyboard
/// protocol (`Ctrl-;`); harmless no-op on legacy terminals.
pub(in crate::editor) fn cmd_collapse_to_anchor_and_exit_extend(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_collapse_and_exit_extend(state, view, fp.pane(), |st| {
        cmd_collapse_selection_to_anchor(st, 0, MotionMode::Move)
    });
    Ok(())
}

// ── Dot repeat ───────────────────────────────────────────────────────────────

/// Replay the last repeatable editing action.
///
/// Count semantics: if the user typed an explicit count before `.`, that count
/// replaces `RepeatableAction::count` and is forwarded to the replayed
/// command's body (`replay_dot`); otherwise the original count is reused.
/// This is narrower than Vim's `3.`: the selection recipe (`replay_dot`'s
/// `for step in &action.selection_recipe` loop) always replays with each
/// step's own recorded `step.count`, never the override, and no built-in
/// repeatable command body reads its `count` parameter today; every one
/// takes `_count: usize`. So for every native command, `3.` and `.` produce
/// identical results; the override is currently observable only by a Steel
/// `#:repeatable` command that reads its `count` lambda argument.
///
/// The handler only enqueues a `PendingRepeat` marker; the actual replay
/// (edit-group bracketing, re-dispatch, insert-key replay) runs in
/// `replay_dot` at the tail of `handle_key`, where `&mut Editor` is
/// available for `Editor::dispatch`. This keeps the
/// invariant that no `EditorCmd` handler takes `&mut Editor` (see
/// `EditorCmdBody` in `registry/command.rs`).
pub(in crate::editor) fn cmd_repeat(
    state: &mut EditorState,
    _view: &mut EngineView,
    _fp: FocusedPane,
    count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    // Peek without taking: replay_dot owns the take so it can
    // restore the recording after replay.
    let Some(orig_count) = state.recording.as_ref().map(|r| r.count()) else {
        return Ok(());
    };
    // Prefer an explicit user count; fall back to the count from the original action.
    let effective = if count.is_typed() {
        count.get()
    } else {
        orig_count
    };
    debug_assert!(
        state.pending_repeat.is_none(),
        "one key queues at most one repeat"
    );
    state.pending_repeat = Some(PendingRepeat { count: effective });
    Ok(())
}
