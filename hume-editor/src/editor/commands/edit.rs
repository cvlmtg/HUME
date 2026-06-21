use hume_engine::pipeline::{BufferId, EngineView, PaneId};

use crate::ops::MotionMode;
use crate::ops::edit::{delete_selection, paste_after, paste_before, replace_selections};
use crate::ops::register::{
    BLACK_HOLE_REGISTER, CLIPBOARD_REGISTER, KILL_RING_REGISTER, yank_selections,
};
use crate::ops::surround::wrap_each_selection;
use hume_editing::selection::Selection;

use super::super::{EditorState, Severity, doc_ops, register_ops};
use super::{begin_insert_session, focused_buffer_id};
use crate::editor::error::CommandError;
use crate::editor::registry::CmdCategory;

/// Commands that keep Smart-p in "ring" mode: bare `p`/`P` reads the ring
/// head when `last_command` is one of these; otherwise reads the clipboard.
///
/// Separate from paste-family membership (which is tracked by `CmdCategory::Paste`
/// at registration). Kill→ring is a distinct concept: only `c`/`d` trigger it.
const SMART_P_LAST_CMDS: &[&str] = &["change", "delete"];

// ── Edit composites ───────────────────────────────────────────────────────────

/// Yank selections into the active register, then delete them.
///
/// **Bare default** (no `"<reg>` prefix): pushes to the kill ring only.
/// **Explicit register**: routes through `write_register`.
pub fn cmd_delete(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let yanked = yank_selections(
        super::doc(state, view).text(),
        super::current_selections(state, view),
    );
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_edit(
        &mut state.buffers,
        &mut state.panes.state,
        focused,
        buf,
        delete_selection,
    );
    match state.take_register_prefix() {
        None | Some(KILL_RING_REGISTER) => state.kill_ring.push(yanked),
        Some(reg) => state.write_register(reg, yanked),
    }
    Ok(())
}

/// Yank, delete, then enter insert mode — all in one undo group.
///
/// **Bare default**: pushes to kill ring only. **Explicit register**: routes through
/// `write_register` — same as `cmd_delete`.
pub fn cmd_change(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let yanked = yank_selections(
        super::doc(state, view).text(),
        super::current_selections(state, view),
    );
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    begin_insert_session(state, view);
    doc_ops::apply_doc_edit_grouped(
        &mut state.buffers,
        &mut state.panes.state,
        focused,
        buf,
        delete_selection,
    );
    match state.take_register_prefix() {
        None | Some(KILL_RING_REGISTER) => state.kill_ring.push(yanked),
        Some(reg) => state.write_register(reg, yanked),
    }
    Ok(())
}

/// Yank selections without deleting.
///
/// **Bare default**: writes to the system clipboard AND pushes to the kill ring.
/// **Explicit register**: routes through `write_register`.
pub fn cmd_yank(
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
            state.kill_ring.push(yanked);
        }
        // "ky: push to ring only (no clipboard).
        Some(KILL_RING_REGISTER) => state.kill_ring.push(yanked),
        Some(reg) => state.write_register(reg, yanked),
    }
    Ok(())
}

