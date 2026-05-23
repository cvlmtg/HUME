use crate::core::selection::SelectionSet;
use crate::core::text::Text;
use crate::ops::MotionMode;
use crate::ops::edit::{delete_selection, paste_after, paste_before};
use crate::ops::register::{CLIPBOARD_REGISTER, yank_selections};

use super::super::{doc_ops, register_ops, Severity};
use super::super::Editor;
use super::{SMART_P_LAST_CMDS};
use crate::core::error::CommandError;

// ── Edit composites ───────────────────────────────────────────────────────────

impl Editor {
    /// Write `values` to the system clipboard only (no kill-ring push).
    fn write_clipboard(&mut self, values: &[String]) {
        if let Some(w) = register_ops::write_clipboard(&mut self.registers, &mut self.clipboard, values) {
            self.report(Severity::Warning, w);
        }
    }
}

/// Yank selections into the active register, then delete them.
///
/// **Bare default** (no `"<reg>` prefix): pushes to the kill ring only.
/// Clipboard is not written — use `"cy` / `"cp` for explicit clipboard ops.
///
/// **Explicit register**: routes through `write_register` as before.
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
    // After begin_insert_session so clipboard warnings are logged inside the session.
    match ed.take_register_prefix() {
        None => ed.kill_ring.push(yanked),
        Some(reg) => ed.write_register(reg, yanked),
    }
    Ok(())
}

/// Yank selections without deleting.
///
/// **Bare default**: writes to the system clipboard AND pushes to the kill ring.
/// This is the only operation that reaches both destinations without an explicit
/// prefix — the intent of bare `y` is always "I want this in the clipboard".
///
/// **Explicit register**: routes through `write_register` (e.g. `"cy` → clipboard
/// only, `"5y` → in-memory register 5).
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

/// Shared body for paste commands: resolve what to read (Smart-p or explicit
/// register), run `paste_fn`, then write displaced text back if any selection
/// was non-cursor (replace-and-swap).
///
/// **Bare default** (no `"<reg>` prefix): Smart-p — ring head or clipboard.
/// **`"<digit>`**: kill-ring slot N.
/// **`"c`**: system clipboard.
/// **`"b`**: black hole (paste always no-ops).
fn do_paste(
    ed: &mut Editor,
    paste_fn: impl Fn(
        Text,
        SelectionSet,
        &[String],
    ) -> (
        Text,
        SelectionSet,
        crate::core::changeset::ChangeSet,
        Vec<String>,
    ),
) {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();

    match ed.take_register_prefix() {
        None => {
            let prefer_ring = ed
                .last_command
                .as_deref()
                .is_some_and(|c| SMART_P_LAST_CMDS.contains(&c));
            if prefer_ring {
                // Borrow kill_ring immutably; apply_doc_edit takes &mut buffers /
                // &mut pane_state — disjoint fields, so no clone needed.
                if let Some(head) = ed.kill_ring.head() {
                    let (displaced, _) = doc_ops::apply_doc_edit(
                        &mut ed.buffers,
                        &mut ed.pane_state,
                        focused,
                        buf,
                        |b, s| paste_fn(b, s, head),
                    );
                    // head's last use was inside the closure; NLL ends the
                    // kill_ring borrow here, so push() is safe.
                    if let Some(d) = displaced
                        && d.iter().any(|s| !s.is_empty())
                    {
                        ed.kill_ring.push(d);
                    }
                }
            } else {
                let (values, warn) = register_ops::read_register_text(
                    &ed.registers,
                    &mut ed.clipboard,
                    &ed.kill_ring,
                    CLIPBOARD_REGISTER,
                );
                if let Some(values) = values {
                    let (displaced, _) = doc_ops::apply_doc_edit(
                        &mut ed.buffers,
                        &mut ed.pane_state,
                        focused,
                        buf,
                        |b, s| paste_fn(b, s, &values),
                    );
                    if let Some(d) = displaced
                        && d.iter().any(|s| !s.is_empty())
                        && let Some(w) =
                            register_ops::write_clipboard(&mut ed.registers, &mut ed.clipboard, &d)
                    {
                        ed.report(Severity::Warning, w);
                    }
                }
                if let Some(w) = warn {
                    ed.report(Severity::Warning, w);
                }
            }
        }
        Some(c) if c.is_ascii_digit() => {
            if let Some(values) = register_ops::read_digit_register(&ed.kill_ring, c) {
                let (displaced, _) = doc_ops::apply_doc_edit(
                    &mut ed.buffers,
                    &mut ed.pane_state,
                    focused,
                    buf,
                    |b, s| paste_fn(b, s, values),
                );
                if let Some(d) = displaced
                    && d.iter().any(|s| !s.is_empty())
                {
                    ed.kill_ring.push(d);
                }
            }
        }
        Some(c) => {
            let (values, warn) = register_ops::read_register_text(
                &ed.registers,
                &mut ed.clipboard,
                &ed.kill_ring,
                c,
            );
            if let Some(values) = values {
                let (displaced, _) = doc_ops::apply_doc_edit(
                    &mut ed.buffers,
                    &mut ed.pane_state,
                    focused,
                    buf,
                    |b, s| paste_fn(b, s, &values),
                );
                if let Some(d) = displaced
                    && d.iter().any(|s| !s.is_empty())
                    && let Some(w) = register_ops::write_register(
                        &mut ed.registers,
                        &mut ed.clipboard,
                        c,
                        d,
                    )
                {
                    ed.report(Severity::Warning, w);
                }
            }
            if let Some(w) = warn {
                ed.report(Severity::Warning, w);
            }
        }
    }
}

