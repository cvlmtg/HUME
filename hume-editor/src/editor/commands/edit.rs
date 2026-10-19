use crate::editor::registry::Count;
use hume_engine::pipeline::EngineView;

use crate::editor::buffer::{Buffer, HistoryWalkResult};
use hume_editing::selection::{Facing, Selection};
use hume_editing::word::WordChars;
use hume_ops::MotionMode;
use hume_ops::edit::yank_selections;
use hume_ops::edit::{
    IncrementRefusal, Stepping, align_selections, apply_increment_plan, delete_selection,
    delete_selection_content, delete_word_backward, indent_lines, join_lines_select_spaces,
    plan_increment, replace_selections, unindent_lines,
};
use hume_ops::register::{CLIPBOARD_REGISTER, KILL_RING_REGISTER, Piece};
use hume_ops::surround::wrap_each_selection;

use super::super::{EditorState, Severity, doc_ops};
use super::{
    CommandPane, ExitCursor, FocusedPane, apply_focused_edit_grouped, apply_pane_edit,
    apply_pane_motion, begin_insert_session_preserving_register, begin_typed_run, doc, tab_format,
    word_chars_owned,
};
use crate::editor::error::CommandError;
use crate::editor::position_stores::PositionStores;
use hume_engine::pipeline::{BufferId, PaneId};

// ── Edit composites ───────────────────────────────────────────────────────────

/// Delete the selections and put what they removed in the active register.
/// Nothing is written when no selection removed anything.
///
/// **Bare default** (no `"<reg>` prefix): pushes to the kill ring only.
/// **Explicit register**: routes through `write_register`.
pub(in crate::editor) fn cmd_delete(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    super::ensure_writable(state, view, t)?;
    let mut yanked = Vec::new();
    let edited = apply_pane_edit(state, view, t, |s| {
        let removal = delete_selection(s);
        yanked = removal.yanked;
        removal.edited
    });
    state.route_kill(yanked);
    edited
}

/// Yank, delete, then enter insert mode, all in one undo group.
///
/// **Bare default**: pushes to kill ring only. **Explicit register**: routes through
/// `write_register`, same as `cmd_delete`.
///
/// Unlike `d`, a trailing `\n` at the end of a selection is not deleted: `c`
/// clears line content but keeps the line. The register gets what was
/// removed.
pub(in crate::editor) fn cmd_change(
    state: &mut EditorState,
    view: &mut EngineView,
    fp: FocusedPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    // Preserving, not `begin_insert_session`: `c` is itself a register-
    // consuming operator (see `state.route_kill` below), so clearing the
    // prefix here would consume it a step too early.
    let entry = begin_insert_session_preserving_register(state, view, fp, std::convert::identity)?;
    let mut yanked = Vec::new();
    apply_focused_edit_grouped(state, view, fp, |s| {
        let removal = delete_selection_content(s);
        yanked = removal.yanked;
        removal.edited
    });
    // Pins the anchor `mii` and (if `insert-select-on-exit` is on) Esc itself
    // reconstruct the typed replacement from: the same helper every insert-entry
    // command uses, so `c`'s auto-select behaves identically to theirs. `c`
    // never steps the cursor back on an empty run, same as `i`/`I`.
    begin_typed_run(state, view, &entry, fp, ExitCursor::StayPut);
    // Kill-opened only when the yank actually captured to the ring: the
    // capture stamped `PasteStamp`, but every keystroke about to be typed in
    // the session bumps `edit_seq` and would strand it. The flag makes
    // `tear_down_insert` refresh the stamp's `seq` once typing stops. An
    // explicit-register change (`"5c`) writes no stamp, and refreshing
    // whatever stale stamp might pre-exist would wrongly resurrect it.
    if state.route_kill(yanked) {
        state.input.entered_mut(&entry).kill_opened_session = true;
    }
    Ok(())
}

