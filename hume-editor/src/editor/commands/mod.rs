//! Editor-level command functions.
//!
//! Each function in this module is a command operating on
//! `&mut EditorState` + `&mut EngineView` (the D7 handler shape) — composite
//! operations involving mode changes, registers, undo groups, or
//! parameterized motions (find/till/replace).
//!
//! They are registered in [`super::registry`] and called via function pointer
//! from `execute_keymap_command`, exactly like the pure `cmd_*` functions in
//! `ops/motion.rs`, `ops/edit.rs`, etc.
//!
//! The `count` parameter is the user's numeric prefix (default 1). Commands
//! that don't use a count accept it and ignore it (`_count`).

/// Display label used when no named theme is active (the compiled-in default).
pub(super) const DEFAULT_THEME_LABEL: &str = "default (built-in)";

use std::borrow::Cow;

use hume_engine::pipeline::{BufferId, EngineView};
use hume_editing::selection::SelectionSet;

use super::{register_ops, Severity};
use super::{EditorState, InsertSession, Mode, RegisterPrefix, RepeatableAction, SelectionStep};
use super::buffer::Buffer;
use super::doc_ops;
use super::jump_list::JumpEntry;
use super::registry::MappableCommand;
use super::search_state::SearchPattern;
use crate::ops::MotionMode;

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

// ── Command provenance classifier ────────────────────────────────────────────

/// Why a command ran — determines how `last_command` (the smart-p / paste-append
/// marker) is updated after dispatch.
///
/// Consumers (`do_paste`, paste-append detection) only care about whether the
/// most-recent command was a user-initiated kill (→ ring) or anything else (→
/// clipboard). Encoding *why* something ran here keeps that decision in one place
/// instead of spread across replay + insert-tail + native dispatch sites.
pub(super) enum Provenance {
    /// Direct user keypress in Normal/Extend mode. Becomes the new `last_command`.
    User(Cow<'static, str>),
    /// Command ran during an active insert session (exit-insert, in-insert nav).
    /// Preserves the prior kill marker so `change`/`delete` before the session still
    /// routes `p` to the ring — skip the update entirely.
    InsertTail,
    /// Output of macro or dot-repeat replay. Neutralize `last_command` to `None`
    /// so a bare `p` after replay reads the clipboard rather than whatever kill
    /// command happened to run last inside the replay.
    Replay,
}

// ── EditorState helpers ───────────────────────────────────────────────────────

impl EditorState {
    /// Update `last_command` according to how a command was triggered.
    ///
    /// Call once per dispatch, after the command body runs. This is the single
    /// site that classifies command provenance for smart-p and paste-append.
    pub(super) fn record_command(&mut self, provenance: Provenance) {
        match provenance {
            Provenance::User(name) => self.last_command = Some(name),
            Provenance::InsertTail => {} // keep the prior kill marker
            Provenance::Replay     => self.last_command = None,
        }
    }

