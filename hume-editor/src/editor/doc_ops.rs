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
use crate::editor::buffer::{Buffer, HistoryWalkResult};
use crate::editor::edit_session::{EditSession, EditSessionKind};
use crate::editor::error::CommandError;
use crate::editor::jump_list::JumpLists;
use crate::editor::pane_state::PaneBufferState;
use hume_decorations::DecorationStores;
use hume_editing::changeset::ChangeSet;
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use hume_rope::offset::{CharOffset, ExclusiveRange};

/// [`apply_doc_history_walk`]'s result — keeps a read-only refusal
/// distinguishable from genuine root/leaf exhaustion. Both used to collapse
/// to `0`, which is safe only because every current caller
/// (`history_step`) already calls `refuse_if_read_only` first; a caller that
/// leans on this function's own guard alone (the production `goto-revision`
/// `docs/UNDOTREE.md` plans) would otherwise report "Already at oldest
/// change" for a read-only buffer — a wrong diagnosis sending the user to
/// look for missing history that was never there to find.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum HistoryWalk {
    /// The buffer is read-only; nothing was attempted.
    RefusedReadOnly,
    /// The walk ran and took this many steps — short of the requested count
    /// at the root/leaf.
    Took(usize),
}

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
/// edit seq, write `new_sels` back, propagate `cs` to sibling panes and every
/// pane's jump list, and feed both the syntax and LSP/decoration remap
/// streams. A path that forgets one of these steps would silently drift
/// decorations or leave a stale syntax tree, with no compile error — so this
/// is the one place that sequence is spelled out.
///
/// The first six parameters are the same threading sextet every function in
/// this file already receives. Four of them (`buffers`, `decorations`,
/// `pane_state`, `pane_jumps`) are fields reachable from a single
/// `&EditorState`/`&mut EditorState` — `decorations` through `state.config`,
/// the rest directly. `pane_id` and `buf_id` are not: `buf_id` in
/// particular is derived from `EngineView` (`view.panes[pane_id]
/// .buffer_id`), which this function doesn't receive, so collapsing the
/// other four alone wouldn't shrink this list — every caller would still
/// need to pass `buf_id` (and thus keep `view` in scope to compute it). The
/// last four genuinely can't collapse the same way: none is an `EditorState`
/// field, and `text_pre`/`rope_pre` are pre-mutation snapshots this function
/// couldn't re-derive from `state.buffers` even if it wanted to, since it
/// runs after the mutation.
#[allow(clippy::too_many_arguments)]
fn finish_edit(
    buffers: &mut BufferStore,
    decorations: &DecorationStores,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pane_jumps: &mut JumpLists,
    pane_id: PaneId,
    buf_id: BufferId,
    new_sels: SelectionSet,
    cs: &ChangeSet,
    text_pre: &BufferText,
    rope_pre: &ropey::Rope,
) {
    pane_state[pane_id][buf_id].set_selections(new_sels);
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
    // Computed once and shared by both propagation steps below — each would
    // otherwise rebuild the same `Vec` from `cs` (once per sibling pane here,
    // once per pane per jump-list entry there).
    let edits = cs.edited_old_ranges();
    propagate_cs_to_panes(pane_state, pane_id, buf_id, &edits, cs, text_pre);
    let buf = buffers.get(buf_id);
    let text_gen = buf.text_gen;
    pane_jumps.translate(buf_id, &edits, cs, text_pre, buf.text());
    record_syntax_edits(buffers, buf_id, text_gen, cs, rope_pre);
    record_lsp_edits(buffers, decorations, buf_id, text_gen, cs, rope_pre);
}

