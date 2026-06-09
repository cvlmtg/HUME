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

use engine::pipeline::{BufferId, EngineView};
use editing::selection::SelectionSet;

use super::{register_ops, Severity};
use super::{EditorState, InsertSession, Mode, RegisterPrefix};
use super::buffer::Buffer;
use super::doc_ops;
use super::jump_list::JumpEntry;
use super::search_state::SearchPattern;

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

// ── EditorState helpers ───────────────────────────────────────────────────────

impl EditorState {
    /// Consume the pending `"<reg>` prefix and return the explicit register name,
    /// or `None` if no prefix was typed (bare default case).
    pub(super) fn take_register_prefix(&mut self) -> Option<char> {
        match self.register_prefix.take() {
            Some(RegisterPrefix::Selected(c)) => Some(c),
            _ => None,
        }
    }

    /// Write `values` into `name`, routing `'c'` through the OS clipboard.
    pub(super) fn write_register(&mut self, name: char, values: Vec<String>) {
        if let Some(w) = register_ops::write_register(&mut self.registers, &mut self.clipboard, name, values) {
            self.report(Severity::Warning, w);
        }
    }

    /// Write `values` to the system clipboard only (no kill-ring push).
    pub(super) fn write_clipboard(&mut self, values: &[String]) {
        if let Some(w) = register_ops::write_clipboard(&mut self.registers, &mut self.clipboard, values) {
            self.report(Severity::Warning, w);
        }
    }

    /// Commit the open paste session on every pane/buffer pair that has one.
    ///
    /// Records exactly one history revision for the entire paste + all cycles.
    /// Called by `execute.rs` before any non-`[`/`]` dispatch so the session
    /// is committed before undo, motions, or the next `p`/`P`.
    pub(in super) fn commit_paste_session(&mut self) {
        use engine::pipeline::PaneId;
        let open: Vec<(PaneId, BufferId)> = self.pane_state
            .iter()
            .flat_map(|(pid, inner)| {
                inner.iter()
                    .filter(|(_, pbs)| pbs.paste_group.is_some())
                    .map(move |(bid, _)| (pid, bid))
            })
            .collect();
        for (pid, bid) in open {
            let post_sels = self.pane_state[pid][bid].selections.clone();
            let pbs = &mut self.pane_state[pid][bid];
            self.buffers.get_mut(bid).commit_edit_group(&mut pbs.paste_group, post_sels);
        }
    }
}

// ── Free helpers for EditorCmd handlers ──────────────────────────────────────

/// Buffer id the focused pane is viewing.
pub(super) fn focused_buffer_id(state: &EditorState, view: &EngineView) -> BufferId {
    view.panes[state.focused_pane_id].buffer_id
}

/// Shared reference to the focused buffer.
pub(super) fn doc<'a>(state: &'a EditorState, view: &EngineView) -> &'a Buffer {
    state.buffers.get(focused_buffer_id(state, view))
}

/// `true` when the focused buffer is read-only.
pub(super) fn focused_buffer_read_only(state: &EditorState, view: &EngineView) -> bool {
    doc(state, view).is_read_only()
}

/// Focused pane's selections for the current buffer.
pub(super) fn current_selections<'a>(state: &'a EditorState, view: &EngineView) -> &'a SelectionSet {
    let bid = focused_buffer_id(state, view);
    &state.pane_state[state.focused_pane_id][bid].selections
}

/// The most-recently-focused buffer other than the current one.
pub(super) fn alternate_buffer(state: &EditorState, view: &EngineView) -> Option<BufferId> {
    state.buffers.mru_excluding(focused_buffer_id(state, view))
}

/// `true` when the focused (pane, buffer) has an open edit group.
fn is_group_open_current(state: &EditorState, view: &EngineView) -> bool {
    let bid = focused_buffer_id(state, view);
    state.pane_state[state.focused_pane_id][bid].edit_group.is_some()
}

/// Open a new edit group on the focused (pane, buffer) pair.
pub(super) fn begin_edit_group_current(state: &mut EditorState, view: &EngineView) {
    let pid = state.focused_pane_id;
    let bid = focused_buffer_id(state, view);
    doc_ops::begin_edit_group(&state.buffers, &mut state.pane_state, pid, bid);
}

/// Commit and close the open edit group on the focused (pane, buffer) pair.
pub(super) fn commit_edit_group_current(state: &mut EditorState, view: &EngineView) {
    let pid = state.focused_pane_id;
    let bid = focused_buffer_id(state, view);
    doc_ops::commit_edit_group(&mut state.buffers, &mut state.pane_state, pid, bid);
}

/// Active search pattern on the focused buffer, if any.
pub(super) fn search_pattern<'a>(state: &'a EditorState, view: &EngineView) -> Option<&'a SearchPattern> {
    state.buffers.get(focused_buffer_id(state, view)).search_pattern.as_ref()
}

/// Viewport state of the focused pane.
pub(super) fn viewport<'a>(state: &EditorState, view: &'a EngineView) -> &'a engine::pane::ViewportState {
    &view.panes[state.focused_pane_id].viewport
}