/// Select the span(s) typed during the most recently completed insert
/// session (`i`/`a`/`o`/`O`/`A`/`I`/`c`/…), bound at `mii`.
///
/// Like every other object in the `mi`/`ma` trie, honors [`MotionMode`]:
/// `Move` replaces the current selection with just the insertion spans;
/// `Extend` unions them into the current selection set instead of
/// discarding it, matching the `.extendable()` contract.
///
/// Refuses (leaving selections untouched) if there is no stashed insertion,
/// or if the buffer's text was replaced since (see
/// `BufferPositions::last_inserts`).
pub(in crate::editor) fn cmd_select_last_insertion(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let buf = doc(state, view, t);
    let fresh = state
        .buffer_positions
        .last_inserts
        .get(t.bid(view))
        .and_then(|last| last.get(buf.text()))
        .cloned();
    let Some(spans) = fresh else {
        return Err(CommandError::transient("no last insertion"));
    };
    // Non-empty: `tear_down_insert` only ever stashes a
    // non-empty `spans` vec (see `begin_typed_run`'s caller). The last
    // span is spatially last (stashed in ascending-start order), so primary
    // there, matching the entry command's own cursor placement.
    let insertion_primary = spans.len() - 1;
    let insertion_sels: Vec<Selection> = spans
        .into_iter()
        .map(|r| Selection::covering(r, Facing::Forward))
        .collect();
    apply_pane_motion(state, view, t, move |st| match mode {
        MotionMode::Move => st.with_selections(insertion_sels, insertion_primary),
        MotionMode::Extend => {
            // `with_selections` sorts and merges overlapping selections,
            // so this is a plain union, with no need to zip against current
            // selections one-to-one (their counts can differ freely, e.g.
            // `mii` invoked after the selection count changed since the
            // insert). Merely-adjacent (touching, non-overlapping) spans
            // stay separate selections, same as everywhere else in the
            // codebase. The pre-existing primary stays primary, consistent
            // with how every other `mi*` object behaves in Extend mode.
            let view = st.view();
            let primary = view.primary().index();
            let mut combined: Vec<Selection> = view.iter().map(|s| s.selection()).collect();
            combined.extend(insertion_sels);
            st.with_selections(combined, primary)
        }
    });
    Ok(())
}

/// Yank selections without deleting.
///
/// **Bare default**: writes to the system clipboard AND pushes to the kill ring.
/// **Explicit register**: routes through `write_register`.
pub(in crate::editor) fn cmd_yank(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let text = super::doc(state, view, t).text();
    let yanked = yank_selections(&t.state(&state.panes.entries, view).state(text));
    let prefix = state.take_register_prefix();
    if yanked.iter().all(Piece::is_empty) {
        return Ok(());
    }
    match prefix {
        None => {
            state.write_register(CLIPBOARD_REGISTER, yanked.clone());
            state.capture_to_ring(yanked);
        }
        // "ky: push to ring only (no clipboard).
        Some(KILL_RING_REGISTER) => state.capture_to_ring(yanked),
        Some(reg) => state.write_register(reg, yanked),
    }
    Ok(())
}

/// Exhaustion messages `history_step` reports below.
const UNDO_EXHAUSTED_MSG: &str = "Already at oldest change";
const REDO_EXHAUSTED_MSG: &str = "Already at newest change";

/// Run one history walk on `t`'s buffer through `t`'s pane: the one place
/// `PositionStores` is assembled for `doc_ops::apply_doc_history_walk`.
fn walk_history(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
    walk: impl FnOnce(
        &mut Buffer,
        BufferId,
        &mut PositionStores<'_>,
        PaneId,
    ) -> Result<HistoryWalkResult, CommandError>,
) -> Result<doc_ops::HistoryWalk, CommandError> {
    let buf = t.bid(view);
    let (buffers, mut stores) = state.edit_parts();
    doc_ops::apply_doc_history_walk(buffers, &mut stores, t.pid(), buf, walk)
}

