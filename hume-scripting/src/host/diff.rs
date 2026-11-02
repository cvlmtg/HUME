//! BufferText diffing against live editor state.

use hume_core::BufferId;
use hume_editing::hunk::ChangeHunk;
use hume_editing::text::BufferText;

/// Diffs that need a buffer's live text or undo history, accessed through
/// [`EditorHost::diff`](super::EditorHost::diff). Backs
/// `(diff-buffer-lines pane ref-text)`, `(buffer-revision-diff pane id)` and
/// `(buffer-changes-since pane generation)`.
pub trait DiffHost {
    /// `bid`'s live (dirty) text, `None` for a buffer the host has no text
    /// for. The clone shares the buffer's rope, so it costs O(1) and never
    /// materializes the buffer as a Steel string.
    fn live_text(&self, bid: BufferId) -> Option<BufferText>;

    /// What separates `bid`'s live text from revision `revision` of its undo
    /// history: the revision's text is the old side, the live text the new.
    /// `Err` for a revision the history has no record of. Hunks carry the
    /// word spans the history's own changesets give them.
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
