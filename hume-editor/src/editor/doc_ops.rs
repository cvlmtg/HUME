//! Free functions for document-editing operations.
//!
//! Extracted from `impl Editor` so command bodies can hold disjoint borrows
//! from other `Editor` fields while editing, avoiding per-keystroke
//! `SelectionSet` clones.
//!
//! Uses `std::mem::take` in place of `SelectionSet::clone()` wherever the set
//! is immediately overwritten by the edit result. `SelectionSet::default()` is
//! a minimal-valid cursor-at-0 state specifically designed for this use.

use slotmap::SecondaryMap;

use hume_engine::pipeline::{BufferId, PaneId};

use crate::editor::buffer::store::BufferStore;
use crate::editor::decorations::DecorationStores;
use crate::editor::pane_state::PaneBufferState;
use hume_editing::changeset::ChangeSet;
use hume_editing::selection::SelectionSet;
use hume_editing::text::Text;
use hume_treesitter::edits::record_pending_edits;

/// No-op when `buf_id` has no grammar attached (`syntax` is `None`).
/// Called immediately after every text mutation.
fn record_syntax_edits(
    buffers: &mut BufferStore,
    buf_id: BufferId,
    text_gen: u64,
    cs: &ChangeSet,
    rope_pre: &ropey::Rope,
) {
    if let Some(syn) = buffers.get_mut(buf_id).syntax.as_mut() {
        record_pending_edits(syn, text_gen, cs, rope_pre);
    }
}

/// No-op when `buf_id` has no LSP server attached and no char-offset
/// decorations (inlay hints / extra highlights) that need to stay in sync
/// with edits — decorations are not LSP-owned, LSP is just their first
/// client, so a buffer with `set-extra-highlights!`/`set-inlay-hints!` data
/// but no attached server still needs its edits queued here. Called
/// immediately after every text mutation, alongside `record_syntax_edits` —
/// same chokepoint, same "text changed, notify the machinery" shape, queued
/// for the LSP per-frame flush (`Editor::flush_lsp_pending_changes`, which
/// also does the decoration remap) instead of dispatched inline.
fn record_lsp_edits(
    buffers: &mut BufferStore,
    decorations: &DecorationStores,
    buf_id: BufferId,
    text_gen: u64,
    cs: &ChangeSet,
    rope_pre: &ropey::Rope,
) {
    let buf = buffers.get_mut(buf_id);
    if buf.lsp_server.is_some() || decorations.has_any(buf_id) {
        buf.lsp_pending
            .push(crate::editor::lsp::sync::LspPendingChange {
                cs: cs.clone(),
                before: rope_pre.clone(),
                version: text_gen,
            });
    }
}

/// Apply an ungrouped edit to the focused buffer and propagate the resulting
/// `ChangeSet` to all other panes viewing the same buffer.
///
/// Uses `std::mem::take` on the active `SelectionSet` instead of `clone()`.
/// The default state (cursor-at-0) is transient: it is overwritten by
/// `new_sels` before this function returns. `apply_edit` is infallible, so
/// no panic can leave the set in its default state.
pub(crate) fn apply_doc_edit(
    buffers: &mut BufferStore,
    decorations: &DecorationStores,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    focused_pane_id: PaneId,
    buf_id: BufferId,
    cmd: impl FnOnce(Text, SelectionSet) -> (Text, SelectionSet, ChangeSet),
) {
    if buffers.get(buf_id).is_read_only() {
        return;
    }
    // O(1) clones — ropey uses structural sharing (reference-counted tree nodes).
    let buf_pre = buffers.get(buf_id).text().clone();
    let rope_pre = buf_pre.rope().clone();
    let sels = std::mem::take(&mut pane_state[focused_pane_id][buf_id].selections);
    let (new_sels, cs) = buffers.get_mut(buf_id).apply_edit(sels, cmd);
    pane_state[focused_pane_id][buf_id].selections = new_sels;
    propagate_cs_to_panes(pane_state, focused_pane_id, buf_id, &cs, &buf_pre);
    let text_gen = buffers.get(buf_id).text_gen;
    record_syntax_edits(buffers, buf_id, text_gen, &cs, &rope_pre);
    record_lsp_edits(buffers, decorations, buf_id, text_gen, &cs, &rope_pre);
}

