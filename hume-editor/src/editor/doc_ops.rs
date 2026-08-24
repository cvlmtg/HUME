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
use hume_editing::text::BufferText;

/// Shared signature of [`apply_doc_undo`] and [`apply_doc_redo`] — lets a
/// caller (e.g. `commands/edit.rs`'s `history_step`) pick one by function
/// pointer instead of duplicating the call site per direction.
pub(crate) type ApplyDocFn = fn(
    &mut BufferStore,
    &DecorationStores,
    &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    PaneId,
    BufferId,
);

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
        syn.record_edit(text_gen, cs, rope_pre);
    }
}

/// No-op when `buf_id` has no LSP server attached and no decorations, of
/// any kind, that need to stay in sync with edits — decorations are not
/// LSP-owned, LSP is just their first client, so a buffer with e.g.
/// `set-signs!`/`set-inlay-hints!` data but no attached server still needs
/// its edits queued here. Called immediately after every text mutation,
/// alongside `record_syntax_edits` — same chokepoint, same "text changed,
/// notify the machinery" shape, queued for the LSP per-frame flush
/// (`Editor::flush_lsp_pending_changes`, which also does the decoration
/// remap) instead of dispatched inline.
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

/// Shared post-mutation bookkeeping for every text-mutating path: bump the
/// edit seq, write `new_sels` back, propagate `cs` to sibling panes, and feed
/// both the syntax and LSP/decoration remap streams. A path that forgets one
/// of these steps would silently drift decorations or leave a stale syntax
/// tree, with no compile error — so this is the one place that sequence is
/// spelled out.
///
/// The first five parameters are the same threading quintet every function
/// in this file already receives; the last four are each caller's own
/// pre/post-edit state. Bundling either group into a struct would only move
/// the field list, not shrink it.
#[allow(clippy::too_many_arguments)]
fn finish_edit(
    buffers: &mut BufferStore,
    decorations: &DecorationStores,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    focused_pane_id: PaneId,
    buf_id: BufferId,
    new_sels: SelectionSet,
    cs: &ChangeSet,
    text_pre: &BufferText,
    rope_pre: &ropey::Rope,
) {
    pane_state[focused_pane_id][buf_id].selections = new_sels;
    // An identity `cs` moved no bytes: `Buffer::apply_edit*` skipped
    // `set_text` for it directly, and `commit_edit_group` never records it as
    // a revision for `undo`/`redo` to later replay — so `text_gen` did not
    // move either way. Feeding the syntax and LSP streams an edit tagged with
    // an already-parsed generation would be actively wrong, and paste-stamping
    // must not count a no-op as an edit. Selections are still written above —
    // a no-op edit can still move cursors.
    if cs.is_identity() {
        return;
    }
    buffers.bump_edit_seq();
    propagate_cs_to_panes(pane_state, focused_pane_id, buf_id, cs, text_pre);
    let text_gen = buffers.get(buf_id).text_gen;
    record_syntax_edits(buffers, buf_id, text_gen, cs, rope_pre);
    record_lsp_edits(buffers, decorations, buf_id, text_gen, cs, rope_pre);
}

/// Apply an edit to the focused buffer and propagate the resulting
/// `ChangeSet` to all other panes viewing the same buffer.
///
/// Routes into [`apply_doc_edit_grouped`] when an edit group is already open
/// on this (pane, buffer) — an insert session, dot-repeat replay, or any
/// edit applied mid-session (e.g. an LSP completion accept) must compose
/// into that group rather than record a standalone undo revision; the two
/// would otherwise go out of sync and the next grouped edit's
/// `ChangeSet::compose` panics on a length mismatch. This is the single
/// chokepoint every edit-applying caller goes through, so no caller needs
/// its own open-group check.
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
    cmd: impl FnOnce(BufferText, SelectionSet) -> (BufferText, SelectionSet, ChangeSet),
) {
    if buffers.get(buf_id).is_read_only() {
        return;
    }
    if pane_state[focused_pane_id][buf_id].edit_group.is_some() {
        apply_doc_edit_grouped(
            buffers,
            decorations,
            pane_state,
            focused_pane_id,
            buf_id,
            cmd,
        );
        return;
    }
    // O(1) clones — ropey uses structural sharing (reference-counted tree nodes).
    let text_pre = buffers.get(buf_id).text().clone();
    let rope_pre = text_pre.rope().clone();
    let sels = std::mem::take(&mut pane_state[focused_pane_id][buf_id].selections);
    let (new_sels, cs) = buffers.get_mut(buf_id).apply_edit(sels, cmd);
    finish_edit(
        buffers,
        decorations,
        pane_state,
        focused_pane_id,
        buf_id,
        new_sels,
        &cs,
        &text_pre,
        &rope_pre,
    );
}

