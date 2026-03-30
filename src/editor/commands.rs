//! Editor-level command functions.
//!
//! Each function in this module is a command that requires `&mut Editor`
//! context — composite operations involving mode changes, registers, undo
//! groups, or parameterized motions (find/till/replace).
//!
//! They are registered in [`super::registry`] and called via function pointer
//! from `execute_keymap_command`, exactly like the pure `cmd_*` functions in
//! `ops/motion.rs`, `ops/edit.rs`, etc.
//!
//! The `count` parameter is the user's numeric prefix (default 1). Commands
//! that don't use a count accept it and ignore it (`_count`).

use crate::core::buffer::Buffer;
use crate::core::selection::{Selection, SelectionSet};
use crate::ops::edit::{delete_selection, insert_char};
use crate::ops::motion::{
    cmd_goto_first_nonblank, cmd_goto_line_end, cmd_goto_line_start, cmd_move_left, cmd_move_right,
    find_char_backward, find_char_forward, MotionMode,
};
use crate::ops::register::{yank_selections, DEFAULT_REGISTER};
use crate::ops::selection_cmd::cmd_collapse_selection;

use super::registry::MappableCommand;
use super::{Editor, FindChar, FindKind, MiniBuffer, Mode};

// ── Mode transitions ──────────────────────────────────────────────────────────

pub(super) fn cmd_insert_before(ed: &mut Editor, _count: usize) {
    ed.apply_motion(|_b, sels| sels.map(|s| Selection::cursor(s.start())));
    ed.set_mode(Mode::Insert);
}

pub(super) fn cmd_insert_after(ed: &mut Editor, _count: usize) {
    ed.apply_motion(|b, s| cmd_move_right(b, s, 1));
    ed.set_mode(Mode::Insert);
}

pub(super) fn cmd_insert_at_line_start(ed: &mut Editor, _count: usize) {
    ed.apply_motion(|b, s| cmd_goto_first_nonblank(b, s, 1));
    ed.set_mode(Mode::Insert);
}

pub(super) fn cmd_insert_at_line_end(ed: &mut Editor, _count: usize) {
    ed.apply_motion(|b, s| cmd_goto_line_end(b, s, 1));
    ed.apply_motion(|b, s| cmd_move_right(b, s, 1));
    ed.set_mode(Mode::Insert);
}

/// Open a new line below the cursor and enter insert mode.
///
/// The edit group is opened here so the structural `\n` and everything typed
/// before Esc form one undo step — the same pattern as `cmd_change`.
pub(super) fn cmd_open_line_below(ed: &mut Editor, _count: usize) {
    ed.doc.begin_edit_group();
    ed.apply_motion(|b, s| cmd_goto_line_end(b, s, 1));
    ed.apply_motion(|b, s| cmd_move_right(b, s, 1));
    ed.doc.apply_edit_grouped(|b, s| insert_char(b, s, '\n'));
    ed.set_mode(Mode::Insert);
}

/// Open a new line above the cursor and enter insert mode.
pub(super) fn cmd_open_line_above(ed: &mut Editor, _count: usize) {
    ed.doc.begin_edit_group();
    ed.apply_motion(|b, s| cmd_goto_line_start(b, s, 1));
    ed.doc.apply_edit_grouped(|b, s| insert_char(b, s, '\n'));
    ed.apply_motion(|b, s| cmd_move_left(b, s, 1));
    ed.set_mode(Mode::Insert);
}

pub(super) fn cmd_command_mode(ed: &mut Editor, _count: usize) {
    ed.set_mode(Mode::Command);
    ed.minibuf = Some(MiniBuffer { prompt: ':', input: String::new() });
}

pub(super) fn cmd_exit_insert(ed: &mut Editor, _count: usize) {
    ed.set_mode(Mode::Normal);
}

// ── Edit composites ───────────────────────────────────────────────────────────

/// Yank selections into the default register, then delete them.
pub(super) fn cmd_delete(ed: &mut Editor, _count: usize) {
    let yanked = yank_selections(ed.doc.buf(), ed.doc.sels());
    ed.doc.apply_edit(delete_selection);
    ed.registers.write(DEFAULT_REGISTER, yanked);
}

