//! Free functions for buffer lifecycle operations.
//!
//! Free functions (not `impl Editor` methods) so the same logic can be
//! called by both the `Editor` methods (which take `&mut self`) and the
//! Steel builtins. Both hold a whole `EditorState`/`EngineView` pair (the
//! host through `SteelCtx`), just never the same `Editor` these live on, so
//! they take `state`/`ev` directly instead of `&mut self`.
//!
//! The `impl Editor` choke-points (`switch_to_buffer_with_jump`,
//! `open_read_only_view`) are thin delegators; all logic lives here.
//! Production closes call [`close_buffer_and_notify`] directly.

use hume_editing::text::BufferText;
use hume_engine::pipeline::{BufferId, EngineView, PaneId};
use hume_platform::path::AbsolutePath;
use hume_rope::column::GraphemeCol;
use hume_rope::line::ContentLine;

use crate::editor::EditorState;
use crate::editor::buffer::store::BufferStore;
use crate::editor::buffer::{Buffer, FileTarget, OnUnsaved, Unsaved, ViewLabel};
use crate::editor::commands::CommandPane;
use crate::editor::event::EditorEvent;
use crate::editor::focus::switch_pane_to_buffer;
use crate::editor::jump_list::{JumpRule, with_jump};
use crate::editor::pane_state;
use crate::editor::pipe_run::{PipeRefusal, PipeRunning};

// ── open_or_dedup_and_notify / open_buffer ────────────────────────────────────

/// Allocate a new buffer slot (engine + BufferStore) and return the
/// allocated `BufferId`. No pane shows it yet; whichever pane first
/// switches to it seeds its own `pane_state` entry lazily, via
/// `switch_pane_to_buffer`/`write_cursor`'s own `pane_state::ensure` calls.
///
/// `undo_levels` seeds `doc`'s `undo-levels` cap: the current global
/// setting, since new buffers always start out tracking it.
pub(in crate::editor) fn open_buffer(
    ev: &mut EngineView,
    buffers: &mut BufferStore,
    mut doc: Buffer,
    undo_levels: usize,
) -> BufferId {
    doc.set_undo_levels(undo_levels);
    let bid = ev.buffers.insert(());
    buffers.open(bid, doc);
    bid
}

/// Marks `bid` `open_hook_pending` and queues it for language detection:
/// every fresh `BufferId` must go through this exactly once, or
/// `Editor::detect_pending_languages` never announces its `OnBufferOpen`
/// (see that function's doc). Does **not** run detection
/// inline: that needs `set_buffer_language`, which can activate lazy
/// language plugins via `self.scripting`, a full `&mut Editor`/Steel-eval
/// capability neither of this function's callers hold. The drain runs at the
/// end of the Steel eval that queued `bid`, or else at the next
/// `Editor::settle`. Also leaves `open_hook_pending` set until that drain
/// fires its `OnBufferOpen`, read by [`close_buffer_and_notify`] so a
/// buffer closed before the drain runs announces neither hook, rather than
/// an `OnBufferClose` with no matching open.
///
/// Two callers: [`open_buffer_and_notify`] below, and `Editor::open`, which
/// builds the startup buffer inline (bootstrapping the very
/// `EngineView`/pane-state maps `open_buffer` needs, so it can't call that
/// helper) but must leave the buffer in the same state.
pub(in crate::editor) fn queue_open_announcement(state: &mut EditorState, bid: BufferId) {
    state.buffers.get_mut(bid).open_hook_pending = true;
    state.config.pending_language_detection.push(bid);
}

/// [`open_buffer`] plus [`queue_open_announcement`]. See the latter for why
/// detection doesn't run inline. `Editor::detect_pending_languages` fires
/// `OnBufferOpen` once detection (and `OnLanguageSet`) for `bid` has run, so
/// plugins observing both hooks see `OnLanguageSet` first.
pub(in crate::editor::buffer) fn open_buffer_and_notify(
    ev: &mut EngineView,
    state: &mut EditorState,
    doc: Buffer,
) -> BufferId {
    let bid = open_buffer(ev, &mut state.buffers, doc, state.settings.undo_levels);
    queue_open_announcement(state, bid);
    bid
}

/// Dedup-open the file `typed` names: if a buffer already has its resolved
/// path, returns `(existing_id, false)`; otherwise reads the file (or opens
/// an empty new-file buffer if it doesn't exist yet; see
/// `Buffer::from_file_or_new`) and allocates via [`open_buffer_and_notify`]
/// (which seeds the `undo-levels` cap from `state.settings`), returning
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
    let doc = Buffer::from_file_or_new(target)?;
    Ok((open_buffer_and_notify(ev, state, doc), true))
}