/// Apply a grouped edit (inside an insert session) to the focused buffer.
///
/// Reads and writes selections via `pane_state`, propagates `cs` to other panes.
///
/// Uses `std::mem::take` on the active `SelectionSet` instead of `clone()`.
/// `apply_edit_grouped` is infallible, so no panic can leave the set in its
/// default state.
pub(crate) fn apply_doc_edit_grouped(
    buffers: &mut BufferStore,
    decorations: &DecorationStores,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    focused_pane_id: PaneId,
    buf_id: BufferId,
    cmd: impl FnOnce(Text, SelectionSet) -> (Text, SelectionSet, ChangeSet),
) {
    if buffers.get(buf_id).is_read_only() {
        return;
    }
    let buf_pre = buffers.get(buf_id).text().clone();
    let rope_pre = buf_pre.rope().clone();
    let sels = std::mem::take(&mut pane_state[focused_pane_id][buf_id].selections);
    let doc = buffers.get_mut(buf_id);
    let pbs = &mut pane_state[focused_pane_id][buf_id];
    let (new_sels, cs) = doc.apply_edit_grouped(sels, &mut pbs.edit_group, cmd);
    pbs.selections = new_sels;
    propagate_cs_to_panes(pane_state, focused_pane_id, buf_id, &cs, &buf_pre);
    let text_gen = buffers.get(buf_id).text_gen;
    record_syntax_edits(buffers, buf_id, text_gen, &cs, &rope_pre);
    record_lsp_edits(buffers, decorations, buf_id, text_gen, &cs, &rope_pre);
}

/// Re-paste from the paste-session snapshot into the focused buffer, replacing
/// the accumulated CS in the open `paste_group`.
///
/// Propagates the resulting CS (mapping current text → new text) to all other
/// panes. `pane_state[focused_pane_id][buf_id].paste_group` must be `Some`;
/// caller must have opened the session with `Buffer::begin_edit_group` first.
pub(crate) fn apply_doc_edit_regrouped(
    buffers: &mut BufferStore,
    decorations: &DecorationStores,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    focused_pane_id: PaneId,
    buf_id: BufferId,
    cmd: impl FnOnce(Text, SelectionSet) -> (Text, SelectionSet, ChangeSet),
) {
    if buffers.get(buf_id).is_read_only() {
        return;
    }
    let buf_pre = buffers.get(buf_id).text().clone();
    let rope_pre = buf_pre.rope().clone();
    // Borrow paste_group from pane_state; NLL ends this borrow after the call.
    let pbs = &mut pane_state[focused_pane_id][buf_id];
    let (new_sels, propagation_cs) = buffers
        .get_mut(buf_id)
        .apply_edit_regrouped(&mut pbs.paste_group, cmd);
    pane_state[focused_pane_id][buf_id].selections = new_sels;
    propagate_cs_to_panes(
        pane_state,
        focused_pane_id,
        buf_id,
        &propagation_cs,
        &buf_pre,
    );
    let text_gen = buffers.get(buf_id).text_gen;
    record_syntax_edits(buffers, buf_id, text_gen, &propagation_cs, &rope_pre);
    record_lsp_edits(
        buffers,
        decorations,
        buf_id,
        text_gen,
        &propagation_cs,
        &rope_pre,
    );
}

/// Apply undo to the focused buffer and propagate the inverse `ChangeSet` to
/// all other panes viewing the same buffer.
pub(crate) fn apply_doc_undo(
    buffers: &mut BufferStore,
    decorations: &DecorationStores,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    focused_pane_id: PaneId,
    buf_id: BufferId,
) {
    if buffers.get(buf_id).is_read_only() {
        return;
    }
    // buf_pre/rope_pre are the current (post-edit) text: undo's CS maps
    // post-edit positions back to pre-edit, so non-acting panes' heads must be
    // translated through that CS.
    let buf_pre = buffers.get(buf_id).text().clone();
    let rope_pre = buf_pre.rope().clone();
    if let Some((new_sels, cs)) = buffers.get_mut(buf_id).undo() {
        pane_state[focused_pane_id][buf_id].selections = new_sels;
        propagate_cs_to_panes(pane_state, focused_pane_id, buf_id, &cs, &buf_pre);
        let text_gen = buffers.get(buf_id).text_gen;
        record_syntax_edits(buffers, buf_id, text_gen, &cs, &rope_pre);
        record_lsp_edits(buffers, decorations, buf_id, text_gen, &cs, &rope_pre);
    }
}

