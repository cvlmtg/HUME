use engine::pipeline::{BufferId, PaneId};

use editing::selection::Selection;
use crate::ops::MotionMode;
use crate::ops::edit::{delete_selection, paste_after, paste_before, replace_selections};
use crate::ops::register::{BLACK_HOLE_REGISTER, CLIPBOARD_REGISTER, KILL_RING_REGISTER, yank_selections};
use crate::ops::surround::wrap_each_selection;

use super::super::{doc_ops, register_ops, Severity};
use super::super::Editor;
use super::{PASTE_FAMILY_CMDS, SMART_P_LAST_CMDS};
use crate::editor::error::CommandError;

// ── Edit composites ───────────────────────────────────────────────────────────

impl Editor {
    /// Write `values` to the system clipboard only (no kill-ring push).
    fn write_clipboard(&mut self, values: &[String]) {
        if let Some(w) = register_ops::write_clipboard(&mut self.state.registers, &mut self.state.clipboard, values) {
            self.report(Severity::Warning, w);
        }
    }

    /// Commit the open paste session on the focused pane/buffer (if any).
    ///
    /// Records exactly one history revision for the entire paste + all cycles.
    /// Called by `execute.rs` before any non-`[`/`]` dispatch so the session
    /// is committed before undo, motions, or the next `p`/`P`.
    pub(in super::super) fn commit_paste_session(&mut self) {
        // Commit every open paste group across all pane/buffer combinations.
        // Normally only the focused slot has one open, but a macro that switches
        // buffers mid-replay can leave a group open on a de-focused buffer.
        let open: Vec<(PaneId, BufferId)> = self.state.pane_state
            .iter()
            .flat_map(|(pid, inner)| {
                inner.iter()
                    .filter(|(_, pbs)| pbs.paste_group.is_some())
                    .map(move |(bid, _)| (pid, bid))
            })
            .collect();
        for (pid, bid) in open {
            let post_sels = self.state.pane_state[pid][bid].selections.clone();
            let pbs = &mut self.state.pane_state[pid][bid];
            self.state.buffers.get_mut(bid).commit_edit_group(&mut pbs.paste_group, post_sels);
        }
    }
}

/// Yank selections into the active register, then delete them.
///
/// **Bare default** (no `"<reg>` prefix): pushes to the kill ring only.
/// **Explicit register**: routes through `write_register`.
pub fn cmd_delete(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let yanked = yank_selections(ed.doc().text(), ed.current_selections());
    let focused = ed.state.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_edit(&mut ed.state.buffers, &mut ed.state.pane_state, focused, buf, delete_selection);
    match ed.take_register_prefix() {
        None | Some(KILL_RING_REGISTER) => ed.state.kill_ring.push(yanked),
        Some(reg) => ed.write_register(reg, yanked),
    }
    Ok(())
}

/// Yank, delete, then enter insert mode — all in one undo group.
///
/// **Bare default**: pushes to kill ring only. **Explicit register**: routes through
/// `write_register` — same as `cmd_delete`.
pub fn cmd_change(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let yanked = yank_selections(ed.doc().text(), ed.current_selections());
    let focused = ed.state.focused_pane_id;
    let buf = ed.focused_buffer_id();
    ed.begin_insert_session();
    doc_ops::apply_doc_edit_grouped(&mut ed.state.buffers, &mut ed.state.pane_state, focused, buf, delete_selection);
    match ed.take_register_prefix() {
        None | Some(KILL_RING_REGISTER) => ed.state.kill_ring.push(yanked),
        Some(reg) => ed.write_register(reg, yanked),
    }
    Ok(())
}

/// Yank selections without deleting.
///
/// **Bare default**: writes to the system clipboard AND pushes to the kill ring.
/// **Explicit register**: routes through `write_register`.
pub fn cmd_yank(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let yanked = yank_selections(ed.doc().text(), ed.current_selections());
    match ed.take_register_prefix() {
        None => {
            ed.write_clipboard(&yanked);
            ed.state.kill_ring.push(yanked);
        }
        // "ky: push to ring only (no clipboard).
        Some(KILL_RING_REGISTER) => ed.state.kill_ring.push(yanked),
        Some(reg) => ed.write_register(reg, yanked),
    }
    Ok(())
}

