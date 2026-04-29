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

use std::sync::Arc;

use crate::core::grapheme::next_grapheme_boundary;
use crate::core::search_state::SearchPattern;
use crate::core::selection::{Selection, SelectionSet};
use crate::core::text::Text;
use crate::helpers::is_word_boundary;
use crate::ops::MotionMode;
use crate::ops::edit::{delete_selection, insert_char, paste_after, paste_before};
use crate::ops::surround::wrap_each_selection;
use crate::ops::motion::{
    cmd_goto_first_nonblank, cmd_goto_line_end, cmd_goto_line_start, cmd_move_left, cmd_move_right,
    find_char_backward, find_char_forward,
};
use crate::ops::register::{CLIPBOARD_REGISTER, SEARCH_REGISTER, yank_selections};
use crate::ops::search::{
    compile_search_regex, escape_regex, find_all_matches, find_match_from_cache, find_next_match,
};
use crate::ops::selection_cmd::cmd_collapse_selection;
use crate::ops::text_object::inner_word_impl;

use engine::pipeline::BufferId;
use engine::types::EditorMode;

use super::{ScratchView, Severity};

use super::{Editor, FindChar, MiniBuffer, Mode, RegisterPrefix, SearchDirection};
use crate::core::error::CommandError;
use crate::ops::motion::FindKind;

// ── Mode transitions ──────────────────────────────────────────────────────────

pub(super) fn cmd_insert_before(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.apply_motion(|_b, sels| sels.map(|s| Selection::collapsed(s.start())));
    ed.begin_insert_session();
    Ok(())
}

pub(super) fn cmd_insert_after(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.apply_motion(|b, s| cmd_move_right(b, s, 1, MotionMode::Move));
    ed.begin_insert_session();
    Ok(())
}

pub(super) fn cmd_insert_at_line_start(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.apply_motion(|b, s| cmd_goto_first_nonblank(b, s, 1, MotionMode::Move));
    ed.begin_insert_session();
    Ok(())
}

pub(super) fn cmd_insert_at_line_end(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.apply_motion(|b, s| cmd_goto_line_end(b, s, 1, MotionMode::Move));
    ed.apply_motion(|b, s| cmd_move_right(b, s, 1, MotionMode::Move));
    ed.begin_insert_session();
    Ok(())
}

/// Enter insert mode at the start of each selection (min of anchor and head).
/// For a collapsed cursor this is identical to `i`.
pub(super) fn cmd_insert_at_selection_start(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.apply_motion(|_b, sels| sels.map(|sel| Selection::collapsed(sel.start())));
    ed.begin_insert_session();
    Ok(())
}

/// Enter insert mode after the end of each selection (one past max of anchor and head).
/// For a collapsed cursor this is identical to `a`.
///
/// Clamps to `len_chars() - 1` so pressing `a` on the structural trailing `\n`
/// (the last char in the buffer) does not place the cursor out of bounds.
pub(super) fn cmd_insert_at_selection_end(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.apply_motion(|b, sels| {
        // len_chars() - 1 is safe: the buffer invariant guarantees at least one char.
        let max = b.len_chars() - 1;
        sels.map(|sel| Selection::collapsed(next_grapheme_boundary(b, sel.end()).min(max)))
    });
    ed.begin_insert_session();
    Ok(())
}

/// Open a new line below the cursor and enter insert mode.
///
/// `begin_insert_session` opens the edit group so the structural `\n` and
/// everything typed before Esc form one undo step — the same pattern as
/// `cmd_change`.
pub(super) fn cmd_open_line_below(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.begin_insert_session();
    ed.apply_motion(|b, s| cmd_goto_line_end(b, s, 1, MotionMode::Move));
    ed.apply_motion(|b, s| cmd_move_right(b, s, 1, MotionMode::Move));
    ed.doc_edit_grouped(|b, s| insert_char(b, s, '\n'));
    Ok(())
}

/// Open a new line above the cursor and enter insert mode.
pub(super) fn cmd_open_line_above(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.begin_insert_session();
    ed.apply_motion(|b, s| cmd_goto_line_start(b, s, 1, MotionMode::Move));
    ed.doc_edit_grouped(|b, s| insert_char(b, s, '\n'));
    ed.apply_motion(|b, s| cmd_move_left(b, s, 1, MotionMode::Move));
    Ok(())
}

pub(super) fn cmd_command_mode(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.history.begin_session_all();
    ed.set_mode(Mode::Command);
    ed.minibuf = Some(MiniBuffer {
        prompt: ':',
        input: String::new(),
        cursor: 0,
    });
    Ok(())
}

pub(super) fn cmd_exit_insert(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.end_insert_session();
    Ok(())
}

// ── Register helpers ──────────────────────────────────────────────────────────

// ── Kill-ring command name sets ───────────────────────────────────────────────
// Two aspects of the same lifecycle, kept adjacent so they're maintained
// together:
//
//  SMART_P_LAST_CMDS — the allow-list for the Smart-p heuristic: bare `p`/`P`
//    reads the kill-ring head when the most recent command is in this set.
//
//  RING_CYCLE_CMDS — commands that must NOT reset the `[`/`]` cycle cursor.
//    Any other dispatched command calls `kill_ring.reset_cycle()`.

/// Commands that keep Smart-p in "ring" mode: bare `p`/`P` reads the ring
/// head when `last_command` is one of these; otherwise reads the clipboard.
pub(super) const SMART_P_LAST_CMDS: &[&str] = &[
    "change",
    "delete",
    "paste-after",
    "paste-before",
    "paste-ring-older",
    "paste-ring-newer",
];

/// Commands that must not reset the kill-ring cycle cursor; every other
/// dispatch resets it so the next `[` starts from slot 1.
pub(super) const RING_CYCLE_CMDS: &[&str] = &["paste-ring-older", "paste-ring-newer"];

impl Editor {
    /// Consume the pending `"<reg>` prefix and return the explicit register name,
    /// or `None` if no prefix was typed (bare default case).
    ///
    /// Call once per command at entry — calling twice returns `None` on the
    /// second call because the pending state is cleared by `take()`.
    pub(super) fn take_register_prefix(&mut self) -> Option<char> {
        match self.register_prefix.take() {
            Some(RegisterPrefix::Selected(c)) => Some(c),
            _ => None,
        }
    }

    /// Write `values` into `name`, routing `'c'` through the OS clipboard.
    ///
    /// On clipboard failure logs a warning; always mirrors to in-memory 'c' so
    /// reads work even when the clipboard server is unavailable.
    pub(super) fn write_register(&mut self, name: char, values: Vec<String>) {
        if name == CLIPBOARD_REGISTER {
            let blob = values.join("\n");
            if let Err(e) = self.clipboard.write(&blob) {
                self.warn_clipboard_unavailable(&e);
            }
            // Always mirror to in-memory so reads fall back correctly.
            self.registers.write_text(CLIPBOARD_REGISTER, values);
        } else {
            self.registers.write_text(name, values);
        }
    }

