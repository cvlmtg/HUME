//! Editor-level command functions.
//!
//! Each function in this module is a command operating on
//! `&mut EditorState` + `&mut EngineView` (the native `EditorCmd` shape, see
//! `registry/command.rs`'s `EditorCmdBody`; never `&mut Editor`): composite operations
//! involving mode changes, registers, undo groups, or parameterized motions
//! (find/till/replace).
//!
//! They are registered in [`super::registry`] and called via function pointer
//! from `execute_keymap_command`, exactly like the pure `cmd_*` functions in
//! `hume-ops`'s `motion`, `edit`, etc. modules.
//!
//! The `count` parameter is the user's numeric prefix (default 1). Commands
//! that don't use a count accept it and ignore it (`_count`).

/// Display label used when no named theme is active (the compiled-in default).
pub(in crate::editor::commands) const DEFAULT_THEME_LABEL: &str = "default (built-in)";

use hume_editing::selection::SelectionSet;
use hume_editing::tab_style::TabStyle;
use hume_editing::text::BufferText;
use hume_engine::display_lines::DisplayLineMap;
use hume_engine::display_lines::line_store::FormatKey;
use hume_engine::pane::{Pane, Viewport};
use hume_engine::pipeline::{EngineView, PaneId};

use super::buffer::Buffer;
use super::doc_ops;
use super::jump_list::JumpEntry;
use super::register_ops;
use super::register_ops::RegisterPrefix;
use super::search::SearchPattern;
use super::{EditorState, Severity};
use crate::editor::error::CommandError;
use crate::editor::settings::EditorSettings;

// ── EditorState helpers ───────────────────────────────────────────────────────

impl EditorState {
    /// Consume the pending `"<reg>` prefix and return the explicit register name,
    /// or `None` if no prefix was typed (bare default case).
    pub(in crate::editor::commands) fn take_register_prefix(&mut self) -> Option<char> {
        match self.register_prefix.take() {
            Some(RegisterPrefix::Selected(c)) => Some(c),
            _ => None,
        }
    }

    /// Write `values` into `name`, routing `'c'` through the OS clipboard.
    pub(super) fn write_register(&mut self, name: char, values: Vec<String>) {
        if let Some(w) =
            register_ops::write_register(&mut self.registers, &mut self.clipboard, name, values)
        {
            self.report(Severity::Warning, w);
        }
    }

    /// Route a kill (`d`/`c`) yank: bare default and `"k` both go to the kill
    /// ring; any other explicit register prefix routes through `write_register`.
    /// Returns `true` when the yank was captured to the ring (and stamped),
    /// `false` for an explicit-register route, which never stamps.
    pub(in crate::editor::commands) fn route_kill(&mut self, yanked: Vec<String>) -> bool {
        match self.take_register_prefix() {
            None | Some(hume_ops::register::KILL_RING_REGISTER) => {
                self.capture_to_ring(yanked);
                true
            }
            Some(reg) => {
                self.write_register(reg, yanked);
                false
            }
        }
    }
}

// ── Free helpers for EditorCmd handlers ──────────────────────────────────────

/// Reference to `t`'s buffer.
pub(super) fn doc<'a>(state: &'a EditorState, view: &EngineView, t: CommandPane) -> &'a Buffer {
    state.buffers.get(t.bid(view))
}

/// Apply a motion to `t`'s (pane, buffer) pair.
///
/// Thin wrapper around [`doc_ops::apply_doc_motion`] that resolves `t`'s
/// buffer so call sites don't repeat that lookup.
pub(super) fn apply_pane_motion(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
    f: impl FnOnce(&BufferText, SelectionSet) -> SelectionSet,
) {
    let buf = t.bid(view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, t.pid(), buf, f);
}

/// Apply an edit to `t`'s (pane, buffer) pair.
///
/// Thin wrapper around [`doc_ops::apply_doc_edit`]; see [`apply_pane_motion`].
pub(in crate::editor::commands) fn apply_pane_edit(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
    cmd: impl FnOnce(
        BufferText,
        SelectionSet,
    ) -> (BufferText, SelectionSet, hume_editing::changeset::ChangeSet),
) -> Result<(), CommandError> {
    let buf = t.bid(view);
    doc_ops::apply_doc_edit(
        &mut state.buffers,
        &state.config.decorations,
        &mut state.panes.state,
        &mut state.panes.jumps,
        &mut state.active_session,
        t.pid(),
        buf,
        cmd,
    )
}

