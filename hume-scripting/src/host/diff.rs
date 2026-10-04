//! BufferText diffing against live editor state.

use hume_editing::hunk::ChangeHunk;
use hume_engine::pipeline::BufferId;

/// Diffs that need a buffer's live text or undo history, accessed through
/// [`EditorHost::diff`](super::EditorHost::diff). Backs
/// `(diff-buffer-lines pane ref-text)` and `(buffer-revision-diff pane id)`.
pub trait DiffHost {
    /// The line hunks of `bid`'s live (dirty) text against `ref_text`, read
    /// as buffer content (see [`text_hunks`](hume_editing::hunk::text_hunks)),
    /// without materializing the buffer as a Steel string. `None` for a
    /// buffer the host has no text for.
    fn diff_buffer_lines(&self, bid: BufferId, ref_text: &str) -> Option<Vec<ChangeHunk>>;

    /// What separates `bid`'s live text from revision `revision` of its undo
    /// history: the revision's text is the old side, the live text the new.
    /// `Err` for a revision the history has no record of. Hunks carry the
    /// word spans the history's own changesets give them.
    fn revision_diff(&self, bid: BufferId, revision: usize) -> Result<Vec<ChangeHunk>, String>;
}