    /// Write `values` to the system clipboard only (no kill-ring push).
    fn write_clipboard(&mut self, values: &[String]) {
        let blob = values.join("\n");
        if let Err(e) = self.clipboard.write(&blob) {
            self.warn_clipboard_unavailable(&e);
        }
        self.registers.write_text(CLIPBOARD_REGISTER, values.to_vec());
    }

    /// Read text from an explicitly named register.
    ///
    /// `'c'` → OS clipboard (with in-memory fallback).
    /// `'0'`–`'9'` → kill-ring slot N (fallback to in-memory if ring slot empty).
    /// All others → in-memory `RegisterSet`.
    ///
    /// On clipboard failure logs a warning and falls back to the in-memory mirror.
    pub(super) fn read_register_text(&mut self, name: char) -> Option<Vec<String>> {
        if name == CLIPBOARD_REGISTER {
            match self.clipboard.read() {
                Ok(text) => Some(vec![text]),
                Err(e) => {
                    self.warn_clipboard_unavailable(&e);
                    self.read_in_memory(CLIPBOARD_REGISTER)
                }
            }
        } else if name.is_ascii_digit() {
            self.read_digit_register(name)
        } else {
            self.read_in_memory(name)
        }
    }

    fn warn_clipboard_unavailable(&mut self, err: &str) {
        self.report(
            super::Severity::Warning,
            format!("system clipboard unavailable ({err}), using in-memory 'c'"),
        );
    }

    fn read_in_memory(&self, name: char) -> Option<Vec<String>> {
        self.registers.read(name).and_then(|r| r.as_text()).map(|v| v.to_vec())
    }

    fn read_digit_register(&self, name: char) -> Option<Vec<String>> {
        debug_assert!(name.is_ascii_digit());
        let slot = (name as u8 - b'0') as usize;
        // Kill ring is the authoritative source for digit registers — no in-memory
        // fallback. This keeps "5y (in-memory named slot) and "5p (ring slot N)
        // orthogonal so a "5y write can never be silently shadowed by an older ring entry.
        self.kill_ring.slot(slot).map(|s| s.to_vec())
    }
}

// ── Edit composites ───────────────────────────────────────────────────────────

/// Yank selections into the active register, then delete them.
///
/// **Bare default** (no `"<reg>` prefix): pushes to the kill ring only.
/// Clipboard is not written — use `"cy` / `"cp` for explicit clipboard ops.
///
/// **Explicit register**: routes through `write_register` as before.
pub(super) fn cmd_delete(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let yanked = yank_selections(ed.doc().text(), ed.current_selections());
    ed.doc_edit(delete_selection);
    match ed.take_register_prefix() {
        None => ed.kill_ring.push(yanked),
        Some(reg) => ed.write_register(reg, yanked),
    }
    Ok(())
}

