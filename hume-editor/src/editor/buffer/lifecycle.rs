//! Free functions for buffer lifecycle operations.
//!
//! Free functions (not `impl Editor` methods) so the same logic can be
//! called by both the `Editor` methods (which take `&mut self`) and the
//! Steel builtins (which receive individual `&mut` references via
//! `SteelCtx`).
//!
//! The `impl Editor` choke-points (`open_buffer`, `close_buffer`,
//! `switch_to_buffer_with_jump`, `replace_buffer_in_place`) are thin
//! delegators; all logic lives here.

use slotmap::SecondaryMap;

use hume_engine::pipeline::{BufferId, EngineView, PaneId};

use crate::editor::EditorState;
use crate::editor::buffer::Buffer;
use crate::editor::buffer::store::BufferStore;
use crate::editor::event::EditorEvent;
use crate::editor::jump_list::{JumpEntry, JumpList};
use crate::editor::lsp::LspState;
use crate::editor::pane_state::{self, PaneBufferState};

// ── open_or_dedup / open_buffer ───────────────────────────────────────────────

/// Allocate a new buffer slot (engine + BufferStore), seed the focused pane's
/// `pane_state` with initial selections, and return the allocated `BufferId`.
///
/// `undo_levels` seeds `doc`'s `undo-levels` cap — the current global
/// setting, since new buffers always start out tracking it.
pub(crate) fn open_buffer(
    ev: &mut EngineView,
    buffers: &mut BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    focused_pane_id: PaneId,
    mut doc: Buffer,
    undo_levels: usize,
) -> BufferId {
    doc.set_undo_levels(undo_levels);
    let bid = ev.buffers.insert(());
    buffers.open(bid, doc);
    pane_state::ensure(pane_state, buffers, focused_pane_id, bid);
    bid
}

/// [`open_buffer`] plus queuing language detection — the disjoint-borrow
/// (`view`/`state`) chokepoint every buffer-opening path shares:
/// `Editor::open_buffer`, `EditorHostImpl::open_buffer` (`(open-buffer! …)`),
/// and `lsp::edits::resolve_or_open` (workspace edits, goto-definition).
///
/// Deliberately does **not** run detection inline: that needs
/// `set_buffer_language`, which can activate lazy language plugins via
/// `self.scripting` — a full `&mut Editor`/Steel-eval capability this
/// disjoint-borrow chokepoint never holds. Instead `bid` is queued onto
/// `state.config.pending_language_detection`; every caller drains it once it holds
/// (or regains) that capability — see `Editor::detect_pending_languages`,
/// which also fires `OnBufferOpen` once detection (and `OnLanguageSet`) for
/// `bid` has run, so plugins observing both hooks see `OnLanguageSet` first.
///
/// Also marks the buffer `open_hook_pending` until that drain fires its
/// `OnBufferOpen` — read by [`close_buffer_and_notify`] so a buffer closed
/// before the drain runs (e.g. opened and closed within one Steel eval)
/// announces neither hook, rather than an `OnBufferClose` with no matching
/// open.
pub(crate) fn open_buffer_and_notify(
    ev: &mut EngineView,
    state: &mut EditorState,
    doc: Buffer,
) -> BufferId {
    let bid = open_buffer(
        ev,
        &mut state.buffers,
        &mut state.panes.state,
        state.focused_pane_id,
        doc,
        state.settings.undo_levels,
    );
    state.buffers.get_mut(bid).open_hook_pending = true;
    state.config.pending_language_detection.push(bid);
    bid
}

