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
use crate::editor::edit_session::{self, EditSession, EditSessionKind};
use crate::editor::error::CommandError;
use crate::editor::jump_list::JumpLists;
use crate::editor::pane_state::PaneBufferState;
use hume_decorations::DecorationStores;
use hume_editing::changeset::ChangeSet;
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use hume_rope::offset::{CharOffset, ExclusiveRange};

/// [`apply_doc_history_walk`]'s result: keeps a read-only refusal
/// distinguishable from genuine root/leaf exhaustion. Collapsing both to
/// `0` would be safe only because every current caller
/// (`history_step`) already calls `refuse_if_read_only` first; a caller that
/// leans on this function's own guard alone (the production `goto-revision`
/// `docs/UNDOTREE.md` plans) would then report "Already at oldest
/// change" for a read-only buffer, a wrong diagnosis sending the user to
/// look for missing history that was never there to find.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum HistoryWalk {
    /// The buffer is read-only; nothing was attempted.
    RefusedReadOnly,
    /// The walk ran and took this many steps, short of the requested count
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
/// any kind, that need to stay in sync with edits. Decorations are not
/// LSP-owned, LSP is just their first client, so a buffer with e.g.
/// `set-signs!`/`set-inlay-hints!` data but no attached server still needs
/// its edits queued here. Called immediately after every text mutation,
/// alongside `record_syntax_edits`: same chokepoint, same "text changed,
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
/// decorations or leave a stale syntax tree, with no compile error, so this
/// is the one place that sequence is spelled out.
///
/// The first six parameters are the same threading sextet every function in
/// this file already receives. Four of them (`buffers`, `decorations`,
/// `pane_state`, `pane_jumps`) are fields reachable from a single
/// `&EditorState`/`&mut EditorState`: `decorations` through `state.config`,
/// the rest directly. `pane_id` and `buf_id` are not: `buf_id` in
/// particular is derived from `EngineView` (`view.panes[pane_id]
/// .buffer_id`), which this function doesn't receive, so collapsing the
/// other four alone wouldn't shrink this list. Every caller would still
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
    pane_state[pane_id][buf_id].set_selections(new_sels, buffers.get(buf_id).text());
    // An identity `cs` moved no bytes: `Buffer::apply_edit*` skipped
    // `set_text` for it directly, and `commit_edit_group` never records it as
    // a revision for `undo`/`redo` to later replay, so `text_gen` did not
    // move either way. Feeding the syntax and LSP streams an edit tagged with
    // an already-parsed generation would be actively wrong, and paste-stamping
    // must not count a no-op as an edit. Selections are still written above:
    // a no-op edit can still move cursors.
    if cs.is_identity() {
        return;
    }
    // A real edit reveals the acting pane even when it didn't move the
    // primary head (`r` replacing the character under the cursor): the
    // selection funnel above only raises this for a head move, but the
    // buffer under this pane just changed regardless.
    pane_state[pane_id][buf_id].reveal_pending = true;
    buffers.bump_edit_seq();
    // Computed once and shared by both propagation steps below. Each would
    // otherwise rebuild the same `Vec` from `cs` (once per sibling pane here,
    // once per pane per jump-list entry there).
    let edits = cs.edited_old_ranges();
    let buf = buffers.get(buf_id);
    propagate_cs_to_panes(
        pane_state,
        pane_id,
        buf_id,
        &edits,
        cs,
        text_pre,
        buf.text(),
    );
    let text_gen = buf.text_gen;
    pane_jumps.translate(buf_id, &edits, cs, text_pre, buf.text());
    record_syntax_edits(buffers, buf_id, text_gen, cs, rope_pre);
    record_lsp_edits(buffers, decorations, buf_id, text_gen, cs, rope_pre);
}

