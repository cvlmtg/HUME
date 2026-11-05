//! Free functions for buffer lifecycle operations.
//!
//! Free functions (not `impl Editor` methods) so the same logic can be
//! called by both the `Editor` methods (which take `&mut self`) and the
//! Steel builtins. Both hold a whole `EditorState`/`EngineView` pair (the
//! host through `SteelCtx`), just never the same `Editor` these live on, so
//! they take `state`/`ev` directly instead of `&mut self`.
//!
//! The `impl Editor` choke-points (`enter_buffer`, `open_read_only_view`)
//! are thin delegators; all logic lives here.
//! Production closes call [`close_buffer_and_notify`] directly.

use hume_core::{BufferId, OnUnsaved, PaneId};
use hume_editing::text::BufferText;
use hume_engine::pipeline::EngineView;
use hume_platform::path::AbsolutePath;
use hume_rope::column::GraphemeCol;
use hume_rope::line::ContentLine;

use crate::editor::EditorState;
use crate::editor::buffer::{FileTarget, NewBuffer, Unsaved, ViewLabel};
use crate::editor::commands::CommandPane;
use crate::editor::event::EditorEvent;
use crate::editor::focus::switch_pane_to_buffer;
use crate::editor::pane_state;
use crate::editor::pipe_run::{PipeRefusal, PipeRunning};

// ── open_and_notify / open_or_dedup_and_notify ────────────────────────────────

/// Everything a fresh `BufferId` needs once the settings are live: restore
/// its file's undo history and queue it for language detection. Every fresh
/// `BufferId` must go through this, or `Editor::detect_pending_languages`
/// never announces its `OnBufferOpen` (see that function's doc).
///
/// Does **not** run detection inline: that needs `set_buffer_language`,
/// which can activate lazy language plugins via `self.scripting`, a full
/// `&mut Editor`/Steel-eval capability this function's callers do not hold.
/// The drain runs at the end of the Steel eval that queued `bid`, or else at
/// the next `Editor::settle`. A buffer closed before the drain announced its
/// open announces neither hook (see [`close_buffer_and_notify`]).
///
/// The undo history is restored here rather than at the drain, so a script
/// that opens a buffer and edits it in one eval edits the restored history.
pub(in crate::editor) fn finish_open(state: &mut EditorState, bid: BufferId) {
    state.restore_undo(bid);
    state.config.pending_language_detection.push(bid);
}

/// Open the buffer `new` describes, the chokepoint every open after startup
/// goes through: allocate its slot (engine + BufferStore), seed its
/// `undo-levels` cap from the current global setting, and
/// [`finish_open`] it. See the latter for why detection doesn't run inline.
/// `Editor::detect_pending_languages` fires `OnBufferOpen` once detection
/// (and `OnLanguageSet`) for the buffer has run, so plugins observing both
/// hooks see `OnLanguageSet` first.
///
/// No pane shows the buffer yet; whichever pane first switches to it seeds
/// its own `pane_state` entry lazily, via
/// `switch_pane_to_buffer`/`write_cursor`'s own `pane_state::ensure` calls.
pub(in crate::editor) fn open_and_notify(
    ev: &mut EngineView,
    state: &mut EditorState,
    new: NewBuffer,
) -> BufferId {
    let bid = ev.buffers.insert(());
    state.buffers.open(bid, new);
    state
        .buffers
        .get_mut(bid)
        .set_undo_levels(state.settings.undo_levels);
    finish_open(state, bid);
    bid
}

/// Dedup-open the file `typed` names: if a buffer already has its resolved
/// path, returns `(existing_id, false)`; otherwise reads the file (or opens
/// an empty new-file buffer if it doesn't exist yet; see
/// [`NewBuffer::file_or_new`]) and opens it via [`open_and_notify`], returning
/// `(new_id, true)`. The new buffer's typed path is `typed`, symlinks kept.
/// Dedup-opening an already-open path enqueues no hook and detects no
/// language. The caller is responsible for any other post-open work (pane
/// switching).
pub(in crate::editor) fn open_or_dedup_and_notify(
    ev: &mut EngineView,
    state: &mut EditorState,
    typed: AbsolutePath,
) -> std::io::Result<(BufferId, bool)> {
    let target = FileTarget::new(typed)?;
    if let Some(existing) = state.buffers.find_by_path(target.resolved()) {
        return Ok((existing, false));
    }
    let new = NewBuffer::file_or_new(target)?;
    Ok((open_and_notify(ev, state, new), true))
}

