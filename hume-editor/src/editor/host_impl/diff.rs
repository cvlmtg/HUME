//! `EditorHostImpl`'s `BufferText` diffing.

use hume_core::BufferId;
use hume_editing::hunk::{HunkDiff, text_hunks};
use hume_editing::text::BufferText;

use super::EditorHostImpl;
use hume_scripting::host::DiffHost;

impl<'a> DiffHost for EditorHostImpl<'a> {
    fn diff_buffer_lines(&self, bid: BufferId, ref_text: &str) -> Option<HunkDiff> {
        let buffer = self.buffer(bid)?;
        Some(text_hunks(&BufferText::from(ref_text), buffer.text()))
    }

    fn revision_diff(&self, bid: BufferId, revision: usize) -> Result<HunkDiff, String> {
        self.state.revision_diff(bid, revision).map(HunkDiff::exact)
    }
}