/// `Err` when [`EditorState::active_session`](crate::editor::EditorState::active_session)
/// holds a session on `buf_id` opened by some pane other than `exclude`: the
/// exclusivity check every buffer-mutating chokepoint below runs before
/// touching `buf_id`'s text.
///
/// Remote dispatch (`(call! "cmd" pane)` targeting a pane other than the
/// focused one) means a *different* pane's own in-progress insert/paste
/// session can otherwise be mutated out from under it: its eventual commit
/// inverts the composed `ChangeSet` against a `text_snapshot` a concurrent
/// edit from another pane would make stale, and its next grouped edit panics
/// in `ChangeSet::compose`'s length assert. Scoped to "some *other* pane"
/// (not any open session at all) because the focused/target pane's own open
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
        .is_some_and(|s| s.pane() != exclude && s.buffer() == buf_id)
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
/// Every edit-applying caller goes through here, so session handling lives
/// here rather than at each caller:
/// - An Insert session open on this (pane, buffer) routes the edit into
///   [`apply_doc_edit_grouped`]. A standalone revision would desync the group
///   and make the next `ChangeSet::compose` panic on a length mismatch.
/// - A Paste session open on this (pane, buffer) is committed first via
///   [`commit_paste_group`]. Otherwise its `text_snapshot` goes stale and the
///   next `[`/`]` re-paste discards this edit. Callers that bypass key
///   dispatch (and so `step_paste_commit`) rely on this.
///
/// `Err` when [`check_no_conflicting_session`] finds another pane's session on
/// this buffer.
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
    // Scoped to this exact (pane, buffer): `commit_paste_group` alone would
    // commit *any* open Paste session, but `check_no_conflicting_session`
    // above only refused a different pane's session on `buf_id`. An
    // unrelated Paste session still open on the focused pane, for a
    // *different* buffer, must not be ended early just because a remote
    // `call!` is editing something else through a background pane.
    if active_session
        .as_ref()
        .is_some_and(|s| s.owned_by(pane_id, buf_id))
    {
        commit_paste_group(buffers, pane_state, active_session);
    }
    // O(1) clones: ropey uses structural sharing (reference-counted tree nodes).
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
/// Returns the applied `ChangeSet`. `input_stack/insert.rs`'s `apply_insert_edit`
/// uses it to remap an open LSP completion session's anchor through every
/// keystroke, not just the primary cursor's own position.
///
/// `active_session` must hold an Insert-kind session on `(pane_id,
/// buf_id)`, a caller contract, same as `Buffer::apply_edit_grouped`'s own
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
    // Bound once, reused below for the capture-feed push too: both need
    // the same `is_insert_at`-filtered session.
    let session = active_session
        .as_mut()
        .filter(|s| s.is_insert_at(pane_id, buf_id))
        .expect(
            "apply_doc_edit_grouped called without an open Insert session on this (pane, buffer)",
        );
    let (new_sels, cs) = doc.apply_edit_grouped(sels, session.group_mut(), cmd);
    let pbs = &mut pane_state[pane_id][buf_id];
    // `ChangeSet::map_ranges` maps (start, end) pairs directly, but with
    // `Assoc::After` on starts and `Assoc::Before` on ends: it shrinks a
    // range around inserted text. A typed run needs the opposite: it must
    // grow to include what was just typed, so anchors and ends are mapped
    // separately with `Assoc` reversed from what `map_ranges` would use.
    if let Some(run) = pbs.typed_run.as_mut() {
        cs.map_positions(&mut run.anchors, hume_editing::changeset::Assoc::Before);
        cs.map_positions(&mut run.ends, hume_editing::changeset::Assoc::After);
    }
    // Shrinks each record around any edit landing exactly at its start/end;
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
    // This is the one funnel every grouped edit goes through, so it's the
    // one place `DotCapture::edits` (`edit_session.rs`) can be fed without
    // every caller remembering to do it itself; see that type's own doc.
    if let Some(cap) = session.dot_capture_mut() {
        cap.edits.push(cs.clone());
        // `session.group_mut()`'s own `apply_edit_grouped` call above
        // already bumped this via `Buffer::set_text`; see `DotCapture::
        // text_gen`'s own doc for what this guards.
        cap.text_gen = buffers.get(buf_id).text_gen;
    }
    cs
}