/// Yank, delete, then enter insert mode — all in one undo group.
///
/// The group is opened here so the delete is folded in; `set_mode(Insert)`
/// sees the group is already open and skips its own `begin_edit_group`.
pub(super) fn cmd_change(ed: &mut Editor, _count: usize) {
    let yanked = yank_selections(ed.doc.buf(), ed.doc.sels());
    ed.doc.begin_edit_group();
    ed.doc.apply_edit_grouped(delete_selection);
    ed.registers.write(DEFAULT_REGISTER, yanked);
    ed.set_mode(Mode::Insert);
}

/// Yank selections into the default register without deleting.
pub(super) fn cmd_yank(ed: &mut Editor, _count: usize) {
    let yanked = yank_selections(ed.doc.buf(), ed.doc.sels());
    ed.registers.write(DEFAULT_REGISTER, yanked);
}

/// Shared body for paste commands: read the default register, run `paste_fn`,
/// then write displaced text back if any selection was non-cursor (replace-and-swap).
fn do_paste(
    ed: &mut Editor,
    paste_fn: impl Fn(Buffer, SelectionSet, &[String]) -> (Buffer, SelectionSet, crate::core::changeset::ChangeSet, Vec<String>),
) {
    if let Some(reg) = ed.registers.read(DEFAULT_REGISTER) {
        let values = reg.values().to_vec();
        let displaced = ed.doc.apply_edit(|b, s| paste_fn(b, s, &values));
        if displaced.iter().any(|s| !s.is_empty()) {
            ed.registers.write(DEFAULT_REGISTER, displaced);
        }
    }
}

/// Paste after the selection; swap displaced text back into the register when
/// the selection was non-cursor (replace-and-swap semantics).
pub(super) fn cmd_paste_after(ed: &mut Editor, _count: usize) {
    use crate::ops::edit::paste_after;
    do_paste(ed, paste_after);
}

/// Paste before the selection; same replace-and-swap semantics as `cmd_paste_after`.
pub(super) fn cmd_paste_before(ed: &mut Editor, _count: usize) {
    use crate::ops::edit::paste_before;
    do_paste(ed, paste_before);
}

pub(super) fn cmd_undo(ed: &mut Editor, _count: usize) {
    ed.doc.undo();
}

pub(super) fn cmd_redo(ed: &mut Editor, _count: usize) {
    ed.doc.redo();
}

// ── Selection state ───────────────────────────────────────────────────────────

pub(super) fn cmd_toggle_extend(ed: &mut Editor, _count: usize) {
    ed.extend = !ed.extend;
}

/// Collapse each selection to its cursor AND exit extend mode.
///
/// Collapsing is a "done selecting" signal, so extend mode is always cleared.
pub(super) fn cmd_collapse_and_exit_extend(ed: &mut Editor, _count: usize) {
    ed.extend = false;
    ed.apply_motion(cmd_collapse_selection);
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
    find_fn: fn(&Buffer, SelectionSet, MotionMode, usize, char, FindKind) -> SelectionSet,
) {
    if let Some(ch) = ed.pending_char.take() {
        ed.apply_motion(|b, s| find_fn(b, s, mode, count, ch, kind));
        ed.last_find = Some(FindChar { ch, kind });
    }
}

pub(super) fn cmd_find_forward(ed: &mut Editor, count: usize) {
    find_char(ed, count, MotionMode::Move, FindKind::Inclusive, find_char_forward);
}
pub(super) fn cmd_extend_find_forward(ed: &mut Editor, count: usize) {
    find_char(ed, count, MotionMode::Extend, FindKind::Inclusive, find_char_forward);
}
pub(super) fn cmd_find_backward(ed: &mut Editor, count: usize) {
    find_char(ed, count, MotionMode::Move, FindKind::Inclusive, find_char_backward);
}
pub(super) fn cmd_extend_find_backward(ed: &mut Editor, count: usize) {
    find_char(ed, count, MotionMode::Extend, FindKind::Inclusive, find_char_backward);
}
pub(super) fn cmd_till_forward(ed: &mut Editor, count: usize) {
    find_char(ed, count, MotionMode::Move, FindKind::Exclusive, find_char_forward);
}
pub(super) fn cmd_extend_till_forward(ed: &mut Editor, count: usize) {
    find_char(ed, count, MotionMode::Extend, FindKind::Exclusive, find_char_forward);
}
pub(super) fn cmd_till_backward(ed: &mut Editor, count: usize) {
    find_char(ed, count, MotionMode::Move, FindKind::Exclusive, find_char_backward);
}
pub(super) fn cmd_extend_till_backward(ed: &mut Editor, count: usize) {
    find_char(ed, count, MotionMode::Extend, FindKind::Exclusive, find_char_backward);
}

