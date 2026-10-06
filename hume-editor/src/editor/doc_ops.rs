//! Free functions for document-editing operations.
//!
//! Extracted from `impl Editor` so command bodies can hold disjoint borrows
//! from other `Editor` fields while editing.
//!
//! The acting pane's selections are cloned into the command's `EditState`
//! and written back afterwards: the stored set stays valid for the whole
//! edit, so the text change carries it like every other stored position.

use slotmap::SecondaryMap;

use hume_engine::pipeline::{BufferId, PaneId};

use crate::editor::buffer::store::BufferStore;
use crate::editor::buffer::{Buffer, HistoryWalkResult};
use crate::editor::edit_session::{self, EditSession, EditSessionKind};
use crate::editor::error::CommandError;
use crate::editor::pane_state::PaneBufferState;
use crate::editor::position_stores::PositionStores;
use hume_editing::changeset::ChangeSet;
use hume_editing::edit::{Edited, TextChange};
use hume_editing::selection::SelectionSet;
use hume_editing::state::EditState;

/// [`apply_doc_history_walk`]'s result: keeps a read-only refusal
/// distinguishable from genuine root/leaf exhaustion, so each caller reports
/// it in its own way (`history_step` as a refused command, `goto-revision!` as
/// an error). Collapsing both to `0` would report "Already at oldest change"
/// for a read-only buffer, sending the user to look for missing history that
/// was never there to find.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum HistoryWalk {
    /// The buffer is read-only; nothing was attempted.
    RefusedReadOnly,
    /// The walk ran and took this many steps, short of the requested count
    /// at the root/leaf.
    Took(usize),
}

/// Shared post-mutation bookkeeping for every text-mutating path: write
/// `new_sels` back to the acting pane and bump the edit seq. Stored
/// positions (other panes, jump lists, prompt snapshots), the syntax edit
/// chain and the language servers' change queue are not here: the `Buffer`
/// mutator already carried them through the change.
fn finish_edit(
    buffers: &mut BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pane_id: PaneId,
    buf_id: BufferId,
    new_sels: SelectionSet,
    cs: &ChangeSet,
) {
    // An identity `cs` moved no bytes: `Buffer::apply_edit*` skipped
    // `install` for it directly, and `commit_edit_group` never records it as
    // a revision for `undo`/`redo` to later replay, so the text generation did
    // not move either way. Paste-stamping must not count a no-op as an
    // edit. Selections are still written: a no-op edit can still move
    // cursors.
    if cs.is_identity() {
        pane_state[pane_id][buf_id].set_selections(new_sels, buffers.get(buf_id).text());
        return;
    }
    // A real edit reveals the acting pane even when it didn't move the
    // primary head (`r` replacing the character under the cursor). `install`
    // left this pane's selections for the new ones to replace.
    pane_state[pane_id][buf_id].set_selections_after_edit(new_sels);
    buffers.bump_edit_seq();
}

/// `Err` when [`EditorState::active_session`](crate::editor::EditorState::active_session)
/// holds a session on `buf_id` opened by some pane other than `exclude`: the
/// exclusivity check every buffer-mutating chokepoint below runs before
/// touching `buf_id`'s text.
///
/// Remote dispatch (`(call! "cmd" pane)` targeting a pane other than the
/// focused one) means a *different* pane's own in-progress insert/paste
/// session can otherwise be mutated out from under it: its eventual commit
/// inverts the composed `ChangeSet` against a `snapshot` a concurrent
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