/// Redirect the focused pane to `target`, pushing the outgoing position onto
/// `pane_jumps[focused_pane_id]`, unless `target` is the buffer already
/// focused, which would be a no-op switch (e.g. `:tutor` run a second time
/// while already viewing it): `push` truncates forward history
/// unconditionally, so recording a jump to nowhere would corrupt it for
/// nothing. Most callers already avoid this (`enter_buffer` gates on it
/// explicitly), but not all do, so the guard lives here instead of being
/// re-derived at each one.
///
/// Caller contract: all fallible steps must succeed before calling this:
/// `push` truncates forward history.
pub(in crate::editor) fn switch_to_buffer_with_jump(
    state: &mut EditorState,
    ev: &mut EngineView,
    pid: PaneId,
    target: BufferId,
) {
    let Some(t) = CommandPane::existing(ev, pid).filter(|t| t.bid(ev) != target) else {
        return;
    };
    with_jump(state, ev, t, JumpRule::IfMoved, |state, ev| {
        switch_pane_to_buffer(state, ev, pid, target);
    });
}

/// Show `content` on pane `t` in the read-only view buffer labelled
/// `label`, recording no jump. The first call opens the buffer; a later
/// call replaces its text in place, so the buffer list never holds two
/// views with one label. The cursor lands on `cursor_line` (clamped to the
/// last content line), or on the last content line when `None`.
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
        let doc = Buffer::read_only_view(BufferText::from(content), label.clone());
        open_buffer_and_notify(ev, state, doc)
    };

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
    bid
}

/// Remove buffer `id`. Every pane showing it (active tab or not) redirects
/// to the MRU replacement buffer (or, when `id` was the only buffer, to a
/// freshly allocated scratch buffer via [`open_buffer`], seeded with
/// `undo_levels`, the current global `undo-levels` setting), the same way
/// any other buffer open would be. `id`'s own slot is always freed: a
/// versioned key is never reused for different content, so a captured `id`
/// can never silently start naming the replacement; a same-slot in-place
/// replace would leave that failure mode open for any `LivePane` builtin
/// whose bid outlived the close.
///
/// Returns, only when the last-buffer branch fired, the freshly allocated
/// scratch buffer's id, for [`close_buffer_and_notify`] to announce with
/// [`queue_open_announcement`] exactly like any other open.
///
/// # Panics
/// Panics when `id` is busy: [`close_buffer_and_notify`] refuses that.
fn close_buffer(state: &mut EditorState, ev: &mut EngineView, id: BufferId) -> Option<BufferId> {
    assert!(
        !state.buffers.get(id).is_busy(),
        "close_buffer: a busy buffer's run would stop"
    );
    let (next, opened) = match state.buffers.mru_excluding(id) {
        Some(next) => (next, None),
        None => {
            let bid = open_buffer(
                ev,
                &mut state.buffers,
                Buffer::scratch(),
                state.settings.undo_levels,
            );
            (bid, Some(bid))
        }
    };
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
    opened
}

/// Close buffer `id`, the chokepoint every close goes through:
/// [`close_buffer`] plus the pre-close LSP sync and post-close cleanup,
/// which are the `didClose` notification, diagnostics clear, decoration
/// clear, and the `OnBufferClose` hook enqueue.
///
/// Unlike buffer open, none of this needs Steel eval (`didClose` is pure
/// protocol, diagnostics/decorations are plain state), so, unlike
/// `open_buffer_and_notify`, this runs identically from both paths, no
/// deferred effect needed.
///
/// `OnBufferClose` is queued only when `id`'s `OnBufferOpen` already fired
/// (`!open_hook_pending`): hooks announce as a pair or not at all. A buffer
/// opened and closed before `Editor::detect_pending_languages`'s drain ran
/// (e.g. within one Steel eval) never announced its open, so it must not
/// announce a close either.
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
    let open_announced = !state.buffers.get(id).open_hook_pending;
    let opened = close_buffer(state, ev, id);
    // The last-buffer branch fired: a fresh scratch buffer was allocated in
    // `id`'s place and must announce its own `OnBufferOpen` like any other
    // open: it is a fresh `BufferId`, not `id` reused.
    if let Some(bid) = opened {
        queue_open_announcement(state, bid);
    }
    if open_announced {
        // Fire with the ID that was closed, not the new current buffer.
        state.queue_event(EditorEvent::OnBufferClose { buffer: id });
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