/// Paste after the selection; swap displaced text back into the register when
/// the selection was non-cursor (replace-and-swap semantics).
pub fn cmd_paste_after(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste(ed, paste_after);
    Ok(())
}

/// Paste before the selection; same replace-and-swap semantics as `cmd_paste_after`.
pub fn cmd_paste_before(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste(ed, paste_before);
    Ok(())
}

/// Cycle the kill ring one step older and paste-after.
///
/// Each press walks one entry further back in the ring (clamped at the oldest).
/// The cycle cursor is reset by any non-`[`/`]` command dispatch, or when
/// displaced text from a selection paste is pushed onto the ring head.
pub fn cmd_paste_ring_older(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    // Compute pane/buf IDs before cycle_older takes &mut kill_ring, since
    // focused_buffer_id() is a &self method and can't coexist with &mut kill_ring.
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    // cycle_older returns &[String] from &mut kill_ring; apply_doc_edit takes
    // &mut buffers / &mut pane_state — disjoint fields, so no clone needed.
    if let Some(values) = ed.kill_ring.cycle_older() {
        let (displaced, _) = doc_ops::apply_doc_edit(
            &mut ed.buffers,
            &mut ed.pane_state,
            focused,
            buf,
            |b, s| paste_after(b, s, values),
        );
        if let Some(d) = displaced
            && d.iter().any(|s| !s.is_empty())
        {
            ed.kill_ring.push(d);
        }
    }
    Ok(())
}

/// Cycle the kill ring one step newer and paste-after.
///
/// Retreats the cycle cursor one step toward the head. If the cursor is already
/// at the head (slot 0), stays there. Displaced text from a selection paste is
/// pushed onto the ring head (resetting the cycle cursor).
pub fn cmd_paste_ring_newer(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    if let Some(values) = ed.kill_ring.cycle_newer() {
        let (displaced, _) = doc_ops::apply_doc_edit(
            &mut ed.buffers,
            &mut ed.pane_state,
            focused,
            buf,
            |b, s| paste_after(b, s, values),
        );
        if let Some(d) = displaced
            && d.iter().any(|s| !s.is_empty())
        {
            ed.kill_ring.push(d);
        }
    }
    Ok(())
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