    /// Record a repeatable action for dot-repeat.
    ///
    /// Shared by the native dispatch path (`dispatch_native`) and the Steel path
    /// (`execute_keymap_command`) so the struct literal is not duplicated.
    pub(super) fn record_repeatable(
        &mut self,
        command: Cow<'static, str>,
        count: usize,
        char_arg: Option<char>,
        selection_recipe: Vec<SelectionStep>,
    ) {
        self.last_repeatable_action = Some(RepeatableAction {
            command,
            count,
            char_arg,
            insert_keys: Vec::new(),
            selection_recipe,
        });
    }

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
    pub(super) fn commit_paste_session(&mut self) {
        use hume_engine::pipeline::PaneId;
        let open: Vec<(PaneId, BufferId)> = self.panes.state
            .iter()
            .flat_map(|(pid, inner)| {
                inner.iter()
                    .filter(|(_, pbs)| pbs.paste_group.is_some())
                    .map(move |(bid, _)| (pid, bid))
            })
            .collect();
        for (pid, bid) in open {
            let post_sels = self.panes.state[pid][bid].selections.clone();
            let pbs = &mut self.panes.state[pid][bid];
            self.buffers.get_mut(bid).commit_edit_group(&mut pbs.paste_group, post_sels);
        }
    }
}

// ── Native command dispatch funnel ────────────────────────────────────────────

/// Execute a native command (`Motion`/`Selection`/`Edit`/`EditorCmd`).
///
/// This is the **single** dispatch funnel for native commands. Both the
/// interactive keypress path (`execute_keymap_command`) and the Steel sync path
/// (`run_command_sync`) delegate here so bookkeeping never diverges:
///
/// - Paste session committed before dispatch (except ring-cycle commands).
/// - Jump-list: pre/post snapshot for all motions and explicit jump commands.
/// - Dot-repeat: `last_repeatable_action` updated for repeatable commands.
/// - Smart-p: `last_command` updated via `record_command` — `User` in Normal/Extend,
///   skipped during the insert-session tail so the prior kill marker is preserved.
///
/// Register-prefix is **not** armed here — the caller is responsible (`run_command_sync`
/// arms `state.register_prefix` before calling; the keypress path relies on the
/// normal prefix-key path). Commands consume the prefix via `take_register_prefix`.
///
/// `cmd` must be a native variant (`Motion`/`Selection`/`Edit`/`EditorCmd`); passing
/// `SteelBacked` or `Lazy` triggers `unreachable!`.
pub(super) fn dispatch_native(
    state: &mut EditorState,
    view: &mut EngineView,
    cmd: MappableCommand,
    name: Cow<'static, str>,
    count: usize,
    extend: bool,
) {
    let motion_mode = if extend { MotionMode::Extend } else { MotionMode::Move };

    // Capture classifier bools BEFORE the by-value match consumes `cmd`.
    let is_jump       = cmd.is_jump();
    let is_visual     = cmd.is_visual_move();
    let is_motion     = matches!(cmd, MappableCommand::Motion { .. });
    let is_repeatable = cmd.is_repeatable();
    // Motion and Selection commands are potential selection-builders. EditorCmd,
    // Edit, Steel, etc. are NOT — gating on variant prevents undo/redo (which are
    // non-repeatable EditorCmds that can leave a non-collapsed selection) from
    // polluting the recipe buffer.
    let is_sel_builder = matches!(
        cmd,
        MappableCommand::Motion { .. } | MappableCommand::Selection { .. }
    );

    // Commit any open paste session before non-cycle dispatch so all `[`/`]`
    // cycles fold into a single undo step. Must happen before the actual dispatch
    // so that `undo` sees a committed revision.
    if !RING_CYCLE_CMDS.contains(&name.as_ref()) {
        state.commit_paste_session();
    }

    // Snapshot pending_char before dispatch — commands consume it via `.take()`.
    let char_arg = state.pending_char;

    // Smart-p gate: capture mode before dispatch so the post-dispatch stamp
    // (below) can tell whether this command ran inside an insert session.
    let pre_mode = state.mode();

    // Jump list: pre-command snapshot for motions and explicit jump commands.
    let pre_jump = (is_jump || is_visual || is_motion).then(|| {
        let bid = focused_buffer_id(state, view);
        let primary = current_selections(state, view).primary();
        (primary, doc(state, view).text().char_to_line(primary.head()), bid)
    });

    let buf     = focused_buffer_id(state, view);
    let focused = state.focused_pane_id;
    match cmd {
        MappableCommand::Motion { fun, .. } => {
            doc_ops::apply_doc_motion(
                &state.buffers, &mut state.panes.state, focused, buf,
                |b, s| fun(b, s, count, motion_mode),
            );
        }
        MappableCommand::Selection { fun, .. } => {
            doc_ops::apply_doc_motion(
                &state.buffers, &mut state.panes.state, focused, buf,
                |b, s| fun(b, s, motion_mode),
            );
        }
        MappableCommand::Edit { fun, .. } => {
            doc_ops::apply_doc_edit(
                &mut state.buffers, &mut state.panes.state, focused, buf, fun,
            );
        }
        MappableCommand::EditorCmd { fun, .. } => {
            // EditorCmd errors are surfaced to the user, NOT propagated to the caller.
            // Intentional: keeps the Steel sync path (`run_command_sync`) identical to
            // the keypress path, where a failed EditorCmd reports and dispatch completes.
            // Do not "fix" this into a `Result` return — it would be a semantic change.
            if let Err(e) = fun(state, view, count, motion_mode) {
                state.report(Severity::Error, e.message().to_owned());
            }
        }
        MappableCommand::SteelBacked { .. } | MappableCommand::Lazy { .. } => {
            unreachable!("dispatch_native called on non-native command '{name}'");
        }
    }

    // Jump list: record if this was a large enough jump / explicit jump command.
    if let Some((pre_primary, pre_line, pre_bid)) = pre_jump {
        let post_line = doc(state, view)
            .text()
            .char_to_line(current_selections(state, view).primary().head());
        if is_jump || pre_line.abs_diff(post_line) > state.settings.jump_line_threshold {
            state.panes.jumps[state.focused_pane_id]
                .push(JumpEntry::from_pre_motion(pre_primary, pre_line, pre_bid));
        }
    }

    // Dot-repeat: record repeatable commands and maintain the selection recipe.
    //
    // The recipe buffer (`state.selection_recipe`) tracks how the current selection
    // was built: Motion/Selection commands append/reset it; a repeatable edit
    // snapshots it (via mem::take) into `RepeatableAction::selection_recipe` so
    // `.` can re-establish the same extent before replaying the edit.
    //
    // Accumulation rule (variant-gated to exclude undo/redo/mode-changes):
    //   repeatable edit/paste  → snapshot + clear
    //   sel-builder, Extend    → append (grew the selection)
    //   sel-builder, Move, non-collapsed → reset (fresh real selection)
    //   sel-builder, Move, collapsed     → clear (plain navigation)
    //   everything else        → clear
    if is_repeatable {
        let recipe = std::mem::take(&mut state.selection_recipe);
        state.record_repeatable(name.clone(), count, char_arg, recipe);
    } else if is_sel_builder {
        if extend {
            // Grew the existing selection — append this step.
            state.selection_recipe.push(SelectionStep {
                command: name.clone(),
                count,
                char_arg,
                extend: true,
            });
        } else if !current_selections(state, view).primary().is_collapsed() {
            // Move-mode established a real (non-collapsed) selection: start fresh.
            state.selection_recipe.clear();
            state.selection_recipe.push(SelectionStep {
                command: name.clone(),
                count,
                char_arg,
                extend: false,
            });
        } else {
            // Plain navigation — collapsed cursor, nothing to rebuild.
            state.selection_recipe.clear();
        }
    } else {
        // Non-repeatable EditorCmd (undo, redo, mode-changes, …): not a
        // selection-builder; any in-progress recipe is now stale.
        state.selection_recipe.clear();
    }

    // Smart-p: classify provenance and update last_command via record_command.
    // InsertTail preserves the prior kill marker; User stamps the new name.
    let prov = if pre_mode == Mode::Insert {
        Provenance::InsertTail
    } else {
        Provenance::User(name)
    };
    state.record_command(prov);
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
    &state.panes.state[state.focused_pane_id][bid].selections
}

/// The most-recently-focused buffer other than the current one.
pub(super) fn alternate_buffer(state: &EditorState, view: &EngineView) -> Option<BufferId> {
    state.buffers.mru_excluding(focused_buffer_id(state, view))
}

/// `true` when the focused (pane, buffer) has an open edit group.
fn is_group_open_current(state: &EditorState, view: &EngineView) -> bool {
    let bid = focused_buffer_id(state, view);
    state.panes.state[state.focused_pane_id][bid].edit_group.is_some()
}

/// Open a new edit group on the focused (pane, buffer) pair.
pub(super) fn begin_edit_group_current(state: &mut EditorState, view: &EngineView) {
    let pid = state.focused_pane_id;
    let bid = focused_buffer_id(state, view);
    doc_ops::begin_edit_group(&state.buffers, &mut state.panes.state, pid, bid);
}

/// Commit and close the open edit group on the focused (pane, buffer) pair.
pub(super) fn commit_edit_group_current(state: &mut EditorState, view: &EngineView) {
    let pid = state.focused_pane_id;
    let bid = focused_buffer_id(state, view);
    doc_ops::commit_edit_group(&mut state.buffers, &mut state.panes.state, pid, bid);
}

/// Active search pattern on the focused buffer, if any.
pub(super) fn search_pattern<'a>(state: &'a EditorState, view: &EngineView) -> Option<&'a SearchPattern> {
    state.buffers.get(focused_buffer_id(state, view)).search_pattern.as_ref()
}

