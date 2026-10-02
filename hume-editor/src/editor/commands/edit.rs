use hume_engine::pipeline::EngineView;

use crate::editor::buffer::{Buffer, HistoryWalkResult};
use hume_editing::selection::{Facing, Selection};
use hume_editing::word::WordChars;
use hume_ops::MotionMode;
use hume_ops::edit::yank_selections;
use hume_ops::edit::{
    align_selections, delete_selection, delete_selection_content, delete_word_backward,
    indent_lines, join_lines_select_spaces, replace_selections, unindent_lines,
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
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if super::refuse_if_read_only(state, view, t) {
        return Ok(());
    }
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
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if super::refuse_if_read_only(state, view, fp.pane()) {
        return Ok(());
    }
    // Preserving, not `begin_insert_session`: `c` is itself a register-
    // consuming operator (see `state.route_kill` below), so clearing the
    // prefix here would consume it a step too early.
    begin_insert_session_preserving_register(state, view, fp)?;
    let mut yanked = Vec::new();
    apply_focused_edit_grouped(state, view, fp, |s| {
        let removal = delete_selection_content(s);
        yanked = removal.yanked;
        removal.edited
    });
    // Pins the anchor `mii` and (if `select-inserted-text` is on) Esc itself
    // reconstruct the typed replacement from: the same helper every insert-entry
    // command uses, so `c`'s auto-select behaves identically to theirs. `c`
    // never steps the cursor back on an empty run, same as `i`/`I`.
    begin_typed_run(state, view, fp, ExitCursor::StayPut);
    // Kill-opened only when the yank actually captured to the ring: the
    // capture stamped `PasteStamp`, but every keystroke about to be typed in
    // the session bumps `edit_seq` and would strand it. The flag makes
    // `end_insert_session` refresh the stamp's `seq` once typing stops. An
    // explicit-register change (`"5c`) writes no stamp, and refreshing
    // whatever stale stamp might pre-exist would wrongly resurrect it. Lives
    // on `PaneBufferState` for the same reason `step_back_on_exit` does (see
    // its doc).
    if state.route_kill(yanked) {
        let bid = fp.bid(view);
        state.panes.state[fp.pid()][bid].kill_opened_session = true;
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
    _count: usize,
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
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let text = super::doc(state, view, t).text();
    let yanked = yank_selections(&t.state(&state.panes.state, view).state(text));
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
    doc_ops::apply_doc_history_walk(
        &mut state.buffers,
        &mut PositionStores::new(
            &mut state.panes,
            &mut state.input,
            &mut state.buffer_positions,
            &mut state.config.decorations,
        ),
        &mut state.active_session,
        t.pid(),
        buf,
        walk,
    )
}

/// Walk the undo/redo history `count` steps as one composed transform,
/// reporting exhaustion when the walk fell short. Calls `finish_edit`
/// once per walk.
fn history_step(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: usize,
    walk: fn(&mut Buffer, BufferId, &mut PositionStores<'_>, PaneId, usize) -> HistoryWalkResult,
    exhausted_msg: &str,
) -> Result<(), CommandError> {
    let result = walk_history(state, view, t, |b, id, stores, pane| {
        Ok(walk(b, id, stores, pane, count))
    })?;
    // `RefusedReadOnly` stays a distinct arm rather than folding into
    // `Took(0)`. See `HistoryWalk`'s own doc for why.
    if let doc_ops::HistoryWalk::Took(taken) = result
        && taken < count
    {
        state.report(Severity::Info, exhausted_msg.to_string());
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

pub(in crate::editor) fn cmd_undo(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if super::refuse_if_read_only(state, view, t) {
        return Ok(());
    }
    history_step(state, view, t, count, Buffer::undo_n, UNDO_EXHAUSTED_MSG)
}

/// See [`cmd_undo`]'s doc: same sharing, redo direction.
pub(in crate::editor) fn cmd_redo(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if super::refuse_if_read_only(state, view, t) {
        return Ok(());
    }
    history_step(state, view, t, count, Buffer::redo_n, REDO_EXHAUSTED_MSG)
}

// ── Replace / surround ────────────────────────────────────────────────────────

/// Replace every character in each selection with the next typed character.
pub(in crate::editor) fn cmd_replace(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if let Some(ch) = state.pending_char.take() {
        apply_pane_edit(state, view, t, |s| replace_selections(s, ch))?;
    }
    Ok(())
}

/// Join lines inside each selection and select the inserted spaces.
pub(in crate::editor) fn cmd_join_lines_select_spaces(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: usize,
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
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let buf_id = t.bid(view);
    let tab_width = state
        .buffers
        .get(buf_id)
        .overrides
        .tab_width(&state.settings);
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
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let buf_id = t.bid(view);
    let (style, tab_width) = tab_format(state.buffers.get(buf_id), &state.settings);
    apply_pane_edit(state, view, t, move |sels| {
        indent_lines(sels, style, tab_width, count)
    })?;
    Ok(())
}

/// Unindent every line touched by a selection by `count` levels (`<`).
pub(in crate::editor) fn cmd_unindent(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let buf_id = t.bid(view);
    let (style, tab_width) = tab_format(state.buffers.get(buf_id), &state.settings);
    apply_pane_edit(state, view, t, move |sels| {
        unindent_lines(sels, style, tab_width, count)
    })?;
    Ok(())
}

/// Delete the word before each cursor (Ctrl-w in insert mode).
///
/// Promoted from a plain `MappableCommand::Edit` to an `EditorCmd` so it can
/// resolve this buffer's `word-chars` and close over it, the same pattern
/// [`cmd_align_selections`] uses for `tab_width`. Ctrl-w is a *word*
/// operation by name: leaving it on the built-in word rule would mean `b`
/// then `d` deletes a whole hyphenated run while Ctrl-w deletes only the
/// last piece, a split a user would notice within a minute.
pub(in crate::editor) fn cmd_delete_word_backward(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let buf_id = t.bid(view);
    let word_chars = word_chars_owned(state.buffers.get(buf_id), &state.settings);
    apply_pane_edit(state, view, t, move |sels| {
        delete_word_backward(sels, WordChars::new(&word_chars))
    })?;
    Ok(())
}

/// Wrap every selection with a pair determined by the next typed character.
pub(in crate::editor) fn cmd_surround_add(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let Some(ch) = state.pending_char.take() else {
        return Ok(());
    };
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
