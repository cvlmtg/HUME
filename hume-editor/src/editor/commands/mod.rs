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

/// Display label used when the compiled-in fallback theme is active.
pub(in crate::editor::commands) const FALLBACK_THEME_LABEL: &str = "fallback (built-in)";

use hume_editing::selection::SelectionSet;
use hume_editing::tab_style::TabStyle;
use hume_engine::display_lines::line_store::FormatKey;
use hume_engine::display_lines::{DisplayColTarget, DisplayLineMap};
use hume_engine::pane::{Pane, Viewport};
use hume_engine::pipeline::{BufferId, ContentWidth, EngineView, PaneId};

use super::buffer::{Buffer, TextRefusal};
use super::doc_ops;
use super::jump_list::{JumpRule, with_jump};
use super::pipe_run::{PipeRunning, STOP_KEY_NAME};
use super::register_ops;
use super::{EditorState, Severity};
use crate::editor::error::CommandError;
use crate::editor::settings::EditorSettings;
use hume_ops::LineEnd;
use hume_ops::register::Piece;

// ── EditorState helpers ───────────────────────────────────────────────────────

impl EditorState {
    /// Consume the pending `"<reg>` prefix and return the explicit register name,
    /// or `None` if no prefix was typed (bare default case).
    pub(in crate::editor::commands) fn take_register_prefix(&mut self) -> Option<char> {
        self.register_prefix.take()
    }

    /// Write `values` into `name`, routing `'c'` through the OS clipboard.
    pub(super) fn write_register(&mut self, name: char, values: Vec<Piece>) {
        if let Some(w) =
            register_ops::write_register(&mut self.registers, &mut self.clipboard, name, values)
        {
            self.report(Severity::Warning, w);
        }
    }