/// Viewport state of the focused pane.
pub(super) fn viewport<'a>(state: &EditorState, view: &'a EngineView) -> &'a hume_engine::pane::ViewportState {
    &view.panes[state.focused_pane_id].viewport
}

/// Resolved `(wrap_mode, tab_width, whitespace)` for the focused doc and pane.
pub(super) fn focused_format_context(
    state: &EditorState,
    view: &EngineView,
) -> (hume_engine::pane::WrapMode, u8, hume_engine::pane::WhitespaceConfig) {
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
    let sels = state.panes.state[pid][bid].selections.clone();
    JumpEntry::new(sels, state.buffers.get(bid).text(), bid)
}

/// Redirect the focused pane to `target` without recording a jump.
pub(super) fn switch_to_buffer_without_jump(
    state: &mut EditorState,
    view: &mut EngineView,
    target: BufferId,
) {
    let pid = state.focused_pane_id;
    super::ops::switch_pane_to_buffer(view, &state.buffers, &mut state.panes.state, pid, target);
}

/// Replace the focused pane's selections for the current buffer.
pub(super) fn set_current_selections(state: &mut EditorState, view: &EngineView, sels: SelectionSet) {
    let bid = focused_buffer_id(state, view);
    state.panes.state[state.focused_pane_id][bid].selections = sels;
}