/// Re-paste from the paste-session snapshot into `buf_id`, replacing
/// the accumulated CS in the open session's group.
///
/// Propagates the resulting CS (mapping current text → new text) to all other
/// panes. `active_session` must hold a Paste-kind session on
/// `(pane_id, buf_id)`. The caller must have opened it via
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
    let group = active_session
        .as_mut()
        .filter(|s| s.is_paste_at(pane_id, buf_id))
        .expect(
            "apply_doc_edit_regrouped called without an open paste session on this (pane, buffer)",
        )
        .group_mut();
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

/// Run one history walk on `buf_id` through `pane_id` (`walk`, typically
/// `|b| b.undo_n(count)` or `|b| b.redo_n(count)`) and propagate the net
/// `ChangeSet` to all other panes viewing the same buffer.
///
/// `walk` already composes every revision it crosses into one transform (see
/// `Buffer::apply_transactions`), so there is a single `finish_edit` call and
/// panes, jump lists, tree-sitter, and LSP see one edit. An unbounded
/// `:earlier`/`:later` costs the same as one `u`.
///
/// Returns [`HistoryWalk::Took`] with the steps actually taken (fewer than
/// requested at the root/leaf) or [`HistoryWalk::RefusedReadOnly`]. `Err`
/// when [`check_no_conflicting_session`] finds another pane's session on this
/// buffer.
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
            .is_some_and(|s| s.owned_by(pane_id, buf_id)),
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
    // `finish_edit` skips `bump_edit_seq` for an identity `cs` (correctly:
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
    pane_state[pane_id][buf_id].restore_selections(new_sels, old_head, buffers.get(buf_id).text());
}

/// Open an Insert-kind session on `(pane_id, buf_id)`, always the focused
/// pane, since Insert only ever opens there. The
/// dedicated paste-session opener (`commands::paste`'s `do_paste`) builds
/// its own `EditSession` directly instead of calling this: a paste session
/// stores `before` in its `EditSessionKind::Paste`, which this function has
/// no parameter for.
///
/// Snapshots the current selections (via `.clone()`) for use as `pre_sels`
/// in the recorded undo revision. The field must NOT be taken because the
/// ongoing insert session continues to read it between keystrokes.
///
/// `Err` when a session is already open elsewhere, or a real `Insert`/`Paste`
/// session is already open here; see [`edit_session::open_or_retarget`],
/// which this delegates to. A `Replay`-kind placeholder already open here
/// (`Editor::replay_dot`'s own pre-open, about to be superseded by whatever
/// the replayed body actually needs) is retargeted to `Insert` in place
/// instead of refused.
pub(in crate::editor) fn begin_edit_group(
    buffers: &BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    active_session: &mut Option<EditSession>,
    pane_id: PaneId,
    buf_id: BufferId,
) -> Result<(), CommandError> {
    edit_session::open_or_retarget(
        active_session,
        pane_id,
        buf_id,
        EditSessionKind::Insert,
        // Cloned only here, inside the closure `open_or_retarget` calls
        // solely for a fresh open. A refusal or a placeholder retarget
        // never needs it.
        || {
            buffers
                .get(buf_id)
                .begin_edit_group(pane_state[pane_id][buf_id].selections().clone())
        },
    )?;
    // A fresh group never inherits a typed run, an autoindent record, or exit
    // flags from a previous session (interactive or replay-preopened).
    let pbs = &mut pane_state[pane_id][buf_id];
    pbs.typed_run = None;
    pbs.autoindent = None;
    pbs.step_back_on_exit = false;
    pbs.kill_opened_session = false;
    Ok(())
}

