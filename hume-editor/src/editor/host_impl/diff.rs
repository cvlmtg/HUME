//! `EditorHostImpl`'s diffs against a buffer's undo history.

use hume_core::BufferId;
use hume_editing::hunk::ChangeHunk;

use super::EditorHostImpl;
use hume_scripting::host::DiffHost;

impl<'a> DiffHost for EditorHostImpl<'a> {
    fn revision_diff(&self, bid: BufferId, revision: usize) -> Result<Vec<ChangeHunk>, String> {
        self.state.revision_diff(bid, revision)
    }

    fn changes_since(
        &self,
        bid: BufferId,
        generation: u64,
    ) -> Result<Option<Vec<ChangeHunk>>, String> {
        let buffer = self.buffer(bid).ok_or("no such buffer")?;
        Ok(buffer.changes_since(generation))
    }
}
