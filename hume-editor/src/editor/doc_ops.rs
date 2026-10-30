//! Free functions for document-editing operations.
//!
//! Extracted from `impl Editor` so command bodies can hold disjoint borrows
//! from other `Editor` fields while editing.
//!
//! The acting pane's selections are cloned into the command's `EditState`
//! and written back afterwards: the stored set stays valid for the whole
//! edit, so the text change carries it like every other stored position.

use hume_core::{BufferId, PaneHandle, PaneId};

use crate::editor::buffer::store::BufferStore;
use crate::editor::buffer::{Applied, Buffer, HistoryWalkResult};
use crate::editor::edit_session::{
    EditSession, EditSessionKind, check_no_conflicting_session, commit_paste_group,
};
use crate::editor::error::CommandError;
use crate::editor::pane_state::PaneEntries;
use crate::editor::position_stores::PositionStores;
use hume_editing::changeset::ChangeSet;
use hume_editing::edit::{Edited, TextChange};
use hume_editing::selection::SelectionSet;
use hume_editing::state::EditState;
use hume_rope::offset::CharOffset;

/// Shared post-mutation bookkeeping for every text-mutating path: write
/// `applied`'s selections back to `target`'s pane, if it has one, and bump
/// the edit seq when the text changed. Stored positions (other panes, jump lists, prompt snapshots), the
/// syntax edit chain and the language servers' change queue are not here: the
/// `Buffer` mutator already carried them through the change.
fn finish_edit(
    buffers: &mut BufferStore,
    pane_state: &mut PaneEntries,
    target: PaneHandle,
    applied: Applied,
) {
    let buf_id = target.buffer();
    match applied {
        // An identity changeset moved no bytes: `Buffer::apply_edit*` skipped
        // `install` for it directly, and `commit_edit_group` never records it
        // as a revision for `undo`/`redo` to later replay, so the text
        // generation did not move either way. Paste-stamping must not count a
        // no-op as an edit. Selections are still written: a no-op edit can
        // still move cursors.
        Applied::Unchanged(new_sels) => {
            if let Some(pane_id) = target.pane() {
                pane_state
                    .seeded_mut(pane_id, buf_id)
                    .set_selections(new_sels, buffers.get(buf_id).text());
            }
        }
        // A real edit reveals the acting pane even when it didn't move the
        // primary head (`r` replacing the character under the cursor).
        // `install` left this pane's selections for the new ones to replace.
        Applied::Changed(new_sels) => {
            if let Some(pane_id) = target.pane() {
                pane_state
                    .seeded_mut(pane_id, buf_id)
                    .set_selections_after_edit(new_sels);
            }
            buffers.bump_edit_seq();
        }
    }
}

/// `Err` when `buf_id`'s text may not change
/// ([`Buffer::check_text_change`]). The check
/// every buffer-mutating chokepoint below runs before touching the text,
/// and the one a command runs to refuse before an earlier side effect.
pub(in crate::editor) fn text_gate(
    buffers: &BufferStore,
    buf_id: BufferId,
) -> Result<(), CommandError> {
    buffers
        .get(buf_id)
        .check_text_change()
        .map_err(CommandError::transient)
}

