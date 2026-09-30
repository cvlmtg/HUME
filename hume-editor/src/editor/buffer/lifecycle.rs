//! Free functions for buffer lifecycle operations.
//!
//! Free functions (not `impl Editor` methods) so the same logic can be
//! called by both the `Editor` methods (which take `&mut self`) and the
//! Steel builtins. Both hold a whole `EditorState`/`EngineView` pair (the
//! host through `SteelCtx`), just never the same `Editor` these live on, so
//! they take `state`/`ev` directly instead of `&mut self`.
//!
//! The `impl Editor` choke-points (`open_buffer`, `close_buffer`,
//! `switch_to_buffer_with_jump`) are thin delegators; all logic lives here.

use hume_engine::pipeline::{BufferId, EngineView, PaneId};

use crate::editor::EditorState;
use crate::editor::buffer::Buffer;
use crate::editor::buffer::store::BufferStore;
use crate::editor::commands::CommandPane;
use crate::editor::event::EditorEvent;
use crate::editor::jump_list::{JumpRule, with_jump};
use crate::editor::lsp::LspState;
use crate::editor::pane_state;
use crate::editor::position_stores::PositionStores;

// ── open_or_dedup / open_buffer ───────────────────────────────────────────────

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
/// (see that function's doc). Deliberately does **not** run detection
/// inline: that needs `set_buffer_language`, which can activate lazy
/// language plugins via `self.scripting`, a full `&mut Editor`/Steel-eval
/// capability neither of this function's callers hold. Also leaves
/// `open_hook_pending` set until that drain fires its `OnBufferOpen`, read
/// by [`close_buffer_and_notify`] so a buffer closed before the drain runs
/// announces neither hook, rather than an `OnBufferClose` with no matching
/// open.
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

/// Dedup-open a file path: if already open returns `(existing_id, false)`,
/// otherwise reads the file (or opens an empty new-file buffer if it doesn't
/// exist yet; see `Buffer::from_file_or_new`) and allocates via
/// [`open_buffer_and_notify`] (which seeds the `undo-levels` cap from
/// `state.settings`), returning `(new_id, true)`. Dedup-opening an
/// already-open path enqueues no hook and detects no language, matching
/// `Editor::open_buffer`'s "every call is a genuinely new buffer" contract.
/// The caller is responsible for any other post-open work (pane switching).
pub(in crate::editor) fn open_or_dedup_and_notify(
    ev: &mut EngineView,
    state: &mut EditorState,
    resolved: &std::path::Path,
) -> std::io::Result<(BufferId, bool)> {
    if let Some(existing) = state.buffers.find_by_path(resolved) {
        return Ok((existing, false));
    }
    let doc = Buffer::from_file_or_new(resolved, &state.cwd)?;
    Ok((open_buffer_and_notify(ev, state, doc), true))
}