/// Core paste implementation: open/extend a paste session and apply.
///
/// `before`: true for `P` (paste before), false for `p` (paste after).
fn do_paste(ed: &mut Editor, before: bool) {
    if ed.focused_buffer_read_only() {
        ed.report(Severity::Info, "Buffer is read-only".to_string());
        return;
    }
    // Clone last_command so the borrow on ed ends before the mutable call below.
    let last_command = ed.state.last_command.clone();
    let last_cmd = last_command.as_deref();

    // An explicit register prefix (`"Xp`) overrides the append path — the user
    // is asking for a specific register, not a repeat of the last paste.
    let is_append = last_cmd.is_some_and(|c| PASTE_FAMILY_CMDS.contains(&c))
        && ed.state.register_prefix.is_none();

    let focused = ed.state.focused_pane_id;
    let buf = ed.focused_buffer_id();

    // Append path: re-paste the verbatim values from the previous paste.
    // No ring/clipboard re-lookup — ring emptiness is irrelevant.
    if is_append && let Some(values) = ed.state.last_paste.clone() {
        // Collapse the just-pasted selection so the new paste stacks
        // adjacent to it rather than replacing it.
        let text = ed.state.buffers.get(buf).text();
        let sels = std::mem::take(&mut ed.state.pane_state[focused][buf].selections);
        ed.state.pane_state[focused][buf].selections = sels.map(|s| {
            if before {
                Selection::collapsed(s.start())
            } else {
                Selection::collapsed(s.end_inclusive(text))
            }
        });
        ed.state.pane_state[focused][buf].paste_before = before;
        open_paste_session_and_apply(ed, focused, buf, before, &values);
        // Preserve the cycle position — the seeded origin from the first
        // paste in this run remains correct for `[`/`]`.
        return;
    }

    // Fall through: is_append but nothing ever pasted (e.g. lone `[`/`]`
    // no-op before any paste) or !is_append — treat as a fresh paste.

    // Fresh paste: resolve source via register prefix / Smart-p / clipboard.
    // Returns None to signal a no-op (black-hole, empty register, empty ring+clipboard).
    let Some((values, cycle_origin)) = resolve_paste_values(ed, last_cmd) else {
        return;
    };

    ed.state.pane_state[focused][buf].paste_before = before;
    ed.state.last_paste = Some(values.clone());
    open_paste_session_and_apply(ed, focused, buf, before, &values);
    ed.state.kill_ring.seed_cycle(cycle_origin);
}