/// Walk the undo/redo history `count` steps as one composed transform,
/// reporting exhaustion when the walk fell short. Calls `finish_edit`
/// once per walk.
fn history_step(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
    count: usize,
    walk: fn(&mut Buffer, BufferId, &mut PositionStores<'_>, PaneId, usize) -> HistoryWalkResult,
    exhausted_msg: &str,
) -> Result<(), CommandError> {
    let result = walk_history(state, view, t, |b, id, stores, pane| {
        Ok(walk(b, id, stores, pane, count))
    })?;
    match result {
        doc_ops::HistoryWalk::RefusedReadOnly => return Err(super::read_only_refusal(state)),
        doc_ops::HistoryWalk::Took(taken) if taken < count => {
            state.report(Severity::Info, exhausted_msg.to_string());
        }
        doc_ops::HistoryWalk::Took(_) => {}
    }
    Ok(())
}

/// Jump `t`'s buffer to revision `n` of its undo history, across branches,
/// as one composed transform. `Err` when the buffer has no revision `n` once
/// the walk's session handling has run; otherwise the walk's own result, so a
/// caller can tell a read-only refusal from a jump to the revision it is
/// already on (`Took(0)`).
pub(in crate::editor) fn goto_revision(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
    n: usize,
) -> Result<doc_ops::HistoryWalk, CommandError> {
    walk_history(state, view, t, |b, id, stores, pane| {
        let target = b.revision(n).ok_or_else(|| {
            CommandError::transient(format!("no revision {n} in this buffer's undo history"))
        })?;
        Ok(b.goto_revision(id, stores, pane, target))
    })
}

/// Undo `steps` revisions of `t`'s buffer: the walk `u` and `:earlier` share.
/// `steps` is exact, so zero takes no step.
pub(in crate::editor) fn undo_steps(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
    steps: usize,
) -> Result<(), CommandError> {
    history_step(state, view, t, steps, Buffer::undo_n, UNDO_EXHAUSTED_MSG)
}

/// [`undo_steps`] in the redo direction: `Ctrl-r` and `:later`.
pub(in crate::editor) fn redo_steps(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
    steps: usize,
) -> Result<(), CommandError> {
    history_step(state, view, t, steps, Buffer::redo_n, REDO_EXHAUSTED_MSG)
}

pub(in crate::editor) fn cmd_undo(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    undo_steps(state, view, t, count.get())
}

pub(in crate::editor) fn cmd_redo(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    redo_steps(state, view, t, count.get())
}

// ── Replace / surround ────────────────────────────────────────────────────────

/// Replace every character in each selection with `ch`.
pub(in crate::editor) fn cmd_replace(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
    ch: char,
) -> Result<(), CommandError> {
    apply_pane_edit(state, view, t, |s| replace_selections(s, ch))?;
    Ok(())
}

/// Join lines inside each selection and select the inserted spaces.
pub(in crate::editor) fn cmd_join_lines_select_spaces(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    apply_pane_edit(state, view, t, join_lines_select_spaces)?;
    Ok(())
}

/// Align each selection's anchor to the primary selection's anchor column.
pub(in crate::editor) fn cmd_align_selections(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let buf_id = t.bid(view);
    let tab_width = state
        .buffers
        .get(buf_id)
        .overrides
        .indent_width(&state.settings);
    apply_pane_edit(state, view, t, move |sels| {
        align_selections(sels, tab_width)
    })?;
    Ok(())
}

/// Indent every line touched by a selection by `count` levels (`>`).
pub(in crate::editor) fn cmd_indent(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let buf_id = t.bid(view);
    let (style, tab_width) = tab_format(state.buffers.get(buf_id), &state.settings);
    apply_pane_edit(state, view, t, move |sels| {
        indent_lines(sels, style, tab_width, count.get())
    })?;
    Ok(())
}

/// Unindent every line touched by a selection by `count` levels (`<`).
pub(in crate::editor) fn cmd_unindent(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let buf_id = t.bid(view);
    let (style, tab_width) = tab_format(state.buffers.get(buf_id), &state.settings);
    apply_pane_edit(state, view, t, move |sels| {
        unindent_lines(sels, style, tab_width, count.get())
    })?;
    Ok(())
}

// ── Increment / decrement ─────────────────────────────────────────────────────

/// Add `count` to the number, date or time at each selection (`Ctrl-a`).
pub(in crate::editor) fn cmd_increment(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    step_targets(state, view, t, step_of(count), Stepping::Uniform)
}

/// Subtract `count` from the number, date or time at each selection
/// (`Ctrl-s`).
pub(in crate::editor) fn cmd_decrement(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    step_targets(state, view, t, -step_of(count), Stepping::Uniform)
}

/// Add n times `count` to the n-th selection's number, date or time
/// (`G A`).
pub(in crate::editor) fn cmd_increment_sequence(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    step_targets(state, view, t, step_of(count), Stepping::Sequence)
}

/// Subtract n times `count` from the n-th selection's number, date or time
/// (`G S`).
pub(in crate::editor) fn cmd_decrement_sequence(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    step_targets(state, view, t, -step_of(count), Stepping::Sequence)
}

fn step_of(count: Count) -> i128 {
    i128::try_from(count.get()).expect("a count fits an i128")
}

fn increment_refusal(refusal: IncrementRefusal) -> CommandError {
    match refusal {
        IncrementRefusal::NoTarget => CommandError::transient("No number, date or time to change"),
        IncrementRefusal::OutOfRange => CommandError::transient("Result out of range"),
    }
}

/// Step each selection's target by `step`. Planning before touching the
/// buffer lets a refusal leave it untouched, with no undo revision.
fn step_targets(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    step: i128,
    stepping: Stepping,
) -> Result<(), CommandError> {
    super::ensure_writable(state, view, t)?;
    let word_chars = word_chars_owned(state.buffers.get(t.bid(view)), &state.settings);
    let plan = plan_increment(
        super::pane_view(state, view, t),
        step,
        stepping,
        WordChars::new(&word_chars),
    )
    .map_err(increment_refusal)?;
    apply_pane_edit(state, view, t, move |edit| apply_increment_plan(edit, plan))
}

/// Delete the word before each cursor (Ctrl-w in insert mode).
///
/// Promoted from a plain `NativeCommand::Edit` to an `EditorCmd` so it can
/// resolve this buffer's `word-chars` and close over it, the same pattern
/// [`cmd_align_selections`] uses for `tab_width`. Ctrl-w is a *word*
/// operation by name: leaving it on the built-in word rule would mean `b`
/// then `d` deletes a whole hyphenated run while Ctrl-w deletes only the
/// last piece, a split a user would notice within a minute.
pub(in crate::editor) fn cmd_delete_word_backward(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let buf_id = t.bid(view);
    let word_chars = word_chars_owned(state.buffers.get(buf_id), &state.settings);
    apply_pane_edit(state, view, t, move |sels| {
        delete_word_backward(sels, WordChars::new(&word_chars))
    })?;
    Ok(())
}

/// Wrap every selection with the pair `ch` opens or closes, or with `ch` on
/// both sides when it is neither.
pub(in crate::editor) fn cmd_surround_add(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: Count,
    _mode: MotionMode,
    ch: char,
) -> Result<(), CommandError> {
    let (_ap_enabled, ap_pairs) = super::doc(state, view, t)
        .overrides
        .auto_pairs_ref(&state.settings);
    let (open, close) = ap_pairs
        .iter()
        .find(|p| p.open == ch || p.close == ch)
        .map(|p| (p.open, p.close))
        .unwrap_or((ch, ch));
    apply_pane_edit(state, view, t, |s| wrap_each_selection(s, open, close))?;
    Ok(())
}