/// `Err` when [`EditorState::active_session`](crate::editor::EditorState::active_session)
/// holds a session on `buf_id` opened by some pane other than `exclude` — the
/// exclusivity check every buffer-mutating chokepoint below runs before
/// touching `buf_id`'s text.
///
/// Remote dispatch (`(call! "cmd" pane)` targeting a pane other than the
/// focused one) means a *different* pane's own in-progress insert/paste
/// session can otherwise be mutated out from under it: its eventual commit
/// inverts the composed `ChangeSet` against a `text_snapshot` a concurrent
/// edit from another pane would make stale, and its next grouped edit panics
/// in `ChangeSet::compose`'s length assert. Scoped to "some *other* pane" —
/// not any open session at all — because the focused/target pane's own open
/// session is exactly what routes an edit into [`apply_doc_edit_grouped`]
/// below, a distinct, unrelated case this check must not shadow.
///
/// [`apply_doc_edit`] runs this right before committing;
/// [`super::lsp::edits::apply_workspace_edit`] runs it for every file in its
/// plan, before any file's commit, so a conflict on file *k* of a multi-file
/// edit is caught before files `0..k` are touched rather than partway
/// through the commit loop.
pub(in crate::editor) fn check_no_conflicting_session(
    active_session: &Option<EditSession>,
    exclude: PaneId,
    buf_id: BufferId,
) -> Result<(), CommandError> {
    if active_session
        .as_ref()
        .is_some_and(|s| s.pane != exclude && s.buffer == buf_id)
    {
        return Err(CommandError::new(
            "buffer has an open insert/paste session on another pane",
        ));
    }
    Ok(())
}

/// Apply an edit to `buf_id` through `pane_id` and propagate the resulting
/// `ChangeSet` to all other panes viewing the same buffer.
///
/// Routes into [`apply_doc_edit_grouped`] when an Insert session is already
/// open on this (pane, buffer) — dot-repeat replay, or any edit applied
/// mid-session (e.g. an LSP completion accept) must compose into that group
/// rather than record a standalone undo revision; the two would otherwise go
/// out of sync and the next grouped edit's `ChangeSet::compose` panics on a
/// length mismatch. This is the single chokepoint every edit-applying caller
/// goes through, so no caller needs its own open-session check.
///
/// `Err` when [`check_no_conflicting_session`] finds one — refusing loudly
/// beats the alternative of silently mutating the buffer underneath another
/// pane's open session (see that function's own doc).
///
/// Uses `std::mem::take` on the active `SelectionSet` instead of `clone()`.
/// The default state (cursor-at-0) is transient: it is overwritten by
/// `new_sels` before this function returns.
// Same non-collapsible-params shape as `finish_edit`'s own allow, above.
#[allow(clippy::too_many_arguments)]
pub(in crate::editor) fn apply_doc_edit(
    buffers: &mut BufferStore,
    decorations: &DecorationStores,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pane_jumps: &mut JumpLists,
    active_session: &mut Option<EditSession>,
    pane_id: PaneId,
    buf_id: BufferId,
    cmd: impl FnOnce(BufferText, SelectionSet) -> (BufferText, SelectionSet, ChangeSet),
) -> Result<(), CommandError> {
    if buffers.get(buf_id).is_read_only() {
        return Ok(());
    }
    check_no_conflicting_session(active_session, pane_id, buf_id)?;
    let insert_session_open_here = active_session
        .as_ref()
        .is_some_and(|s| s.is_insert_at(pane_id, buf_id));
    if insert_session_open_here {
        apply_doc_edit_grouped(
            buffers,
            decorations,
            pane_state,
            pane_jumps,
            active_session,
            pane_id,
            buf_id,
            cmd,
        );
        return Ok(());
    }
    // O(1) clones — ropey uses structural sharing (reference-counted tree nodes).
    let text_pre = buffers.get(buf_id).text().clone();
    let rope_pre = text_pre.rope().clone();
    let sels = pane_state[pane_id][buf_id].take_selections();
    let (new_sels, cs) = buffers.get_mut(buf_id).apply_edit(sels, cmd);
    finish_edit(
        buffers,
        decorations,
        pane_state,
        pane_jumps,
        pane_id,
        buf_id,
        new_sels,
        &cs,
        &text_pre,
        &rope_pre,
    );
    Ok(())
}