/// [`text_gate`] on `target`'s buffer, then `Err` when
/// [`check_no_conflicting_session`] refuses an edit through `target`.
fn admit(
    buffers: &BufferStore,
    stores: &PositionStores<'_>,
    target: PaneHandle,
) -> Result<(), CommandError> {
    text_gate(buffers, target.buffer())?;
    check_no_conflicting_session(stores.open_session(), target)
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
/// `Err` when the buffer's text may not change ([`text_gate`]), or when
/// [`check_no_conflicting_session`] finds another pane's session on it.
pub(in crate::editor) fn apply_doc_edit(
    buffers: &mut BufferStore,
    stores: &mut PositionStores<'_>,
    pane_id: PaneId,
    buf_id: BufferId,
    cmd: impl FnOnce(EditState) -> Edited,
) -> Result<(), CommandError> {
    let sels = stores.panes.seeded(pane_id, buf_id).selections().clone();
    apply_doc_edit_on(buffers, stores, pane_id, buf_id, sels, cmd)
}

/// [`apply_doc_edit`] from `sels`, a set for `buf_id`'s current text, in
/// place of the pane's own selections for it: the edit acts on `sels`, the
/// undo step restores `sels`, and the result becomes the pane's selections
/// for `buf_id`, shown or not.
pub(in crate::editor) fn apply_doc_edit_on(
    buffers: &mut BufferStore,
    stores: &mut PositionStores<'_>,
    pane_id: PaneId,
    buf_id: BufferId,
    sels: SelectionSet,
    cmd: impl FnOnce(EditState) -> Edited,
) -> Result<(), CommandError> {
    let target = PaneHandle::with_pane(buf_id, pane_id);
    admit(buffers, stores, target)?;
    if stores.insert_at_mut(pane_id, buf_id).is_some() {
        apply_grouped_from(buffers, stores, pane_id, buf_id, sels, cmd);
        return Ok(());
    }
    // Scoped to this exact (pane, buffer): `commit_paste_group` alone would
    // commit *any* open Paste session, but `check_no_conflicting_session`
    // above only refused a different pane's session on `buf_id`. An
    // unrelated Paste session still open on the focused pane, for a
    // *different* buffer, must not be ended early just because a remote
    // `call!` is editing something else through a background pane.
    if stores
        .session
        .as_ref()
        .is_some_and(|s| s.owned_by(pane_id, buf_id))
    {
        commit_paste_group(buffers, stores.panes, stores.session);
    }
    let applied = buffers
        .get_mut(buf_id)
        .apply_edit(target, stores, sels, cmd);
    finish_edit(buffers, stores.panes, target, applied);
    Ok(())
}

fn edit_from_changes(cs: ChangeSet) -> impl FnOnce(EditState) -> Edited {
    move |s| {
        Edited::from_changes(s, cs)
            .expect("cs built for this buffer's text, keeping its final newline")
    }
}

/// Applies `cs`, a changeset built against `target`'s buffer's current text,
/// as one edit.
///
/// Through a pane, it goes through [`apply_doc_edit`] and that pane's session
/// rules. Without one, every pane's stored selections are carried through the
/// change and none is revealed; the undo revision records a cursor at the
/// first change, which an undo restores into whichever pane walks it. `Err`
/// then when any session is open on the buffer: an edit no pane owns cannot
/// join a session's group, and would leave a paste session's snapshot stale.
pub(in crate::editor) fn apply_changes(
    buffers: &mut BufferStore,
    stores: &mut PositionStores<'_>,
    target: PaneHandle,
    cs: ChangeSet,
) -> Result<(), CommandError> {
    let buf_id = target.buffer();
    if let Some(pane_id) = target.pane() {
        return apply_doc_edit(buffers, stores, pane_id, buf_id, edit_from_changes(cs));
    }
    admit(buffers, stores, target)?;
    let text = buffers.get(buf_id).text();
    let first_change = cs
        .edited_regions()
        .first()
        .map_or_else(CharOffset::default, |region| region.old.start);
    let sels = EditState::with_cursor(text.clone(), text.snap(first_change)).into_selections();
    let applied = buffers
        .get_mut(buf_id)
        .apply_edit(target, stores, sels, edit_from_changes(cs));
    finish_edit(buffers, stores.panes, target, applied);
    Ok(())
}

/// Apply a grouped edit (inside an Insert session) to `buf_id` through `pane_id`.
///
/// Reads and writes the acting pane's selections via `stores`; the buffer
/// carries every other stored position through the change.
///
/// Returns the applied `ChangeSet`: completion accept maps positions a
/// later resolve response computed against the pre-accept text through it.
/// `Err` when the text may not change ([`text_gate`]).
///
/// An Insert session must be open on `(pane_id, buf_id)`, a caller
/// contract. Panics otherwise.
pub(in crate::editor) fn apply_doc_edit_grouped(
    buffers: &mut BufferStore,
    stores: &mut PositionStores<'_>,
    pane_id: PaneId,
    buf_id: BufferId,
    cmd: impl FnOnce(EditState) -> Edited,
) -> Result<ChangeSet, CommandError> {
    text_gate(buffers, buf_id)?;
    let sels = stores.panes.seeded(pane_id, buf_id).selections().clone();
    Ok(apply_grouped_from(
        buffers, stores, pane_id, buf_id, sels, cmd,
    ))
}

/// [`apply_doc_edit_grouped`] past its refusal checks, acting on `sels`.
fn apply_grouped_from(
    buffers: &mut BufferStore,
    stores: &mut PositionStores<'_>,
    pane_id: PaneId,
    buf_id: BufferId,
    sels: SelectionSet,
    cmd: impl FnOnce(EditState) -> Edited,
) -> ChangeSet {
    let text_pre = buffers.get(buf_id).text().clone();
    assert!(
        stores.insert_at_mut(pane_id, buf_id).is_some(),
        "apply_doc_edit_grouped called without an open Insert session on this (pane, buffer)"
    );
    let (new_sels, cs) = buffers
        .get_mut(buf_id)
        .apply_edit_grouped(buf_id, stores, pane_id, sels, cmd);
    finish_edit(
        buffers,
        stores.panes,
        PaneHandle::with_pane(buf_id, pane_id),
        Applied::of(new_sels, &cs),
    );
    let session = stores
        .insert_at_mut(pane_id, buf_id)
        .expect("checked above, and an edit closes no session");
    session.group_mut().compose(&cs);
    // This is the one funnel every grouped edit goes through, so it's the
    // one place `DotCapture::chain` (`replay/capture.rs`) can be fed without
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
/// panes. `stores.session` must hold a Paste-kind session on
/// `(pane_id, buf_id)`. The caller must have opened it via
/// `commands::paste`'s `do_paste` first. Panics otherwise. `Err` when the
/// text may not change ([`text_gate`]).
pub(in crate::editor) fn apply_doc_edit_regrouped(
    buffers: &mut BufferStore,
    stores: &mut PositionStores<'_>,
    pane_id: PaneId,
    buf_id: BufferId,
    cmd: impl FnOnce(EditState) -> Edited,
) -> Result<(), CommandError> {
    text_gate(buffers, buf_id)?;
    let group = stores
        .session
        .as_ref()
        .filter(|s| s.is_paste_at(pane_id, buf_id))
        .expect(
            "apply_doc_edit_regrouped called without an open paste session on this (pane, buffer)",
        )
        .group();
    let (snapshot, to_snapshot) = (group.snapshot.clone(), group.inverse());
    let (applied, new_cs) = buffers.get_mut(buf_id).apply_edit_regrouped(
        buf_id,
        stores,
        pane_id,
        snapshot,
        to_snapshot,
        cmd,
    );
    stores
        .session
        .as_mut()
        .expect("checked above, and an edit closes no session")
        .group_mut()
        .cs = Some(new_cs);
    finish_edit(
        buffers,
        stores.panes,
        PaneHandle::with_pane(buf_id, pane_id),
        applied,
    );
    Ok(())
}

/// The refusal of a history walk under an open Insert session or Replay
/// placeholder, whose group is mid-composition.
fn session_refuses_walk() -> CommandError {
    CommandError::transient("cannot move through undo history while an edit session is open")
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
/// Returns the steps actually taken, fewer than requested at the root/leaf.
/// `Err` when the text may not change ([`text_gate`]), when
/// [`check_no_conflicting_session`] finds another pane's session on this
/// buffer, or when this pane has an Insert or Replay session open on it.
pub(in crate::editor) fn apply_doc_history_walk(
    buffers: &mut BufferStore,
    stores: &mut PositionStores<'_>,
    pane_id: PaneId,
    buf_id: BufferId,
    walk: impl FnOnce(
        &mut Buffer,
        BufferId,
        &mut PositionStores<'_>,
        PaneId,
    ) -> Result<HistoryWalkResult, CommandError>,
) -> Result<usize, CommandError> {
    let target = PaneHandle::with_pane(buf_id, pane_id);
    admit(buffers, stores, target)?;
    if stores.insert_at_mut(pane_id, buf_id).is_some() {
        return Err(session_refuses_walk());
    }
    match stores
        .session
        .as_ref()
        .filter(|s| s.owned_by(pane_id, buf_id))
        .map(EditSession::kind)
    {
        None => {}
        Some(EditSessionKind::Paste { .. }) => {
            commit_paste_group(buffers, stores.panes, stores.session);
        }
        Some(EditSessionKind::Replay) => return Err(session_refuses_walk()),
    }
    let Some((new_sels, cs, steps)) = walk(buffers.get_mut(buf_id), buf_id, stores, pane_id)?
    else {
        return Ok(0);
    };
    finish_edit(buffers, stores.panes, target, Applied::of(new_sels, &cs));
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
    Ok(steps)
}

/// Apply a motion function and store the resulting selection in `pane_state`.
pub(in crate::editor) fn apply_doc_motion(
    buffers: &BufferStore,
    pane_state: &mut PaneEntries,
    pane_id: PaneId,
    buf_id: BufferId,
    f: impl FnOnce(EditState) -> EditState,
) {
    let text = buffers.get(buf_id).text();
    let sels = pane_state.seeded(pane_id, buf_id).selections().clone();
    let new_sels = f(EditState::bind(text, sels)).into_selections();
    pane_state
        .seeded_mut(pane_id, buf_id)
        .set_selections(new_sels, text);
}