/// Redirect pane `pid` to `target` without recording a jump.
///
/// When `pid` is focused and `target` differs from its current buffer:
/// - ends any open Insert/paste session first
///   ([`crate::editor::focus::end_focus_sessions`]), before the `buffer_id`
///   write it reads through. A stale session would make the next keystroke
///   panic in `doc_ops::apply_doc_edit_grouped`.
/// - promotes `target` in `BufferStore.mru`. With
///   [`crate::editor::focus::focus_pane`], this keeps `mru`'s tail equal to
///   the buffer on screen.
///
/// Saves the pane's scroll for the old buffer, restores `target`'s saved scroll
/// (zero on first visit), and seeds `pane_state[pid][target]` if this pane has
/// never viewed `target` before. Only the pane's own `buffer_id` is
/// written; no denormalised copy of it is updated.
pub(in crate::editor) fn switch_pane_to_buffer(
    state: &mut EditorState,
    ev: &mut EngineView,
    pid: PaneId,
    target: BufferId,
) {
    let is_focused_switch = pid == state.focus.id() && ev.panes[pid].buffer_id != target;
    if is_focused_switch {
        crate::editor::focus::end_focus_sessions(state, ev);
    }
    ev.panes[pid].remember_scroll();
    ev.panes[pid].buffer_id = target;
    if is_focused_switch {
        state.buffers.touch_mru(target);
    }
    ev.panes[pid].recall_scroll(target, state.buffers.get(target).text().last_content_line());
    // Seeds `pane_state[pid][target]` on this pane's first visit to
    // `target`. Required regardless of reveal, since `frame.rs`'s scroll
    // step indexes it directly. A different buffer's cursor/viewport pairing
    // always needs re-settling, whether or not the recalled selection's head
    // happens to equal the outgoing one: `EditorState::layout_key`'s
    // `buffer_tag` names the buffer, so a switch to a different one always
    // differs from `PaneBufferState::last_layout_key`: the very first read
    // for a `(pane, buffer)` pair is `None`, which differs from anything.
    pane_state::ensure(
        &mut state.panes.state,
        &state.buffers,
        &ev.panes,
        pid,
        target,
    );
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
pub(in crate::editor) fn close_buffer(
    state: &mut EditorState,
    ev: &mut EngineView,
    id: BufferId,
) -> Option<BufferId> {
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
        .filter(|(_, p)| p.buffer_id == id)
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

/// [`close_buffer`] plus the pre-close LSP sync and post-close cleanup
/// `Editor::close_buffer` performs: `didClose` notification, diagnostics
/// clear, decoration clear, and the `OnBufferClose` hook enqueue.
///
/// Unlike buffer open, none of this needs Steel eval (`didClose` is pure
/// protocol, diagnostics/decorations are plain state), so, unlike
/// `open_buffer_and_notify`, this runs identically from both paths, no
/// deferred effect needed. `lsp` is `Option` to mirror `EditorHostImpl.lsp`'s
/// own `Option<&mut LspState>` shape: when `None`, the LSP side effects are
/// skipped rather than panicking, though in practice this is never observed
/// because `close-buffer!` is command-gated, and command dispatch always supplies
/// `Some`.
///
/// `OnBufferClose` is queued only when `id`'s `OnBufferOpen` already fired
/// (`!open_hook_pending`): hooks announce as a pair or not at all. A buffer
/// opened and closed before `Editor::detect_pending_languages`'s drain ran
/// (e.g. within one Steel eval) never announced its open, so it must not
/// announce a close either.
pub(in crate::editor) fn close_buffer_and_notify(
    ev: &mut EngineView,
    state: &mut EditorState,
    lsp: Option<&mut LspState>,
    id: BufferId,
) {
    if let Some(lsp) = lsp {
        // Must run before the slot is freed below: needs the buffer's path
        // and lsp_server to build the didClose notification.
        crate::editor::lsp::sync::lsp_did_close(state, lsp, id);
    }
    // Read before the slot is freed by `close_buffer` below.
    let open_announced = !state.buffers.get(id).open_hook_pending;
    let opened = close_buffer(state, ev, id);
    // The last-buffer branch fired: a fresh scratch buffer was allocated in
    // `id`'s place and must announce its own `OnBufferOpen` like any other
    // open: it is a genuinely new `BufferId`, not `id` reused.
    if let Some(bid) = opened {
        queue_open_announcement(state, bid);
    }
    if open_announced {
        // Fire with the ID that was closed, not the new current buffer.
        state.queue_event(EditorEvent::OnBufferClose { buffer: id });
    }
}

/// Drop every pane's saved scroll *and* wrap-mode pin for `id`, whose content
/// was reset wholesale (`set_view_content`'s history-resetting in-place
/// replace (`Editor::open_read_only_view`)): not limited to viewers, since a
/// background pane's *saved* scroll or pin for `id` is just as stale as a
/// live one's, and a regenerated view buffer starts unpinned again, same as a
/// freshly opened one would. Its stored positions were already reset by
/// `PositionStores::reset`.
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
    state.retire_stale_confirm(ev, |c| c.targets_buffer(id));
    forget_saved_views(ev, id);
    PositionStores::new(
        &mut state.panes,
        &mut state.input,
        &mut state.buffer_positions,
        &mut state.config.decorations,
    )
    .forget_buffer(id);
}