/// Replace the primary selection in the focused pane (merging overlaps).
pub(super) fn set_primary_selection(state: &mut EditorState, view: &EngineView, new_sel: hume_editing::selection::Selection) {
    let pid = state.focused_pane_id;
    let bid = focused_buffer_id(state, view);
    let idx = state.panes.state[pid][bid].selections.primary_index();
    let sels = std::mem::take(&mut state.panes.state[pid][bid].selections);
    state.panes.state[pid][bid].selections = sels.replace(idx, new_sel).merge_overlapping();
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
    // Guard is load-bearing for dot-repeat replay: `drain_pending_repeat` opens
    // an edit group before re-dispatching the command, so a group already being
    // open here means "we are replaying" → skip session creation and re-type from
    // `insert_keys` instead of recording fresh. Do NOT weaken this into a separate
    // flag without also fixing the replay signal.
    //
    // The implied assumption — that no Steel body can reach `begin_insert_session`
    // with a group already open outside of replay — holds because Steel has no
    // transaction / begin-edit-group builtin, and none should ever be added:
    // fine-grained undo grouping belongs to native commands, not scripts.
    if !is_group_open_current(state, view) {
        begin_edit_group_current(state, view);
        state.insert_session = Some(InsertSession {
            keystrokes: Vec::new(),
            step_back_on_exit: false,
        });
    }
    state.set_mode(Mode::Insert);
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
            &mut state.panes.state,
            focused,
            buf,
            |b, sels| {
                sels.map(|sel| {
                    let head = sel.head();
                    let line_start = b.line_to_char(b.char_to_line(head));
                    let new_head = if head > line_start {
                        hume_editing::grapheme::prev_grapheme_boundary(b, head)
                    } else {
                        head
                    };
                    hume_editing::selection::Selection::collapsed(new_head)
                })
            },
        );
    }
    state.set_mode(Mode::Normal);
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
