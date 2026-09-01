use hume_engine::pipeline::EngineView;

use crate::editor::buffer::Buffer;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::word::WordChars;
use hume_ops::MotionMode;
use hume_ops::edit::{
    align_selections, change_span, delete_selection, delete_selection_content,
    delete_word_backward, indent_lines, join_lines_select_spaces, replace_selections,
    unindent_lines,
};
use hume_ops::register::{CLIPBOARD_REGISTER, KILL_RING_REGISTER, yank_selections};
use hume_ops::surround::wrap_each_selection;

use super::super::{EditorState, Severity, doc_ops};
use super::{
    apply_focused_edit, apply_focused_edit_grouped, apply_focused_motion,
    begin_insert_session_preserving_register, doc, focused_buffer_id, pin_insert_anchors,
    tab_format, word_chars_owned,
};
use crate::editor::error::CommandError;

// ── Edit composites ───────────────────────────────────────────────────────────

/// Yank selections into the active register, then delete them.
///
/// **Bare default** (no `"<reg>` prefix): pushes to the kill ring only.
/// **Explicit register**: routes through `write_register`.
pub(crate) fn cmd_delete(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if super::refuse_if_read_only(state, view) {
        return Ok(());
    }
    let yanked = yank_selections(
        super::doc(state, view).text(),
        super::current_selections(state, view),
    );
    apply_focused_edit(state, view, delete_selection);
    state.route_kill(yanked);
    Ok(())
}

