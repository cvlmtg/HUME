//! Editor-level command functions.
//!
//! Each function in this module is a command operating on
//! `&mut EditorState` + `&mut EngineView` (the D7 handler shape) — composite
//! operations involving mode changes, registers, undo groups, or
//! parameterized motions (find/till/replace).
//!
//! They are registered in [`super::registry`] and called via function pointer
//! from `execute_keymap_command`, exactly like the pure `cmd_*` functions in
//! `hume-ops`'s `motion`, `edit`, etc. modules.
//!
//! The `count` parameter is the user's numeric prefix (default 1). Commands
//! that don't use a count accept it and ignore it (`_count`).

/// Display label used when no named theme is active (the compiled-in default).
pub(super) const DEFAULT_THEME_LABEL: &str = "default (built-in)";

use hume_editing::selection::SelectionSet;
use hume_editing::text::Text;
use hume_engine::format::FormatScratch;
use hume_engine::pane::{Pane, ViewportState, WhitespaceConfig};
use hume_engine::pipeline::{BufferId, EngineView};
use hume_engine::rows::RowMap;

use super::buffer::Buffer;
use super::doc_ops;
use super::jump_list::JumpEntry;
use super::register_ops;
use super::register_ops::RegisterPrefix;
use super::search::SearchPattern;
use super::{EditorState, Severity};
use crate::settings::EditorSettings;

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
        if let Some(w) =
            register_ops::write_register(&mut self.registers, &mut self.clipboard, name, values)
        {
            self.report(Severity::Warning, w);
        }
    }

    /// Route a kill (`d`/`c`) yank: bare default and `"k` both go to the kill
    /// ring; any other explicit register prefix routes through `write_register`.
    /// Returns `true` when the yank was captured to the ring (and stamped) —
    /// `false` for an explicit-register route, which never stamps.
    pub(super) fn route_kill(&mut self, yanked: Vec<String>) -> bool {
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

/// Buffer id the focused pane is viewing.
pub(super) fn focused_buffer_id(state: &EditorState, view: &EngineView) -> BufferId {
    view.panes[state.focused_pane_id].buffer_id
}

/// Shared reference to the focused buffer.
pub(super) fn doc<'a>(state: &'a EditorState, view: &EngineView) -> &'a Buffer {
    state.buffers.get(focused_buffer_id(state, view))
}

/// Apply a motion to the focused (pane, buffer) pair.
///
/// Thin wrapper around [`doc_ops::apply_doc_motion`] that resolves the
/// focused pane/buffer so call sites don't repeat that lookup.
pub(super) fn apply_focused_motion(
    state: &mut EditorState,
    view: &EngineView,
    f: impl FnOnce(&Text, SelectionSet) -> SelectionSet,
) {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.state, focused, buf, f);
}

/// Apply an edit to the focused (pane, buffer) pair.
///
/// Thin wrapper around [`doc_ops::apply_doc_edit`]; see [`apply_focused_motion`].
pub(super) fn apply_focused_edit(
    state: &mut EditorState,
    view: &EngineView,
    cmd: impl FnOnce(Text, SelectionSet) -> (Text, SelectionSet, hume_editing::changeset::ChangeSet),
) {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_edit(
        &mut state.buffers,
        &state.config.decorations,
        &mut state.panes.state,
        focused,
        buf,
        cmd,
    );
}

/// Apply a grouped edit (inside an open insert/paste session) to the focused
/// (pane, buffer) pair.
///
/// Thin wrapper around [`doc_ops::apply_doc_edit_grouped`]; see
/// [`apply_focused_motion`].
pub(super) fn apply_focused_edit_grouped(
    state: &mut EditorState,
    view: &EngineView,
    cmd: impl FnOnce(Text, SelectionSet) -> (Text, SelectionSet, hume_editing::changeset::ChangeSet),
) {
    let focused = state.focused_pane_id;
    let buf = focused_buffer_id(state, view);
    doc_ops::apply_doc_edit_grouped(
        &mut state.buffers,
        &state.config.decorations,
        &mut state.panes.state,
        focused,
        buf,
        cmd,
    );
}

/// `true` when the focused buffer is read-only.
pub(super) fn focused_buffer_read_only(state: &EditorState, view: &EngineView) -> bool {
    doc(state, view).is_read_only()
}

/// Focused pane's selections for the current buffer.
pub(super) fn current_selections<'a>(
    state: &'a EditorState,
    view: &EngineView,
) -> &'a SelectionSet {
    let bid = focused_buffer_id(state, view);
    &state.panes.state[state.focused_pane_id][bid].selections
}

/// The most-recently-focused buffer other than the current one.
pub(super) fn alternate_buffer(state: &EditorState, view: &EngineView) -> Option<BufferId> {
    state.buffers.mru_excluding(focused_buffer_id(state, view))
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
pub(super) fn search_pattern<'a>(
    state: &'a EditorState,
    view: &EngineView,
) -> Option<&'a SearchPattern> {
    state
        .buffers
        .get(focused_buffer_id(state, view))
        .search_pattern
        .as_ref()
}

/// Viewport state of the focused pane.
pub(super) fn viewport<'a>(
    state: &EditorState,
    view: &'a EngineView,
) -> &'a hume_engine::pane::ViewportState {
    &view.panes[state.focused_pane_id].viewport
}