/// Apply an edit to `buf_id` through `pane_id`; the buffer carries every
/// other stored position through the change.
///
/// Every edit-applying caller goes through here, so session handling lives
/// here rather than at each caller:
/// - An Insert session open on this (pane, buffer) routes the edit into
///   [`apply_doc_edit_grouped`]. A standalone revision would desync the group
///   and make the next `ChangeSet::compose` panic on a length mismatch.
/// - A Paste session open on this (pane, buffer) is committed first via
///   [`commit_paste_group`]. Otherwise its `snapshot` goes stale and the
///   next `[`/`]` re-paste discards this edit. Callers that bypass key
///   dispatch (and so `step_paste_commit`) rely on this.
///
/// `Err` when [`check_no_conflicting_session`] finds another pane's session on
/// this buffer.
// Same non-collapsible-params shape as `finish_edit`'s own allow, above.
#[allow(clippy::too_many_arguments)]
pub(in crate::editor) fn apply_doc_edit(
    buffers: &mut BufferStore,
    stores: &mut PositionStores<'_>,
    active_session: &mut Option<EditSession>,
    pane_id: PaneId,
    buf_id: BufferId,
    cmd: impl FnOnce(EditState) -> Edited,
) -> Result<(), CommandError> {
    if buffers.get(buf_id).is_read_only() {
        return Ok(());
    }
    check_no_conflicting_session(active_session, pane_id, buf_id)?;
    let insert_session_open_here = active_session
        .as_ref()
        .is_some_and(|s| s.is_insert_at(pane_id, buf_id));
    if insert_session_open_here {
        apply_doc_edit_grouped(buffers, stores, active_session, pane_id, buf_id, cmd);
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
        commit_paste_group(buffers, stores.panes, active_session);
    }
    let sels = stores.panes[pane_id][buf_id].selections().clone();
    let (new_sels, cs) = buffers
        .get_mut(buf_id)
        .apply_edit(buf_id, stores, pane_id, sels, cmd);
    finish_edit(buffers, stores.panes, pane_id, buf_id, new_sels, &cs);
    Ok(())
}