/// Commit whatever session is open, read from the session's own `(pane,
/// buffer)`, never from focus, so the commit lands correctly however focus
/// moved in the meantime. No-op if nothing is open. Shared by
/// [`commit_paste_group`] and [`commit_edit_group`], which each confirm the
/// session's kind first (a no-op skip for the one that doesn't match, a
/// panic for the one that must never mismatch) before delegating here.
///
/// Snapshots the current selections as `post_sels` for the undo revision. It
/// must `.clone()`, not `take`, since a still-open Insert session keeps
/// reading `pre_sels` between keystrokes. Falls back to the group's own
/// `pre_sels` when `pane_state` no longer has an entry for the session's
/// `(pane, buffer)`: a replayed Steel body can close the pane or buffer a
/// pre-opened session lives on before this ever runs; nothing moved the
/// cursor from this session's own perspective past that point, so the
/// pre-edit selection is the best available record, still worth recording
/// rather than losing the revision entirely.
pub(in crate::editor) fn commit_open_session(
    buffers: &mut BufferStore,
    pane_state: &SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    active_session: &mut Option<EditSession>,
) {
    let Some(session) = active_session.take() else {
        return;
    };
    let (pane, buffer) = (session.pane(), session.buffer());
    let group = session.into_group();
    let post_sels = pane_state
        .get(pane)
        .and_then(|m| m.get(buffer))
        .map(|pbs| pbs.selections().clone())
        .unwrap_or_else(|| group.pre_sels.clone());
    buffers.get_mut(buffer).commit_edit_group(group, post_sels);
}

/// Close the open Paste-kind session and record it as a single undo step:
/// the `doc_ops`-level counterpart to [`commit_edit_group`], for the other
/// kinds [`EditSession`] can hold. No-op if the open session (if any)
/// isn't Paste-kind, so every caller can route through this unconditionally
/// instead of checking first.
pub(in crate::editor) fn commit_paste_group(
    buffers: &mut BufferStore,
    pane_state: &SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    active_session: &mut Option<EditSession>,
) {
    let is_paste = active_session
        .as_ref()
        .is_some_and(|s| matches!(s.kind(), EditSessionKind::Paste { .. }));
    if !is_paste {
        return;
    }
    commit_open_session(buffers, pane_state, active_session);
}

/// Close the open Insert-kind session and record it as a single undo step.
///
/// Panics if no session is open, or if the open one isn't Insert-kind:
/// committing a Paste or still-unclaimed Replay session as an Insert
/// revision would record the wrong undo step silently. Mirrors
/// `EditorState::commit_paste_session`'s own kind check.
pub(in crate::editor) fn commit_edit_group(
    buffers: &mut BufferStore,
    pane_state: &SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    active_session: &mut Option<EditSession>,
) {
    match active_session.as_ref() {
        None => panic!("commit_edit_group called without an open session"),
        Some(s) => assert!(
            matches!(s.kind(), EditSessionKind::Insert),
            "commit_edit_group called on a non-Insert session"
        ),
    }
    commit_open_session(buffers, pane_state, active_session);
}

/// Propagate `cs` to every pane except `pane_id` that views `buf_id`,
/// keeping their selections valid after an edit `pane_id` performed.
///
/// `text_pre` must be the buffer text **before** the edit. `translate_in_place_with`
/// uses it to identify which line each head was on pre-edit, which governs
/// whether `Selection.sticky_display_col` is reset after the translation.
/// `text_post` is the text after the edit, which the mapped selections are
/// snapped against.
/// `edits` must be `cs.edited_old_ranges()`; see `finish_edit` for why it's
/// computed there instead of here.
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
    text_post: &hume_editing::text::BufferText,
) {
    // Collect IDs first; can't iterate and mutate the same SecondaryMap.
    let affected: Vec<PaneId> = pane_state
        .iter()
        .filter_map(|(pid, buf_map)| {
            (pid != pane_id && buf_map.contains_key(buf_id)).then_some(pid)
        })
        .collect();
    for pid in affected {
        pane_state[pid][buf_id].translate_selections_in_place(edits, cs, text_pre, text_post);
    }
}