/// Yank, delete, then enter insert mode — all in one undo group.
///
/// **Bare default**: pushes to kill ring only. Same Smart-p routing as `cmd_delete`.
pub(super) fn cmd_change(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let yanked = yank_selections(ed.doc().text(), ed.current_selections());
    ed.begin_insert_session();
    ed.doc_edit_grouped(delete_selection);
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
pub(super) fn cmd_yank(
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
    enum PasteSource { Ring, Clipboard, Register(char) }
    let (values, source) = match ed.take_register_prefix() {
        None => {
            let prefer_ring = ed
                .last_command
                .as_deref()
                .is_some_and(|c| SMART_P_LAST_CMDS.contains(&c));
            if prefer_ring {
                let v = ed.kill_ring.head().map(|s| s.to_vec());
                (v, PasteSource::Ring)
            } else {
                let v = ed.read_register_text(CLIPBOARD_REGISTER);
                (v, PasteSource::Clipboard)
            }
        }
        Some(c) if c.is_ascii_digit() => {
            (ed.read_digit_register(c), PasteSource::Ring)
        }
        Some(c) => {
            let v = ed.read_register_text(c);
            (v, PasteSource::Register(c))
        }
    };

    if let Some(values) = values {
        let (displaced, _cs) = ed.doc_edit(|b, s| paste_fn(b, s, &values));
        if let Some(displaced) = displaced
            && displaced.iter().any(|s| !s.is_empty())
        {
            match source {
                PasteSource::Ring => ed.kill_ring.push(displaced),
                PasteSource::Clipboard => ed.write_clipboard(&displaced),
                PasteSource::Register(c) => ed.write_register(c, displaced),
            }
        }
    }
}

/// Paste after the selection; swap displaced text back into the register when
/// the selection was non-cursor (replace-and-swap semantics).
pub(super) fn cmd_paste_after(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    do_paste(ed, paste_after);
    Ok(())
}

/// Paste before the selection; same replace-and-swap semantics as `cmd_paste_after`.
pub(super) fn cmd_paste_before(
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
pub(super) fn cmd_paste_ring_older(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if let Some(values) = ed.kill_ring.cycle_older().map(|s| s.to_vec()) {
        let (displaced, _cs) = ed.doc_edit(|b, s| paste_after(b, s, &values));
        if let Some(displaced) = displaced
            && displaced.iter().any(|s| !s.is_empty())
        {
            ed.kill_ring.push(displaced);
        }
    }
    Ok(())
}

/// Cycle the kill ring one step newer and paste-after.
///
/// Retreats the cycle cursor one step toward the head. If the cursor is already
/// at the head (slot 0), stays there. Displaced text from a selection paste is
/// pushed onto the ring head (resetting the cycle cursor).
pub(super) fn cmd_paste_ring_newer(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if let Some(values) = ed.kill_ring.cycle_newer().map(|s| s.to_vec()) {
        let (displaced, _cs) = ed.doc_edit(|b, s| paste_after(b, s, &values));
        if let Some(displaced) = displaced
            && displaced.iter().any(|s| !s.is_empty())
        {
            ed.kill_ring.push(displaced);
        }
    }
    Ok(())
}

pub(super) fn cmd_undo(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.doc_undo();
    Ok(())
}

pub(super) fn cmd_redo(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.doc_redo();
    Ok(())
}

// ── Selection state ───────────────────────────────────────────────────────────

pub(super) fn cmd_toggle_extend(
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
pub(super) fn cmd_collapse_and_exit_extend(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    // Mode is SSOT for extend state; setting Normal implicitly clears Extend.
    ed.mode = EditorMode::Normal;
    ed.apply_motion(|b, s| cmd_collapse_selection(b, s, MotionMode::Move));
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
        ed.apply_motion(|b, s| find_fn(b, s, mode, count, ch, kind));
        ed.last_find = Some(FindChar { ch, kind });
    }
}

pub(super) fn cmd_find_forward(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(ed, count, mode, FindKind::Inclusive, find_char_forward);
    Ok(())
}
pub(super) fn cmd_find_backward(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(ed, count, mode, FindKind::Inclusive, find_char_backward);
    Ok(())
}
pub(super) fn cmd_till_forward(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    find_char(ed, count, mode, FindKind::Exclusive, find_char_forward);
    Ok(())
}
pub(super) fn cmd_till_backward(
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
        ed.apply_motion(|b, s| find_fn(b, s, mode, count, ch, kind));
    }
}

pub(super) fn cmd_repeat_find_forward(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    repeat_find(ed, count, mode, find_char_forward);
    Ok(())
}
pub(super) fn cmd_repeat_find_backward(
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
pub(super) fn cmd_replace(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    use crate::ops::edit::replace_selections;
    if let Some(ch) = ed.pending_char.take() {
        ed.doc_edit(|b, s| replace_selections(b, s, ch));
    }
    Ok(())
}

// ── Surround add ─────────────────────────────────────────────────────────────

/// Wrap every selection with a pair determined by the next typed character.
///
/// Reads the delimiter from `ed.pending_char`. Looks up the configured pair
/// (so `mw[` and `mw]` both wrap with `[` `]`); falls back to symmetric
/// (open == close == ch) for characters not in any configured pair (e.g. `mw*`).
pub(super) fn cmd_surround_add(
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
    ed.doc_edit(|b, s| wrap_each_selection(b, s, open, close));
    Ok(())
}

// ── Dot repeat ───────────────────────────────────────────────────────────────

/// Replay the last repeatable editing action.
///
/// Count semantics: if the user typed an explicit count before `.`, that count
/// overrides the original; otherwise the original count is reused. This mirrors
/// Vim's behaviour (`3.` → repeat with 3; `.` alone → repeat with original count).
pub(super) fn cmd_repeat(
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
    ed.execute_keymap_command(action.command.clone(), effective_count, false, None);

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

// ── Page / half-page scroll ───────────────────────────────────────────────────
//
// Uses `view.height` (or half of it) as the move count rather than the user's
// numeric prefix. Calls the visual-move commands directly instead of going
// through the registry to avoid a runtime string lookup.

pub(super) fn cmd_page_down(
    ed: &mut Editor,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let count = ed.viewport().height as usize;
    cmd_visual_move_down(ed, count, mode)
}
pub(super) fn cmd_page_up(
    ed: &mut Editor,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let count = ed.viewport().height as usize;
    cmd_visual_move_up(ed, count, mode)
}
pub(super) fn cmd_half_page_down(
    ed: &mut Editor,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let count = (ed.viewport().height as usize / 2).max(1);
    cmd_visual_move_down(ed, count, mode)
}
pub(super) fn cmd_half_page_up(
    ed: &mut Editor,
    _count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    let count = (ed.viewport().height as usize / 2).max(1);
    cmd_visual_move_up(ed, count, mode)
}

// Visual-line movement lives in visual_move.rs; re-export for the registry glob.
pub(super) use super::visual_move::{cmd_visual_move_down, cmd_visual_move_up};

// ── Search ────────────────────────────────────────────────────────────────────

/// Enter forward search mode.
///
/// Snapshots the current selections for cancel-restore, then opens the
/// mini-buffer with the `/` prompt.
pub(super) fn cmd_search_forward(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let pre_sels = ed.current_selections().clone();
    let extend = ed.mode == EditorMode::Extend;
    let pid = ed.focused_pane_id;
    ed.search.direction = SearchDirection::Forward;
    // Capture extend state before mode becomes Search — live search uses it.
    ed.pane_transient[pid].pre_search_sels = Some(pre_sels);
    ed.pane_transient[pid].search_extend = extend;
    ed.history.begin_session_all();
    ed.set_mode(Mode::Search);
    ed.minibuf = Some(MiniBuffer {
        prompt: '/',
        input: String::new(),
        cursor: 0,
    });
    Ok(())
}

/// Enter backward search mode.
pub(super) fn cmd_search_backward(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let pre_sels = ed.current_selections().clone();
    let extend = ed.mode == EditorMode::Extend;
    let pid = ed.focused_pane_id;
    ed.search.direction = SearchDirection::Backward;
    // Capture extend state before mode becomes Search — live search uses it.
    ed.pane_transient[pid].pre_search_sels = Some(pre_sels);
    ed.pane_transient[pid].search_extend = extend;
    ed.history.begin_session_all();
    ed.set_mode(Mode::Search);
    ed.minibuf = Some(MiniBuffer {
        prompt: '?',
        input: String::new(),
        cursor: 0,
    });
    Ok(())
}

/// Build the primary selection after a search match.
///
/// `anchor = Some(a)` — extend mode: keep the caller's anchor, move head to
/// the match edge that faces the search direction.
/// `anchor = None` — move mode: cover the matched text exactly.
pub(super) fn search_sel(
    start: usize,
    end_incl: usize,
    anchor: Option<usize>,
    direction: SearchDirection,
) -> Selection {
    match anchor {
        Some(a) => Selection::new(
            a,
            match direction {
                SearchDirection::Forward => end_incl,
                SearchDirection::Backward => start,
            },
        ),
        None => Selection::new(start, end_incl),
    }
}

/// Ensure the focused buffer has an active search pattern, compiling from
/// `SEARCH_REGISTER` if needed. Returns `true` if a usable pattern is now
/// in place, `false` otherwise.
fn ensure_search_regex(ed: &mut Editor) -> bool {
    if ed.search_pattern().is_some() {
        return true;
    }
    let pattern = ed
        .registers
        .read(SEARCH_REGISTER)
        .and_then(|r| r.as_text().and_then(|v| v.first()).cloned())
        .unwrap_or_default();
    if pattern.is_empty() {
        return false;
    }
    match compile_search_regex(&pattern) {
        Some(r) => {
            let bid = ed.focused_buffer_id();
            ed.buffers.get_mut(bid).search_pattern = Some(SearchPattern {
                regex: Arc::new(r),
                pattern_str: pattern,
            });
            true
        }
        None => false,
    }
}

/// Shared body for `search-next` / `search-prev` / extend variants.
///
/// Reads the cached `search_regex` (compiled during the search session), or
/// recompiles from the `'s'` register if the cache is empty. Repeats `count`
/// times (e.g. `3n` jumps 3 matches forward). Moves or extends the primary
/// selection depending on `extend`.
fn search_jump(
    ed: &mut Editor,
    count: usize,
    direction: SearchDirection,
    mode: MotionMode,
) -> Result<(), CommandError> {
    if !ensure_search_regex(ed) {
        return Ok(());
    }

    let regex = {
        let bid = ed.focused_buffer_id();
        match ed.buffers.get(bid).search_pattern.as_ref() {
            Some(sp) => Arc::clone(&sp.regex),
            None => return Ok(()),
        }
    };

    // Capture anchor before the loop (extend mode keeps the original anchor fixed).
    let (mut from_char, anchor) = {
        let buf = ed.doc().text();
        let primary = ed.current_selections().primary();
        let from = match direction {
            // Step past the current match so we don't re-find it on the first jump.
            SearchDirection::Forward => next_grapheme_boundary(buf, primary.end_inclusive(buf)),
            SearchDirection::Backward => primary.start(),
        };
        (
            from,
            if mode == MotionMode::Extend {
                Some(primary.anchor)
            } else {
                None
            },
        )
    };

    // Jump `count` times, advancing `from_char` after each match so that
    // `3n` really does land on the 3rd match from the current position.
    //
    // When the match cache is populated we binary-search it (O(log M) per
    // jump). When it is empty — e.g. the very first `n` after startup before
    // the cache is warmed — we fall back to the O(buffer) regex-scan path.
    let count = count.max(1);
    let mut last_match: Option<(usize, usize)> = None;
    let mut any_wrapped = false;
    let bid = ed.focused_buffer_id();

    if !ed.buffers.get(bid).search_matches.matches.is_empty() {
        let cached_matches = &ed.buffers.get(bid).search_matches.matches;
        for _ in 0..count {
            match find_match_from_cache(cached_matches, from_char, direction) {
                Some((start, end_incl, wrapped)) => {
                    any_wrapped |= wrapped;
                    last_match = Some((start, end_incl));
                    from_char = match direction {
                        SearchDirection::Forward => {
                            next_grapheme_boundary(ed.doc().text(), end_incl)
                        }
                        SearchDirection::Backward => start,
                    };
                }
                None => {
                    last_match = None;
                    break;
                }
            }
        }
    } else {
        for _ in 0..count {
            match find_next_match(ed.doc().text(), &regex, from_char, direction) {
                Some((start, end_incl, wrapped)) => {
                    any_wrapped |= wrapped;
                    last_match = Some((start, end_incl));
                    from_char = match direction {
                        SearchDirection::Forward => {
                            next_grapheme_boundary(ed.doc().text(), end_incl)
                        }
                        SearchDirection::Backward => start,
                    };
                }
                None => {
                    last_match = None;
                    break;
                }
            }
        }
    }

    match last_match {
        Some((start, end_incl)) => {
            ed.current_search_cursor_mut().wrapped = any_wrapped;
            let new_sel = search_sel(start, end_incl, anchor, direction);
            ed.set_primary_selection(new_sel);
            Ok(())
        }
        None => Err(CommandError("no match".into())),
    }
}

/// Clear the active search regex and dismiss all match highlights.
///
/// Also invocable as `:clear-search` / `:cs` in command mode.
pub(super) fn cmd_clear_search(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let bid = ed.focused_buffer_id();
    ed.clear_buffer_search(bid);
    Ok(())
}

pub(super) fn cmd_search_next(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    search_jump(ed, count, SearchDirection::Forward, mode)
}
pub(super) fn cmd_search_prev(
    ed: &mut Editor,
    count: usize,
    mode: MotionMode,
) -> Result<(), CommandError> {
    search_jump(ed, count, SearchDirection::Backward, mode)
}

// ── Select all matches ────────────────────────────────────────────────────────

/// Turn every search match in the buffer into a selection.
///
/// Uses the active search regex, falling back to recompiling from the `'s'`
/// register (same as `n`/`N`). If there is no active search, does nothing.
/// The first match becomes primary.
pub(super) fn cmd_select_all_matches(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    if !ensure_search_regex(ed) {
        return Ok(());
    }
    let bid = ed.focused_buffer_id();
    let regex = match ed.buffers.get(bid).search_pattern.as_ref() {
        Some(sp) => Arc::clone(&sp.regex),
        None => return Ok(()),
    };

    let matches = find_all_matches(ed.doc().text(), &regex);
    if matches.is_empty() {
        return Err(CommandError("no matches".into()));
    }

    let sels: Vec<Selection> = matches
        .into_iter()
        .map(|(s, e)| Selection::new(s, e))
        .collect();
    ed.set_current_selections(SelectionSet::from_vec(sels, 0));
    Ok(())
}

// ── Select within (s) ────────────────────────────────────────────────────────

/// Enter Select mode.
///
/// Snapshots the current selections for cancel-restore, then opens the
/// mini-buffer with the `s` prompt. The user types a regex; all matches
/// within the current selections become new selections (live preview).
pub(super) fn cmd_select_within(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    // Nothing meaningful to search within a single-char selection.
    if ed
        .current_selections()
        .iter_sorted()
        .all(Selection::is_collapsed)
    {
        return Ok(());
    }
    let pre_sels = ed.current_selections().clone();
    let pid = ed.focused_pane_id;
    ed.pane_transient[pid].pre_select_sels = Some(pre_sels);
    ed.set_mode(Mode::Select);
    ed.minibuf = Some(MiniBuffer {
        prompt: '⫽',
        input: String::new(),
        cursor: 0,
    });
    Ok(())
}

// ── Use selection as search (*) ──────────────────────────────────────────────

/// Use the primary selection text as the search pattern.
///
/// If the primary selection is a cursor (1-char), expands to the word under
/// the cursor first (same as Helix). The escaped text is compiled as a search
/// regex, stored in the `'s'` register, and search highlights appear immediately.
pub(super) fn cmd_use_selection_as_search(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let buf = ed.doc().text();
    let primary = ed.current_selections().primary();

    // If cursor (1-char selection), expand to inner word first.
    let (text, new_sel): (String, Option<Selection>) = if primary.is_collapsed() {
        let Some((start, end)) = inner_word_impl(buf, primary.head, is_word_boundary) else {
            return Ok(()); // cursor on structural newline or similar — nothing to do
        };
        let word_text = buf.slice(start..end + 1).to_string();
        (word_text, Some(Selection::new(start, end)))
    } else {
        let text = buf
            .slice(primary.start()..primary.end_inclusive(buf) + 1)
            .to_string();
        (text, None)
    };

    if text.is_empty() {
        return Ok(());
    }

    // Update the primary selection to cover the word (for cursor expansion).
    if let Some(sel) = new_sel {
        ed.set_primary_selection(sel);
    }

    let escaped = escape_regex(&text);
    let Some(regex) = compile_search_regex(&escaped) else {
        return Ok(());
    };

    // Store in search register and set as active search.
    ed.registers
        .write_text(SEARCH_REGISTER, vec![escaped.clone()]);
    ed.search.direction = SearchDirection::Forward;
    let bid = ed.focused_buffer_id();
    ed.buffers.get_mut(bid).search_pattern = Some(SearchPattern {
        regex: Arc::new(regex),
        pattern_str: escaped,
    });
    Ok(())
}

// ── Misc ──────────────────────────────────────────────────────────────────────

pub(super) fn cmd_quit(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    ed.should_quit = true;
    Ok(())
}

// ── Typed command implementations ────────────────────────────────────────────
//
// These functions are registered in `CommandRegistry` as typed commands
// (`:` command line). They are `pub(super)` so `registry.rs` can import them.

pub(super) fn typed_quit(
    ed: &mut Editor,
    _arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    if !force && ed.doc().is_dirty() {
        Err(CommandError("Unsaved changes (add ! to override)".into()))
    } else {
        ed.should_quit = true;
        Ok(())
    }
}

pub(super) fn typed_write(
    ed: &mut Editor,
    arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    write_file(ed, arg, force)
}

pub(super) fn typed_write_quit(
    ed: &mut Editor,
    arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    // force applies to both write (chmod-retry on readonly targets) and quit
    // (quit even if the write fails).
    match write_file(ed, arg, force) {
        Ok(()) => {
            ed.should_quit = true;
            Ok(())
        }
        Err(e) if force => {
            ed.should_quit = true;
            Err(e)
        }
        Err(e) => Err(e),
    }
}

pub(super) fn typed_toggle_soft_wrap(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    use engine::pane::WrapMode;
    let currently_wrapping = ed.doc().overrides.wrap_mode(&ed.settings).is_wrapping();
    if currently_wrapping {
        ed.doc_mut().overrides.wrap_mode = Some(WrapMode::None);
        // Horizontal offset is now meaningful; scroll stays where it is.
    } else {
        // width: 0 is the sentinel for "content width" — resolved via
        // WrapMode::resolve(content_width) at render time, so this reflows on resize.
        ed.doc_mut().overrides.wrap_mode = Some(WrapMode::Indent { width: 0 });
        ed.viewport_mut().horizontal_offset = 0;
        ed.viewport_mut().top_row_offset = 0;
    }
    let state = if currently_wrapping { "off" } else { "on" };
    ed.report(Severity::Info, format!("Soft wrap {state}"));
    Ok(())
}

pub(super) fn typed_set(
    ed: &mut Editor,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    const USAGE: &str = "Usage: :set global|buffer key=value";
    let Some(arg) = arg else {
        return Err(CommandError(USAGE.into()));
    };
    let Some((scope, rest)) = arg.split_once(' ') else {
        return Err(CommandError(USAGE.into()));
    };
    let Some((key, value)) = rest.split_once('=') else {
        return Err(CommandError("Expected key=value".into()));
    };
    let bid = ed.focused_buffer_id();
    let result = match scope {
        "global" => crate::settings::apply_setting(
            crate::settings::SettingScope::Global,
            key,
            value,
            &mut ed.settings,
            &mut ed.buffers.get_mut(bid).overrides,
        ),
        "buffer" => crate::settings::apply_setting(
            crate::settings::SettingScope::Text,
            key,
            value,
            &mut ed.settings,
            &mut ed.buffers.get_mut(bid).overrides,
        ),
        _ => Err(format!(
            "unknown scope '{scope}': expected 'global' or 'buffer'"
        )),
    };
    if result.is_ok() && key == "history-capacity" {
        ed.history.set_capacity(ed.settings.history_capacity);
    }
    if result.is_ok() && key == "theme" && scope == "global" {
        let name = ed.settings.theme.clone();
        if !name.is_empty() {
            ed.load_theme_by_name(&name);
        }
    }
    result.map_err(CommandError)
}

/// Serialize the buffer and write it to disk.
///
/// If `arg` is `Some(path)`, performs a save-as: writes to the specified
/// path and updates `ed.file_path` / `ed.file_meta` so that subsequent
/// `:w` (no argument) targets the same path.
///
/// If `arg` is `None`, writes to the current file. Errors with
/// "no file name" if the buffer is a scratch buffer with no path.
///
/// When `force` is `true`, a `PermissionDenied` rename error triggers a
/// chmod-retry: the target is made writable, the rename is retried, and the
/// status message includes "(forced)".
///
/// On success, calls `ed.doc_mut().mark_saved()` and sets a status message.
/// Returns `Ok(())` on success, `Err(CommandError)` on any error.
fn write_file(ed: &mut Editor, arg: Option<&str>, force: bool) -> Result<(), CommandError> {
    let (content, line_count) = {
        let buf = ed.doc().text();
        // The rope is always stored LF-normalized; restore CRLF for files that
        // originally used it so we don't silently change line endings on save.
        let content = if buf.line_ending() == crate::core::text::LineEnding::CrLf {
            buf.to_string().replace('\n', "\r\n")
        } else {
            buf.to_string()
        };
        // The buffer always ends with a structural '\n', so len_lines() returns
        // one more than the number of visible lines (ropey counts the empty
        // string after the final newline as an extra line).
        let line_count = buf.len_lines().saturating_sub(1);
        (content, line_count)
    };

    if let Some(path_str) = arg {
        let expanded = crate::os::path::expand(path_str);
        let path: std::path::PathBuf = {
            let p = std::path::Path::new(expanded.as_ref());
            // Resolve relative paths against editor.cwd, not the process cwd,
            // so `:w relpath` is stable regardless of how the process cwd drifts.
            if p.is_relative() { ed.cwd.join(p) } else { p.to_owned() }
        };
        // Try to preserve existing file's permissions; if the file doesn't
        // exist yet, write_file_new creates it with default permissions.
        let result = match crate::os::io::read_file_meta(&path) {
            Ok(meta) => crate::os::io::write_file_atomic(&content, &meta, force)
                .map(|retried| (meta, retried)),
            Err(_) => crate::os::io::write_file_new(&content, &path).map(|meta| (meta, false)),
        };
        match result {
            Ok((meta, retried)) => {
                // Store the canonicalized path so path and file_meta.resolved_path
                // always agree, even when the user supplied a relative or symlink path.
                ed.doc_mut().set_path(Some(meta.resolved_path.clone()));
                ed.doc_mut().file_meta = Some(meta);
                ed.doc_mut().mark_saved();
                ed.report(write_severity(retried), write_msg(line_count, retried));
                ed.fire_hook_buffer_save(ed.focused_buffer_id());
                Ok(())
            }
            Err(e) => Err(CommandError(e.to_string())),
        }
    } else {
        // Write to the current file.
        let Some(meta) = ed.doc().file_meta.as_ref() else {
            return Err(CommandError("no file name".into()));
        };
        match crate::os::io::write_file_atomic(&content, meta, force) {
            Ok(retried) => {
                ed.doc_mut().mark_saved();
                ed.report(write_severity(retried), write_msg(line_count, retried));
                ed.fire_hook_buffer_save(ed.focused_buffer_id());
                Ok(())
            }
            Err(e) => Err(CommandError(e.to_string())),
        }
    }
}

fn write_severity(forced: bool) -> Severity {
    if forced {
        Severity::Warning
    } else {
        Severity::Info
    }
}

fn write_msg(line_count: usize, forced: bool) -> String {
    if forced {
        format!("Written {line_count} lines (forced)")
    } else {
        format!("Written {line_count} lines")
    }
}

// ── Jump list navigation ─────────────────────────────────────────────────────

pub(super) fn cmd_jump_backward(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let pid = ed.focused_pane_id;
    let current = ed.current_jump_entry();
    let nav = ed.pane_jumps[pid]
        .backward(current)
        .map(|e| (e.buffer_id, e.selections.clone()));
    if let Some((target_buf, sels)) = nav {
        if target_buf != ed.focused_buffer_id() {
            ed.switch_to_buffer_without_jump(target_buf);
        }
        ed.set_current_selections(sels);
    }
    Ok(())
}

pub(super) fn cmd_jump_forward(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    let pid = ed.focused_pane_id;
    let nav = ed.pane_jumps[pid]
        .forward()
        .map(|e| (e.buffer_id, e.selections.clone()));
    if let Some((target_buf, sels)) = nav {
        if target_buf != ed.focused_buffer_id() {
            ed.switch_to_buffer_without_jump(target_buf);
        }
        ed.set_current_selections(sels);
    }
    Ok(())
}

// ── Alternate buffer ─────────────────────────────────────────────────────────

/// `Ctrl+6` / `goto-alternate-file` — switch to the most-recently-focused
/// other buffer.
///
/// Uses `switch_to_buffer_without_jump` because `execute_keymap_command` already
/// records the pre-switch state for all `is_jump=true` commands. Using the
/// `_with_jump` variant here would push twice, corrupting the jump list on the
/// second Ctrl+O.
pub(super) fn cmd_goto_alternate_file(
    ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    match ed.alternate_buffer() {
        Some(id) => ed.switch_to_buffer_without_jump(id),
        None => ed.report(Severity::Warning, "No alternate buffer".to_string()),
    }
    Ok(())
}

// ── Message log ──────────────────────────────────────────────────────────────

/// `:messages` — open the message log in a read-only scratch buffer.
///
/// Displays all logged warnings, errors, and trace entries accumulated during
/// the session. Cursor starts at the last entry (most recent). Dismissed with
/// `q` or Escape.
pub(super) fn typed_messages(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let content = ed.message_log.format_for_display();
    if content.is_empty() {
        ed.report(Severity::Info, "No messages".to_string());
        return Ok(());
    }
    let sv = ScratchView::from_text(&content, "[messages]");
    ed.scratch_view = Some(sv);
    ed.message_log.mark_all_seen();
    Ok(())
}

/// `:ls` / `:list-buffers` — open a read-only scratch view listing every open buffer.
///
/// Each row shows: 1-based index, current (`%`) / alternate (`#`) marker,
/// dirty (`+`) flag, short name, and home-shortened absolute path.
/// Cursor is placed on the row corresponding to the currently focused buffer.
pub(super) fn typed_list_buffers(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let current = ed.focused_buffer_id();
    let alternate = ed.alternate_buffer();

    let header = format!("{:>4}    {:<32}  {}\n", "buf", "name", "path");
    let mut out = header;
    // The header occupies rope line 0; each buffer occupies rope line `idx + 1`.
    // `current_rope_line` tracks that index so the cursor opens on the right row.
    let mut current_rope_line: usize = 1;

    for (idx, (id, buf)) in ed.buffers.iter().enumerate() {
        let display_num = idx + 1;
        let rope_line = idx + 1; // 1 header line before buffer rows

        let cur_marker = if id == current {
            '%'
        } else if matches!(alternate, Some(alt) if id == alt) {
            '#'
        } else {
            ' '
        };
        let dirty_marker = if buf.is_dirty() { '+' } else { ' ' };

        let path_ref = buf.path();
        let name = path_ref
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("[scratch]");
        let path = path_ref
            .map(crate::os::path::shorten_home)
            .unwrap_or_default();

        out.push_str(&format!(
            "{:>4}  {}{}  {:<32}  {}\n",
            display_num, cur_marker, dirty_marker, name, path
        ));

        if id == current {
            current_rope_line = rope_line;
        }
    }

    ed.scratch_view = Some(ScratchView::from_text_at_line(
        &out,
        "[buffers]",
        current_rope_line,
    ));
    Ok(())
}

/// `:reload-plugin <name>` — tear down the named plugin's ledger entries and
/// re-evaluate its `plugin.scm`.  If the plugin file no longer exists on disk,
/// teardown still runs but re-eval is silently skipped (same "not on disk →
/// skip" rule as `load-plugin`).
pub(super) fn typed_reload_plugin(
    ed: &mut Editor,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let name = arg.ok_or_else(|| CommandError("Usage: :reload-plugin <name>".into()))?;
    if let Some(host) = ed.scripting.as_mut() {
        let builtin_names: std::collections::HashSet<String> =
            ed.registry.names().map(String::from).collect();
        let (cmds_to_remove, new_cmds) = host
            .reload_plugin(name, &mut ed.settings, &mut ed.keymap, builtin_names)
            .map_err(CommandError)?;
        for cmd_name in cmds_to_remove {
            ed.registry.unregister(&cmd_name);
        }
        ed.register_steel_cmds(new_cmds);
        ed.report(Severity::Info, format!("Reloaded plugin '{name}'"));
    }
    ed.flush_script_messages();
    Ok(())
}

/// `:reload-config` — drop the scripting engine and re-evaluate `init.scm`
/// from scratch, restoring a clean slate.
///
/// Stale `SteelBacked` entries from the previous init must be removed from the
/// registry before `init_scripting()` runs: otherwise the new `builtin_names`
/// set (built from `registry.names()`) would contain every Steel command from
/// the prior load, and every `(define-command!)` in the re-evaluated
/// `init.scm` would fail the builtin-conflict check in
/// `editor/src/scripting/builtins/commands.rs` with "conflicts with a built-in
/// command and cannot be redefined".
pub(super) fn typed_reload_config(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    ed.scripting = None;
    ed.registry.unregister_all_steel_backed();
    ed.init_scripting();
    ed.report(Severity::Info, "Config reloaded".to_string());
    Ok(())
}

// ── Multi-buffer typed commands ───────────────────────────────────────────────

/// `:e [path]` — open a file in the current window.
///
/// - No `path`: reload current file from disk (`:e!` discards unsaved changes).
/// - `path` given and already open: switch to the existing buffer.
/// - `path` given and not open: read from disk, open a new buffer, switch to it.
///
/// Dedup uses `find_by_path` (canonical path comparison). `force` (`!` suffix)
/// only takes effect in the no-arg reload branch: it discards unsaved changes
/// and re-reads the file from disk. When a path is given, `force` is unused.
pub(super) fn typed_edit(
    ed: &mut Editor,
    arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    use std::path::Path;

    if let Some(path_str) = arg {
        let expanded = crate::os::path::expand(path_str);

        // If a buffer is already open for this path, switch without re-reading.
        // Matches Vim semantics and covers the deleted-from-disk case.
        if let Some(bid) = find_buffer_by_path_arg(ed, expanded.as_ref()) {
            if bid != ed.focused_buffer_id() {
                ed.switch_to_buffer_with_jump(bid);
            }
            warn_if_file_gone(ed, bid);
            return Ok(());
        }

        let path = Path::new(expanded.as_ref());
        let canonical = std::fs::canonicalize(path)
            .map_err(|e| CommandError(format!("{}: {e}", path.display())))?;
        let (bid, is_new) = ed
            .open_or_dedup(&canonical)
            .map_err(|e| CommandError(format!("{}: {e}", path.display())))?;
        if is_new {
            let name = canonical
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path_str)
                .to_string();
            ed.switch_to_buffer_with_jump(bid);
            ed.report(Severity::Info, format!("Opened {name}"));
        } else if bid != ed.focused_buffer_id() {
            ed.switch_to_buffer_with_jump(bid);
        }
        Ok(())
    } else {
        // Reload current file.
        let Some(path) = ed.doc().path().map(Path::to_path_buf) else {
            if force {
                // :e! on scratch: replace with fresh scratch.
                let id = ed.focused_buffer_id();
                ed.replace_buffer_in_place(id, crate::editor::buffer::Buffer::scratch());
                return Ok(());
            }
            return Err(CommandError("no file name".into()));
        };
        if ed.doc().is_dirty() && !force {
            return Err(CommandError("unsaved changes (use :e! to force)".into()));
        }
        let doc = crate::editor::buffer::Buffer::from_file(&path)
            .map_err(|e| CommandError(format!("{}: {e}", path.display())))?;
        let id = ed.focused_buffer_id();
        ed.replace_buffer_in_place(id, doc);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        ed.report(Severity::Info, format!("Reloaded {name}"));
        Ok(())
    }
}

/// `:cd [path]` — change the working directory.
///
/// - No arg: change to `$HOME`.
/// - `path` given: `~` / env-var expansion applied first; relative paths
///   resolve against the current process cwd (which mirrors `editor.cwd`).
pub(super) fn typed_cd(
    ed: &mut Editor,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let target = match arg.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => {
            let expanded = crate::os::path::expand(s);
            std::path::PathBuf::from(expanded.as_ref())
        }
        None => crate::os::dirs::home_dir().ok_or_else(|| CommandError("HOME not set".into()))?,
    };

    let resolved = ed
        .set_cwd(&target)
        .map_err(|e| CommandError(format!("{}: {e}", target.display())))?;
    ed.report(Severity::Info, format!("cwd: {}", resolved.display()));
    Ok(())
}

/// `:pwd` / `:print-working-directory` — display the current working directory.
pub(super) fn typed_pwd(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    ed.report(Severity::Info, crate::os::path::shorten_home(&ed.cwd));
    Ok(())
}

/// `:bd` — delete (close) the focused buffer.
///
/// If the buffer is dirty and `force` is false, returns an error.
/// If it is the only buffer, it is replaced with a scratch buffer.
pub(super) fn typed_buffer_delete(
    ed: &mut Editor,
    _arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    if ed.doc().is_dirty() && !force {
        return Err(CommandError("unsaved changes (use :bd! to force)".into()));
    }
    let id = ed.focused_buffer_id();
    ed.close_buffer(id);
    Ok(())
}

/// `:b` / `:buffer` — switch to an open buffer by name, prefix, index, or full path.
///
/// Accepts four argument forms (tried in order):
/// 1. Numeric 1-based index matching `:ls` output.
/// 2. Absolute path — resolved via canonicalize then looked up in the store.
/// 3. Exact display-name match (basename or `*scratch*`).
/// 4. Unique basename prefix.
///
/// The `force` flag is accepted syntactically but has no effect — there is
/// nothing to force on a plain buffer switch.
pub(super) fn typed_buffer(
    ed: &mut Editor,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let arg = arg.ok_or_else(|| CommandError("usage: :b <name|#|index>".into()))?;
    let bid = resolve_buffer_arg(ed, arg)?;
    if bid != ed.focused_buffer_id() {
        ed.switch_to_buffer_with_jump(bid);
    }
    warn_if_file_gone(ed, bid);
    Ok(())
}

/// Find an open buffer matching a path argument.
///
/// Tries `fs::canonicalize` first (resolves symlinks, requires the file to
/// exist), then falls back to `std::path::absolute` (pure lexical: joins with
/// cwd, removes `.`/`..`, no filesystem access). The fallback keeps buffers
/// reachable after their backing file has been deleted.
fn find_buffer_by_path_arg(ed: &Editor, arg: &str) -> Option<BufferId> {
    if let Ok(canonical) = std::fs::canonicalize(arg)
        && let Some(bid) = ed.buffers.find_by_path(&canonical)
    {
        return Some(bid);
    }
    let abs = std::path::absolute(arg).ok()?;
    ed.buffers.find_by_path(&abs)
}

/// Emit a warning if `bid`'s backing file no longer exists on disk.
fn warn_if_file_gone(ed: &mut Editor, bid: BufferId) {
    // Check while holding the borrow; capture only the display string so the
    // borrow is released before the &mut ed.report() call below.
    let display = ed.buffers.get(bid).path().and_then(|p| {
        match std::fs::metadata(p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Some(p.display().to_string())
            }
            _ => None,
        }
    });
    if let Some(msg) = display {
        ed.report(
            Severity::Warning,
            format!("{msg}: file no longer exists on disk"),
        );
    }
}