/// Yank, delete, then enter insert mode — all in one undo group.
///
/// **Bare default**: pushes to kill ring only. **Explicit register**: routes through
/// `write_register` — same as `cmd_delete`.
///
/// Unlike `d`, a trailing `\n` at the end of a selection is not deleted — `c`
/// clears line content but keeps the line. The yank is trimmed accordingly so
/// the kill-ring entry matches what was removed (no trailing `\n`).
pub(crate) fn cmd_change(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if super::refuse_if_read_only(state, view) {
        return Ok(());
    }
    let yanked = {
        let doc = super::doc(state, view);
        let sels = super::current_selections(state, view);
        sels.iter_sorted()
            .map(|sel| {
                let (start, stop) = change_span(doc.text(), sel);
                doc.text().slice(start..stop).to_string()
            })
            .collect::<Vec<_>>()
    };
    // Preserving, not `begin_insert_session`: `c` is itself a register-
    // consuming operator (see `state.route_kill` below) — clearing the
    // prefix here would consume it a step too early.
    begin_insert_session_preserving_register(state, view);
    apply_focused_edit_grouped(state, view, delete_selection_content);
    pin_insert_anchors(state, view);
    // Auto-select the typed replacement on exit only when the setting is on
    // (`mii` can still recover it later regardless — see `pin_insert_anchors`).
    // Gated on the group actually being open (skips read-only buffers, and
    // re-captures correctly on dot-repeat replay, which pre-opens the group).
    if doc(state, view)
        .overrides
        .select_changed_text(&state.settings)
        && super::is_group_open_current(state, view)
    {
        let pid = state.focused_pane_id;
        let bid = focused_buffer_id(state, view);
        state.panes.state[pid][bid].select_on_exit = true;
    }
    // Kill-opened only when the yank actually captured to the ring: the
    // capture stamped `PasteStamp`, but every keystroke about to be typed in
    // the session bumps `edit_seq` and would strand it — the flag makes
    // `end_insert_session` refresh the stamp's `seq` once typing stops. An
    // explicit-register change (`"5c`) writes no stamp, and refreshing
    // whatever stale stamp might pre-exist would wrongly resurrect it. Lives
    // on `PaneBufferState`, not `InsertSession`, for the same reason
    // `select_on_exit` does (see its doc).
    if state.route_kill(yanked) {
        let pid = state.focused_pane_id;
        let bid = focused_buffer_id(state, view);
        state.panes.state[pid][bid].kill_opened_session = true;
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
/// Reports [`Severity::Info`] and leaves selections untouched if there is no
/// stashed insertion, or if a later mutation (any edit, undo, or redo) has
/// moved the buffer's `text_gen` past the stamp — see
/// [`crate::editor::buffer::LastInsert`].
pub(crate) fn cmd_select_last_insertion(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let buf = doc(state, view);
    let fresh = buf
        .last_insert
        .as_ref()
        .filter(|last| last.text_gen == buf.text_gen)
        .map(|last| last.spans.clone());
    let Some(spans) = fresh else {
        state.report(Severity::Info, "no last insertion".to_string());
        return Ok(());
    };
    // Non-empty by construction: `end_insert_session` only ever stashes a
    // non-empty `spans` vec (see `pin_insert_anchors`'s caller). The last
    // span is spatially last (stashed in ascending-start order) — primary
    // there, matching the entry command's own cursor placement.
    let insertion_primary = spans.len() - 1;
    let insertion_sels: Vec<Selection> = spans
        .into_iter()
        .map(|(anchor, head)| Selection::new(anchor, head))
        .collect();
    apply_focused_motion(state, view, move |_b, sels| match mode {
        MotionMode::Move => SelectionSet::from_vec(insertion_sels, insertion_primary),
        MotionMode::Extend => {
            // `from_vec` sorts and merges genuinely overlapping selections,
            // so this is a plain union — no need to zip against current
            // selections one-to-one (their counts can differ freely, e.g.
            // `mii` invoked after the selection count changed since the
            // insert). Merely-adjacent (touching, non-overlapping) spans
            // stay separate selections, same as everywhere else in the
            // codebase. The pre-existing primary stays primary, consistent
            // with how every other `mi*` object behaves in Extend mode.
            let primary = sels.primary_index();
            let mut combined: Vec<Selection> = sels.iter_sorted().copied().collect();
            combined.extend(insertion_sels);
            SelectionSet::from_vec(combined, primary)
        }
    });
    Ok(())
}

/// Yank selections without deleting.
///
/// **Bare default**: writes to the system clipboard AND pushes to the kill ring.
/// **Explicit register**: routes through `write_register`.
pub(crate) fn cmd_yank(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let yanked = yank_selections(
        super::doc(state, view).text(),
        super::current_selections(state, view),
    );
    match state.take_register_prefix() {
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

/// Step the undo/redo history `count` times, stopping (with a status report)
/// as soon as `can` returns false — shared by `cmd_undo`/`cmd_redo`, which
/// differ only in direction.
fn history_step(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    can: fn(&Buffer) -> bool,
    apply: doc_ops::ApplyDocFn,
    exhausted_msg: &str,
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    for _ in 0..count {
        if !can(state.buffers.get(buf)) {
            state.report(Severity::Info, exhausted_msg.to_string());
            break;
        }
        apply(
            &mut state.buffers,
            &state.config.decorations,
            &mut state.panes.state,
            &mut state.panes.jumps,
            focused,
            buf,
        );
    }
    Ok(())
}

pub(crate) fn cmd_undo(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    history_step(
        state,
        view,
        count,
        Buffer::can_undo,
        doc_ops::apply_doc_undo,
        "Already at oldest change",
    )
}

pub(crate) fn cmd_redo(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    history_step(
        state,
        view,
        count,
        Buffer::can_redo,
        doc_ops::apply_doc_redo,
        "Already at newest change",
    )
}

// ── Replace / surround ────────────────────────────────────────────────────────

/// Replace every character in each selection with the next typed character.
pub(crate) fn cmd_replace(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if let Some(ch) = state.pending_char.take() {
        apply_focused_edit(state, view, |b, s| replace_selections(b, s, ch));
    }
    Ok(())
}

/// Join lines inside each selection and select the inserted spaces.
pub(crate) fn cmd_join_lines_select_spaces(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    apply_focused_edit(state, view, join_lines_select_spaces);
    Ok(())
}

/// Align each selection's anchor to the primary selection's anchor column.
pub(crate) fn cmd_align_selections(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let buf_id = focused_buffer_id(state, view);
    let tab_width = state
        .buffers
        .get(buf_id)
        .overrides
        .tab_width(&state.settings);
    apply_focused_edit(state, view, move |text, sels| {
        align_selections(text, sels, tab_width)
    });
    Ok(())
}

/// Indent every line touched by a selection by `count` levels (`>`).
pub(crate) fn cmd_indent(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let buf_id = focused_buffer_id(state, view);
    let (style, tab_width) = tab_format(state.buffers.get(buf_id), &state.settings);
    apply_focused_edit(state, view, move |text, sels| {
        indent_lines(text, sels, style, tab_width, count)
    });
    Ok(())
}

/// Unindent every line touched by a selection by `count` levels (`<`).
pub(crate) fn cmd_unindent(
    state: &mut EditorState,
    view: &mut EngineView,
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let buf_id = focused_buffer_id(state, view);
    let (style, tab_width) = tab_format(state.buffers.get(buf_id), &state.settings);
    apply_focused_edit(state, view, move |text, sels| {
        unindent_lines(text, sels, style, tab_width, count)
    });
    Ok(())
}

/// Delete the word before each cursor (Ctrl-W in insert mode).
///
/// Promoted from a plain `MappableCommand::Edit` to an `EditorCmd` so it can
/// resolve this buffer's `word-chars` and close over it — the same pattern
/// [`cmd_align_selections`] uses for `tab_width`. Ctrl-W is a *word*
/// operation by name: leaving it on the built-in word rule would mean `b`
/// then `d` deletes a whole hyphenated run while Ctrl-W deletes only the
/// last piece — a split a user would notice within a minute.
pub(crate) fn cmd_delete_word_backward(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let buf_id = focused_buffer_id(state, view);
    let word_chars = word_chars_owned(state.buffers.get(buf_id), &state.settings);
    apply_focused_edit(state, view, move |text, sels| {
        delete_word_backward(text, sels, WordChars::new(&word_chars))
    });
    Ok(())
}

/// Wrap every selection with a pair determined by the next typed character.
pub(crate) fn cmd_surround_add(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let Some(ch) = state.pending_char.take() else {
        return Ok(());
    };
    let (_ap_enabled, ap_pairs) = super::doc(state, view)
        .overrides
        .auto_pairs_ref(&state.settings);
    let (open, close) = ap_pairs
        .iter()
        .find(|p| p.open == ch || p.close == ch)
        .map(|p| (p.open, p.close))
        .unwrap_or((ch, ch));
    apply_focused_edit(state, view, |b, s| wrap_each_selection(b, s, open, close));
    Ok(())
}
