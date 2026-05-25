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

/// Display label used when no named theme is active (the compiled-in default).
pub(super) const DEFAULT_THEME_LABEL: &str = "default (built-in)";

use super::{register_ops, Severity};
use super::{Editor, RegisterPrefix};

// ── Kill-ring command name sets ───────────────────────────────────────────────
// Three sets, kept adjacent so they're maintained together:
//
//  SMART_P_LAST_CMDS — allow-list for Smart-p: bare `p`/`P` reads the ring
//    head when `last_command` is in this set; otherwise reads the clipboard.
//
//  RING_CYCLE_CMDS — commands that must NOT commit the paste session before
//    dispatch; every other command commits first so cycles fold into one undo
//    step.
//
//  PASTE_FAMILY_CMDS — all four paste/cycle commands; used for append detection:
//    a fresh `p`/`P` collapses the previous paste output rather than replacing
//    it when `last_command` is in this set.

/// Commands that keep Smart-p in "ring" mode: bare `p`/`P` reads the ring
/// head when `last_command` is one of these; otherwise reads the clipboard.
///
/// Only `change` and `delete` belong here. Paste-family commands are handled
/// via the append path in `do_paste` (which re-uses `last_paste` verbatim);
/// they never reach this check.
pub(crate) const SMART_P_LAST_CMDS: &[&str] = &["change", "delete"];

/// Commands that must not commit the open paste session before dispatch.
/// `[` and `]` re-paste from the same snapshot and should fold into one undo step.
pub(super) const RING_CYCLE_CMDS: &[&str] = &["paste-ring-older", "paste-ring-newer"];

/// All paste-family commands (paste + cycle). A fresh `p`/`P` appends (rather
/// than replaces) when `last_command` is one of these.
pub(super) const PASTE_FAMILY_CMDS: &[&str] =
    &["paste-after", "paste-before", "paste-ring-older", "paste-ring-newer"];

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
        if let Some(w) = register_ops::write_register(&mut self.registers, &mut self.clipboard, name, values) {
            self.report(Severity::Warning, w);
        }
    }
}

mod mode;
mod edit;
mod find;
mod scroll;
mod search;
mod jump;
mod typed_file;
mod typed_buffer;
mod typed_misc;

pub(super) use mode::*;
pub(super) use edit::*;
pub(super) use find::*;
pub(super) use scroll::*;
pub(super) use search::*;
pub(super) use jump::*;
pub(super) use typed_file::*;
pub(super) use typed_buffer::*;
pub(super) use typed_misc::*;

// Visual-line commands live in visual_move.rs; re-export for the registry glob.
pub(super) use super::visual_move::{
    cmd_visual_move_down, cmd_visual_move_up, cmd_visual_select_word_nearest_on_line,
};
