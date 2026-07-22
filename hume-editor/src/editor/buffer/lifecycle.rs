//! Free functions for buffer lifecycle operations.
//!
//! Extracted from `impl Editor` so the same logic can be called by both the
//! `Editor` methods (which take `&mut self`) and the Steel builtins
//! (which receive individual `&mut` references via `SteelCtx`).
//!
//! The `impl Editor` choke-points (`open_buffer`, `close_buffer`,
//! `switch_to_buffer_with_jump`, `replace_buffer_in_place`) are thin
//! delegators; all logic lives here.

use slotmap::SecondaryMap;

use hume_engine::pipeline::{BufferId, EngineView, PaneId};
use hume_scripting::SteelBufferId;
use hume_scripting::hooks::HookId;

use crate::editor::EditorState;
use crate::editor::buffer::Buffer;
use crate::editor::buffer::store::BufferStore;
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
/// `state.pending_language_detection`; every caller drains it once it holds
/// (or regains) that capability — see `Editor::detect_pending_languages`,
/// which also fires `OnBufferOpen` once detection (and `OnLanguageSet`) for
/// `bid` has run, so plugins observing both hooks see `OnLanguageSet` first.
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
    state.pending_language_detection.push(bid);
    bid
}

/// Dedup-open a file path: if already open returns `(existing_id, false)`,
/// otherwise reads the file and allocates via [`open_buffer_and_notify`]
/// (which seeds the `undo-levels` cap from `state.settings`), returning
/// `(new_id, true)`. Dedup-opening an already-open path enqueues no hook and
/// detects no language — matching `Editor::open_buffer`'s "every call is a
/// genuinely new buffer" contract. The caller is responsible for any other
/// post-open work (pane switching).
pub(crate) fn open_or_dedup_and_notify(
    ev: &mut EngineView,
    state: &mut EditorState,
    canonical: &std::path::Path,
) -> std::io::Result<(BufferId, bool)> {
    if let Some(existing) = state.buffers.find_by_path(canonical) {
        return Ok((existing, false));
    }
    let doc = Buffer::from_file(canonical)?;
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
    ev.panes[pid].recall_scroll(target);
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
    state.decorations.remove_buffer(id);
    let new_focused = close_buffer(
        ev,
        &mut state.buffers,
        &mut state.panes.state,
        &mut state.panes.jumps,
        state.focused_pane_id,
        id,
        state.settings.undo_levels,
    );
    // Fire with the ID that was closed, not the new current buffer.
    let val = SteelBufferId::new(id).into_steel_val();
    state.pending_hooks.push((HookId::OnBufferClose, vec![val]));
    new_focused
}

// ── replace_buffer_in_place ───────────────────────────────────────────────────

/// Replace buffer `id` with `new_doc` in-place, reseeding all pane state.
///
/// Used by `:e!` reload and the last-buffer case of `close_buffer`.
/// Caller contract: `new_doc.search_pattern` must be `None`.
pub(crate) fn replace_buffer_in_place(
    ev: &mut EngineView,
    buffers: &mut BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pane_jumps: &mut SecondaryMap<PaneId, JumpList>,
    id: BufferId,
    new_doc: Buffer,
) {
    debug_assert!(
        new_doc.search_pattern.is_none(),
        "replace_buffer_in_place: new_doc must have no active search state",
    );
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