/// Apply a grouped edit (inside an open insert/paste session) to the focused
/// (pane, buffer) pair. Takes [`FocusedPane`], not [`CommandPane`]: an edit
/// group only ever exists on the pane the user is looking at (see
/// `pane_state.rs`'s module doc), so a `Pane`-category body (which may
/// target a pane other than focus) has no business opening one.
///
/// Thin wrapper around [`doc_ops::apply_doc_edit_grouped`]; see
/// [`apply_pane_motion`]. `pub(in crate::editor)`, not `pub(in crate::editor::
/// commands)`: `replay::apply_cursor_replacement` is a caller outside this
/// module, and the alternative (spelling out `doc_ops::apply_doc_edit_
/// grouped`'s own 8 arguments there instead) is exactly the duplication
/// this wrapper exists to avoid.
pub(in crate::editor) fn apply_focused_edit_grouped(
    state: &mut EditorState,
    view: &EngineView,
    fp: FocusedPane,
    cmd: impl FnOnce(
        BufferText,
        SelectionSet,
    ) -> (BufferText, SelectionSet, hume_editing::changeset::ChangeSet),
) -> hume_editing::changeset::ChangeSet {
    let buf = fp.bid(view);
    doc_ops::apply_doc_edit_grouped(
        &mut state.buffers,
        &state.config.decorations,
        &mut state.panes.state,
        &mut state.panes.jumps,
        &mut state.active_session,
        fp.pid(),
        buf,
        cmd,
    )
}

/// Refuse an edit-mode command on a read-only buffer: report why and return
/// `true` so the caller can bail out.
///
/// Clearing `register_prefix` is part of the refusal: for `d`/`c`/`p`, the
/// command consumed the `"<reg>` keystrokes, so leaving the prefix armed
/// would silently redirect the *next* yank/kill into that register. (Insert
/// session entry clears the prefix itself before ever reaching here, for a
/// different reason; see `begin_insert_session`), so this is a no-op on
/// that path, not a second clear of the same kind.)
pub(in crate::editor::commands) fn refuse_if_read_only(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
) -> bool {
    if !doc(state, view, t).is_read_only() {
        return false;
    }
    state.register_prefix = None;
    state.command_refused = true;
    state.report(Severity::Info, "Buffer is read-only".to_string());
    true
}

/// `t`'s pane's selections for `t`'s buffer.
pub(super) fn pane_selections<'a>(
    state: &'a EditorState,
    view: &EngineView,
    t: CommandPane,
) -> &'a SelectionSet {
    t.state(&state.panes.state, view).selections()
}

/// Active search pattern on `t`'s buffer, if any.
pub(super) fn search_pattern<'a>(
    state: &'a EditorState,
    view: &EngineView,
    t: CommandPane,
) -> Option<&'a SearchPattern> {
    state.buffers.get(t.bid(view)).search_pattern.as_ref()
}

/// Viewport state of pane `pid`.
pub(super) fn viewport(view: &EngineView, pid: PaneId) -> &hume_engine::pane::Viewport {
    &view.panes[pid].viewport
}

/// `doc`'s effective `tab-style`/`tab-width` pair: buffer override → global
/// default for each. The one place a caller needing *both* settings together
/// (Tab dispatch, `>`/`<` indent/unindent) reads them, rather than repeating
/// the two `overrides.tab_style(...)`/`overrides.tab_width(...)` calls at
/// each site. A caller needing only `tab_width` (e.g. `cmd_align_selections`,
/// dedent-on-Backspace) has no use for the other half and reads
/// `overrides.tab_width` directly instead.
pub(super) fn tab_format(doc: &Buffer, settings: &EditorSettings) -> (TabStyle, u8) {
    (
        doc.overrides.tab_style(settings),
        doc.overrides.tab_width(settings),
    )
}