/// Apply a grouped edit (inside an Insert session) to `buf_id` through `pane_id`.
///
/// Reads and writes selections via `pane_state`, propagates `cs` to other panes.
///
/// Uses `std::mem::take` on the active `SelectionSet` instead of `clone()`.
/// `apply_edit_grouped` is infallible, so no panic can leave the set in its
/// default state.
///
/// Returns the applied `ChangeSet` — `input_stack/insert.rs`'s `apply_insert_edit`
/// uses it to remap an open LSP completion session's anchor through every
/// keystroke, not just the primary cursor's own position.
///
/// `active_session` must hold an Insert-kind session on `(pane_id,
/// buf_id)` — caller contract, same as the old `Buffer::apply_edit_grouped`'s
/// "must have called `begin_edit_group` first". Panics otherwise.
// Same non-collapsible-params shape as `finish_edit`'s own allow, above.
#[allow(clippy::too_many_arguments)]
pub(in crate::editor) fn apply_doc_edit_grouped(
    buffers: &mut BufferStore,
    decorations: &DecorationStores,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pane_jumps: &mut JumpLists,
    active_session: &mut Option<EditSession>,
    pane_id: PaneId,
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
    let sels = pane_state[pane_id][buf_id].take_selections();
    let doc = buffers.get_mut(buf_id);
    let group = &mut active_session
        .as_mut()
        .filter(|s| s.is_insert_at(pane_id, buf_id))
        .expect(
            "apply_doc_edit_grouped called without an open Insert session on this (pane, buffer)",
        )
        .group;
    let (new_sels, cs) = doc.apply_edit_grouped(sels, group, cmd);
    let pbs = &mut pane_state[pane_id][buf_id];
    // `ChangeSet::map_ranges` maps (start, end) pairs directly, but with
    // `Assoc::After` on starts and `Assoc::Before` on ends — it shrinks a
    // range around inserted text. A typed run needs the opposite: it must
    // grow to include what was just typed, so anchors and ends are mapped
    // separately with `Assoc` reversed from what `map_ranges` would use.
    if let Some(run) = pbs.typed_run.as_mut() {
        cs.map_positions(&mut run.anchors, hume_editing::changeset::Assoc::Before);
        cs.map_positions(&mut run.ends, hume_editing::changeset::Assoc::After);
    }
    // Shrinks each record around any edit landing exactly at its start/end —
    // see `map_ranges`' own doc. That is what makes ownership self-revoking:
    // text typed past a record's end, or a line split before its start, falls
    // outside the mapped range without any key handler needing to clear it.
    if let Some(ranges) = pbs.autoindent.as_mut() {
        cs.map_ranges(ranges);
    }
    finish_edit(
        buffers,
        decorations,
        pane_state,
        pane_jumps,
        pane_id,
        buf_id,
        new_sels,
        &cs,
        &text_pre,
        &rope_pre,
    );
    cs
}

/// Re-paste from the paste-session snapshot into `buf_id`, replacing
/// the accumulated CS in the open session's group.
///
/// Propagates the resulting CS (mapping current text → new text) to all other
/// panes. `active_session` must hold a Paste-kind session on
/// `(pane_id, buf_id)` — caller must have opened it via
/// `commands::paste`'s `do_paste` first. Panics otherwise.
// Same non-collapsible-params shape as `finish_edit`'s own allow, above.
#[allow(clippy::too_many_arguments)]
pub(in crate::editor) fn apply_doc_edit_regrouped(
    buffers: &mut BufferStore,
    decorations: &DecorationStores,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pane_jumps: &mut JumpLists,
    active_session: &mut Option<EditSession>,
    pane_id: PaneId,
    buf_id: BufferId,
    cmd: impl FnOnce(BufferText, SelectionSet) -> (BufferText, SelectionSet, ChangeSet),
) {
    if buffers.get(buf_id).is_read_only() {
        return;
    }
    let text_pre = buffers.get(buf_id).text().clone();
    let rope_pre = text_pre.rope().clone();
    let group = &mut active_session
        .as_mut()
        .filter(|s| {
            s.pane == pane_id
                && s.buffer == buf_id
                && matches!(s.kind, EditSessionKind::Paste { .. })
        })
        .expect(
            "apply_doc_edit_regrouped called without an open paste session on this (pane, buffer)",
        )
        .group;
    let (new_sels, propagation_cs) = buffers.get_mut(buf_id).apply_edit_regrouped(group, cmd);
    finish_edit(
        buffers,
        decorations,
        pane_state,
        pane_jumps,
        pane_id,
        buf_id,
        new_sels,
        &propagation_cs,
        &text_pre,
        &rope_pre,
    );
}

