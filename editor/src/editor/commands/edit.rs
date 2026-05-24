use crate::core::selection::Selection;
use crate::ops::MotionMode;
use crate::ops::edit::{delete_selection, paste_after, paste_before, replace_selections};
use crate::ops::register::{BLACK_HOLE_REGISTER, CLIPBOARD_REGISTER, yank_selections};
use crate::ops::surround::wrap_each_selection;

use super::super::{doc_ops, register_ops, Severity};
use super::super::Editor;
use super::{PASTE_FAMILY_CMDS, SMART_P_LAST_CMDS};
use crate::core::error::CommandError;

// ── Edit composites ───────────────────────────────────────────────────────────

impl Editor {
    /// Write `values` to the system clipboard only (no kill-ring push).
    fn write_clipboard(&mut self, values: &[String]) {
        if let Some(w) = register_ops::write_clipboard(&mut self.registers, &mut self.clipboard, values) {
            self.report(Severity::Warning, w);
        }
    }

    /// Commit the open paste session on the focused pane/buffer (if any).
    ///
    /// Records exactly one history revision for the entire paste + all cycles.
    /// Called by `execute.rs` before any non-`[`/`]` dispatch so the session
    /// is committed before undo, motions, or the next `p`/`P`.
    pub(in super::super) fn commit_paste_session(&mut self) {
        let focused = self.focused_pane_id;
        let buf = self.focused_buffer_id();
        if self.pane_state[focused][buf].paste_group.is_none() {
            return;
        }
        let post_sels = self.pane_state[focused][buf].selections.clone();
        let pbs = &mut self.pane_state[focused][buf];
        self.buffers.get_mut(buf).commit_edit_group(&mut pbs.paste_group, post_sels);
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
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_edit(&mut ed.buffers, &mut ed.pane_state, focused, buf, delete_selection);
    match ed.take_register_prefix() {
        None => ed.kill_ring.push(yanked),
        Some(reg) => ed.write_register(reg, yanked),
    }
    Ok(())
}

/// Yank, delete, then enter insert mode — all in one undo group.
///
/// **Bare default**: pushes to kill ring only. Same Smart-p routing as `cmd_delete`.
pub fn cmd_change(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let yanked = yank_selections(ed.doc().text(), ed.current_selections());
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    ed.begin_insert_session();
    doc_ops::apply_doc_edit_grouped(&mut ed.buffers, &mut ed.pane_state, focused, buf, delete_selection);
    match ed.take_register_prefix() {
        None => ed.kill_ring.push(yanked),
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
            ed.kill_ring.push(yanked);
        }
        Some(reg) => ed.write_register(reg, yanked),
    }
    Ok(())
}

