//! `EditorHostImpl`'s buffer/pane enumeration, reads, lifecycle, and
//! viewport geometry.

use std::path::{Path, PathBuf};

use hume_engine::pipeline::{BufferId, PaneId};
use hume_rope::lines::line_token_content;
use hume_rope::offset::ExclusiveRange;

use super::EditorHostImpl;
use hume_scripting::host::BufferHost;

impl<'a> BufferHost for EditorHostImpl<'a> {
    // ── Enumeration ──────────────────────────────────────────────────────────
    fn buffer_ids(&self) -> Vec<BufferId> {
        self.state.buffers.iter().map(|(id, _)| id).collect()
    }
    fn pane_ids(&self) -> Vec<PaneId> {
        // `(panes)`'s own contract — see its doc.
        self.view
            .panes
            .every_pane_across_all_tabs()
            .map(|(id, _)| id)
            .collect()
    }

    // ── Buffer reads ─────────────────────────────────────────────────────────
    fn buffer_exists(&self, id: BufferId) -> bool {
        self.buffer(id).is_some()
    }
    fn buffer_path(&self, id: BufferId) -> Option<PathBuf> {
        self.buffer(id)?.path().map(Path::to_path_buf)
    }
    fn buffer_display_path(&self, id: BufferId) -> Option<String> {
        self.buffer(id)?.display_path().map(str::to_owned)
    }
    fn buffer_display_name(&self, id: BufferId) -> Option<String> {
        self.buffer(id).map(|buf| buf.display_name())
    }
    fn buffer_is_dirty(&self, id: BufferId) -> Option<bool> {
        self.buffer(id).map(|buf| buf.is_dirty())
    }
    fn buffer_stored_language(&self, id: BufferId) -> Option<String> {
        let lang_id = self.buffer(id)?.language?;
        Some(self.state.config.languages.name_of(lang_id).to_owned())
    }

    // ── Buffer lifecycle ─────────────────────────────────────────────────────
    fn open_buffer(&mut self, path: &Path) -> Result<BufferId, String> {
        // `resolve_buffer_path`, not a hard `canonicalize`: a missing path is
        // openable here exactly like `:e` on one — see `Buffer::from_file_or_new`.
        let resolved = crate::editor::Editor::resolve_buffer_path(path, &self.state.cwd);
        // Language detection is deliberately not done here — see
        // `Effect::DetectBufferLanguage`'s doc; the `open-buffer!` builtin
        // queues it once this returns.
        let (bid, _is_new) = crate::editor::buffer::lifecycle::open_or_dedup_and_notify(
            self.view, self.state, &resolved,
        )
        .map_err(|e| format!("open-buffer!: {}: {e}", resolved.display()))?;
        Ok(bid)
    }
    fn close_buffer(&mut self, id: BufferId) -> Result<BufferId, String> {
        if self.state.buffers.try_get(id).is_none() {
            return Err(format!("close-buffer!: buffer {id:?} does not exist"));
        }
        Ok(crate::editor::buffer::lifecycle::close_buffer_and_notify(
            self.view,
            self.state,
            self.lsp.as_deref_mut(),
            id,
        ))
    }
    fn switch_to_buffer(&mut self, current: BufferId, target: BufferId) -> Result<(), String> {
        crate::editor::buffer::lifecycle::switch_to_buffer_with_jump(
            self.view,
            &self.state.buffers,
            &mut self.state.panes.state,
            &mut self.state.panes.jumps,
            self.state.focus.id(),
            current,
            target,
        );
        Ok(())
    }

    fn buffer_generation(&self, id: BufferId) -> Option<u64> {
        Some(self.buffer(id)?.text_gen)
    }

    fn buffer_text(&self, id: BufferId) -> Option<String> {
        Some(self.buffer(id)?.text().to_string())
    }

    fn buffer_line_count(&self, id: BufferId) -> Option<usize> {
        Some(self.buffer(id)?.text().content_line_count().get())
    }

    fn buffer_lines(
        &self,
        id: BufferId,
        range: ExclusiveRange<hume_rope::line::ContentLine>,
    ) -> Option<Vec<String>> {
        let text = self.buffer(id)?.text();
        let count = range.end.lines_since(range.start);
        Some(
            text.line_tokens_at(hume_rope::line::RopeyLine::from(range.start))
                .take(count)
                .map(line_token_content)
                .collect(),
        )
    }

    fn line_to_offset(&self, id: BufferId, line: hume_rope::line::ContentLine) -> Option<usize> {
        Some(self.buffer(id)?.text().line_to_char(line.into()).index())
    }

    fn viewport_range(&self, id: BufferId) -> Option<ExclusiveRange<hume_rope::line::ContentLine>> {
        crate::editor::lsp::introspect::viewport_range(self.state, self.view, id)
    }
}