/// Apply a grouped edit (inside an insert session) to the focused buffer.
///
/// Reads and writes selections via `pane_state`, propagates `cs` to other panes.
///
/// Uses `std::mem::take` on the active `SelectionSet` instead of `clone()`.
/// `apply_edit_grouped` is infallible, so no panic can leave the set in its
/// default state.
///
/// Returns the applied `ChangeSet` — `mappings/insert.rs`'s `apply_insert_edit`
/// uses it to remap an open LSP completion session's anchor through every
/// keystroke, not just the primary cursor's own position.
pub(crate) fn apply_doc_edit_grouped(
    buffers: &mut BufferStore,
    decorations: &DecorationStores,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    focused_pane_id: PaneId,
    buf_id: BufferId,
    cmd: impl FnOnce(BufferText, SelectionSet) -> (BufferText, SelectionSet, ChangeSet),
) -> ChangeSet {
    if buffers.get(buf_id).is_read_only() {
        // Identity changeset: a read-only buffer means nothing happened, so a
        // caller remapping other state through the result must see a no-op,
        // not a stale/mismatched-length one.
        return ChangeSet::identity(buffers.get(buf_id).text().len_chars());
    }
    let text_pre = buffers.get(buf_id).text().clone();
    let rope_pre = text_pre.rope().clone();
    let sels = std::mem::take(&mut pane_state[focused_pane_id][buf_id].selections);
    let doc = buffers.get_mut(buf_id);
    let pbs = &mut pane_state[focused_pane_id][buf_id];
    let (new_sels, cs) = doc.apply_edit_grouped(sels, &mut pbs.edit_group, cmd);
    if let Some(anchors) = pbs.pinned_anchors.as_mut() {
        cs.map_positions(anchors, hume_editing::changeset::Assoc::Before);
    }
    finish_edit(
        buffers,
        decorations,
        pane_state,
        focused_pane_id,
        buf_id,
        new_sels,
        &cs,
        &text_pre,
        &rope_pre,
    );
    cs
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
    cmd: impl FnOnce(BufferText, SelectionSet) -> (BufferText, SelectionSet, ChangeSet),
) {
    if buffers.get(buf_id).is_read_only() {
        return;
    }
    let text_pre = buffers.get(buf_id).text().clone();
    let rope_pre = text_pre.rope().clone();
    // Borrow paste_group from pane_state; NLL ends this borrow after the call.
    let pbs = &mut pane_state[focused_pane_id][buf_id];
    let (new_sels, propagation_cs) = buffers
        .get_mut(buf_id)
        .apply_edit_regrouped(&mut pbs.paste_group, cmd);
    finish_edit(
        buffers,
        decorations,
        pane_state,
        focused_pane_id,
        buf_id,
        new_sels,
        &propagation_cs,
        &text_pre,
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
    debug_assert!(
        pane_state[focused_pane_id][buf_id].edit_group.is_none(),
        "apply_doc_undo called while an edit group is open on this buffer"
    );
    // text_pre/rope_pre are the current (post-edit) text: undo's CS maps
    // post-edit positions back to pre-edit, so non-acting panes' heads must be
    // translated through that CS.
    let text_pre = buffers.get(buf_id).text().clone();
    let rope_pre = text_pre.rope().clone();
    if let Some((new_sels, cs)) = buffers.get_mut(buf_id).undo() {
        finish_edit(
            buffers,
            decorations,
            pane_state,
            focused_pane_id,
            buf_id,
            new_sels,
            &cs,
            &text_pre,
            &rope_pre,
        );
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
    debug_assert!(
        pane_state[focused_pane_id][buf_id].edit_group.is_none(),
        "apply_doc_redo called while an edit group is open on this buffer"
    );
    let text_pre = buffers.get(buf_id).text().clone();
    let rope_pre = text_pre.rope().clone();
    if let Some((new_sels, cs)) = buffers.get_mut(buf_id).redo() {
        finish_edit(
            buffers,
            decorations,
            pane_state,
            focused_pane_id,
            buf_id,
            new_sels,
            &cs,
            &text_pre,
            &rope_pre,
        );
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
    f: impl FnOnce(&BufferText, SelectionSet) -> SelectionSet,
) {
    let new_sels = {
        let text = buffers.get(buf_id).text();
        let sels = std::mem::take(&mut pane_state[focused_pane_id][buf_id].selections);
        f(text, sels)
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
    // A fresh group never inherits pins (or the select-on-exit/kill-opened
    // flags) from a previous session (interactive or replay-preopened).
    pbs.pinned_anchors = None;
    pbs.select_on_exit = false;
    pbs.kill_opened_session = false;
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
/// `text_pre` must be the buffer text **before** the edit — `translate_in_place`
/// uses it to identify which line each head was on pre-edit, which governs
/// whether `Selection.sticky_display_col` is reset after the translation.
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
    text_pre: &hume_editing::text::BufferText,
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
            .translate_in_place(cs, text_pre);
    }
}