    /// Route a kill (`d`/`c`) yank: bare default and `"k` both go to the kill
    /// ring; any other explicit register prefix routes through `write_register`.
    /// The prefix is consumed even when nothing was removed, and an all-empty
    /// yank writes nowhere. Returns `true` when the yank was captured to the
    /// ring (and stamped), `false` for an explicit-register route or an empty
    /// yank, which never stamp.
    pub(in crate::editor::commands) fn route_kill(&mut self, yanked: Vec<Piece>) -> bool {
        let prefix = self.take_register_prefix();
        if yanked.iter().all(Piece::is_empty) {
            return false;
        }
        match prefix {
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
    f: impl FnOnce(hume_editing::state::EditState) -> hume_editing::state::EditState,
) {
    let buf = t.bid(view);
    doc_ops::apply_doc_motion(&state.buffers, &mut state.panes.entries, t.pid(), buf, f);
}

/// Apply an edit to `t`'s (pane, buffer) pair.
///
/// Thin wrapper around [`doc_ops::apply_doc_edit`]; see [`apply_pane_motion`].
pub(in crate::editor::commands) fn apply_pane_edit(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
    cmd: impl FnOnce(hume_editing::state::EditState) -> hume_editing::edit::Edited,
) -> Result<(), CommandError> {
    let buf = t.bid(view);
    let (buffers, mut stores) = state.edit_parts();
    doc_ops::apply_doc_edit(buffers, &mut stores, t.pid(), buf, cmd)
}

/// Apply a grouped edit (inside an open insert/paste session) to the focused
/// (pane, buffer) pair. Takes [`FocusedPane`], not [`CommandPane`]: an edit
/// group only ever exists on the pane the user is looking at (see
/// `pane_state.rs`'s module doc), so a `Pane`-category body (which may
/// target a pane other than focus) has no business opening one.
///
/// Thin wrapper around [`doc_ops::apply_doc_edit_grouped`]; see
/// [`apply_pane_motion`]. `pub(in crate::editor)`, not `pub(in crate::editor::
/// commands)`: callers outside this module (`replay`, Insert-mode paste)
/// would otherwise spell out `doc_ops::apply_doc_edit_grouped`'s own 8
/// arguments, the duplication this wrapper exists to avoid.
pub(in crate::editor) fn try_apply_focused_edit_grouped(
    state: &mut EditorState,
    view: &EngineView,
    fp: FocusedPane,
    cmd: impl FnOnce(hume_editing::state::EditState) -> hume_editing::edit::Edited,
) -> Result<(), CommandError> {
    let buf = fp.bid(view);
    let (buffers, mut stores) = state.edit_parts();
    doc_ops::apply_doc_edit_grouped(buffers, &mut stores, fp.pid(), buf, cmd).map(drop)
}

/// [`try_apply_focused_edit_grouped`], reporting a refusal.
pub(in crate::editor) fn apply_focused_edit_grouped(
    state: &mut EditorState,
    view: &EngineView,
    fp: FocusedPane,
    cmd: impl FnOnce(hume_editing::state::EditState) -> hume_editing::edit::Edited,
) {
    if let Err(e) = try_apply_focused_edit_grouped(state, view, fp, cmd) {
        state.report_error(&e);
    }
}

/// `Err` for an edit-mode command on a read-only buffer, so the caller bails
/// out with `?` and the dispatch funnel reports the refusal.
///
/// Clearing `register_prefix` is part of the refusal: for `d`/`c`/`p`, the
/// command consumed the `"<reg>` keystrokes, so leaving the prefix armed
/// would redirect the *next* yank/kill into that register. (Insert session
/// entry clears the prefix itself before ever reaching here, for a
/// different reason (see `begin_insert_session`), so this is a no-op on
/// that path, not a second clear of the same kind.)
pub(in crate::editor::commands) fn ensure_writable(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
) -> Result<(), CommandError> {
    let buffer = doc(state, view, t);
    match buffer.text_refusal() {
        Some(refusal) => {
            let error = doc_ops::refusal_error(refusal, buffer);
            Err(refuse_text_change(state, error))
        }
        None => Ok(()),
    }
}

/// The refusal [`ensure_writable`] returns for `t`'s read-only buffer, for a
/// caller that learned the buffer is read-only from somewhere else
/// (`history_step`, from the walk's own result).
pub(in crate::editor::commands) fn read_only_refusal(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
) -> CommandError {
    let error = doc_ops::refusal_error(TextRefusal::ReadOnly, doc(state, view, t));
    refuse_text_change(state, error)
}

/// The refusal of a close that would stop `running`, naming `way_out`, the
/// typed command that goes ahead. The stop key is offered too when the
/// run's buffer is in the focused pane, the only buffer it stops, and the
/// key still runs `cancel-pipe` there.
pub(in crate::editor::commands) fn pipe_running_refusal(
    state: &EditorState,
    view: &EngineView,
    PipeRunning(bid): PipeRunning,
    way_out: &str,
) -> CommandError {
    let busy = TextRefusal::Busy.message(state.buffers.get(bid));
    let stops = FocusedPane::current(view).bid(view) == bid && state.stop_key_cancels_pipe(bid);
    CommandError::transient(if stops {
        format!("{busy}. {STOP_KEY_NAME} stops it, or use {way_out}")
    } else {
        format!("{busy}. Use {way_out}")
    })
}

/// `error`, with the register prefix the refused command consumed cleared.
fn refuse_text_change(state: &mut EditorState, error: CommandError) -> CommandError {
    state.register_prefix = None;
    error
}

/// `t`'s pane's selections for `t`'s buffer.
pub(super) fn pane_selections<'a>(
    state: &'a EditorState,
    view: &EngineView,
    t: CommandPane,
) -> &'a SelectionSet {
    t.state(&state.panes.entries, view).selections()
}

/// `t`'s selections bound to its buffer's text: the read side of
/// [`pane_selections`].
pub(in crate::editor) fn pane_view<'a>(
    state: &'a EditorState,
    view: &EngineView,
    t: CommandPane,
) -> hume_editing::selection::EditView<'a> {
    let text = state.buffers.get(t.bid(view)).text();
    t.state(&state.panes.entries, view).view(text)
}

