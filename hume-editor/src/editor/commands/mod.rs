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
use hume_editing::selection::{Selection, SelectionSet};

use super::registry::MappableCommand;

use super::{register_ops, Severity};
use super::{EditorState, InsertSession, Mode, RegisterPrefix, RepeatableAction, SelectionStep};
use super::buffer::Buffer;
use super::doc_ops;
use super::jump_list::JumpEntry;
use super::registry::{CmdCategory, CmdMeta, PasteFamily};
use super::search_state::SearchPattern;
use super::CmdCtx;
use crate::ops::MotionMode;

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

// ── Native command body execution ───────────────────────────────────────────

/// Run the body of a native command (Motion/Selection/Edit/EditorCmd) with
/// no dispatch bookkeeping.  Infallible — EditorCmd errors are reported but
/// never propagated.
///
/// Called by both the unified pipeline ([`run_dispatch_pipeline`]) and the
/// dot-repeat replay path ([`Editor::replay_dot`]).
pub(super) fn run_native_body(
    state: &mut EditorState,
    view: &mut EngineView,
    cmd: MappableCommand,
    count: usize,
    extend: bool,
) {
    let motion_mode = if extend { MotionMode::Extend } else { MotionMode::Move };
    let buf = focused_buffer_id(state, view);
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
            if let Err(e) = fun(state, view, count, motion_mode) {
                state.report(Severity::Error, e.message().to_owned());
            }
        }
        MappableCommand::SteelBacked { .. } | MappableCommand::Lazy { .. } => {
            unreachable!("run_native_body called on non-native command");
        }
    }
}

// ── Dispatch step functions ─────────────────────────────────────────────────

// ── Shared steps (used by both native and Steel dispatch paths) ──────────────

/// Commit paste session unless the command is a ring-cycle paste.
pub(super) fn step_paste_commit(state: &mut EditorState, category: &CmdCategory) {
    if !matches!(category, CmdCategory::Paste { family: PasteFamily::RingCycle }) {
        state.commit_paste_session();
    }
}

// ── Native-only pre-body steps ─────────────────────────────────────────────────

/// Capture pre-jump cursor position for jump-list recording.
///
/// Selection commands are excluded: a large text-object selection is a
/// select-then-act staging step, not deliberate navigation, so it must not
/// pollute the jump list on a threshold-exceeding extent. Jump-flagged
/// selections (e.g. `%` select-all, `jump: true`) still record via the
/// `cmd.is_jump()` arm.
pub(super) fn step_capture_pre_jump(
    state: &EditorState,
    view: &EngineView,
    cmd: &MappableCommand,
    meta: &CmdMeta,
) -> Option<(Selection, usize, BufferId)> {
    let is_motion = matches!(meta.category, CmdCategory::Motion { .. });
    (cmd.is_jump() || cmd.is_visual_move() || is_motion).then(|| {
        let bid = focused_buffer_id(state, view);
        let primary = current_selections(state, view).primary();
        let line = doc(state, view).text().char_to_line(primary.head());
        (primary, line, bid)
    })
}

/// Snapshot pending_char before body (commands consume via .take()).
pub(super) fn step_capture_pending_char(state: &EditorState) -> Option<char> {
    state.pending_char
}

/// Snapshot selection recipe before body for dot-repeat recording.
///
/// The snapshot captures the selection extent the user built before the edit,
/// so `.` can re-establish it.  Inner dispatches (Steel `call!`) may overwrite
/// `selection_recipe` during the body; the snapshot is taken before they run.
pub(super) fn step_snapshot_recipe(
    state: &mut EditorState,
    repeatable: bool,
) -> Option<Vec<SelectionStep>> {
    if repeatable {
        Some(std::mem::take(&mut state.selection_recipe))
    } else {
        None
    }
}

// ── AFTER (native steps) ────────────────────────────────────────────────────

/// Stamp last_command after body.  Skip in Insert mode to preserve the
/// prior kill marker for smart-p.
///
/// Called **after** body for native (paste smart-p reads old value), **before**
/// body for Steel (outer name pre-stamped; inner dispatch overrides).
pub(super) fn step_stamp_last_command(
    state: &mut EditorState,
    name: Cow<'static, str>,
    pre_mode: Mode,
) {
    if pre_mode != Mode::Insert {
        state.last_command = Some(name);
    }
}

