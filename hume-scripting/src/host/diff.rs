//! Diffs against a buffer's undo history.

use hume_core::BufferId;
use hume_editing::hunk::ChangeHunk;

/// Diffs that need a buffer's undo history, accessed through
/// [`EditorHost::diff`](super::EditorHost::diff). Backs
/// `(buffer-revision-diff pane id)` and `(buffer-changes-since pane
/// generation)`.
pub trait DiffHost {
    /// What separates `bid`'s live text from revision `revision` of its undo
    /// history: the revision's text is the old side, the live text the new.
    /// `Err` for a revision the history has no record of. Hunks carry the
    /// word spans the history's own changesets give them.
    ///
    /// Hunks cross to Steel and the changeset does not. A changeset's ops are
    /// char counts valid against one text only, and applying one from a
    /// script would be an edit path around `EditBuilder`. Hunks are
    /// line-domain and carry their own text.
    fn revision_diff(&self, bid: BufferId, revision: usize) -> Result<Vec<ChangeHunk>, String>;

    /// The hunks separating `bid`'s text at `generation` (old side) from its
    /// live text (new side), `Ok(None)` when the buffer cannot say (a view
    /// refresh replaced the text since, its change log keeps no step that
    /// far back, or the changed lines are too large to report). `Err` for a
    /// buffer the host has no text for.
    fn changes_since(
        &self,
        bid: BufferId,
        generation: u64,
    ) -> Result<Option<Vec<ChangeHunk>>, String>;
}