/// `doc`'s effective `indent-style`/`indent-width` pair: buffer override → global
/// default for each. The one place a caller needing *both* settings together
/// (Tab dispatch, `>`/`<` indent/unindent) reads them, rather than repeating
/// the two `overrides.tab_style(...)`/`overrides.tab_width(...)` calls at
/// each site. A caller needing only `tab_width` (e.g. `cmd_align_selections`,
/// dedent-on-Backspace) has no use for the other half and reads
/// `overrides.tab_width` directly instead.
pub(super) fn tab_format(doc: &Buffer, settings: &EditorSettings) -> (TabStyle, u8) {
    (
        doc.overrides.indent_style(settings),
        doc.overrides.indent_width(settings),
    )
}

/// `doc`'s effective `word-chars`: buffer override → global default, as
/// `BufferOverrides::word_chars`, the one resolver, applies it. Every
/// word-family dispatch site reads through this or [`word_chars_owned`].
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

/// Where a motion that stops at the end of a line lands in `pid` viewing
/// `buffer`: on the line break while that pane is inserting into that
/// buffer, on the last character otherwise. Every such motion reads it here.
pub(super) fn line_end_for(state: &EditorState, pid: PaneId, buffer: BufferId) -> LineEnd {
    if state.input.inserting_into(pid, buffer) {
        LineEnd::LineBreak
    } else {
        LineEnd::LastCluster
    }
}

/// How a sticky column resolves on the line a vertical move or a scroll
/// carries a cursor to in `pid` viewing `buffer`: [`line_end_for`]'s landing
/// as a display-column policy.
pub(super) fn sticky_landing(
    state: &EditorState,
    pid: PaneId,
    buffer: BufferId,
) -> DisplayColTarget {
    match line_end_for(state, pid, buffer) {
        LineEnd::LastCluster => DisplayColTarget::NearestContent,
        LineEnd::LineBreak => DisplayColTarget::NearestInsertionPoint,
    }
}

/// `pane`'s effective wrap mode for the buffer it currently views: pane
/// override → buffer override → global default. `Pane::wrap().mode` is
/// `Some` only once `:wrap` or `:set pane wrap-mode=…` has pinned this pane
/// for this buffer; until then it inherits whatever the buffer (or, failing
/// that, the global) resolves to: the single place this three-way
/// precedence is applied. Folded into [`FormatKey::wrap_mode`](hume_engine::display_lines::line_store::FormatKey::wrap_mode) by
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
/// `key` and `content_width` are resolved by the caller *before* this call
/// takes `pane` mutably, from [`EditorState::layout_key`](super::EditorState::layout_key),
/// or from `format_key` and `pane_geometry` for a caller that also needs the
/// gutter width: everything they
/// need lives on `pane` itself, but a `&Pane` used to build them cannot
/// coexist with the `&mut Pane` this function requires. A caller with no
/// viewport to write (the movement consumers, which read the display-line
/// list and rewrite selections instead) destructures `(mut dlm, _)`.
pub(super) fn pane_display_lines<'a>(
    doc: &'a Buffer,
    pane: &'a mut Pane,
    key: FormatKey,
    content_width: ContentWidth,
) -> (DisplayLineMap<'a>, &'a mut Viewport) {
    let Pane {
        providers,
        line_store,
        viewport,
        ..
    } = pane;
    let dlm = DisplayLineMap::new(doc.text().rope(), providers, content_width, key, line_store);
    (dlm, viewport)
}

/// Move `t`'s pane to `(bid, char_pos)`, recording a jump entry (only if it
/// actually lands somewhere else) and centering the viewport by display
/// line, the same way `zz` does. Shared tail of every navigation that jumps
/// to an already-resolved buffer/position pair: `goto_location`
/// (`lsp/edits.rs`) and `:e`'s own `path:line[:col]` suffix
/// (`typed_buffer::typed_edit`).
///
/// Caller contract: `t` must still be showing its pre-jump buffer when this
/// is called. This is the only switch to `bid`; a caller that already
/// switched (`enter_buffer`, `switch_to_buffer_with_jump`) would make the
/// jump entry this function records snapshot `bid`'s own position instead of
/// the buffer the jump actually started from.
pub(super) fn jump_pane_to(
    state: &mut EditorState,
    view: &mut EngineView,
    t: CommandPane,
    bid: hume_engine::pipeline::BufferId,
    pos: hume_rope::cluster::ClusterStart,
) {
    let pid = t.pid();
    with_jump(state, view, t, JumpRule::IfMoved, |state, view| {
        crate::editor::focus::switch_pane_to_buffer(state, view, pid, bid);
        crate::editor::pane_state::write_cursor(
            &mut state.panes.entries,
            &state.buffers,
            &view.panes,
            pid,
            bid,
            pos,
        );
    });
    view_center(state, view, pid);
}