/// Core paste implementation: open/extend a paste session and apply.
///
/// `before`: true for `P` (paste before), false for `p` (paste after).
fn do_paste(state: &mut EditorState, view: &mut EngineView, before: bool) {
    if super::focused_buffer_read_only(state, view) {
        state.report(Severity::Info, "Buffer is read-only".to_string());
        return;
    }
    // Clone last_command so the borrow on state ends before the mutable call below.
    let last_command = state.last_command.clone();
    let last_cmd = last_command.as_deref();

    // An explicit register prefix (`"Xp`) overrides the append path — the user
    // is asking for a specific register, not a repeat of the last paste.
    // Append when the previous command was any paste-family command (p, P, [, ]).
    // Membership is read from the registry category — no parallel string list.
    let is_append = last_cmd
        .and_then(|c| state.registry.get_mappable(c))
        .is_some_and(|cmd| matches!(cmd.meta().category, CmdCategory::Paste { .. }))
        && state.register_prefix.is_none();

    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);

    // Append path: re-paste the verbatim values from the previous paste.
    // No ring/clipboard re-lookup — ring emptiness is irrelevant.
    if is_append && let Some(values) = state.last_paste.clone() {
        // Collapse the just-pasted selection so the new paste stacks
        // adjacent to it rather than replacing it.
        let text = state.buffers.get(buf).text();
        let sels = std::mem::take(&mut state.panes.state[focused][buf].selections);
        state.panes.state[focused][buf].selections = sels.map(|s| {
            if before {
                Selection::collapsed(s.start())
            } else {
                Selection::collapsed(s.end_inclusive(text))
            }
        });
        state.panes.state[focused][buf].paste_before = before;
        open_paste_session_and_apply(state, focused, buf, before, &values);
        // Preserve the cycle position — the seeded origin from the first
        // paste in this run remains correct for `[`/`]`.
        return;
    }

    // Fall through: is_append but nothing ever pasted (e.g. lone `[`/`]`
    // no-op before any paste) or !is_append — treat as a fresh paste.

    // Fresh paste: resolve source via register prefix / Smart-p / clipboard.
    // Returns None to signal a no-op (black-hole, empty register, empty ring+clipboard).
    let Some((values, cycle_origin)) = resolve_paste_values(state, last_cmd) else {
        return;
    };

    state.panes.state[focused][buf].paste_before = before;
    state.last_paste = Some(values.clone());
    open_paste_session_and_apply(state, focused, buf, before, &values);
    state.kill_ring.seed_cycle(cycle_origin);
}

/// Snapshot the pre-paste selections, open a paste group, and apply one paste
/// from `values`. Does **not** seed the kill-ring cycle cursor — the caller
/// handles that for the fresh-paste path; the append path preserves the existing
/// cycle position.
fn open_paste_session_and_apply(
    state: &mut EditorState,
    focused: PaneId,
    buf: BufferId,
    before: bool,
    values: &[String],
) {
    let pre_sels = state.panes.state[focused][buf].selections.clone();
    state
        .buffers
        .get(buf)
        .begin_edit_group(&mut state.panes.state[focused][buf].paste_group, pre_sels);
    let paste_fn = if before { paste_before } else { paste_after };
    doc_ops::apply_doc_edit_regrouped(
        &mut state.buffers,
        &mut state.panes.state,
        focused,
        buf,
        |b, s| paste_fn(b, s, values),
    );
}

/// Resolve values for a **fresh** paste and the ring-slot origin for seeding.
///
/// Returns `(values, cycle_origin)` where `cycle_origin` is `Some(slot)` for
/// ring-seeded pastes and `None` for clipboard / named-register pastes.
/// Returns `None` to signal a no-op (black-hole, empty register, or empty ring+clipboard).
fn resolve_paste_values(
    state: &mut EditorState,
    last_cmd: Option<&str>,
) -> Option<(Vec<String>, Option<usize>)> {
    match state.take_register_prefix() {
        None => {
            let prefer_ring = last_cmd.is_some_and(|c| SMART_P_LAST_CMDS.contains(&c));
            if prefer_ring {
                let values = state.kill_ring.head()?.to_vec();
                Some((values, Some(0)))
            } else {
                // Read clipboard; convert Cow to owned before calling report().
                let (cow, warn) = register_ops::read_register_text(
                    &state.registers,
                    &mut state.clipboard,
                    CLIPBOARD_REGISTER,
                );
                let values = cow.map(|c| c.to_vec()); // end borrow of state.registers
                // When clipboard is unavailable, fall back to ring head silently.
                // Only emit the warning when the fallback also fails — otherwise the
                // user sees a warning alongside a successful paste, which is confusing.
                match values {
                    None => {
                        if let Some(head) = state.kill_ring.head() {
                            return Some((head.to_vec(), Some(0)));
                        }
                        if let Some(w) = warn {
                            state.report(Severity::Warning, w);
                        }
                        None
                    }
                    Some(vals) => {
                        if let Some(w) = warn {
                            state.report(Severity::Warning, w);
                        }
                        Some((vals, None))
                    }
                }
            }
        }
        Some(BLACK_HOLE_REGISTER) => None,
        // "kp: paste kill-ring head; seed cycle so [/] continue from slot 0.
        Some(KILL_RING_REGISTER) => {
            let values = state.kill_ring.head()?.to_vec();
            Some((values, Some(0)))
        }
        Some(c) => {
            // Digits and clipboard. Digits read in-memory RegisterSet (symmetric
            // with "Ny writes). Clipboard routes through the OS clipboard.
            let (cow, warn) =
                register_ops::read_register_text(&state.registers, &mut state.clipboard, c);
            let values = cow.map(|c| c.to_vec()); // end borrow of state.registers
            if let Some(w) = warn {
                state.report(Severity::Warning, w);
            }
            Some((values?, None))
        }
    }
}

