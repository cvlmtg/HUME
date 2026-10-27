//! A buffer's undo history: walking it (undo, redo, `goto-revision!`,
//! `:earlier`/`:later`), reading it, and whether the buffer differs from
//! what it last saved.

use std::time::{Duration, SystemTime};

use hume_core::{BufferId, OnUnsaved, PaneId};
use hume_editing::changeset::ChangeSet;
use hume_editing::history::{RevisionId, RevisionNode, RevisionPath};
use hume_editing::hunk::{ChangeHunk, change_hunks};

use super::{Buffer, HistoryMark, HistoryWalkResult};
use crate::editor::EditorState;
use crate::editor::position_stores::PositionStores;

impl Buffer {
    /// Undo up to `count` steps as one composed transform: the production
    /// path for `5u` and an age-resolved `:earlier`, so a multi-step travel
    /// pays for one `install`/`finish_edit` cycle instead of `count` of
    /// them. See [`Self::apply_transactions`] for the return contract.
    pub(crate) fn undo_n(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        acting: PaneId,
        count: usize,
    ) -> HistoryWalkResult {
        let txns = self.history.undo_n(count);
        self.apply_transactions(id, stores, acting, txns)
    }

    /// Redo up to `count` steps forward as one composed transform. See
    /// `undo_n`'s doc: same contract, redo direction.
    pub(crate) fn redo_n(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        acting: PaneId,
        count: usize,
    ) -> HistoryWalkResult {
        let txns = self.history.redo_n(count);
        self.apply_transactions(id, stores, acting, txns)
    }

    /// What `on-undo-history-changed` announces changes of.
    pub(in crate::editor) fn history_mark(&self) -> HistoryMark {
        HistoryMark::of(&self.history, self.saved_revision)
    }

    /// The current revision in the undo history.
    pub(in crate::editor) fn revision_id(&self) -> RevisionId {
        self.history.current_id()
    }

    /// Test-only: production does not branch on this. `undo_n`/`redo_n`
    /// clamp at the root themselves and report `taken < requested` instead
    /// of checking `can_undo` up front.
    #[cfg(test)]
    pub(in crate::editor) fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// Undo steps back to the state as of `age` ago, for `:earlier`: a narrow
    /// delegate so the typed layer never touches `History` itself. See
    /// `History::undo_steps_older_than`'s own doc for the `Result` contract.
    pub(in crate::editor) fn undo_steps_older_than(&self, age: Duration) -> Result<usize, usize> {
        self.history.undo_steps_older_than(age)
    }

    /// Redo steps forward to the state as of `age` ago, for `:later`.
    pub(in crate::editor) fn redo_steps_newer_than(&self, age: Duration) -> Result<usize, usize> {
        self.history.redo_steps_newer_than(age)
    }

    /// Jump to an arbitrary revision in the undo tree as one composed
    /// transform. See [`Self::apply_transactions`] for the return contract: a
    /// `target` equal to the current revision walks nothing and is `None`.
    pub(in crate::editor) fn goto_revision(
        &mut self,
        id: BufferId,
        stores: &mut PositionStores<'_>,
        acting: PaneId,
        target: RevisionId,
    ) -> HistoryWalkResult {
        let txns = self.history.goto_revision(target)?;
        self.apply_transactions(id, stores, acting, txns)
    }

    /// The revision numbered `n` in this buffer's history, `None` when it was
    /// never recorded or has been evicted.
    pub(in crate::editor) fn revision(&self, n: usize) -> Option<RevisionId> {
        RevisionId::checked(&self.history, n)
    }

    /// What separates the live text from revision `n`, as hunks: the
    /// revision's text is the old side. `unrecorded` maps the live text back
    /// to the history's current text when an open edit session holds edits
    /// no revision records yet. `Err` when `n` was never recorded or has been
    /// evicted. The history is not moved.
    fn revision_diff(
        &self,
        n: usize,
        unrecorded: Option<ChangeSet>,
    ) -> Result<Vec<ChangeHunk>, String> {
        let missing = || format!("no revision {n} in this buffer's undo history");
        let target = self.revision(n).ok_or_else(missing)?;
        let recorded = match self.history.changes_to(target) {
            RevisionPath::Changes(cs) => Some(cs),
            RevisionPath::Here => None,
            RevisionPath::Unknown => return Err(missing()),
        };
        Ok(
            ChangeSet::compose_all(unrecorded.into_iter().chain(recorded))
                .map(|net| change_hunks(&net, &self.text))
                .unwrap_or_default(),
        )
    }

    /// Every revision of the undo history, in id order, aged as of `now`.
    pub(in crate::editor) fn revision_nodes(&self, now: SystemTime) -> Vec<RevisionNode> {
        self.history.nodes(now)
    }

    /// The revision the buffer was last saved at, `None` once an
    /// `undo-levels` trim has removed it.
    pub(in crate::editor) fn saved_revision(&self) -> Option<RevisionId> {
        self.saved_revision
    }
}

impl EditorState {
    /// [`Buffer::revision_diff`] for `bid`, counting the edits of an open
    /// Insert or paste session on it as part of the live text.
    pub(crate) fn revision_diff(&self, bid: BufferId, n: usize) -> Result<Vec<ChangeHunk>, String> {
        let unrecorded = self
            .open_session()
            .filter(|s| s.buffer() == bid)
            .and_then(|s| s.unrecorded_inverse());
        self.buffers
            .try_get(bid)
            .ok_or_else(|| format!("invalid buffer id {bid:?}"))?
            .revision_diff(n, unrecorded)
    }

    /// Whether `bid` has changes that are not on disk: a revision past the
    /// saved one, a line ending different from the saved one, or edits in the
    /// open Insert or paste session, which have no revision until the
    /// session commits. Every "is this buffer unsaved" question outside
    /// `buffer` asks this, never [`Buffer`]'s own revision check.
    pub(crate) fn has_unsaved_changes(&self, bid: BufferId) -> bool {
        let buf = self.buffers.get(bid);
        buf.revision_dirty()
            || buf.line_ending_changed()
            || self
                .open_session()
                .is_some_and(|s| s.buffer() == bid && s.has_edits())
    }
}

/// A close, reload or quit refused because it would drop `bid`'s unsaved
/// changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) struct Unsaved(pub(in crate::editor) BufferId);

impl Unsaved {
    /// The refusal's message, naming `way_out`, the consent that goes
    /// ahead.
    pub(in crate::editor) fn message(self, state: &EditorState, way_out: &str) -> String {
        let name = state.buffers.get(self.0).display_name();
        format!("{name} has unsaved changes. Use {way_out}")
    }
}

impl EditorState {
    /// The guard every action that drops `bid`'s text runs first: refused
    /// when `bid` has unsaved changes and `on_unsaved` does not consent.
    pub(in crate::editor) fn refuse_unsaved(
        &self,
        bid: BufferId,
        on_unsaved: OnUnsaved,
    ) -> Result<(), Unsaved> {
        match on_unsaved {
            OnUnsaved::Refuse if self.has_unsaved_changes(bid) => Err(Unsaved(bid)),
            _ => Ok(()),
        }
    }
}
