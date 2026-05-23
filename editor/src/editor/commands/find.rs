use crate::core::selection::SelectionSet;
use crate::core::text::Text;
use crate::ops::MotionMode;
use crate::ops::edit::replace_selections;
use crate::ops::surround::wrap_each_selection;
use crate::ops::motion::{find_char_backward, find_char_forward};
use crate::ops::selection_cmd::cmd_collapse_selection;

use engine::types::EditorMode;

use super::super::{doc_ops, FindChar};
use super::super::Editor;
use crate::core::error::CommandError;
use crate::ops::motion::FindKind;

// ── Selection state ───────────────────────────────────────────────────────────

pub fn cmd_toggle_extend(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.mode = if ed.mode == EditorMode::Extend {
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
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    // Mode is SSOT for extend state; setting Normal implicitly clears Extend.
    ed.mode = EditorMode::Normal;
    let focused = ed.focused_pane_id;
    let buf = ed.focused_buffer_id();
    doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
        cmd_collapse_selection(b, s, MotionMode::Move)
    });
    Ok(())
}

// ── Find / till character ─────────────────────────────────────────────────────
//
// All eight find/till commands read the character argument from
// `ed.pending_char`, which was stored by the WaitChar consumption path.

/// Shared implementation for the eight find/till commands.
fn find_char(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
    kind: FindKind,
    find_fn: fn(&Text, SelectionSet, MotionMode, usize, char, FindKind) -> SelectionSet,
) {
    if let Some(ch) = ed.pending_char.take() {
        let focused = ed.focused_pane_id;
        let buf = ed.focused_buffer_id();
        doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
            find_fn(b, s, mode, count, ch, kind)
        });
        ed.last_find = Some(FindChar { ch, kind });
    }
}

pub fn cmd_find_forward(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(ed, count, mode, FindKind::Inclusive, find_char_forward);
    Ok(())
}
pub fn cmd_find_backward(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(ed, count, mode, FindKind::Inclusive, find_char_backward);
    Ok(())
}
pub fn cmd_till_forward(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(ed, count, mode, FindKind::Exclusive, find_char_forward);
    Ok(())
}
pub fn cmd_till_backward(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(ed, count, mode, FindKind::Exclusive, find_char_backward);
    Ok(())
}

// ── Repeat find ───────────────────────────────────────────────────────────────

/// Shared implementation for the four repeat-find commands.
fn repeat_find(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
    find_fn: fn(&Text, SelectionSet, MotionMode, usize, char, FindKind) -> SelectionSet,
) {
    if let Some(FindChar { ch, kind }) = ed.last_find {
        let focused = ed.focused_pane_id;
        let buf = ed.focused_buffer_id();
        doc_ops::apply_doc_motion(&ed.buffers, &mut ed.pane_state, focused, buf, |b, s| {
            find_fn(b, s, mode, count, ch, kind)
        });
    }
}

pub fn cmd_repeat_find_forward(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    repeat_find(ed, count, mode, find_char_forward);
    Ok(())
}
pub fn cmd_repeat_find_backward(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    repeat_find(ed, count, mode, find_char_backward);
    Ok(())
}

// ── Replace ───────────────────────────────────────────────────────────────────

/// Replace every character in each selection with the next typed character.
///
/// Reads the replacement character from `ed.pending_char`.
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

// ── Surround add ─────────────────────────────────────────────────────────────

/// Wrap every selection with a pair determined by the next typed character.
///
/// Reads the delimiter from `ed.pending_char`. Looks up the configured pair
/// (so `mw[` and `mw]` both wrap with `[` `]`); falls back to symmetric
/// (open == close == ch) for characters not in any configured pair (e.g. `mw*`).
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

// ── Dot repeat ───────────────────────────────────────────────────────────────

/// Replay the last repeatable editing action.
///
/// Count semantics: if the user typed an explicit count before `.`, that count
/// overrides the original; otherwise the original count is reused. This mirrors
/// Vim's behaviour (`3.` → repeat with 3; `.` alone → repeat with original count).
pub fn cmd_repeat(
    ed: &mut Editor,
    count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let Some(action) = ed.last_repeatable_action.take() else {
        return Ok(());
    };

    // Prefer an explicit user count; fall back to the count from the original action.
    let effective_count = if ed.explicit_count {
        count
    } else {
        action.count
    };

    // Restore the char arg so wait-char commands (replace, find/till) work.
    ed.pending_char = action.char_arg;

    // Pre-open the edit group before re-executing. This is the replay signal:
    // `begin_insert_session` checks `is_group_open()` and suppresses both the
    // redundant `begin_edit_group` call and keystroke recording when the group
    // is already open. For non-insert commands the group stays empty and the
    // commit below is a no-op.
    ed.begin_edit_group_current();

    // Re-execute the original command through the normal dispatch path.
    // extend=false because the replayed command was already resolved to its
    // final form (the resolved name is what gets stored in RepeatableAction).
    // Clone the name while `action` is locally owned (moved out via `.take()`).
    ed.execute_keymap_command(action.command.clone(), effective_count, false, vec![]);

    // Feed recorded insert keystrokes through the normal insert handler.
    // `KeyEvent` is `Copy`, so iterate by reference and dereference each key.
    for key in &action.insert_keys {
        ed.handle_insert(*key);
    }

    // Close the insert session / edit group:
    // - For insert commands: `end_insert_session` commits the group (delete +
    //   typed text as one undo step). `insert_session` is `None` here (replay
    //   suppressed it), so no keystrokes are moved into `last_repeatable_action`.
    // - For non-insert commands: the group is empty (no `apply_edit_grouped`
    //   calls), so `commit_edit_group` is a no-op and the command's own
    //   `apply_edit` revision stands alone in history.
    if ed.mode == EditorMode::Insert {
        ed.end_insert_session();
    } else {
        ed.commit_edit_group_current();
    }

    // Restore the original action so `.` can be pressed again.
    // `execute_keymap_command` may have overwritten `last_repeatable_action` during
    // replay; this final assignment ensures the stored action is always the
    // one the user actually performed.
    ed.last_repeatable_action = Some(action);
    Ok(())
}