/// Paste after the selection.
pub fn cmd_paste_after(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste(state, view, false);
    Ok(())
}

/// Paste before the selection.
pub fn cmd_paste_before(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste(state, view, true);
    Ok(())
}

/// Shared implementation for `[` and `]`: advance/retreat the kill-ring cycle
/// cursor and re-paste from the session snapshot.
///
/// Noop when no paste session is open or when the cycle is already at a boundary.
fn do_paste_cycle(
    state: &mut EditorState,
    view: &mut EngineView,
    older: bool,
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    if state.panes.state[focused][buf].paste_group.is_none() {
        return Ok(());
    }
    // Eagerly convert to owned Vec so the borrow of state.kill_ring ends before
    // state.buffers and state.panes.state are borrowed mutably below.
    let values = if older {
        state.kill_ring.cycle_older()
    } else {
        state.kill_ring.cycle_newer()
    }
    .map(|v| v.to_vec());
    if let Some(values) = values {
        let before = state.panes.state[focused][buf].paste_before;
        let paste_fn = if before { paste_before } else { paste_after };
        doc_ops::apply_doc_edit_regrouped(
            &mut state.buffers,
            &mut state.panes.state,
            focused,
            buf,
            |b, s| paste_fn(b, s, &values),
        );
        state.last_paste = Some(values);
    }
    Ok(())
}

/// Cycle the kill ring one step older and re-paste from the session snapshot.
pub fn cmd_paste_ring_older(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste_cycle(state, view, true)
}

/// Cycle the kill ring one step newer and re-paste from the session snapshot.
pub fn cmd_paste_ring_newer(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste_cycle(state, view, false)
}

pub fn cmd_undo(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_undo(&mut state.buffers, &mut state.panes.state, focused, buf);
    Ok(())
}

pub fn cmd_redo(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_redo(&mut state.buffers, &mut state.panes.state, focused, buf);
    Ok(())
}

// ── Replace / surround ────────────────────────────────────────────────────────

/// Replace every character in each selection with the next typed character.
pub fn cmd_replace(
    state: &mut EditorState,
    view: &mut EngineView,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if let Some(ch) = state.pending_char.take() {
        let focused = state.focused_pane_id;
        let buf = focused_buffer_id(state, view);
        doc_ops::apply_doc_edit(
            &mut state.buffers,
            &mut state.panes.state,
            focused,
            buf,
            |b, s| replace_selections(b, s, ch),
        );
    }
    Ok(())
}

/// Wrap every selection with a pair determined by the next typed character.
pub fn cmd_surround_add(
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
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_edit(
        &mut state.buffers,
        &mut state.panes.state,
        focused,
        buf,
        |b, s| wrap_each_selection(b, s, open, close),
    );
    Ok(())
}