/// The doc-level format settings a row map needs, resolving buffer overrides
/// against the global settings. The one place that precedence is applied.
fn format_overrides(doc: &Buffer, settings: &EditorSettings) -> (u8, WhitespaceConfig) {
    (
        doc.overrides.tab_width(settings),
        doc.overrides.whitespace(settings),
    )
}

/// A [`RowMap`] over `pane`'s view of `doc` — the display-row list every
/// scroll, cursor and movement consumer reads instead of walking rows itself.
///
/// Borrows come in already split so a caller can keep a `&mut` on a disjoint
/// field while holding the map: `visual_move` rewrites `state.panes`
/// selections, and [`pane_row_map_mut`] hands back the pane's viewport.
pub(super) fn pane_row_map<'a>(
    doc: &'a Buffer,
    settings: &'a EditorSettings,
    pane: &'a Pane,
    scratch: &'a mut FormatScratch,
) -> RowMap<'a> {
    let (tab_width, whitespace) = format_overrides(doc, settings);
    RowMap::new(
        doc.text().rope(),
        pane.wrap_mode,
        tab_width,
        whitespace,
        &pane.providers,
        pane.content_width(doc.text().len_lines()),
        scratch,
    )
}

/// [`pane_row_map`] plus the pane's viewport, for the scroll consumers that
/// write the viewport while reading the map. The two are disjoint fields of
/// `pane`, which a caller holding only `&mut Pane` cannot split apart itself
/// without also re-deriving the map's inputs.
pub(super) fn pane_row_map_mut<'a>(
    doc: &'a Buffer,
    settings: &'a EditorSettings,
    pane: &'a mut Pane,
    scratch: &'a mut FormatScratch,
) -> (RowMap<'a>, &'a mut ViewportState) {
    let (tab_width, whitespace) = format_overrides(doc, settings);
    // Both need the whole pane, so they are read before it is split.
    let wrap_mode = pane.wrap_mode;
    let content_width = pane.content_width(doc.text().len_lines());
    let rm = RowMap::new(
        doc.text().rope(),
        wrap_mode,
        tab_width,
        whitespace,
        &pane.providers,
        content_width,
        scratch,
    );
    (rm, &mut pane.viewport)
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
    super::buffer::lifecycle::switch_pane_to_buffer(
        view,
        &state.buffers,
        &mut state.panes.state,
        pid,
        target,
    );
}

/// Replace the focused pane's selections for the current buffer.
pub(super) fn set_current_selections(
    state: &mut EditorState,
    view: &EngineView,
    sels: SelectionSet,
) {
    let bid = focused_buffer_id(state, view);
    state.panes.state[state.focused_pane_id][bid].selections = sels;
}

/// Replace the primary selection in the focused pane (merging overlaps).
pub(super) fn set_primary_selection(
    state: &mut EditorState,
    view: &EngineView,
    new_sel: hume_editing::selection::Selection,
) {
    let pid = state.focused_pane_id;
    let bid = focused_buffer_id(state, view);
    let idx = state.panes.state[pid][bid].selections.primary_index();
    let sels = std::mem::take(&mut state.panes.state[pid][bid].selections);
    state.panes.state[pid][bid].selections = sels.replace(idx, new_sel);
}

mod edit;
mod find;
mod insert_session;
mod jump;
mod mode;
mod pane;
mod paste;
mod pipeline;
mod scroll;
mod search;
mod typed_buffer;
mod typed_file;
mod typed_misc;

pub(super) use edit::*;
pub(super) use find::*;
use insert_session::*;
pub(super) use jump::*;
pub(super) use mode::*;
pub(super) use paste::*;
pub(super) use scroll::*;
pub(super) use search::*;
pub(super) use typed_buffer::*;
pub(super) use typed_file::*;
pub(super) use typed_misc::*;

// insert_session.rs's remaining items (begin_insert_session, pin_insert_anchors,
// is_group_open_current, has_blank_line_cursor) are re-exported privately above
// (visible only within `commands` and its descendants — every other `mod` in
// this file, and the registry glob) since nothing outside `commands` calls
// them. pane.rs and pipeline.rs export nothing else siblings need, so both are
// re-exported explicitly instead of via glob. The items below ARE called
// directly by `dispatch.rs`, `replay.rs`, `mappings/insert.rs`, `host_impl.rs`,
// `editor/mod.rs`, and the `editor::tests` tree — they need `pub(in editor)`
// breadth.
pub(in crate::editor) use insert_session::end_insert_session;
use pane::{SPLIT_TOO_SMALL_MSG, close_focused_pane};
pub(in crate::editor) use pane::{fits_split, split_pane_onto};
// `open_pane` has no non-test caller outside `pane.rs` itself (which reaches
// it directly, not through this re-export) — only `editor::tests` seeds
// panes through it, so the re-export is test-only to avoid an "unused
// import" warning on every non-test build.
#[cfg(test)]
pub(in crate::editor) use pane::open_pane;
pub(in crate::editor) use pipeline::{
    run_dispatch_pipeline, run_native_body, step_paste_commit, step_stamp_repeatable,
};

// Visual-line commands live in visual_move.rs; re-export for the registry glob.
pub(super) use super::visual_move::{
    cmd_visual_move_down, cmd_visual_move_up, cmd_visual_select_word_nearest_on_line,
};