/// Resolve a `:b` argument to a `BufferId`.  See [`typed_buffer`] for the
/// four-step resolution order.
fn resolve_buffer_arg(ed: &Editor, arg: &str) -> Result<BufferId, CommandError> {
    use crate::editor::buffer::Buffer;
    use std::path::Path;

    // Label used in ambiguity messages: full path when available, literal
    // `*scratch*` otherwise. Unambiguous regardless of whether the collision
    // was on basename or prefix.
    let label = |buf: &Buffer| -> String {
        buf.path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| Buffer::SCRATCH_BUFFER_NAME.to_owned())
    };

    // 1. Numeric 1-based index.
    if let Ok(n) = arg.parse::<usize>() {
        let idx = n
            .checked_sub(1)
            .ok_or_else(|| CommandError(format!("no buffer at index {n}")))?;
        return ed
            .buffers
            .iter()
            .nth(idx)
            .map(|(id, _)| id)
            .ok_or_else(|| CommandError(format!("no buffer at index {n}")));
    }

    // 2. Absolute path — match an open buffer by canonical OR lexical path.
    //    Lexical fallback keeps buffers reachable after their file is deleted.
    if Path::new(arg).is_absolute() {
        return find_buffer_by_path_arg(ed, arg)
            .ok_or_else(|| CommandError(format!("{arg}: not an open buffer")));
    }

    // 3. Exact display-name match.
    let exact: Vec<BufferId> = ed
        .buffers
        .iter()
        .filter(|(_, buf)| buf.display_name() == arg)
        .map(|(id, _)| id)
        .collect();
    match exact.len() {
        1 => return Ok(exact[0]),
        n if n > 1 => {
            let labels: Vec<String> = exact.iter().map(|&id| label(ed.buffers.get(id))).collect();
            return Err(CommandError(format!(
                "ambiguous buffer name '{arg}': {}",
                labels.join(", ")
            )));
        }
        _ => {} // fall through to prefix match
    }

    // 4. Unique basename-prefix match.
    let prefix_matches: Vec<BufferId> = ed
        .buffers
        .iter()
        .filter(|(_, buf)| buf.display_name().starts_with(arg))
        .map(|(id, _)| id)
        .collect();
    match prefix_matches.len() {
        0 => Err(CommandError(format!("no buffer matching '{arg}'"))),
        1 => Ok(prefix_matches[0]),
        _ => {
            let labels: Vec<String> = prefix_matches
                .iter()
                .map(|&id| label(ed.buffers.get(id)))
                .collect();
            Err(CommandError(format!(
                "ambiguous prefix '{arg}': {}",
                labels.join(", ")
            )))
        }
    }
}