/// Dedup-open a file path: if already open returns `(existing_id, false)`,
/// otherwise reads the file (or opens an empty new-file buffer if it doesn't
/// exist yet — see `Buffer::from_file_or_new`) and allocates via
/// [`open_buffer_and_notify`] (which seeds the `undo-levels` cap from
/// `state.settings`), returning `(new_id, true)`. Dedup-opening an
/// already-open path enqueues no hook and detects no language — matching
/// `Editor::open_buffer`'s "every call is a genuinely new buffer" contract.
/// The caller is responsible for any other post-open work (pane switching).
pub(crate) fn open_or_dedup_and_notify(
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

// ── switch_pane_to_buffer ──────────────────────────────────────────────────────

/// Redirect pane `pid` to `target` without recording a jump.
///
/// Saves the pane's scroll for the old buffer, restores `target`'s saved scroll
/// (zero on first visit), and seeds `pane_state[pid][target]` if this pane has
/// never viewed `target` before. Does not touch any denormalised `buffer_id`.
pub(crate) fn switch_pane_to_buffer(
    ev: &mut EngineView,
    buffers: &BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pid: PaneId,
    target: BufferId,
) {
    ev.panes[pid].remember_scroll();
    ev.panes[pid].buffer_id = target;
    ev.panes[pid].recall_scroll(target, buffers.get(target).text().len_lines());
    pane_state::ensure(pane_state, buffers, pid, target);
}

// ── switch_to_buffer_with_jump ────────────────────────────────────────────────

/// Redirect the focused pane to `target`, pushing the outgoing position onto
/// `pane_jumps[focused_pane_id]`.
///
/// Caller contract: all fallible steps must succeed before calling this —
/// `push` truncates forward history.
pub(crate) fn switch_to_buffer_with_jump(
    ev: &mut EngineView,
    buffers: &BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pane_jumps: &mut SecondaryMap<PaneId, JumpList>,
    focused_pane_id: PaneId,
    current_buffer_id: BufferId,
    target: BufferId,
) {
    let sels = pane_state[focused_pane_id][current_buffer_id]
        .selections
        .clone();
    let entry = JumpEntry::new(
        sels,
        buffers.get(current_buffer_id).text(),
        current_buffer_id,
    );
    pane_jumps[focused_pane_id].push(entry);
    switch_pane_to_buffer(ev, buffers, pane_state, focused_pane_id, target);
}

// ── close_buffer ──────────────────────────────────────────────────────────────

/// Remove buffer `id`, handling both cases:
///
/// - At least one other buffer: redirect every pane viewing `id` to the
///   MRU replacement, then free the slot.
/// - Only buffer: replace in-place with a fresh scratch buffer, seeded with
///   `undo_levels` (the current global `undo-levels` setting).
///
/// Returns the `BufferId` that the focused pane is now viewing.
pub(crate) fn close_buffer(
    ev: &mut EngineView,
    buffers: &mut BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pane_jumps: &mut SecondaryMap<PaneId, JumpList>,
    focused_pane_id: PaneId,
    id: BufferId,
    undo_levels: usize,
) -> BufferId {
    match buffers.mru_excluding(id) {
        Some(next) => {
            // Collect before mutating (borrow checker); n≈1 in the single-pane case.
            let panes_to_redirect: Vec<PaneId> = ev
                .panes
                .iter()
                .filter(|(_, p)| p.buffer_id == id)
                .map(|(pid, _)| pid)
                .collect();
            for pid in panes_to_redirect {
                switch_pane_to_buffer(ev, buffers, pane_state, pid, next);
            }
            buffers.close(id);
            ev.buffers.remove(id);
            forget_buffer_in_all_panes(ev, pane_state, pane_jumps, id);
            ev.panes[focused_pane_id].buffer_id
        }
        None => {
            let mut scratch = Buffer::scratch();
            scratch.set_undo_levels(undo_levels);
            replace_buffer_in_place(ev, buffers, pane_state, pane_jumps, id, scratch);
            id
        }
    }
}

/// [`close_buffer`] plus the pre-close LSP sync and post-close cleanup
/// `Editor::close_buffer` performs: `didClose` notification, diagnostics
/// clear, decoration clear, and the `OnBufferClose` hook enqueue — the
/// disjoint-borrow (`view`/`state`/`lsp`) chokepoint shared by `Editor::
/// close_buffer` and `EditorHostImpl::close_buffer` (`(close-buffer! …)`).
///
/// Unlike buffer open, none of this needs Steel eval (`didClose` is pure
/// protocol, diagnostics/decorations are plain state), so — unlike
/// `open_buffer_and_notify` — this runs identically from both callers, no
/// deferred effect needed. `lsp` is `Option` to mirror `EditorHostImpl.lsp`'s
/// own `Option<&mut LspState>` shape: when `None`, the LSP side effects are
/// skipped rather than panicking, though in practice this is never observed
/// — `close-buffer!` is command-gated, and command dispatch always supplies
/// `Some`.
///
/// `OnBufferClose` is queued only when `id`'s `OnBufferOpen` already fired
/// (`!open_hook_pending`) — hooks announce as a pair or not at all. A buffer
/// opened and closed before `Editor::detect_pending_languages`'s drain ran
/// (e.g. within one Steel eval) never announced its open, so it must not
/// announce a close either.
pub(crate) fn close_buffer_and_notify(
    ev: &mut EngineView,
    state: &mut EditorState,
    lsp: Option<&mut LspState>,
    id: BufferId,
) -> BufferId {
    if let Some(lsp) = lsp {
        // Must run before the slot is freed below — needs the buffer's path
        // and lsp_server to build the didClose notification.
        crate::editor::lsp::sync::lsp_did_close(state, lsp, id);
        // Purely a leak fix — `id` is a versioned slotmap key, so a future
        // reused slot can never alias with these stale entries — but there
        // is no other chokepoint that ever frees them.
        lsp.remove_buffer_diagnostics(id);
    }
    state.config.decorations.remove_buffer(id);
    // A reload confirm naming `id` would otherwise outlive its subject: the
    // slot is freed below, or — in the last-buffer branch — reused in place
    // for a fresh scratch, so `reload_buffer_from_disk` would bail on
    // `try_get` or on the scratch's missing path and the user's `r` would do
    // nothing. Retire the question rather than leave one that can't be
    // answered; with `can_open_confirm`'s `confirm.is_none()` guard, leaving
    // it would also block every later prompt until some stray key happened
    // to dismiss it.
    if state
        .config
        .confirm
        .as_ref()
        .is_some_and(|c| c.targets_buffer(id))
    {
        state.config.confirm = None;
    }
    // Read before the slot is freed by `close_buffer` below.
    let open_announced = !state.buffers.get(id).open_hook_pending;
    let new_focused = close_buffer(
        ev,
        &mut state.buffers,
        &mut state.panes.state,
        &mut state.panes.jumps,
        state.focused_pane_id,
        id,
        state.settings.undo_levels,
    );
    if open_announced {
        // Fire with the ID that was closed, not the new current buffer.
        state.queue_event(EditorEvent::OnBufferClose { buffer: id });
    }
    new_focused
}

// ── replace_buffer_in_place ───────────────────────────────────────────────────

/// Replace buffer `id` with `new_doc` in-place, reseeding all pane state.
///
/// Used by the last-buffer case of `close_buffer`.
/// Caller contract: `new_doc.search_pattern` must be `None`.
pub(crate) fn replace_buffer_in_place(
    ev: &mut EngineView,
    buffers: &mut BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pane_jumps: &mut SecondaryMap<PaneId, JumpList>,
    id: BufferId,
    mut new_doc: Buffer,
) {
    debug_assert!(
        new_doc.search_pattern.is_none(),
        "replace_buffer_in_place: new_doc must have no active search state",
    );
    let prev = buffers.get(id);
    // Carry the stamp forward past whatever `new_doc`'s constructor set it
    // to (always 0) — see `Buffer::replace_stamp`'s doc for why this bump,
    // not the buffer's content, is what marks `id` as "not the same buffer
    // instance a snapshot taken before this call meant".
    new_doc.replace_stamp = prev.replace_stamp.wrapping_add(1);
    // `text_gen`/`announced_text_gen` are a per-`BufferId` observation baseline,
    // not per-`Buffer`-instance state: `take_text_changed` diffs them to raise
    // `on-text-changed`. Letting `new_doc`'s constructor reset both to 0 would
    // make a total content replacement under a live id read as "nothing
    // happened". Carry the baseline forward and bump past it so the swap
    // announces itself exactly once.
    new_doc.text_gen = prev.text_gen + 1;
    new_doc.announced_text_gen = prev.announced_text_gen;
    // The new doc carries no syntax attachment (Buffer.syntax = None by
    // construction — the flip made this assignment alone sufficient to drop
    // any stale committed layers, since they now live inside Buffer.syntax).
    *buffers.get_mut(id) = new_doc;
    // Collect before mutating (borrow checker); n≈1 in the single-pane case.
    let pane_ids: Vec<PaneId> = ev
        .panes
        .iter()
        .filter(|(_, p)| p.buffer_id == id)
        .map(|(pid, _)| pid)
        .collect();
    for pid in pane_ids {
        // Unconditional overwrite: caller replaced the buffer, so old view state
        // (selections, edit group) is stale and must be discarded.
        pane_state[pid].insert(id, pane_state::fresh_from_buf(buffers.get(id)));
    }
    for jumps in pane_jumps.values_mut() {
        jumps.prune_buffer(id);
    }
    for pane in ev.panes.values_mut() {
        pane.forget_buffer(id);
    }
}

// ── forget_buffer_in_all_panes ────────────────────────────────────────────────

fn forget_buffer_in_all_panes(
    ev: &mut EngineView,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pane_jumps: &mut SecondaryMap<PaneId, JumpList>,
    id: BufferId,
) {
    for pane in ev.panes.values_mut() {
        pane.forget_buffer(id);
    }
    for buf_state in pane_state.values_mut() {
        buf_state.remove(id);
    }
    for jumps in pane_jumps.values_mut() {
        jumps.prune_buffer(id);
    }
}
