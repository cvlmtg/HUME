//! `EditorHostImpl`'s `BufferText` diffing.

use hume_engine::pipeline::BufferId;

use crate::editor::diff_bridge;

use super::EditorHostImpl;
use hume_scripting::host::{DiffHost, DiffHunk, WordDiffHunk};

impl<'a> DiffHost for EditorHostImpl<'a> {
    fn diff_lines(&self, old: &str, new: &str) -> Vec<DiffHunk> {
        diff_bridge::line_hunks(old, new)
    }

    fn diff_buffer_lines(&self, bid: BufferId, ref_text: &str) -> Option<Vec<DiffHunk>> {
        let buffer_text = self.buffer(bid)?.text();
        Some(diff_bridge::line_hunks_against_buffer(
            ref_text,
            buffer_text,
        ))
    }

    fn diff_words(&self, old: &str, new: &str) -> (Vec<WordDiffHunk>, bool) {
        diff_bridge::word_hunks(old, new)
    }
}