/// Core paste implementation: open/extend a paste session and apply.
///
/// `before`: true for `P` (paste before), false for `p` (paste after).
fn do_paste(ed: &mut Editor, before: bool) {
    // Clone last_command so the borrow on ed ends before the mutable call below.
    let last_command = ed.last_command.clone();
    let last_cmd = last_command.as_deref();

    let is_append = last_cmd.is_some_and(|c| PASTE_FAMILY_CMDS.contains(&c));

    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();

    // Append path: re-paste the verbatim values from the previous paste.
    // No ring/clipboard re-lookup — ring emptiness is irrelevant.
    if is_append {
        if let Some(values) = ed.last_paste.clone() {
            // Collapse the just-pasted selection so the new paste stacks
            // adjacent to it rather than replacing it.
            let text = ed.buffers.get(buf).text();
            let sels = std::mem::take(&mut ed.pane_state[focused][buf].selections);
            ed.pane_state[focused][buf].selections = sels.map(|s| {
                if before {
                    Selection::collapsed(s.start())
                } else {
                    Selection::collapsed(s.end_inclusive(text))
                }
            });

            // Open a new paste session (snapshot pre-paste state).
            let pre_sels = ed.pane_state[focused][buf].selections.clone();
            ed.buffers.get(buf).begin_edit_group(&mut ed.pane_state[focused][buf].paste_group, pre_sels);

            let paste_fn = if before { paste_before } else { paste_after };
            doc_ops::apply_doc_edit_regrouped(
                &mut ed.buffers,
                &mut ed.pane_state,
                focused,
                buf,
                |b, s| paste_fn(b, s, &values),
            );
            // Preserve the cycle position — the seeded origin from the first
            // paste in this run remains correct for `[`/`]`.
            return;
        }
        // Fall through: is_append but nothing ever pasted (e.g. lone `[`/`]`
        // no-op before any paste) — treat as a fresh paste.
    }

    // Fresh paste: resolve source via register prefix / Smart-p / clipboard.
    // Returns None to signal a no-op (black-hole, empty register, empty ring+clipboard).
    let Some((values, cycle_origin)) = resolve_paste_values(ed, last_cmd) else {
        return;
    };

    ed.last_paste = Some(values.clone());

    // Open a new paste session (snapshot pre-paste state).
    let pre_sels = ed.pane_state[focused][buf].selections.clone();
    ed.buffers.get(buf).begin_edit_group(&mut ed.pane_state[focused][buf].paste_group, pre_sels);

    let paste_fn = if before { paste_before } else { paste_after };
    doc_ops::apply_doc_edit_regrouped(
        &mut ed.buffers,
        &mut ed.pane_state,
        focused,
        buf,
        |b, s| paste_fn(b, s, &values),
    );

    ed.kill_ring.seed_cycle(cycle_origin);
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
                let values = ed.kill_ring.head()?.to_vec();
                Some((values, Some(0)))
            } else {
                // Read clipboard; convert Cow to owned before calling report().
                let (cow, warn) = register_ops::read_register_text(
                    &ed.registers,
                    &mut ed.clipboard,
                    &ed.kill_ring,
                    CLIPBOARD_REGISTER,
                );
                let values = cow.map(|c| c.to_vec()); // end borrow of ed.registers
                if let Some(w) = warn {
                    ed.report(Severity::Warning, w);
                }
                // When both clipboard and in-memory 'c' are empty, fall back to
                // ring head so the user can still paste a recent delete/yank.
                if values.is_none() {
                    let head = ed.kill_ring.head()?.to_vec();
                    return Some((head, Some(0)));
                }
                Some((values?, None))
            }
        }
        Some(BLACK_HOLE_REGISTER) => None,
        Some(c) if c.is_ascii_digit() => {
            let slot = (c as u8 - b'0') as usize;
            let values = ed.kill_ring.slot(slot)?.to_vec();
            Some((values, Some(slot)))
        }
        Some(c) => {
            let (cow, warn) = register_ops::read_register_text(
                &ed.registers,
                &mut ed.clipboard,
                &ed.kill_ring,
                c,
            );
            let values = cow.map(|c| c.to_vec()); // end borrow of ed.registers
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
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    if ed.pane_state[focused][buf].paste_group.is_none() {
        return Ok(());
    }
    // Eagerly convert to owned Vec so the borrow of ed.kill_ring ends before
    // ed.buffers and ed.pane_state are borrowed mutably below.
    let values = if older {
        ed.kill_ring.cycle_older()
    } else {
        ed.kill_ring.cycle_newer()
    }
    .map(|v| v.to_vec());
    if let Some(values) = values {
        doc_ops::apply_doc_edit_regrouped(
            &mut ed.buffers,
            &mut ed.pane_state,
            focused,
            buf,
            |b, s| paste_after(b, s, &values),
        );
        ed.last_paste = Some(values);
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
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_undo(&mut ed.buffers, &mut ed.pane_state, focused, buf);
    Ok(())
}

pub fn cmd_redo(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_redo(&mut ed.buffers, &mut ed.pane_state, focused, buf);
    Ok(())
}

// ── Replace / surround ────────────────────────────────────────────────────────

/// Replace every character in each selection with the next typed character.
pub fn cmd_replace(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if let Some(ch) = ed.pending_char.take() {
        let focused = ed.focused_pane_id;
        let buf = ed.focused_buffer_id();
        doc_ops::apply_doc_edit(&mut ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
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
    let Some(ch) = ed.pending_char.take() else {
        return Ok(());
    };
    let (_ap_enabled, ap_pairs) = ed.doc().overrides.auto_pairs_ref(&ed.settings);
    let (open, close) = ap_pairs
        .iter()
        .find(|p| p.open == ch || p.close == ch)
        .map(|p| (p.open, p.close))
        .unwrap_or((ch, ch));
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_edit(&mut ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        wrap_each_selection(b, s, open, close)
    });
    Ok(())
}
