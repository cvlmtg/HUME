//! `EditorHostImpl`'s buffer/pane enumeration, reads, lifecycle, and
//! viewport geometry.

use std::path::{Path, PathBuf};

use hume_engine::pipeline::BufferId;
use hume_rope::lines::line_token_content;
use hume_rope::offset::ExclusiveRange;

use super::EditorHostImpl;
use crate::editor::commands::FocusedPane;
use hume_scripting::PaneHandle;
use hume_scripting::host::BufferHost;

impl<'a> BufferHost for EditorHostImpl<'a> {
    // ── Enumeration ──────────────────────────────────────────────────────────
    fn buffer_ids(&self) -> Vec<BufferId> {
        self.state.buffers.iter().map(|(id, _)| id).collect()
    }
    fn panes(&self) -> Vec<PaneHandle> {
        self.view
            .panes
            .every_pane_across_all_tabs()
            .map(|(pid, p)| PaneHandle::with_pane(p.buffer_id, pid))
            .collect()
    }

    fn focused_pane(&self) -> PaneHandle {
        FocusedPane::current(self.state).handle(self.view)
    }

    fn buffer_panes(&self, pane: PaneHandle) -> Vec<PaneHandle> {
        self.state
            .buffer_panes(self.view, pane.buffer())
            .into_iter()
            .map(|pid| PaneHandle::with_pane(pane.buffer(), pid))
            .collect()
    }

    fn require_focused_pane(&self, pane: PaneHandle) -> Result<(), String> {
        FocusedPane::resolve(self.state, self.view, pane)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn pane_live(&self, pane: PaneHandle) -> bool {
        self.command_pane(pane).is_ok()
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
        self.buffer(id).map(|_| self.state.has_unsaved_changes(id))
    }
    fn buffer_stored_language(&self, id: BufferId) -> Option<String> {
        let lang_id = self.buffer(id)?.language?;
        Some(self.state.config.languages.name_of(lang_id).to_owned())
    }

    // ── Working directory ────────────────────────────────────────────────────
    fn cwd(&self) -> PathBuf {
        self.state.cwd.clone()
    }
    fn set_cwd(&mut self, path: &Path) -> Result<PathBuf, String> {
        self.state
            .set_cwd(path)
            .map_err(|e| format!("set-cwd!: {}: {e}", path.display()))
    }

    // ── Buffer lifecycle ─────────────────────────────────────────────────────
    fn open_buffer(&mut self, path: &Path) -> Result<BufferId, String> {
        // `resolve_buffer_path`, not a hard `canonicalize`: a missing path is
        // openable here exactly like `:e` on one; see `Buffer::from_file_or_new`.
        let resolved = crate::editor::Editor::resolve_buffer_path(path, &self.state.cwd);
        // Language detection is not done here; see
        // `Effect::DetectBufferLanguage`'s doc; the `open-buffer!` builtin
        // queues it once this returns.
        let (bid, _is_new) = crate::editor::buffer::lifecycle::open_or_dedup_and_notify(
            self.view, self.state, &resolved,
        )
        .map_err(|e| format!("open-buffer!: {}: {e}", resolved.display()))?;
        Ok(bid)
    }
    fn close_buffer(&mut self, id: BufferId) -> Result<(), String> {
        if self.state.buffers.try_get(id).is_none() {
            return Err(format!("close-buffer!: buffer {id:?} does not exist"));
        }
        crate::editor::buffer::lifecycle::close_buffer_and_notify(self.view, self.state, id);
        Ok(())
    }
    fn switch_to_buffer(&mut self, pane: PaneHandle, target: BufferId) -> Result<(), String> {
        let t = self.command_pane(pane)?;
        crate::editor::buffer::lifecycle::switch_to_buffer_with_jump(
            self.state,
            self.view,
            t.pid(),
            target,
        );
        Ok(())
    }

    fn buffer_undo_tree(&self, id: BufferId) -> Option<Vec<hume_scripting::host::UndoNode>> {
        let buf = self.buffer(id)?;
        let (current, saved) = (buf.revision_id(), buf.saved_revision());
        let nodes = buf.revision_nodes(std::time::SystemTime::now());
        Some(
            nodes
                .into_iter()
                .map(|node| hume_scripting::host::UndoNode {
                    id: node.id().index(),
                    parent: node.parent().map(hume_editing::history::RevisionId::index),
                    age_secs: node.age().as_secs(),
                    current: node.id() == current,
                    saved: Some(node.id()) == saved,
                })
                .collect(),
        )
    }

    fn buffer_generation(&self, id: BufferId) -> Option<u64> {
        Some(self.buffer(id)?.text().generation())
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

    fn viewport_range(
        &self,
        pane: PaneHandle,
    ) -> Result<ExclusiveRange<hume_rope::line::ContentLine>, String> {
        let t = self.command_pane(pane)?;
        // No active-tab restriction: `Editor::sync_viewport_dims` keeps
        // every tab's panes (not just the active one) sized to the
        // current terminal on every resize (`TabStore::inactive_layouts`),
        // so a background-tab pane's *size* is as trustworthy as an active
        // one's. Its *scroll position* can lag, though: the frame's scroll
        // step only runs over `active_pane_ids()`, so a background pane's
        // scroll stays wherever it last was while still active, until its
        // tab is focused again. "The range of lines this pane would show"
        // is still well-defined regardless of which tab is on screen; it's
        // just not guaranteed to reflect a scroll that happened elsewhere
        // while this pane was hidden.
        Ok(crate::editor::lsp::introspect::viewport_range(
            self.state, self.view, t,
        ))
    }
}