/// Run one history-walking step on `buf_id` through `pane_id` (`walk`, typically
/// `|b| b.undo_n(count)` or `|b| b.redo_n(count)`) and propagate the net
/// `ChangeSet` to all other panes viewing the same buffer. `walk` closes over
/// its own step count or target revision, so this function stays the single
/// entry point for every shape a history walk takes — a count-based
/// `undo_n`/`redo_n` today, a `RevisionId`-based `goto_revision` walk
/// tomorrow — with no second copy of the guard, the pre-walk snapshot, or
/// the `finish_edit` call.
///
/// A single `finish_edit` call for the whole walk, however many revisions it
/// crosses — `walk` itself already composed those into one net transform
/// (see `Buffer::apply_transactions`), so panes, jump lists, tree-sitter, and
/// LSP each see one edit instead of `count` of them. This is what makes an
/// age-resolved `:earlier`/`:later` (an unbounded step count) as cheap as a
/// single `u`.
///
/// Returns [`HistoryWalk::Took`] with the number of steps actually taken —
/// short of `count` when the walk hit the root/leaf, so the caller can
/// report exhaustion — or [`HistoryWalk::RefusedReadOnly`] when the buffer
/// refused the walk outright; see that type's doc for why the two must stay
/// distinguishable.
///
/// `Err` when [`check_no_conflicting_session`] finds another pane's open
/// session on this buffer — same exclusivity rule [`apply_doc_edit`]
/// enforces, upgraded here from a debug-only assert (which checked only the
/// walking pane's own group) to a real, release-mode, buffer-wide refusal.
// Same non-collapsible-params shape as `finish_edit`'s own allow, above.
#[allow(clippy::too_many_arguments)]
pub(in crate::editor) fn apply_doc_history_walk(
    buffers: &mut BufferStore,
    decorations: &DecorationStores,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pane_jumps: &mut JumpLists,
    active_session: &Option<EditSession>,
    pane_id: PaneId,
    buf_id: BufferId,
    walk: impl FnOnce(&mut Buffer) -> HistoryWalkResult,
) -> Result<HistoryWalk, CommandError> {
    if buffers.get(buf_id).is_read_only() {
        return Ok(HistoryWalk::RefusedReadOnly);
    }
    check_no_conflicting_session(active_session, pane_id, buf_id)?;
    debug_assert!(
        !active_session
            .as_ref()
            .is_some_and(|s| s.pane == pane_id && s.buffer == buf_id),
        "apply_doc_history_walk called while a session is open on this (pane, buffer)"
    );
    // text_pre is the current (pre-walk) text: undo's CS maps post-edit
    // positions back to pre-edit, so non-acting panes' heads must be
    // translated through that CS.
    let text_pre = buffers.get(buf_id).text().clone();
    let Some((new_sels, cs, steps)) = walk(buffers.get_mut(buf_id)) else {
        return Ok(HistoryWalk::Took(0));
    };
    finish_edit(
        buffers,
        decorations,
        pane_state,
        pane_jumps,
        pane_id,
        buf_id,
        new_sels,
        &cs,
        &text_pre,
        text_pre.rope(),
    );
    // `finish_edit` skips `bump_edit_seq` for an identity `cs` (correctly —
    // a normal edit that cancels to identity records no revision at all, so
    // nothing happened). A history walk is different: `current` moved to a
    // real revision and the selections moved with it even when the net text
    // didn't, so any paste session live before this walk is over. Bumped
    // here rather than in `finish_edit` itself, since that guard still needs
    // to hold for every *other* caller.
    if cs.is_identity() {
        buffers.bump_edit_seq();
    }
    Ok(HistoryWalk::Took(steps))
}

/// Apply a motion function and store the resulting selection in `pane_state`.
///
/// Uses `std::mem::take` on the active `SelectionSet` instead of `clone()`;
/// the default state is transient and overwritten before this fn returns.
/// The closure `f` is assumed infallible; a panic mid-motion leaves
/// `selections` as `Default` (cursor at 0).
pub(in crate::editor) fn apply_doc_motion(
    buffers: &BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pane_id: PaneId,
    buf_id: BufferId,
    f: impl FnOnce(&BufferText, SelectionSet) -> SelectionSet,
) {
    let old_head = pane_state[pane_id][buf_id].selections().primary().head();
    let new_sels = {
        let text = buffers.get(buf_id).text();
        let sels = pane_state[pane_id][buf_id].take_selections();
        f(text, sels)
    };
    pane_state[pane_id][buf_id].restore_selections(new_sels, old_head);
}