/// `:bnext` / `:bn` — switch to the next buffer in open-order.
pub(super) fn typed_bnext(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let target = ed.buffers.next(ed.focused_buffer_id());
    if target != ed.focused_buffer_id() {
        ed.switch_to_buffer_with_jump(target);
    }
    Ok(())
}

/// `:bprev` / `:bp` — switch to the previous buffer in open-order.
pub(super) fn typed_bprev(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let target = ed.buffers.prev(ed.focused_buffer_id());
    if target != ed.focused_buffer_id() {
        ed.switch_to_buffer_with_jump(target);
    }
    Ok(())
}

// ── Pane focus stubs (M9+) ────────────────────────────────────────────────────

pub(super) fn cmd_pane_focus_next(
    _ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    Err(CommandError(":split not yet implemented".into()))
}

pub(super) fn cmd_pane_focus_left(
    _ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    Err(CommandError(":split not yet implemented".into()))
}

pub(super) fn cmd_pane_focus_right(
    _ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    Err(CommandError(":split not yet implemented".into()))
}

pub(super) fn cmd_pane_focus_up(
    _ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    Err(CommandError(":split not yet implemented".into()))
}

pub(super) fn cmd_pane_focus_down(
    _ed: &mut Editor,
    _count: usize,
    _mode: MotionMode,
) -> Result<(), CommandError> {
    Err(CommandError(":split not yet implemented".into()))
}

