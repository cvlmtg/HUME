use std::io;
use std::path::PathBuf;

use hume_engine::pipeline::{BufferId, PaneId};

use crate::editor::buffer::Buffer;
use hume_scripting::SteelBufferId;
use hume_scripting::hooks::HookId;

use super::lifecycle;
use crate::editor::{Editor, Severity};

impl Editor {
    // ── Working directory ─────────────────────────────────────────────────────

    /// Change the editor's working directory.
    ///
    /// Canonicalizes `path`, rejects non-directories, then updates both
    /// `self.state.cwd` and the process cwd so that relative paths in `:e` and
    /// subprocesses resolve consistently.
    pub(in crate::editor) fn set_cwd(&mut self, path: &std::path::Path) -> io::Result<PathBuf> {
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
    pub(in crate::editor) fn open_or_dedup(
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

    /// Resolve a path argument to an open buffer, opening the file if it isn't
    /// already open. Shared sequence: `expand` → `absolute_unresolved` (display
    /// path) → `canonicalize` → `open_or_dedup` → `set_display_path` if new.
    /// Errors propagate as raw `io::Error`; callers format with whichever path
    /// string suits their reporting.
    pub(in crate::editor) fn resolve_open_path(
        &mut self,
        path_str: &str,
    ) -> io::Result<(BufferId, bool)> {
        let expanded = hume_platform::path::expand(path_str);
        let path = std::path::Path::new(expanded.as_ref());
        let display = hume_platform::path::absolute_unresolved(path, &self.state.cwd);
        let canonical = hume_platform::fs::canonicalize(path)?;
        let (bid, is_new) = self.open_or_dedup(&canonical)?;
        if is_new {
            self.state
                .buffers
                .get_mut(bid)
                .set_display_path(Some(display));
        }
        Ok((bid, is_new))
    }

    fn try_open_extra(&mut self, path: &std::path::Path) -> io::Result<()> {
        self.resolve_open_path(&path.to_string_lossy())?;
        Ok(())
    }

    /// Allocate a new buffer slot (engine + BufferStore), seed the focused pane's
    /// per-buffer state (`state.panes.state`), and return the allocated `BufferId`.
    pub(crate) fn open_buffer(&mut self, doc: Buffer) -> BufferId {
        let bid = lifecycle::open_buffer(
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
        lifecycle::close_buffer(
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

    /// Reload buffer `id` with `new_doc`'s content in place, preserving the
    /// undo tree and the primary cursor line/column across the reload.
    ///
    /// Unlike [`replace_buffer_in_place`](Self::replace_buffer_in_place) (which
    /// swaps the whole `Buffer` and discards `History`), this records the
    /// reload as an ordinary edit via [`Buffer::reload_from_text`]: `u` after
    /// `:e!` reverts to the pre-reload buffer with its full prior undo tree
    /// intact beneath; `Ctrl-r` re-applies the reload. The inverse `ChangeSet`
    /// is line-diff-derived so it carries only the changed lines, not a
    /// full-buffer delete-all + insert-all.
    ///
    /// Each pane's primary cursor is captured as `(line, col)` before the
    /// reload and restored against the new content. Multi-selections collapse
    /// to the primary (they are stale against fresh content). The cursor is
    /// clamped if the file shrank: past-end lines land on the new last line;
    /// past-end columns land on the line's `\n`. Each pane's `top_line` is
    /// clamped the same way.
    ///
    /// The focused pane's pre- and post-reload selections are the ones written
    /// into the history revision: undo restores the focused pane's pre-reload
    /// cursor; redo restores the focused pane's clamped post-reload cursor.
    /// Other panes viewing the same buffer are not stranded — their cursors
    /// ride the inverse `ChangeSet` via `propagate_cs_to_panes` when the undo
    /// runs, exactly as for any ordinary edit; only the explicit cursor restore
    /// is focused-pane-specific.
    ///
    /// State that survives (same buffer id, valid positions): jump list
    /// (prune stays in the `close_buffer` path), per-buffer search state
    /// (reload is treated as an edit — an active search persists and its match
    /// cache re-builds lazily against the new revision). State that is dropped
    /// as stale: in-progress edit groups / paste sessions, engine-side syntax
    /// tree (re-detected and re-parsed downstream), and saved scrolls (a
    /// shrunken file could otherwise leave a recalled scroll past the new last
    /// line).
    ///
    /// Used by the no-arg `:e`/`:e!` reload branch.
    pub(crate) fn reload_buffer_in_place(&mut self, id: BufferId, mut new_doc: Buffer) {
        use hume_editing::{Selection, SelectionSet, snap_to_grapheme_boundary};

        // ── Phase 1: capture (line, col) per pane + focused pane's pre_sels ──
        let pane_ids: Vec<PaneId> = self
            .view
            .panes
            .iter()
            .filter(|(_, p)| p.buffer_id == id)
            .map(|(pid, _)| pid)
            .collect();
        let focused = self.state.focused_pane_id;
        let pre_sels = self.state.panes.state[focused][id].selections.clone();

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

        // ── Phase 2: clamp (line, col) against the new text ──────────────────
        // Borrow `new_doc.text()` immutably, then move `new_text` out below.
        let post_heads: Vec<(PaneId, usize)> = {
            let new_text = new_doc.text();
            let last_line = new_text.len_lines().saturating_sub(2);
            let mut heads = Vec::with_capacity(cursor_coords.len());
            for &(pid, line, col) in &cursor_coords {
                let target_line = line.min(last_line);
                let line_start = new_text.line_to_char(target_line);
                // target_line <= last_line = len_lines - 2, so target_line + 1
                // < len_lines — line_to_char is safe.
                let line_end = new_text.line_to_char(target_line + 1).saturating_sub(1);
                let target = (line_start + col).min(line_end);
                let head = snap_to_grapheme_boundary(new_text, line_start, target);
                heads.push((pid, head));
            }
            heads
        }; // new_text borrow ends here

        // The caller always reloads the focused buffer (`:e!` passes
        // `focused_buffer_id()`), so the focused pane is in `pane_ids` and thus
        // in `post_heads`. A miss means an internal invariant broke — fail loud
        // rather than silently anchoring undo to char 0.
        let focused_post_head = post_heads
            .iter()
            .find(|(pid, _)| *pid == focused)
            .map(|(_, h)| *h)
            .expect("focused pane must view the reloaded buffer");
        let post_sels = SelectionSet::single(Selection::collapsed(focused_post_head));

        // ── Phase 2b: history-preserving reload ──────────────────────────────
        // Refresh `file_meta` so save-time permission/ownership checks see the
        // current on-disk metadata (the whole-Buffer swap this replaces also
        // picked up the fresh `from_file` metadata).
        let new_text = new_doc.text().clone();
        let new_file_meta = std::mem::take(&mut new_doc.file_meta);
        drop(new_doc);

        self.state
            .buffers
            .get_mut(id)
            .reload_from_text(new_text, pre_sels, post_sels);
        self.state.buffers.get_mut(id).file_meta = new_file_meta;

        // Drop the stale engine tree (it references pre-reload content). The
        // highlighter in `state.syntax` survives reload untouched; `set_text` bumped
        // `text_gen`, so `reparse_stale_buffers` will post a fresh full parse on the
        // next tick. `detect_and_set_language` handles a genuine language change
        // (shebang/extension), re-running setup via `set_buffer_language` itself.
        lifecycle::clear_engine_tree(&mut self.view, id);
        self.detect_and_set_language(id);

        // ── Phase 3: reseed per-pane selections / edit groups / scroll ───────
        // Targeted, not `fresh_from_buf`: selections are restored to the clamped
        // post-reload cursor; stale edit groups / paste sessions drop (an open
        // group against pre-reload text cannot compose against the new text).
        let last_line = self
            .state
            .buffers
            .get(id)
            .text()
            .len_lines()
            .saturating_sub(2);
        for &(pid, head) in &post_heads {
            self.state.panes.state[pid][id].selections =
                SelectionSet::single(Selection::collapsed(head));
            self.state.panes.state[pid][id].edit_group = None;
            self.state.panes.state[pid][id].paste_group = None;
            // Clamp scroll: a shrunken file must not leave top_line past last_line.
            let top = self.view.panes[pid].viewport.top_line;
            self.view.panes[pid].viewport.top_line = top.min(last_line);
        }
        // Drop stale saved scrolls for the reloaded buffer on every pane. A
        // shrunken file could otherwise leave a recalled scroll past the new
        // last line (recall_scroll does not clamp). The jump list is preserved
        // — the same buffer id survives, so existing jumps still reference
        // valid positions.
        for pane in self.view.panes.values_mut() {
            pane.forget_buffer(id);
        }
    }

    /// Replace buffer `id` with `new_doc` in-place, reseeding all pane state.
    ///
    /// History-discarding whole-`Buffer` swap. The `:e!` reload path now uses
    /// [`reload_buffer_in_place`](Self::reload_buffer_in_place) (history-preserving)
    /// instead; this wrapper survives for tests that exercise the
    /// `lifecycle::replace_buffer_in_place` reset path (scratch-swap on last-buffer
    /// close, read-only-view refresh invariants). The prod non-test callers go
    /// through `lifecycle::replace_buffer_in_place` directly (`close_buffer`'s
    /// last-buffer branch).
    ///
    /// Caller contract: `new_doc.search_pattern` must be `None` (enforced by
    /// debug_assert — `Buffer::from_file` satisfies this by construction).
    #[cfg(test)]
    pub(crate) fn replace_buffer_in_place(&mut self, id: BufferId, new_doc: Buffer) {
        lifecycle::replace_buffer_in_place(
            &mut self.view,
            &mut self.state.buffers,
            &mut self.state.panes.state,
            &mut self.state.panes.jumps,
            id,
            new_doc,
        );
        // Re-detect language and rebuild syntax (mirrors open_buffer). For scratch
        // (no path/language), detect returns None, set_buffer_language no-ops, and
        // the replaced Buffer.syntax = None already dropped the old highlighter.
        self.detect_and_set_language(id);
    }

    /// Redirect the focused pane to `target` without recording a jump.
    pub(crate) fn switch_to_buffer_without_jump(&mut self, target: BufferId) {
        let pid = self.state.focused_pane_id;
        lifecycle::switch_pane_to_buffer(
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
        lifecycle::switch_to_buffer_with_jump(
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