/// Open an Insert-kind session on `(pane_id, buf_id)` — always the focused
/// pane, since Insert only ever opens there. The
/// dedicated paste-session opener (`commands::paste`'s `do_paste`) builds
/// its own `EditSession` directly instead of calling this — a paste session
/// stores `before` in its `EditSessionKind::Paste`, which this function has
/// no parameter for.
///
/// Snapshots the current selections (via `.clone()`) for use as `pre_sels`
/// in the recorded undo revision — the field must NOT be taken because the
/// ongoing insert session continues to read it between keystrokes.
///
/// Panics (debug) if a session is already open — caller contract, same as
/// the old `Buffer::begin_edit_group`'s.
pub(in crate::editor) fn begin_edit_group(
    buffers: &BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    active_session: &mut Option<EditSession>,
    pane_id: PaneId,
    buf_id: BufferId,
) {
    debug_assert!(
        active_session.is_none(),
        "begin_edit_group called with a session already open"
    );
    let sels = pane_state[pane_id][buf_id].selections().clone();
    let group = buffers.get(buf_id).begin_edit_group(sels);
    *active_session = Some(EditSession {
        pane: pane_id,
        buffer: buf_id,
        kind: EditSessionKind::Insert,
        group,
    });
    // A fresh group never inherits a typed run, an autoindent record, or exit
    // flags from a previous session (interactive or replay-preopened).
    let pbs = &mut pane_state[pane_id][buf_id];
    pbs.typed_run = None;
    pbs.autoindent = None;
    pbs.step_back_on_exit = false;
    pbs.kill_opened_session = false;
}

/// Close the open Insert-kind session and record it as a single undo step on
/// the session's own `(pane, buffer)` — read from the session, never from
/// focus, so the commit lands correctly however focus moved in the meantime.
///
/// Snapshots the current selections as `post_sels` for the undo revision;
/// same rationale as `begin_edit_group` — must `.clone()`, not `take`.
///
/// Panics if no session is open, or if the open one is a paste session —
/// committing a paste group as an Insert revision would record the wrong
/// undo step silently. Mirrors `EditorState::commit_paste_session`'s own
/// kind check.
pub(in crate::editor) fn commit_edit_group(
    buffers: &mut BufferStore,
    pane_state: &SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    active_session: &mut Option<EditSession>,
) {
    let session = active_session
        .take()
        .expect("commit_edit_group called without an open session");
    assert!(
        matches!(session.kind, EditSessionKind::Insert),
        "commit_edit_group called on a paste session"
    );
    let sels = pane_state[session.pane][session.buffer]
        .selections()
        .clone();
    buffers
        .get_mut(session.buffer)
        .commit_edit_group(session.group, sels);
}

/// Propagate `cs` to every pane except `pane_id` that views `buf_id`,
/// keeping their selections valid after an edit `pane_id` performed.
///
/// `text_pre` must be the buffer text **before** the edit — `translate_in_place_with`
/// uses it to identify which line each head was on pre-edit, which governs
/// whether `Selection.sticky_display_col` is reset after the translation.
/// `edits` must be `cs.edited_old_ranges()`; see `finish_edit`, this
/// function's one caller, for why it's computed there instead of here.
///
/// Engine pane mirrors are **not** updated here; `sync_all_pane_mirrors` in
/// the next `prepare_frame` handles that. Only the authoritative `SelectionSet`
/// in `pane_state` must be kept valid between edits, because other
/// mid-event code (e.g. `update_pane_cursor`) reads it.
pub(in crate::editor::doc_ops) fn propagate_cs_to_panes(
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pane_id: PaneId,
    buf_id: BufferId,
    edits: &[ExclusiveRange<CharOffset>],
    cs: &ChangeSet,
    text_pre: &hume_editing::text::BufferText,
) {
    // Collect IDs first; can't iterate and mutate the same SecondaryMap.
    let affected: Vec<PaneId> = pane_state
        .iter()
        .filter_map(|(pid, buf_map)| {
            (pid != pane_id && buf_map.contains_key(buf_id)).then_some(pid)
        })
        .collect();
    for pid in affected {
        pane_state[pid][buf_id].translate_selections_in_place(edits, cs, text_pre);
    }
}
