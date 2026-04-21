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

use engine::pipeline::{BufferId, EngineView, PaneId, SharedBuffer};

use crate::core::jump_list::{JumpEntry, JumpList};
use crate::editor::buffer::Buffer;
use crate::editor::buffer_store::BufferStore;
use crate::editor::pane_state::PaneBufferState;

// ── open_or_dedup / open_buffer ───────────────────────────────────────────────

/// Dedup-open a file path: if already open returns `(existing_id, false)`,
/// otherwise reads the file and allocates a new buffer, returning `(new_id, true)`.
///
/// The caller is responsible for any post-open work (hook firing, pane switching).
pub(crate) fn open_or_dedup(
    ev: &mut EngineView,
    buffers: &mut BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    focused_pane_id: PaneId,
    canonical: &std::path::Path,
) -> std::io::Result<(BufferId, bool)> {
    if let Some(existing) = buffers.find_by_path(canonical) {
        return Ok((existing, false));
    }
    let doc = Buffer::from_file(canonical)?;
    Ok((open_buffer(ev, buffers, pane_state, focused_pane_id, doc), true))
}

/// Allocate a new buffer slot (engine + BufferStore), seed the focused pane's
/// `pane_state` with initial selections, and return the allocated `BufferId`.
pub(crate) fn open_buffer(
    ev: &mut EngineView,
    buffers: &mut BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    focused_pane_id: PaneId,
    doc: Buffer,
) -> BufferId {
    let bid = ev.buffers.insert(SharedBuffer::new());
    let initial_sels = doc.initial_sels();
    buffers.open(bid, doc);
    pane_state[focused_pane_id].insert(bid, PaneBufferState {
        selections: initial_sels,
        ..PaneBufferState::default()
    });
    bid
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
    if !pane_state[pid].contains_key(target) {
        let initial_sels = buffers.get(target).initial_sels();
        pane_state[pid].insert(target, PaneBufferState {
            selections: initial_sels,
            ..PaneBufferState::default()
        });
    }
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
    let sels = pane_state[focused_pane_id][current_buffer_id].selections.clone();
    let entry = JumpEntry::new(sels, buffers.get(current_buffer_id).text(), current_buffer_id);
    pane_jumps[focused_pane_id].push(entry);
    switch_pane_to_buffer(ev, buffers, pane_state, focused_pane_id, target);
}

// ── close_buffer ──────────────────────────────────────────────────────────────

/// Remove buffer `id`, handling both cases:
///
/// - At least one other buffer: redirect every pane viewing `id` to the
///   MRU replacement, then free the slot.
/// - Only buffer: replace in-place with a fresh scratch buffer.
///
/// Returns the `BufferId` that the focused pane is now viewing.
pub(crate) fn close_buffer(
    ev: &mut EngineView,
    buffers: &mut BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pane_jumps: &mut SecondaryMap<PaneId, JumpList>,
    focused_pane_id: PaneId,
    id: BufferId,
) -> BufferId {
    match buffers.mru_excluding(id) {
        Some(next) => {
            let panes_to_redirect: Vec<PaneId> = ev.panes
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
            replace_buffer_in_place(ev, buffers, pane_state, pane_jumps, id, Buffer::scratch());
            id
        }
    }
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
    let initial_sels = new_doc.initial_sels();
    *buffers.get_mut(id) = new_doc;
    let pane_ids: Vec<PaneId> = ev.panes
        .iter()
        .filter(|(_, p)| p.buffer_id == id)
        .map(|(pid, _)| pid)
        .collect();
    for pid in pane_ids {
        pane_state[pid].insert(id, PaneBufferState {
            selections: initial_sels.clone(),
            ..PaneBufferState::default()
        });
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