// ── Repeat find ───────────────────────────────────────────────────────────────

/// Shared implementation for the four repeat-find commands.
fn repeat_find(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
    find_fn: fn(&Buffer, SelectionSet, MotionMode, usize, char, FindKind) -> SelectionSet,
) {
    if let Some(FindChar { ch, kind }) = ed.last_find {
        ed.apply_motion(|b, s| find_fn(b, s, mode, count, ch, kind));
    }
}

pub(super) fn cmd_repeat_find_forward(ed: &mut Editor, count: usize) {
    repeat_find(ed, count, MotionMode::Move, find_char_forward);
}
pub(super) fn cmd_extend_repeat_find_forward(ed: &mut Editor, count: usize) {
    repeat_find(ed, count, MotionMode::Extend, find_char_forward);
}
pub(super) fn cmd_repeat_find_backward(ed: &mut Editor, count: usize) {
    repeat_find(ed, count, MotionMode::Move, find_char_backward);
}
pub(super) fn cmd_extend_repeat_find_backward(ed: &mut Editor, count: usize) {
    repeat_find(ed, count, MotionMode::Extend, find_char_backward);
}

// ── Replace ───────────────────────────────────────────────────────────────────

/// Replace every character in each selection with the next typed character.
///
/// Reads the replacement character from `ed.pending_char`.
pub(super) fn cmd_replace(ed: &mut Editor, _count: usize) {
    use crate::ops::edit::replace_selections;
    if let Some(ch) = ed.pending_char.take() {
        ed.doc.apply_edit(|b, s| replace_selections(b, s, ch));
    }
}

// ── Dot repeat ───────────────────────────────────────────────────────────────

/// Replay the last repeatable editing action.
///
/// Count semantics: if the user typed an explicit count before `.`, that count
/// overrides the original; otherwise the original count is reused. This mirrors
/// Vim's behaviour (`.` is `3.` → repeat with 3; `.` alone → repeat with original).
pub(super) fn cmd_repeat(ed: &mut Editor, count: usize) {
    let Some(action) = ed.last_action.clone() else { return };

    // Prefer an explicit user count; fall back to the count from the original action.
    let effective_count = if ed.explicit_count { count } else { action.count };

    // Restore the char arg so wait-char commands (replace, find/till) work.
    ed.pending_char = action.char_arg;

    // Guard against re-recording: take last_action so the replayed command
    // doesn't overwrite it, then restore it afterwards.
    ed.last_action = None;
    ed.replaying = true;

    // Re-execute the original command through the normal dispatch path.
    let cmd = super::keymap::KeymapCommand { name: action.command, extend_name: None };
    ed.execute_keymap_command(cmd, effective_count);

    // Feed recorded insert keystrokes through the normal insert handler.
    for key in action.insert_keys.clone() {
        ed.handle_insert(key);
    }

    // If the command left us in Insert mode (e.g. `change`, `insert-before`),
    // exit back to Normal — the edit group is committed in set_mode.
    if ed.mode == super::Mode::Insert {
        ed.set_mode(super::Mode::Normal);
    }

    // Restore the action so `.` can be pressed again.
    ed.replaying = false;
    ed.last_action = Some(action);
}

// ── Page scroll ───────────────────────────────────────────────────────────────
//
// Uses `view.height` as the move count rather than the user's numeric prefix.

/// Shared implementation for the four page-scroll commands.
fn page_scroll(ed: &mut Editor, motion_name: &str) {
    let page = ed.view.height.max(1);
    let Some(MappableCommand::Motion { fun, .. }) = ed.registry.get(motion_name).copied() else {
        unreachable!("page_scroll: motion '{}' not in registry", motion_name);
    };
    ed.apply_motion(|b, s| fun(b, s, page));
}

pub(super) fn cmd_page_down(ed: &mut Editor, _count: usize) {
    page_scroll(ed, "move-down");
}
pub(super) fn cmd_extend_page_down(ed: &mut Editor, _count: usize) {
    page_scroll(ed, "extend-down");
}
pub(super) fn cmd_page_up(ed: &mut Editor, _count: usize) {
    page_scroll(ed, "move-up");
}
pub(super) fn cmd_extend_page_up(ed: &mut Editor, _count: usize) {
    page_scroll(ed, "extend-up");
}

// ── Misc ──────────────────────────────────────────────────────────────────────

pub(super) fn cmd_quit(ed: &mut Editor, _count: usize) {
    ed.should_quit = true;
}