/// Record jump list entry if the command is a jump or the cursor moved
/// past the threshold.
pub(super) fn step_record_jump(
    state: &mut EditorState,
    view: &EngineView,
    pre_jump: Option<(Selection, usize, BufferId)>,
    is_jump: bool,
) {
    if let Some((pre_primary, pre_line, pre_bid)) = pre_jump {
        let post_line = doc(state, view)
            .text()
            .char_to_line(current_selections(state, view).primary().head());
        if is_jump || pre_line.abs_diff(post_line) > state.settings.jump_line_threshold {
            state.panes.jumps[state.focused_pane_id]
                .push(JumpEntry::from_pre_motion(pre_primary, pre_line, pre_bid));
        }
    }
}

/// Record last_repeatable_action for dot-repeat from the pre-body
/// selection recipe snapshot.
pub(super) fn step_stamp_repeatable(
    state: &mut EditorState,
    meta: &CmdMeta,
    count: usize,
    char_arg: Option<char>,
    pre_recipe: Option<Vec<SelectionStep>>,
) {
    if let Some(recipe) = pre_recipe {
        state.last_repeatable_action = Some(RepeatableAction {
            command: meta.name.clone(),
            count,
            char_arg,
            insert_keys: Vec::new(),
            selection_recipe: recipe,
        });
    }
}

/// Update the selection recipe buffer after a command dispatch.
///
/// Accumulation rule (matching the behavior at `replay_dot`):
///   sel-builder + extend              → append step
///   sel-builder + move + non-collapsed → reset + push
///   sel-builder + move + collapsed     → clear
///   everything else                     → clear
pub(super) fn step_update_recipe(
    state: &mut EditorState,
    view: &EngineView,
    meta: &CmdMeta,
    ctx: &CmdCtx,
    char_arg: Option<char>,
) {
    if meta.category.tracks_selection() {
        let sels = current_selections(state, view);
        if ctx.extend {
            state.selection_recipe.push(SelectionStep {
                command: meta.name.clone(),
                count: ctx.count,
                char_arg,
                extend: true,
            });
        } else if !sels.primary().is_collapsed() {
            state.selection_recipe.clear();
            state.selection_recipe.push(SelectionStep {
                command: meta.name.clone(),
                count: ctx.count,
                char_arg,
                extend: false,
            });
        } else {
            state.selection_recipe.clear();
        }
    } else {
        state.selection_recipe.clear();
    }
}

// ── Native dispatch pipeline (composed from step functions) ────────────────

/// Execute a native command through the full dispatch pipeline.
///
/// Composed from the step functions above.  Both the keypress path
/// (`Editor::dispatch` → native branch) and the Steel sync path
/// (`EditorHostImpl::run_command_sync`) delegate here.
pub(super) fn run_dispatch_pipeline(
    state: &mut EditorState,
    view: &mut EngineView,
    cmd: MappableCommand,
    ctx: CmdCtx,
) {
    let meta = cmd.meta();
    // A command cannot be both repeatable (an edit that modifies the buffer)
    // and a selection-builder (a pure cursor movement). If this ever fires,
    // a new variant broke the invariant that step_stamp_repeatable and
    // step_update_recipe rely on for their unconditional sequencing.
    debug_assert!(
        !(meta.repeatable && meta.category.tracks_selection()),
        "command '{}' is both repeatable and selection-tracking — \
         step_stamp_repeatable and step_update_recipe would both fire",
        meta.name,
    );
    let pre_mode = state.mode();
    let is_jump   = cmd.is_jump();

    // BEFORE
    step_paste_commit(state, &meta.category);
    let pre_jump = step_capture_pre_jump(state, view, &cmd, &meta);
    let char_arg = step_capture_pending_char(state);
    let pre_recipe = step_snapshot_recipe(state, meta.repeatable);

    // BODY — cmd moved in; bools captured above so no clone needed.
    run_native_body(state, view, cmd, ctx.count, ctx.extend);

    // AFTER
    step_stamp_last_command(state, meta.name.clone(), pre_mode);
    step_record_jump(state, view, pre_jump, is_jump);
    step_stamp_repeatable(state, &meta, ctx.count, char_arg, pre_recipe);
    step_update_recipe(state, view, &meta, &ctx, char_arg);
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
    // Guard is load-bearing for dot-repeat replay: `replay_dot` opens
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