/// Switch pane `pid` to `target`, or do nothing if it already shows it,
/// recording no jump: the entry point that runs it (a key, `:`, the buffer
/// drawer, `switch-to-buffer!`) records the jump around this call.
///
/// External-change detection does not run here: every genuine switch
/// this produces raises `EditorEvent::OnBufferEnter`, observed by
/// `Editor::settle`'s diff regardless of caller, interactive or not. A
/// no-op call raises nothing, matching Vim's `BufEnter`, which doesn't
/// re-fire for re-entering the buffer you're already viewing.
///
/// Accepted cost of that parity: `:e`/`:b` re-targeting the
/// already-focused buffer runs no disk stat at all: it's a
/// no-op, not a deferred one. An external change to that file still
/// surfaces the moment any of terminal `FocusIn`, a genuine buffer-enter
/// (switch away and back), or `:checktime` runs; see
/// `Editor::enter_buffer_disk_check`'s doc for the full list of paths
/// that *do* stat.
pub(in crate::editor) fn enter_buffer(
    state: &mut EditorState,
    ev: &mut EngineView,
    pid: PaneId,
    target: BufferId,
) {
    if ev.panes[pid].buffer_id() != target {
        switch_pane_to_buffer(state, ev, pid, target);
    }
}

/// Show `content` on pane `t` in the read-only view buffer labelled
/// `label`, recording no jump. The first call opens the buffer; a later
/// call replaces its text in place, so the buffer list never holds two
/// views with one label. The cursor lands as [`present_view`] places it.
pub(in crate::editor) fn show_read_only_view(
    state: &mut EditorState,
    ev: &mut EngineView,
    t: CommandPane,
    label: &ViewLabel,
    content: &str,
    cursor_line: Option<ContentLine>,
) -> BufferId {
    let bid = if let Some(existing) = state.buffers.find_by_label(label) {
        // `set_view_content` resets history and every position stored for
        // the buffer: the new content shares only the buffer id with the
        // text it overwrites.
        let (buffers, mut stores) = state.edit_parts();
        buffers
            .get_mut(existing)
            .set_view_content(existing, &mut stores, content);
        forget_saved_views(ev, existing);
        existing
    } else {
        open_view_buffer(state, ev, BufferText::from(content), label)
    };
    present_view(state, ev, t, bid, cursor_line);
    bid
}

/// Open a read-only view buffer labelled `label` holding `text`. No buffer
/// may have that label yet.
pub(in crate::editor) fn open_view_buffer(
    state: &mut EditorState,
    ev: &mut EngineView,
    text: BufferText,
    label: &ViewLabel,
) -> BufferId {
    debug_assert!(
        state.buffers.find_by_label(label).is_none(),
        "open_view_buffer: one buffer per view label"
    );
    let label = label.clone();
    open_and_notify(ev, state, NewBuffer::View { text, label })
}

/// Show view buffer `bid` on pane `t`, recording no jump, with the cursor
/// on `cursor_line` (clamped to the last content line), or on the last
/// content line when `None`.
pub(in crate::editor) fn present_view(
    state: &mut EditorState,
    ev: &mut EngineView,
    t: CommandPane,
    bid: BufferId,
    cursor_line: Option<ContentLine>,
) {
    if t.bid(ev) != bid {
        switch_pane_to_buffer(state, ev, t.pid(), bid);
    }

    let cursor_line =
        cursor_line.unwrap_or_else(|| state.buffers.get(bid).text().last_content_line());
    pane_state::park_cursor_at(
        &mut state.panes.entries,
        &state.buffers,
        &ev.panes,
        t.pid(),
        bid,
        cursor_line,
        GraphemeCol::new(0),
    );
}

/// Remove buffer `id`. Every pane showing it (active tab or not) redirects
/// to the MRU replacement buffer, or, when `id` was the only buffer, to a
/// new empty scratch buffer opened through [`open_and_notify`] like any
/// other open. `id`'s own slot is always freed: a versioned key is never
/// reused for different content, so a captured `id` never comes to name the
/// replacement; a same-slot in-place replace would leave that failure mode
/// open for any pane-taking builtin whose bid outlived the close.
///
/// # Panics
/// Panics when `id` is busy: [`close_buffer_and_notify`] refuses that.
fn close_buffer(state: &mut EditorState, ev: &mut EngineView, id: BufferId) {
    assert!(
        !state.buffers.get(id).is_busy(),
        "close_buffer: a busy buffer's run would stop"
    );
    let next = state
        .buffers
        .mru_excluding(id)
        .unwrap_or_else(|| open_and_notify(ev, state, NewBuffer::Scratch(BufferText::empty())));
    // Collect before mutating (borrow checker); n≈1 in the single-pane case.
    // Every pane showing `id` must redirect, active tab or not. In the
    // last-buffer case this is *every* pane, since `id` was the only buffer
    // any of them could have been showing.
    let panes_to_redirect: Vec<PaneId> = ev
        .panes
        .every_pane_across_all_tabs()
        .filter(|(_, p)| p.buffer_id() == id)
        .map(|(pid, _)| pid)
        .collect();
    for pid in panes_to_redirect {
        switch_pane_to_buffer(state, ev, pid, next);
    }
    state.buffers.close(id);
    ev.buffers.remove(id);
    forget_closed_buffer(state, ev, id);
}

