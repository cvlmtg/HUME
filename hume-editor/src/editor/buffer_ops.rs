use std::io;
use std::path::PathBuf;

use hume_engine::pipeline::{BufferId, PaneId};

use crate::editor::buffer::Buffer;
use hume_scripting::SteelBufferId;
use hume_scripting::hooks::HookId;

use super::{Editor, Severity, ops};

impl Editor {
    // ── Working directory ─────────────────────────────────────────────────────

    /// Change the editor's working directory.
    ///
    /// Canonicalizes `path`, rejects non-directories, then updates both
    /// `self.state.cwd` and the process cwd so that relative paths in `:e` and
    /// subprocesses resolve consistently.
    pub(super) fn set_cwd(&mut self, path: &std::path::Path) -> io::Result<PathBuf> {
        let canonical = hume_platform::fs::canonicalize(path)?;
        if !canonical.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "not a directory",
            ));
        }
        std::env::set_current_dir(&canonical)?;
        self.state.cwd = canonical;
        Ok(self.state.cwd.clone())
    }

    // ── Buffer choke-points ───────────────────────────────────────────────────

    /// Dedup-open a canonicalized path: returns `(id, false)` if already open,
    /// `(id, true)` if newly opened (including `OnBufferOpen` hook fire).
    pub(super) fn open_or_dedup(
        &mut self,
        canonical: &std::path::Path,
    ) -> std::io::Result<(BufferId, bool)> {
        if let Some(existing) = self.state.buffers.find_by_path(canonical) {
            return Ok((existing, false));
        }
        Ok((self.open_buffer(Buffer::from_file(canonical)?), true))
    }

    /// Open additional files without switching focus; errors are logged as warnings.
    pub(crate) fn open_extra_files(&mut self, paths: &[PathBuf]) {
        for path in paths {
            if let Err(e) = self.try_open_extra(path) {
                self.report(
                    Severity::Warning,
                    format!("Failed to open {}: {e}", path.display()),
                );
            }
        }
    }

    fn try_open_extra(&mut self, path: &std::path::Path) -> io::Result<()> {
        let lossy = path.to_string_lossy();
        let expanded = hume_platform::path::expand(&lossy);
        let expanded_path = std::path::Path::new(expanded.as_ref());
        let display = hume_platform::path::absolute_unresolved(expanded_path, &self.state.cwd);
        let canonical = hume_platform::fs::canonicalize(expanded_path)?;
        let (bid, is_new) = self.open_or_dedup(&canonical)?;
        if is_new {
            self.state
                .buffers
                .get_mut(bid)
                .set_display_path(Some(display));
        }
        Ok(())
    }

    /// Allocate a new buffer slot (engine + BufferStore), seed the focused pane's
    /// per-buffer state (`state.panes.state`), and return the allocated `BufferId`.
    pub(crate) fn open_buffer(&mut self, doc: Buffer) -> BufferId {
        let bid = ops::open_buffer(
            &mut self.view,
            &mut self.state.buffers,
            &mut self.state.panes.state,
            self.state.focused_pane_id,
            doc,
        );
        self.detect_and_set_language(bid);
        let val = SteelBufferId::new(bid).into_steel_val();
        self.fire_hook_silent(HookId::OnBufferOpen, &[val]);
        bid
    }

    /// Remove buffer `id`, handling two cases:
    ///
    /// - At least one other buffer: redirect every pane viewing `id` to the
    ///   MRU replacement, then free the slot.
    /// - Only buffer: replace in-place with a fresh scratch buffer.
    pub(crate) fn close_buffer(&mut self, id: BufferId) {
        ops::close_buffer(
            &mut self.view,
            &mut self.state.buffers,
            &mut self.state.panes.state,
            &mut self.state.panes.jumps,
            self.state.focused_pane_id,
            id,
        );
        // Fire with the ID that was closed, not the new current buffer.
        let val = SteelBufferId::new(id).into_steel_val();
        self.fire_hook_silent(HookId::OnBufferClose, &[val]);
    }

    /// Replace buffer `id` with `new_doc` in-place, preserving the primary cursor
    /// line/column across the reload.
    ///
    /// Wraps [`replace_buffer_in_place`](Self::replace_buffer_in_place) (which
    /// resets selections to 0,0) by capturing each pane's primary cursor
    /// as `(line, col)` before the reset and restoring it against the new content.
    ///
    /// Multi-selections collapse to the primary — they are stale against fresh
    /// content. Cursor is clamped if the file shrank: past-end lines land on the
    /// new last line; past-end columns land on the line's `\n`. The viewport's
    /// `top_line` is also clamped so a shrunken file can't leave the scroll past
    /// the new last line.
    ///
    /// Used by the no-arg `:e`/`:e!` reload branch.
    pub(crate) fn reload_buffer_in_place(&mut self, id: BufferId, new_doc: Buffer) {
        use hume_editing::{Selection, SelectionSet, snap_to_grapheme_boundary};

        // ── Phase 1: capture (line, col) for every pane viewing this buffer ───
        // All borrows on self.view and self.state closed before the mutable reset.
        let pane_ids: Vec<PaneId> = self
            .view
            .panes
            .iter()
            .filter(|(_, p)| p.buffer_id == id)
            .map(|(pid, _)| pid)
            .collect();
        let cursor_coords: Vec<(PaneId, usize, usize)> = {
            let text = self.state.buffers.get(id).text();
            pane_ids
                .iter()
                .map(|&pid| {
                    let head = self.state.panes.state[pid][id].selections.primary().head();
                    let line = text.char_to_line(head);
                    let col = head - text.line_to_char(line);
                    (pid, line, col)
                })
                .collect()
        }; // borrows on text and panes.state end here

        // ── Phase 2: reset (reseeds pane state to 0,0, clears engine syntax) ─
        self.replace_buffer_in_place(id, new_doc);

        // ── Phase 3: restore positions against new content ────────────────────
        // Compute positions while holding the text borrow, then apply.
        let new_text = self.state.buffers.get(id).text();
        let last_line = new_text.len_lines().saturating_sub(2);
        let positions: Vec<(PaneId, usize)> = cursor_coords
            .into_iter()
            .map(|(pid, line, col)| {
                let target_line = line.min(last_line);
                let line_start = new_text.line_to_char(target_line);
                // line_end: position of the \n that terminates target_line.
                // target_line <= last_line = len_lines - 2, so target_line + 1
                // < len_lines — line_to_char is safe.
                let line_end = new_text.line_to_char(target_line + 1).saturating_sub(1);
                let target = (line_start + col).min(line_end);
                let head = snap_to_grapheme_boundary(new_text, line_start, target);
                (pid, head)
            })
            .collect(); // new_text borrow ends at ;

        for (pid, head) in positions {
            self.state.panes.state[pid][id].selections =
                SelectionSet::single(Selection::collapsed(head));
            // Clamp scroll: a shrunken file must not leave top_line past last_line.
            let top = self.view.panes[pid].viewport.top_line;
            self.view.panes[pid].viewport.top_line = top.min(last_line);
        }
    }

    /// Replace buffer `id` with `new_doc` in-place, reseeding all pane state.
    ///
    /// Used by `:e!` reload. Caller contract: `new_doc.search_pattern` must be `None`
    /// (enforced by debug_assert — `Buffer::from_file` satisfies this by construction).
    pub(crate) fn replace_buffer_in_place(&mut self, id: BufferId, new_doc: Buffer) {
        ops::replace_buffer_in_place(
            &mut self.view,
            &mut self.state.buffers,
            &mut self.state.panes.state,
            &mut self.state.panes.jumps,
            id,
            new_doc,
        );
        // Re-detect language and rebuild syntax (mirrors open_buffer). For scratch
        // (no path/language), detect returns None and set_buffer_language no-ops —
        // the engine-side clear in ops::replace_buffer_in_place is the cleanup there.
        self.detect_and_set_language(id);
    }

    /// Redirect the focused pane to `target` without recording a jump.
    pub(crate) fn switch_to_buffer_without_jump(&mut self, target: BufferId) {
        let pid = self.state.focused_pane_id;
        ops::switch_pane_to_buffer(
            &mut self.view,
            &self.state.buffers,
            &mut self.state.panes.state,
            pid,
            target,
        );
    }

    /// Redirect the focused pane to `target`, recording the outgoing position
    /// in `panes.jumps[focused_pane]`.
    ///
    /// Caller contract: all fallible steps (path resolution, file read, etc.)
    /// must succeed before calling this — `push()` truncates forward history.
    pub(crate) fn switch_to_buffer_with_jump(&mut self, target: BufferId) {
        let current = self.focused_buffer_id();
        ops::switch_to_buffer_with_jump(
            &mut self.view,
            &self.state.buffers,
            &mut self.state.panes.state,
            &mut self.state.panes.jumps,
            self.state.focused_pane_id,
            current,
            target,
        );
    }

    /// Open or refresh a read-only view buffer (`:messages`, `:ls`, `:plugin-status`).
    ///
    /// If a buffer with this label already exists, replaces its content in-place
    /// so repeated calls don't accumulate duplicates in `:ls`. Otherwise opens a
    /// fresh read-only buffer. Then switches the focused pane to it and positions
    /// the cursor at `cursor_line` (0-indexed, clamped to last content line).
    pub(crate) fn open_read_only_view(
        &mut self,
        label: &'static str,
        content: &str,
        cursor_line: usize,
    ) {
        use hume_editing::selection::{Selection, SelectionSet};
        use hume_editing::text::Text;

        let text = Text::from(content);
        let bid = if let Some(existing) = self.state.buffers.find_by_label(label) {
            self.state.buffers.get_mut(existing).set_view_content(text);
            existing
        } else {
            let doc = Buffer::read_only_view(text, label.to_owned());
            self.open_buffer(doc)
        };

        let already_focused = self.focused_buffer_id() == bid;
        if !already_focused {
            self.switch_to_buffer_without_jump(bid);
        }

        // Position cursor at the requested line (clamped to last content line).
        let pid = self.state.focused_pane_id;
        let rope = self.state.buffers.get(bid).text().rope();
        let last_content = rope.len_lines().saturating_sub(2); // skip trailing \n line
        let target_line = cursor_line.min(last_content);
        let char_pos = rope.line_to_char(target_line);
        self.state.panes.state[pid][bid].selections =
            SelectionSet::single(Selection::collapsed(char_pos));
    }
}