/// Apply a grouped edit (inside an Insert session) to `buf_id` through `pane_id`.
///
/// Reads and writes the acting pane's selections via `stores`; the buffer
/// carries every other stored position through the change.
///
/// Returns the applied `ChangeSet`: completion accept maps positions a
/// later resolve response computed against the pre-accept text through it.
///
/// `active_session` must hold an Insert-kind session on `(pane_id,
/// buf_id)`, a caller contract, same as `Buffer::apply_edit_grouped`'s own
/// "must have called `begin_edit_group` first". Panics otherwise.
// Same non-collapsible-params shape as `finish_edit`'s own allow, above.
#[allow(clippy::too_many_arguments)]
pub(in crate::editor) fn apply_doc_edit_grouped(
    buffers: &mut BufferStore,
    stores: &mut PositionStores<'_>,
    active_session: &mut Option<EditSession>,
    pane_id: PaneId,
    buf_id: BufferId,
    cmd: impl FnOnce(EditState) -> Edited,
) -> ChangeSet {
    if buffers.get(buf_id).is_read_only() {
        // Identity changeset: a read-only buffer means nothing happened, so a
        // caller remapping other state through the result must see a no-op,
        // not a stale/mismatched-length one.
        return ChangeSet::identity(buffers.get(buf_id).text().len_chars());
    }
    let text_pre = buffers.get(buf_id).text().clone();
    let sels = stores.panes[pane_id][buf_id].selections().clone();
    let doc = buffers.get_mut(buf_id);
    // Bound once, reused below for the capture-feed push too: both need
    // the same `is_insert_at`-filtered session.
    let session = active_session
        .as_mut()
        .filter(|s| s.is_insert_at(pane_id, buf_id))
        .expect(
            "apply_doc_edit_grouped called without an open Insert session on this (pane, buffer)",
        );
    let (new_sels, cs) =
        doc.apply_edit_grouped(buf_id, stores, pane_id, sels, session.group_mut(), cmd);
    finish_edit(buffers, stores.panes, pane_id, buf_id, new_sels, &cs);
    // This is the one funnel every grouped edit goes through, so it's the
    // one place `DotCapture::chain` (`edit_session.rs`) can be fed without
    // every caller remembering to do it itself; see that type's own doc.
    if let Some(cap) = session.dot_capture_mut() {
        cap.chain
            .push(&TextChange::new(&text_pre, buffers.get(buf_id).text(), &cs));
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
    stores: &mut PositionStores<'_>,
    active_session: &mut Option<EditSession>,
    pane_id: PaneId,
    buf_id: BufferId,
    cmd: impl FnOnce(EditState) -> Edited,
) {
    if buffers.get(buf_id).is_read_only() {
        return;
    }
    let group = active_session
        .as_mut()
        .filter(|s| s.is_paste_at(pane_id, buf_id))
        .expect(
            "apply_doc_edit_regrouped called without an open paste session on this (pane, buffer)",
        )
        .group_mut();
    let (new_sels, propagation_cs) = buffers
        .get_mut(buf_id)
        .apply_edit_regrouped(buf_id, stores, pane_id, group, cmd);
    finish_edit(
        buffers,
        stores.panes,
        pane_id,
        buf_id,
        new_sels,
        &propagation_cs,
    );
}

/// Run one history walk on `buf_id` through `pane_id` (`walk`, typically
/// `|b, id, stores, pane| b.undo_n(id, stores, pane, count)`), which carries every stored
/// position through the net `ChangeSet`.
///
/// `walk` already composes every revision it crosses into one transform (see
/// `Buffer::apply_transactions`), so there is a single `finish_edit` call and
/// panes, jump lists, tree-sitter, and LSP see one edit. An unbounded
/// `:earlier`/`:later` costs the same as one `u`.
///
/// Session handling lives here, as in [`apply_doc_edit`]: a Paste session open
/// on this (pane, buffer) is committed first, so the walk sees its revision
/// and its snapshot cannot go stale. An Insert session or a Replay placeholder
/// open there is an `Err`: its group is mid-composition, and a walk underneath
/// it would desync the group from the text.
///
/// Returns [`HistoryWalk::Took`] with the steps actually taken (fewer than
/// requested at the root/leaf) or [`HistoryWalk::RefusedReadOnly`]. `Err`
/// when [`check_no_conflicting_session`] finds another pane's session on this
/// buffer, or this pane has an Insert or Replay session open on it.
// Same non-collapsible-params shape as `finish_edit`'s own allow, above.
#[allow(clippy::too_many_arguments)]
pub(in crate::editor) fn apply_doc_history_walk(
    buffers: &mut BufferStore,
    stores: &mut PositionStores<'_>,
    active_session: &mut Option<EditSession>,
    pane_id: PaneId,
    buf_id: BufferId,
    walk: impl FnOnce(
        &mut Buffer,
        BufferId,
        &mut PositionStores<'_>,
        PaneId,
    ) -> Result<HistoryWalkResult, CommandError>,
) -> Result<HistoryWalk, CommandError> {
    if buffers.get(buf_id).is_read_only() {
        return Ok(HistoryWalk::RefusedReadOnly);
    }
    check_no_conflicting_session(active_session, pane_id, buf_id)?;
    match active_session
        .as_ref()
        .filter(|s| s.owned_by(pane_id, buf_id))
        .map(EditSession::kind)
    {
        None => {}
        Some(EditSessionKind::Paste { .. }) => {
            commit_paste_group(buffers, stores.panes, active_session);
        }
        Some(EditSessionKind::Insert | EditSessionKind::Replay) => {
            return Err(CommandError::transient(
                "cannot move through undo history while an edit session is open",
            ));
        }
    }
    let Some((new_sels, cs, steps)) = walk(buffers.get_mut(buf_id), buf_id, stores, pane_id)?
    else {
        return Ok(HistoryWalk::Took(0));
    };
    finish_edit(buffers, stores.panes, pane_id, buf_id, new_sels, &cs);
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
pub(in crate::editor) fn apply_doc_motion(
    buffers: &BufferStore,
    pane_state: &mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    pane_id: PaneId,
    buf_id: BufferId,
    f: impl FnOnce(EditState) -> EditState,
) {
    let text = buffers.get(buf_id).text();
    let sels = pane_state[pane_id][buf_id].selections().clone();
    let new_sels = f(EditState::bind(text, sels)).into_selections();
    pane_state[pane_id][buf_id].set_selections(new_sels, text);
}

/// Open an Insert-kind session on `(pane_id, buf_id)`, always the focused
/// pane, since Insert only ever opens there. The
/// dedicated paste-session opener (`commands::paste`'s `do_paste`) builds
/// its own `EditSession` directly instead of calling this: a paste session
/// stores `before` in its `EditSessionKind::Paste`, which this function has
/// no parameter for.
///
/// Snapshots the current selections as the session's `undo_sels`, what undo
/// restores: cloned, not taken, since the pane keeps editing with them.
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
            let sels = pane_state[pane_id][buf_id].selections();
            buffers
                .get(buf_id)
                .begin_edit_group(sels.clone(), sels.clone())
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
/// Snapshots the current selections as `post_sels` for the undo revision:
/// cloned, not taken, since the pane keeps them.
///
/// # Panics
/// Panics if `pane_state` has no entry for the session's `(pane, buffer)`:
/// closing a pane or buffer ends its sessions first, so an open session's
/// state outlives it.
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
        .expect("an open session's (pane, buffer) state outlives it");
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