/// Apply redo to the focused buffer and propagate the forward `ChangeSet` to
/// all other panes viewing the same buffer.
pub(crate) fn apply_doc_redo(
    buffers: &mut BufferStore,
    decorations: &DecorationStores,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    focused_pane_id: PaneId,
    buf_id: BufferId,
) {
    if buffers.get(buf_id).is_read_only() {
        return;
    }
    let buf_pre = buffers.get(buf_id).text().clone();
    let rope_pre = buf_pre.rope().clone();
    if let Some((new_sels, cs)) = buffers.get_mut(buf_id).redo() {
        pane_state[focused_pane_id][buf_id].selections = new_sels;
        propagate_cs_to_panes(pane_state, focused_pane_id, buf_id, &cs, &buf_pre);
        let text_gen = buffers.get(buf_id).text_gen;
        record_syntax_edits(buffers, buf_id, text_gen, &cs, &rope_pre);
        record_lsp_edits(buffers, decorations, buf_id, text_gen, &cs, &rope_pre);
    }
}

/// Apply a motion function and store the resulting selection in `pane_state`.
///
/// Uses `std::mem::take` on the active `SelectionSet` instead of `clone()`;
/// the default state is transient and overwritten before this fn returns.
/// The closure `f` is assumed infallible; a panic mid-motion leaves
/// `selections` as `Default` (cursor at 0).
pub(crate) fn apply_doc_motion(
    buffers: &BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    focused_pane_id: PaneId,
    buf_id: BufferId,
    f: impl FnOnce(&Text, SelectionSet) -> SelectionSet,
) {
    let new_sels = {
        let buf = buffers.get(buf_id).text();
        let sels = std::mem::take(&mut pane_state[focused_pane_id][buf_id].selections);
        f(buf, sels)
    };
    pane_state[focused_pane_id][buf_id].selections = new_sels;
}

/// Open an edit group on the focused buffer.
///
/// Snapshots the current selections (via `.clone()`) for use as `pre_sels`
/// in the recorded undo revision — the field must NOT be taken because the
/// ongoing insert session continues to read it between keystrokes.
pub(crate) fn begin_edit_group(
    buffers: &BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    focused_pane_id: PaneId,
    buf_id: BufferId,
) {
    let sels = pane_state[focused_pane_id][buf_id].selections.clone();
    let doc = buffers.get(buf_id);
    let pbs = &mut pane_state[focused_pane_id][buf_id];
    doc.begin_edit_group(&mut pbs.edit_group, sels);
}

/// Close the current edit group and record it as a single undo step.
///
/// Snapshots the current selections as `post_sels` for the undo revision;
/// same rationale as `begin_edit_group` — must `.clone()`, not `take`.
pub(crate) fn commit_edit_group(
    buffers: &mut BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    focused_pane_id: PaneId,
    buf_id: BufferId,
) {
    let sels = pane_state[focused_pane_id][buf_id].selections.clone();
    let doc = buffers.get_mut(buf_id);
    let pbs = &mut pane_state[focused_pane_id][buf_id];
    doc.commit_edit_group(&mut pbs.edit_group, sels);
}

/// Propagate `cs` to every pane except `focused_pane_id` that views `buf_id`,
/// keeping their selections valid after an edit the focused pane performed.
///
/// `buf_pre` must be the buffer text **before** the edit — `translate_in_place`
/// uses it to identify which line each head was on pre-edit, which governs
/// whether `Selection.horiz` is reset after the translation.
///
/// Engine pane mirrors are **not** updated here; `sync_all_pane_mirrors` in
/// the next `prepare_frame` handles that. Only the authoritative `SelectionSet`
/// in `pane_state` must be kept valid between edits, because other
/// mid-event code (e.g. `update_pane_cursor`) reads it.
pub(crate) fn propagate_cs_to_panes(
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    focused_pane_id: PaneId,
    buf_id: BufferId,
    cs: &ChangeSet,
    buf_pre: &hume_editing::text::Text,
) {
    // Collect IDs first; can't iterate and mutate the same SecondaryMap.
    let affected: Vec<PaneId> = pane_state
        .iter()
        .filter_map(|(pid, buf_map)| {
            (pid != focused_pane_id && buf_map.contains_key(buf_id)).then_some(pid)
        })
        .collect();
    for pid in affected {
        pane_state[pid][buf_id]
            .selections
            .translate_in_place(cs, buf_pre);
    }
}