// ── :split / :vsplit typed stubs ──────────────────────────────────────────────

pub(super) fn typed_split(
    _ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    Err(CommandError(":split not yet implemented".into()))
}

pub(super) fn typed_vsplit(
    _ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    Err(CommandError(":vsplit not yet implemented".into()))
}

/// `:theme <name>` — load a theme by name from the theme search path.
///
/// On success the engine view's theme is replaced and re-baked.
/// On failure a warning is shown and the current theme is left unchanged.
pub(super) fn typed_theme(
    ed: &mut Editor,
    arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    let Some(name) = arg.map(str::trim).filter(|s| !s.is_empty()) else {
        let current = if ed.settings.theme.is_empty() {
            "default (built-in)".to_owned()
        } else {
            ed.settings.theme.clone()
        };
        ed.report(Severity::Info, format!("Current theme: {current}"));
        return Ok(());
    };
    ed.load_theme_by_name(name);
    // Update settings regardless of success — if load fails, the warning already
    // appeared; persisting the name lets init.scm re-try on reload.
    ed.settings.theme = name.to_owned();
    Ok(())
}

/// `:theme-debug` — print the resolved style chain for key UI scopes.
///
/// Reports the scope name, resolution chain, and final fg/bg/modifiers for
/// the cursor, selection, and cursorline scopes from the active theme.
pub(super) fn typed_theme_debug(
    ed: &mut Editor,
    _arg: Option<&str>,
    _force: bool,
) -> Result<(), CommandError> {
    use ratatui::style::Color;

    fn color_str(c: Option<Color>) -> String {
        match c {
            Some(Color::Rgb(r, g, b)) => format!("#{r:02x}{g:02x}{b:02x}"),
            Some(other) => format!("{other:?}"),
            None => "-".to_owned(),
        }
    }

    fn scope_chain(theme: &engine::theme::Theme, scope: &str) -> String {
        // Walk the dot-notation prefix chain and collect names that have entries.
        let mut chain: Vec<&str> = Vec::new();
        let mut cur = scope;
        loop {
            if theme.raw_contains(cur) {
                chain.push(cur);
            }
            match cur.rfind('.') {
                Some(dot) => cur = &cur[..dot],
                None => break,
            }
        }
        if chain.is_empty() {
            format!("{scope} → default")
        } else {
            chain.join(" → ")
        }
    }

    let theme = &ed.engine_view.theme;
    let name = if ed.settings.theme.is_empty() {
        "default (built-in)"
    } else {
        &ed.settings.theme
    };

    let scopes = [
        "ui.cursor.primary",
        "ui.cursor",
        "ui.cursor.insert",
        "ui.selection",
        "ui.cursorline",
        "ui.statusline",
    ];

    let mut lines = vec![format!("Theme: {name}")];
    for scope in scopes {
        let style = theme.resolve_by_name(engine::types::Scope(scope));
        let chain = scope_chain(theme, scope);
        lines.push(format!(
            "  {scope}: chain={chain} fg={} bg={}{}",
            color_str(style.fg),
            color_str(style.bg),
            if style.modifiers.is_empty() {
                String::new()
            } else {
                format!(" modifiers={:?}", style.modifiers)
            },
        ));
    }

    ed.report(Severity::Info, lines.join("\n"));
    Ok(())
}