/// Snapshot the pre-paste selections, open a paste group, and apply one paste
/// from `values`. Does **not** seed the kill-ring cycle cursor — the caller
/// handles that for the fresh-paste path; the append path preserves the existing
/// cycle position.
fn open_paste_session_and_apply(
    ed: &mut Editor,
    focused: PaneId,
    buf: BufferId,
    before: bool,
    values: &[String],
) {
    let pre_sels = ed.state.pane_state[focused][buf].selections.clone();
    ed.state.buffers.get(buf).begin_edit_group(&mut ed.state.pane_state[focused][buf].paste_group, pre_sels);
    let paste_fn = if before { paste_before } else { paste_after };
    doc_ops::apply_doc_edit_regrouped(
        &mut ed.state.buffers,
        &mut ed.state.pane_state,
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
fn resolve_paste_values(ed: &mut Editor, last_cmd: Option<&str>) -> Option<(Vec<String>, Option<usize>)> {
    match ed.take_register_prefix() {
        None => {
            let prefer_ring = last_cmd.is_some_and(|c| SMART_P_LAST_CMDS.contains(&c));
            if prefer_ring {
                let values = ed.state.kill_ring.head()?.to_vec();
                Some((values, Some(0)))
            } else {
                // Read clipboard; convert Cow to owned before calling report().
                let (cow, warn) = register_ops::read_register_text(
                    &ed.state.registers,
                    &mut ed.state.clipboard,
                    CLIPBOARD_REGISTER,
                );
                let values = cow.map(|c| c.to_vec()); // end borrow of ed.state.registers
                // When clipboard is unavailable, fall back to ring head silently.
                // Only emit the warning when the fallback also fails — otherwise the
                // user sees a warning alongside a successful paste, which is confusing.
                match values {
                    None => {
                        if let Some(head) = ed.state.kill_ring.head() {
                            return Some((head.to_vec(), Some(0)));
                        }
                        if let Some(w) = warn {
                            ed.report(Severity::Warning, w);
                        }
                        None
                    }
                    Some(vals) => {
                        if let Some(w) = warn {
                            ed.report(Severity::Warning, w);
                        }
                        Some((vals, None))
                    }
                }
            }
        }
        Some(BLACK_HOLE_REGISTER) => None,
        // "kp: paste kill-ring head; seed cycle so [/] continue from slot 0.
        Some(KILL_RING_REGISTER) => {
            let values = ed.state.kill_ring.head()?.to_vec();
            Some((values, Some(0)))
        }
        Some(c) => {
            // Digits and clipboard. Digits read in-memory RegisterSet (symmetric
            // with "Ny writes). Clipboard routes through the OS clipboard.
            let (cow, warn) = register_ops::read_register_text(
                &ed.state.registers,
                &mut ed.state.clipboard,
                c,
            );
            let values = cow.map(|c| c.to_vec()); // end borrow of ed.state.registers
            if let Some(w) = warn {
                ed.report(Severity::Warning, w);
            }
            Some((values?, None))
        }
    }
}

/// Paste after the selection.
pub fn cmd_paste_after(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste(ed, false);
    Ok(())
}

/// Paste before the selection.
pub fn cmd_paste_before(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste(ed, true);
    Ok(())
}

/// Shared implementation for `[` and `]`: advance/retreat the kill-ring cycle
/// cursor and re-paste from the session snapshot.
///
/// Noop when no paste session is open or when the cycle is already at a boundary.
fn do_paste_cycle(ed: &mut Editor, older: bool) -> Result<(), CommandError> {
    let focused = ed.state.focused_pane_id;
    let buf = ed.focused_buffer_id();
    if ed.state.pane_state[focused][buf].paste_group.is_none() {
        return Ok(());
    }
    // Eagerly convert to owned Vec so the borrow of ed.state.kill_ring ends before
    // ed.state.buffers and ed.state.pane_state are borrowed mutably below.
    let values = if older {
        ed.state.kill_ring.cycle_older()
    } else {
        ed.state.kill_ring.cycle_newer()
    }
    .map(|v| v.to_vec());
    if let Some(values) = values {
        let before = ed.state.pane_state[focused][buf].paste_before;
        let paste_fn = if before { paste_before } else { paste_after };
        doc_ops::apply_doc_edit_regrouped(
            &mut ed.state.buffers,
            &mut ed.state.pane_state,
            focused,
            buf,
            |b, s| paste_fn(b, s, &values),
        );
        ed.state.last_paste = Some(values);
    }
    Ok(())
}

/// Cycle the kill ring one step older and re-paste from the session snapshot.
pub fn cmd_paste_ring_older(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste_cycle(ed, true)
}

/// Cycle the kill ring one step newer and re-paste from the session snapshot.
pub fn cmd_paste_ring_newer(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste_cycle(ed, false)
}

pub fn cmd_undo(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.state.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_undo(&mut ed.state.buffers, &mut ed.state.pane_state, focused, buf);
    Ok(())
}

pub fn cmd_redo(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.state.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_redo(&mut ed.state.buffers, &mut ed.state.pane_state, focused, buf);
    Ok(())
}

// ── Replace / surround ────────────────────────────────────────────────────────

/// Replace every character in each selection with the next typed character.
pub fn cmd_replace(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if let Some(ch) = ed.state.pending_char.take() {
        let focused = ed.state.focused_pane_id;
        let buf = ed.focused_buffer_id();
        doc_ops::apply_doc_edit(&mut ed.state.buffers, &mut ed.state.pane_state, focused, buf, |b, s| {
            replace_selections(b, s, ch)
        });
    }
    Ok(())
}

/// Wrap every selection with a pair determined by the next typed character.
pub fn cmd_surround_add(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let Some(ch) = ed.state.pending_char.take() else {
        return Ok(());
    };
    let (_ap_enabled, ap_pairs) = ed.doc().overrides.auto_pairs_ref(&ed.state.settings);
    let (open, close) = ap_pairs
        .iter()
        .find(|p| p.open == ch || p.close == ch)
        .map(|p| (p.open, p.close))
        .unwrap_or((ch, ch));
    let focused = ed.state.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_edit(&mut ed.state.buffers, &mut ed.state.pane_state, focused, buf, |b, s| {
        wrap_each_selection(b, s, open, close)
    });
    Ok(())
}