/// Resolved `(wrap_mode, tab_width, whitespace)` for the focused doc and pane.
pub(super) fn focused_format_context(
    state: &EditorState,
    view: &EngineView,
) -> (engine::pane::WrapMode, u8, engine::pane::WhitespaceConfig) {
    let buf = doc(state, view);
    let raw_wrap = buf.overrides.wrap_mode(&state.settings);
    let tab_width = buf.overrides.tab_width(&state.settings);
    let whitespace = buf.overrides.whitespace(&state.settings);
    let pane = &view.panes[state.focused_pane_id];
    let wrap_mode = raw_wrap.resolve(pane.content_width(buf.text().len_lines()));
    (wrap_mode, tab_width, whitespace)
}

/// Snapshot the focused pane's current cursor as a `JumpEntry`.
pub(super) fn current_jump_entry(state: &EditorState, view: &EngineView) -> JumpEntry {
    let pid = state.focused_pane_id;
    let bid = focused_buffer_id(state, view);
    let sels = state.pane_state[pid][bid].selections.clone();
    JumpEntry::new(sels, state.buffers.get(bid).text(), bid)
}

/// Redirect the focused pane to `target` without recording a jump.
pub(super) fn switch_to_buffer_without_jump(
    state: &mut EditorState,
    view: &mut EngineView,
    target: BufferId,
) {
    let pid = state.focused_pane_id;
    super::ops::switch_pane_to_buffer(view, &state.buffers, &mut state.pane_state, pid, target);
}

/// Replace the focused pane's selections for the current buffer.
pub(super) fn set_current_selections(state: &mut EditorState, view: &EngineView, sels: SelectionSet) {
    let bid = focused_buffer_id(state, view);
    state.pane_state[state.focused_pane_id][bid].selections = sels;
}

/// Replace the primary selection in the focused pane (merging overlaps).
pub(super) fn set_primary_selection(state: &mut EditorState, view: &EngineView, new_sel: editing::selection::Selection) {
    let pid = state.focused_pane_id;
    let bid = focused_buffer_id(state, view);
    let idx = state.pane_state[pid][bid].selections.primary_index();
    let sels = std::mem::take(&mut state.pane_state[pid][bid].selections);
    state.pane_state[pid][bid].selections = sels.replace(idx, new_sel).merge_overlapping();
}

/// Enter Insert mode as a repeatable insert action.
///
/// No-op (with a warning) if the focused buffer is read-only. Replay-signal:
/// if an edit group is already open, recording is suppressed but the mode
/// change still happens.
pub(super) fn begin_insert_session(state: &mut EditorState, view: &EngineView) {
    if focused_buffer_read_only(state, view) {
        state.report(Severity::Info, "Buffer is read-only".to_string());
        return;
    }
    if !is_group_open_current(state, view) {
        begin_edit_group_current(state, view);
        state.insert_session = Some(InsertSession {
            keystrokes: Vec::new(),
            step_back_on_exit: false,
        });
    }
    state.mode = Mode::Insert;
}

/// Exit Insert mode and finalise the undo/repeat state.
pub(super) fn end_insert_session(state: &mut EditorState, view: &EngineView) {
    let step_back = state.insert_session.as_ref().is_some_and(|s| s.step_back_on_exit);
    commit_edit_group_current(state, view);
    if let (Some(session), Some(action)) =
        (state.insert_session.take(), state.last_repeatable_action.as_mut())
    {
        action.insert_keys = session.keystrokes;
    }
    if step_back {
        let focused = state.focused_pane_id;
        let buf = focused_buffer_id(state, view);
        doc_ops::apply_doc_motion(
            &state.buffers,
            &mut state.pane_state,
            focused,
            buf,
            |b, sels| {
                sels.map(|sel| {
                    let head = sel.head();
                    let line_start = b.line_to_char(b.char_to_line(head));
                    let new_head = if head > line_start {
                        editing::grapheme::prev_grapheme_boundary(b, head)
                    } else {
                        head
                    };
                    editing::selection::Selection::collapsed(new_head)
                })
            },
        );
    }
    state.mode = Mode::Normal;
}

/// Enqueue an `OnModeChange` hook for `(old → new)` in `state.pending_hooks`.
///
/// Drained by `Editor::drain_hooks` after the command returns.
pub(super) fn enqueue_mode_change(state: &mut EditorState, old: Mode, new: Mode) {
    use scripting::hooks::HookId;
    use steel::rvals::IntoSteelVal;
    if old == new {
        return;
    }
    let old_val = mode_name(old).into_steelval().expect("mode str into_steelval");
    let new_val = mode_name(new).into_steelval().expect("mode str into_steelval");
    state.pending_hooks.push((HookId::OnModeChange, vec![old_val, new_val]));
}

fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Normal => "normal",
        Mode::Insert => "insert",
        Mode::Extend => "extend",
        Mode::Command => "command",
        Mode::Search => "search",
        Mode::Select => "select",
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