/// Close buffer `id`, the chokepoint every close goes through:
/// [`close_buffer`] plus the pre-close LSP sync and post-close cleanup,
/// which are the `didClose` notification, diagnostics clear, decoration
/// clear, and the `OnBufferClose` hook enqueue.
///
/// Unlike buffer open, none of this needs Steel eval (`didClose` is pure
/// protocol, diagnostics/decorations are plain state), so, unlike
/// [`open_and_notify`], this runs identically from both paths, no
/// deferred effect needed.
///
/// `OnBufferClose` is queued only when `id`'s `OnBufferOpen` was
/// (`open_announced`): hooks announce as a pair or not at all. A buffer
/// closed before `Editor::detect_pending_languages`'s drain announced it
/// (within the Steel eval that opened it, or by a plugin during startup)
/// announces no close either.
///
/// Refused, with nothing changed, when `id` has unsaved changes and
/// `on_unsaved` does not consent to dropping them, and while a pipe runs in
/// `id`: closing it would stop the run. A caller that means to stop it calls
/// `EditorState::cancel_pipe_run` first.
pub(in crate::editor) fn close_buffer_and_notify(
    ev: &mut EngineView,
    state: &mut EditorState,
    id: BufferId,
    on_unsaved: OnUnsaved,
) -> Result<(), CloseBufferRefused> {
    state
        .refuse_unsaved(id, on_unsaved)
        .map_err(CloseBufferRefused::Unsaved)?;
    if state.buffers.get(id).is_busy() {
        return Err(CloseBufferRefused::PipeRunning(PipeRunning(id)));
    }
    // Must run before the slot is freed below: `didClose` names the
    // buffer's URI.
    state.lsp_buffer_closing(ev, id);
    // Read before the slot is freed by `close_buffer` below.
    let open_announced = state.buffers.get(id).open_announced;
    let language = state.buffers.get(id).plugin_view.language.clone();
    close_buffer(state, ev, id);
    if open_announced {
        // Fire with the ID that was closed, not the new current buffer.
        state.queue_event(EditorEvent::OnBufferClose {
            buffer: id,
            language,
        });
    }
    Ok(())
}

/// Why [`close_buffer_and_notify`] or `EditorState::quit` closed nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum CloseBufferRefused {
    Unsaved(Unsaved),
    PipeRunning(PipeRunning),
}

impl PipeRefusal for CloseBufferRefused {
    fn pipe_running(&self) -> Option<PipeRunning> {
        match self {
            Self::PipeRunning(running) => Some(*running),
            Self::Unsaved(_) => None,
        }
    }
}

/// Drop every pane's saved scroll *and* wrap-mode pin for `id`, once they no
/// longer describe its content: it was replaced wholesale, reloaded, or
/// closed. Not limited to viewers, since a background pane's *saved* scroll
/// or pin for `id` is just as stale as a live one's, and a regenerated view
/// buffer starts unpinned again, same as a freshly opened one would.
pub(in crate::editor::buffer) fn forget_saved_views(ev: &mut EngineView, id: BufferId) {
    for (_, pane) in ev.panes.every_pane_across_all_tabs_mut() {
        pane.forget_buffer(id);
    }
}

/// Drops every `EditorState`/`EngineView` store keyed by the closed buffer
/// `id`.
///
/// A reload confirm naming `id` is retired, not left open: its `r` answer
/// would find no buffer, and an unanswerable confirm blocks every later
/// prompt. See `EditorState::retire_stale_confirm` for why that is an
/// `excise_layer`, not a `truncate_layers`.
fn forget_closed_buffer(state: &mut EditorState, ev: &mut EngineView, id: BufferId) {
    state.config.statusline_text.remove(&id);
    state.config.local_keymaps.forget_buffer(id);
    state.pending_writes.forget_buffer(id);
    state.retire_stale_confirm(ev, |c| c.targets_buffer(id));
    forget_saved_views(ev, id);
    let (_, mut stores) = state.edit_parts();
    stores.forget_buffer(id);
}