/// `doc`'s effective `word-chars`: buffer override → global default. The one
/// place this precedence is applied: every word-family dispatch site reads
/// through this or [`word_chars_owned`] instead of re-resolving the setting
/// by hand.
pub(super) fn effective_word_chars<'a>(
    doc: &'a Buffer,
    settings: &'a EditorSettings,
) -> hume_editing::word::WordChars<'a> {
    hume_editing::word::WordChars::new(doc.overrides.word_chars(settings))
}

/// [`effective_word_chars`], owned rather than borrowed. For a caller that
/// holds `&mut EditorState` across the closure that consumes the result
/// (`apply_focused_edit`/`apply_doc_edit_grouped` and friends all take it by
/// value): a borrow into `state.buffers`/`state.settings` can't survive
/// that call.
pub(super) fn word_chars_owned(doc: &Buffer, settings: &EditorSettings) -> String {
    doc.overrides.word_chars(settings).to_owned()
}

/// `pane`'s effective wrap mode for the buffer it currently views: pane
/// override → buffer override → global default. `Pane::wrap().mode` is
/// `Some` only once `:wrap` or `:set pane wrap-mode=…` has pinned this pane
/// for this buffer; until then it inherits whatever the buffer (or, failing
/// that, the global) resolves to: the single place this three-way
/// precedence is applied. Folded into [`FormatKey::wrap_mode`] by
/// [`EditorState::format_key`](super::EditorState::format_key) alongside
/// `tab_width`/`whitespace`, which only ever have two levels.
pub(super) fn effective_wrap_mode(
    doc: &Buffer,
    settings: &EditorSettings,
    pane: &Pane,
) -> hume_engine::pane::WrapMode {
    pane.wrap()
        .mode
        .unwrap_or_else(|| doc.overrides.wrap_mode(settings))
}

/// A [`DisplayLineMap`] over `pane`'s view of `doc` (the display-line list every
/// scroll, cursor and movement consumer reads instead of walking display
/// lines itself), together with the pane's viewport, for the scroll
/// consumers that write it while reading the map.
///
/// Takes the pane mutably and splits it here: `providers`, `line_store` and
/// `viewport` are disjoint fields, which only a function holding the whole
/// pane can say, and which a caller holding just `&mut Pane` could not split
/// apart itself without also re-deriving the map's inputs. Callers keep their
/// own `&mut` on `state` fields meanwhile (`visual_move` rewrites
/// `state.panes` selections), which is why the pane arrives separately rather
/// than through `&mut Editor`.
///
/// `key` is resolved by the caller ([`EditorState::format_key`](super::EditorState::format_key))
/// *before* this call takes `pane` mutably: everything `key` needs lives on
/// `pane` itself, but a `&Pane` used to build it cannot coexist with the
/// `&mut Pane` this function requires. A caller with no viewport to write
/// (the movement consumers, which read the display-line list and rewrite
/// selections instead) destructures `(mut dlm, _)`.
pub(super) fn pane_display_lines<'a>(
    doc: &'a Buffer,
    pane: &'a mut Pane,
    key: FormatKey,
) -> (DisplayLineMap<'a>, &'a mut Viewport) {
    let content_width = pane.content_width(doc.text().last_ropey_line());
    let Pane {
        providers,
        line_store,
        viewport,
        ..
    } = pane;
    let dlm = DisplayLineMap::new(doc.text().rope(), providers, content_width, key, line_store);
    (dlm, viewport)
}

/// Snapshot `t`'s pane's current cursor as a `JumpEntry`.
pub(super) fn current_jump_entry(
    state: &EditorState,
    view: &EngineView,
    t: CommandPane,
) -> JumpEntry {
    let bid = t.bid(view);
    let sels = t.state(&state.panes.state, view).selections().clone();
    JumpEntry::new(sels, state.buffers.get(bid).text(), bid)
}