/// Replace `t`'s pane's selections for `t`'s buffer.
pub(super) fn set_pane_selections(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
    sels: SelectionSet,
) {
    let text = state.buffers.get(t.bid(view)).text();
    t.state_mut(&mut state.panes.entries, view)
        .set_selections(sels, text);
}

/// Replace the primary selection in `t`'s pane (merging overlaps).
pub(super) fn set_primary_selection(
    state: &mut EditorState,
    view: &EngineView,
    t: CommandPane,
    new_sel: hume_editing::selection::Selection,
) {
    let text = state.buffers.get(t.bid(view)).text();
    let pbs = t.state_mut(&mut state.panes.entries, view);
    let replaced = pbs.state(text).replace_primary(new_sel);
    pbs.store(replaced);
}

mod collect;
mod comment;
mod edit;
mod find;
mod insert_keys;
mod insert_session;
mod jump;
mod mode;
mod pane;
mod paste;
mod pipe;
mod pipeline;
mod scroll;
mod search;
mod smart_indent;
mod structural;
mod tab;
mod target;
mod typed_buffer;
mod typed_file;
mod typed_misc;

pub(super) use comment::*;
pub(super) use edit::*;
pub(super) use find::*;
pub(super) use insert_keys::*;
use insert_session::*;
pub(super) use jump::*;
pub(super) use mode::*;
pub(super) use paste::*;
pub(super) use scroll::*;
pub(super) use search::*;
use smart_indent::*;
pub(super) use tab::*;
pub(super) use typed_buffer::*;
pub(super) use typed_file::*;
pub(super) use typed_misc::*;

// insert_session.rs's other items are re-exported privately above (visible
// only within `commands` and its descendants: every other `mod` in this
// file, and the registry glob) since nothing outside `commands` calls them.
// collect.rs, pane.rs, pipe.rs, pipeline.rs and target.rs export nothing else
// siblings need, so all five are re-exported explicitly instead of via glob. The items below
// ARE called directly by `dispatch.rs`, `replay/`, `input_stack/`,
// `host_impl/`, `editor/mod.rs`, and the `editor::tests` tree; they need
// `pub(in editor)` breadth.
pub(in crate::editor) use collect::{collect_operand, finish_text_operand, run_collected};
pub(in crate::editor) use insert_session::{
    arm_autoindent, autoindent_owned, end_insert_session, tear_down_insert,
};
pub(in crate::editor) use pane::CloseRefused;
use pane::{SPLIT_TOO_SMALL_MSG, close_focused_pane};
pub(in crate::editor) use pane::{fits_split, split_pane_onto};
pub(in crate::editor) use pipe::{apply_pipe_output, cmd_cancel_pipe, cmd_pipe, is_blank_command};
// `open_pane_in_layout` has no non-test caller outside `pane.rs` itself
// (which reaches it directly too). Only `editor::tests` calls it through
// this path, so the re-export is test-only to avoid an "unused import"
// warning on every non-test build.
#[cfg(test)]
pub(in crate::editor) use pane::open_pane_in_layout;
use pipeline::MissingOperand;
pub(in crate::editor) use pipeline::{
    BoundCommand, FocusedBindError, Missing, NativeBody, PendingDispatch, report_arg_error, run,
    run_body, step_paste_commit,
};
pub(in crate::editor) use target::{CommandPane, FocusedPane, TargetError};

// DisplayLineMap-dependent commands live in visual_move.rs; re-export for the registry glob.
pub(super) use super::visual_move::{
    cmd_copy_selection_on_next_line, cmd_copy_selection_on_prev_line, cmd_visual_move_down,
    cmd_visual_move_up, cmd_visual_select_word_nearest_on_line,
};