/// Push `pre` (a [`current_jump_entry`] snapshot taken before some
/// navigation, however long ago) only if `t`'s buffer or its selections
/// have actually changed since. `JumpList::push` truncates forward history
/// unconditionally, so a caller that pushes unconditionally
/// (`:42` already on line 42, `goto-definition` invoked on the definition
/// itself, a search confirmed on the match already under the cursor) can
/// wipe Ctrl-i history for a keypress that moved nothing. Mirrors the
/// native command pipeline's own `moved` guard (`step_record_jump`) for the
/// callers here that push directly instead of going through `CmdMeta`.
pub(super) fn record_jump_if_moved(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
    pre: JumpEntry,
) {
    let post_bid = t.bid(view);
    if pre.buffer_id != post_bid || pre.selections != *pane_selections(state, view, t) {
        state.panes.jumps[t.pid()].push(pre);
    }
}

/// Replace `t`'s pane's selections for `t`'s buffer.
pub(super) fn set_pane_selections(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
    sels: SelectionSet,
) {
    t.state_mut(&mut state.panes.state, view)
        .set_selections(sels);
}

/// Replace the primary selection in `t`'s pane (merging overlaps).
pub(super) fn set_primary_selection(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
    new_sel: hume_editing::selection::Selection,
) {
    let pbs = t.state_mut(&mut state.panes.state, view);
    let idx = pbs.selections().primary_index();
    let old_head = pbs.selections().primary().head();
    let sels = pbs.take_selections();
    pbs.restore_selections(sels.replace(idx, new_sel), old_head);
}

mod edit;
mod find;
mod insert_keys;
mod insert_session;
mod jump;
mod mode;
mod pane;
mod paste;
mod pipeline;
mod scroll;
mod search;
mod structural;
mod tab;
mod typed_buffer;
mod typed_file;
mod typed_misc;

pub(super) use edit::*;
pub(super) use find::*;
pub(super) use insert_keys::*;
use insert_session::*;
pub(super) use jump::*;
pub(super) use mode::*;
pub(super) use paste::*;
pub(super) use scroll::*;
pub(super) use search::*;
pub(super) use tab::*;
pub(super) use typed_buffer::*;
pub(super) use typed_file::*;
pub(super) use typed_misc::*;

// insert_session.rs's remaining items (begin_insert_session, begin_typed_run,
// ExitCursor, has_blank_line_cursor) are re-exported privately above
// (visible only within `commands` and its descendants: every other `mod` in
// this file, and the registry glob) since nothing outside `commands` calls
// them. pane.rs and pipeline.rs export nothing else siblings need, so both are
// re-exported explicitly instead of via glob. The items below ARE called
// directly by `dispatch.rs`, `replay.rs`, `input_stack/insert.rs`, `host_impl.rs`,
// `editor/mod.rs`, and the `editor::tests` tree; they need `pub(in editor)`
// breadth.
pub(in crate::editor) use insert_session::{
    arm_autoindent, autoindent_owned, end_insert_session, tear_down_insert,
};
use pane::{SPLIT_TOO_SMALL_MSG, close_focused_pane};
pub(in crate::editor) use pane::{fits_split, split_pane_onto};
// `open_pane` itself (the raw, unspliced constructor) is private to
// `pane.rs`, not re-exported here or anywhere. `open_pane_in_layout` and
// `open_pane_as_new_tab` are the only two ways, anywhere in the crate, to
// create a pane. `open_pane_as_new_tab` has exactly one caller
// (`commands::tab::open_tab`, which imports it directly from `pane`, not
// through this re-export) so it isn't re-exported here at all.
// `open_pane_in_layout` has no non-test caller outside `pane.rs` itself
// (which reaches it directly too). Only `editor::tests` calls it through
// this path, so the re-export is test-only to avoid an "unused import"
// warning on every non-test build.
#[cfg(test)]
pub(in crate::editor) use pane::open_pane_in_layout;
pub(in crate::editor) use pipeline::{
    BindError, BoundCommand, CommandPane, FocusedPane, NativeBody, TargetError, repeat_slot_owned,
    run, run_body, step_paste_commit, step_stamp_repeatable,
};

// DisplayLineMap-dependent commands live in visual_move.rs; re-export for the registry glob.
pub(super) use super::visual_move::{
    cmd_copy_selection_on_next_line, cmd_copy_selection_on_prev_line, cmd_visual_move_down,
    cmd_visual_move_up, cmd_visual_select_word_nearest_on_line,
};
