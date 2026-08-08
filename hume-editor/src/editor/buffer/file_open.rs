use std::io;
use std::path::PathBuf;

use hume_engine::pipeline::{BufferId, PaneId};

use crate::editor::buffer::Buffer;

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
        let canonical = std::fs::canonicalize(path)?;
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

    /// Resolve a typed path to the canonical form used as buffer identity
    /// (`BufferStore::find_by_path`) and, on save, as `FileMeta::resolved_path`.
    ///
    /// Canonicalizing the whole path requires the file to exist. When it
    /// doesn't, canonicalize the parent instead and re-append the basename —
    /// this still resolves symlinks in the parent chain (e.g. `/tmp` →
    /// `/private/tmp` on macOS), so a new-file buffer opened via
    /// `/tmp/x.txt` keys identically to one opened via its canonical form,
    /// and to the `FileMeta` `:w` produces once the file is written. Falls
    /// back to the lexically-normalized path only when the parent doesn't
    /// exist either (nested missing directories) — `open_or_dedup`'s
    /// `NotFound` branch still opens the buffer; only identity across
    /// re-typed forms is imprecise in that case.
    ///
    /// An associated function, not a `&self` method: `Editor::open` needs it
    /// during construction, before `self` exists, passing its local
    /// `startup_cwd` instead of `self.state.cwd`.
    pub(in crate::editor) fn resolve_buffer_path(
        typed: &std::path::Path,
        cwd: &std::path::Path,
    ) -> PathBuf {
        let lexical = hume_platform::path::absolute_unresolved(typed, cwd);
        if let Ok(canonical) = std::fs::canonicalize(&lexical) {
            return canonical;
        }
        match (lexical.parent(), lexical.file_name()) {
            (Some(parent), Some(name)) => std::fs::canonicalize(parent)
                .map(|p| p.join(name))
                .unwrap_or(lexical),
            _ => lexical,
        }
    }

    /// Dedup-open a resolved path: returns `(id, false)` if already open,
    /// `(id, true)` if newly opened (including `OnBufferOpen` hook fire).
    ///
    /// `resolved` is canonical when the file exists, or the best-effort form
    /// [`resolve_buffer_path`] produces when it doesn't (parent canonicalized,
    /// basename appended lexically) — `find_by_path` compares whichever form
    /// was stored, so dedup still works once the file is later created and
    /// reopened via its now-canonicalizable path.
    ///
    /// Thin wrapper over [`lifecycle::open_or_dedup_and_notify`] — the actual
    /// dedup-and-missing-file logic lives there so Steel's `open-buffer!` and
    /// LSP goto/workspace-edit share it too; this only adds the
    /// `&Editor`-only language detection a genuinely new buffer needs.
    pub(in crate::editor) fn open_or_dedup(
        &mut self,
        resolved: &std::path::Path,
    ) -> std::io::Result<(BufferId, bool)> {
        let (bid, is_new) =
            lifecycle::open_or_dedup_and_notify(&mut self.view, &mut self.state, resolved)?;
        if is_new {
            // Steel eval capability only `&mut Editor` has — see
            // `open_buffer_and_notify`'s doc for why detection can't live there.
            self.detect_pending_languages();
        }
        Ok((bid, is_new))
    }

    /// Open additional files without switching focus; errors are logged as
    /// warnings. A path that doesn't exist opens a new-file buffer instead
    /// of erroring (see `resolve_open_path`) and reports Info `[new file]`,
    /// matching `:e` — otherwise a mistyped trailing CLI argument would
    /// silently open an empty buffer with no feedback at all.
    pub(crate) fn open_extra_files(&mut self, paths: &[PathBuf]) {
        for path in paths {
            match self.try_open_extra(path) {
                Ok((bid, is_new)) => {
                    let buf = self.state.buffers.get(bid);
                    if is_new && buf.is_new_file() {
                        let name = buf.display_name();
                        self.report(Severity::Info, format!("{name} [new file]"));
                    }
                }
                Err(e) => {
                    self.report(
                        Severity::Warning,
                        format!("Failed to open {}: {e}", path.display()),
                    );
                }
            }
        }
    }

    /// Resolve a path argument to an open buffer, opening the file if it isn't
    /// already open — reading it if it exists, or opening an empty
    /// [`Buffer::new_file`] bound to the path if it doesn't (see
    /// `resolve_buffer_path`). Shared sequence: `expand` →
    /// `absolute_unresolved` + `display_form` (display path) →
    /// `resolve_buffer_path` → `open_or_dedup` → `set_display_path` if new
    /// (overwriting `Buffer::from_file`'s canonical-derived default with the
    /// typed-derived form).
    /// Errors propagate as raw `io::Error`; callers format with whichever path
    /// string suits their reporting.
    pub(in crate::editor) fn resolve_open_path(
        &mut self,
        path_str: &str,
    ) -> io::Result<(BufferId, bool)> {
        let expanded = hume_platform::path::expand(path_str);
        let path = std::path::Path::new(expanded.as_ref());
        let display = hume_platform::path::display_form(&hume_platform::path::absolute_unresolved(
            path,
            &self.state.cwd,
        ));
        let resolved = Self::resolve_buffer_path(path, &self.state.cwd);
        let (bid, is_new) = self.open_or_dedup(&resolved)?;
        if is_new {
            self.state
                .buffers
                .get_mut(bid)
                .set_display_path(Some(display));
        }
        Ok((bid, is_new))
    }

    fn try_open_extra(&mut self, path: &std::path::Path) -> io::Result<(BufferId, bool)> {
        self.resolve_open_path(&path.to_string_lossy())
    }

    /// Allocate a new buffer slot (engine + BufferStore), seed the focused pane's
    /// per-buffer state (`state.panes.state`), and return the allocated `BufferId`.
    pub(crate) fn open_buffer(&mut self, doc: Buffer) -> BufferId {
        let bid = lifecycle::open_buffer_and_notify(&mut self.view, &mut self.state, doc);
        // Steel eval capability only `&mut Editor` has — see
        // `open_buffer_and_notify`'s doc for why detection can't live there.
        self.detect_pending_languages();
        bid
    }

    /// Remove buffer `id`, handling two cases:
    ///
    /// - At least one other buffer: redirect every pane viewing `id` to the
    ///   MRU replacement, then free the slot.
    /// - Only buffer: replace in-place with a fresh scratch buffer.
    pub(crate) fn close_buffer(&mut self, id: BufferId) {
        lifecycle::close_buffer_and_notify(
            &mut self.view,
            &mut self.state,
            Some(&mut self.lsp),
            id,
        );
    }

    /// Reload buffer `id` with `new_doc`'s content in place, preserving the
    /// undo tree and the primary cursor line/column across the reload.
    ///
    /// Unlike `replace_buffer_in_place` (which
    /// swaps the whole `Buffer` and discards `History`), this delegates to
    /// [`Buffer::reload_from_text`] — see its doc for the history/undo mechanics.
    ///
    /// Each pane's primary cursor is captured as `(line, col)` before the
    /// reload and restored against the new content; multi-selections collapse
    /// to the primary (stale against fresh content). Cursor and `top_line`
    /// are clamped if the file shrank: past-end lines land on the new last
    /// line, past-end columns on the line's `\n`.
    ///
    /// Only the focused pane's pre/post selections are written into the
    /// history revision (undo/redo restore its cursor); other panes on the
    /// same buffer ride the inverse `ChangeSet` via `propagate_cs_to_panes`
    /// like any edit.
    ///
    /// Survives the reload: jump list, per-buffer search state (match cache
    /// rebuilds lazily). Dropped as stale: in-progress edit groups/paste
    /// sessions, the engine-side syntax tree, and saved scrolls.
    ///
    /// Used by the no-arg `:e`/`:e!` reload branch.
    pub(crate) fn reload_buffer_in_place(&mut self, id: BufferId, mut new_doc: Buffer) {
        use hume_editing::lines::snap_to_grapheme_boundary;
        use hume_editing::selection::{Selection, SelectionSet};

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
            let last_line = new_text.last_content_line();
            let mut heads = Vec::with_capacity(cursor_coords.len());
            for &(pid, line, col) in &cursor_coords {
                let target_line = line.min(last_line);
                let line_start = new_text.line_to_char(target_line);
                // target_line <= last_line = last_content_line(), so
                // target_line + 1 <= last_ropey_line() < ropey_line_count() —
                // line_to_char is safe.
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
        // Refresh `file_meta` so save-time permission/ownership checks see
        // the current on-disk metadata — `reload_from_text` only replaces
        // the buffer's text, not its `file_meta`, so this must be set
        // explicitly.
        let new_text = new_doc.text().clone();
        let new_file_meta = std::mem::take(&mut new_doc.file_meta);
        drop(new_doc);

        let mutated = self
            .state
            .buffers
            .get_mut(id)
            .reload_from_text(new_text, pre_sels, post_sels);
        self.state.buffers.get_mut(id).file_meta = new_file_meta;
        // Flush any didChange already queued for this buffer *before* the
        // whole-document one below — otherwise, under macro replay (an edit
        // followed by `:e!` in the same drain window), the server would see
        // the full reloaded text at the new version first and the queued
        // incremental change (computed against the pre-reload text, at an
        // *older* version) after it: a version regression the server can't
        // recover from, permanently desyncing its copy of the document.
        self.flush_lsp_pending_changes();
        // Everything below discards state computed against the pre-reload
        // text — diagnostics/decorations char offsets, the engine syntax
        // tree, a whole-document didChange at a fresh version. A no-op
        // reload (`mutated == false`) never touched `self.text` or
        // `text_gen`, so that state is still valid against the (unchanged)
        // current content — skip discarding it rather than throw away
        // perfectly good syntax highlighting/diagnostics for nothing.
        if mutated {
            // `reload_from_text` bumped text_gen via set_text but produced no
            // ChangeSet the LSP pending-queue mechanism can consume — send the
            // reload as a whole-document didChange instead.
            self.lsp_did_change_whole_document(id);
            // Diagnostics and LSP-sourced decorations were computed against the
            // pre-reload text — their char offsets are meaningless (and
            // potentially out-of-bounds, e.g. after a shrink) against the new
            // content. The server republishes diagnostics shortly after seeing
            // the didChange above; nothing republishes decorations on its own,
            // so they simply stay cleared until a plugin sets them again.
            if self.lsp.remove_buffer_diagnostics(id) {
                self.queue_diagnostics_changed(id);
            }
            self.state.config.decorations.remove_buffer(id);

            // Drop the stale committed layers (they reference pre-reload
            // content), keeping the grammar attachment and generation
            // bookkeeping intact. `set_text` bumped `text_gen`, so
            // `reparse_stale_buffers` will post a fresh full parse on the
            // next tick.
            if let Some(syn) = self.state.buffers.get_mut(id).syntax.as_mut() {
                syn.clear_layers();
            }
        }
        // `detect_and_set_language` handles a genuine language change
        // (shebang/extension) regardless of `mutated` — re-running setup via
        // `set_buffer_language` itself.
        self.detect_and_set_language(id);

        // ── Phase 3: reseed per-pane selections / edit groups / scroll ───────
        // Targeted, not `fresh_from_buf`: selections are restored to the clamped
        // post-reload cursor; stale edit groups / paste sessions drop (an open
        // group against pre-reload text cannot compose against the new text).
        let last_line = self.state.buffers.get(id).text().last_content_line();
        for &(pid, head) in &post_heads {
            self.state.panes.state[pid][id].selections =
                SelectionSet::single(Selection::collapsed(head));
            self.state.panes.state[pid][id].edit_group = None;
            self.state.panes.state[pid][id].paste_group = None;
            // Clamp scroll: a shrunken file must not leave top_line past last_line.
            let top = self.view.panes[pid].viewport.top_line;
            self.view.panes[pid].viewport.top_line = top.min(last_line);
        }
        // Drop stale saved scrolls for the reloaded buffer on every pane —
        // `recall_scroll` clamps `top_line` to the buffer's current last
        // line, but a saved `top_row_offset`/`horizontal_offset` for a
        // scroll position that no longer exists is still worth discarding
        // outright rather than recalling a clamped-but-arbitrary spot. The
        // jump list is preserved — the same buffer id survives, so existing
        // jumps still reference valid positions.
        for pane in self.view.panes.values_mut() {
            pane.forget_buffer(id);
        }
    }

    /// Replace buffer `id` with `new_doc` in-place, reseeding all pane state.
    ///
    /// History-discarding whole-`Buffer` swap. Test-only entry point for the
    /// `lifecycle::replace_buffer_in_place` reset path (scratch-swap on
    /// last-buffer close, read-only-view refresh invariants). Production
    /// callers go through `lifecycle::replace_buffer_in_place` directly
    /// (`close_buffer`'s last-buffer branch); the `:e!` reload path uses
    /// [`reload_buffer_in_place`](Self::reload_buffer_in_place) instead, which
    /// preserves undo history.
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

    /// Switch the focused pane to `target`, or no-op if it's already focused
    /// — the `:e`/`:b`/`:bn`/`:bp` entry point. Unlike
    /// `switch_to_buffer_with_jump`, safe to call with a target that might
    /// already be the focused buffer: that primitive's `push()` truncates
    /// forward jump history unconditionally, so a same-buffer call would
    /// corrupt it for nothing.
    ///
    /// External-change detection no longer lives here: every genuine switch
    /// this produces raises `EditorEvent::OnBufferEnter`, observed by
    /// `Editor::settle`'s diff regardless of caller — interactive or not. A
    /// no-op call raises nothing, matching Vim's `BufEnter`, which doesn't
    /// re-fire for re-entering the buffer you're already viewing.
    ///
    /// Accepted cost of that parity: `:e`/`:b`/`:bn`/`:bp` re-targeting the
    /// already-focused buffer runs no disk stat at all — it's genuinely a
    /// no-op, not a deferred one. An external change to that file still
    /// surfaces the moment any of terminal `FocusIn`, a genuine buffer-enter
    /// (switch away and back), or `:checktime` runs — see
    /// `Editor::enter_buffer_disk_check`'s doc for the full list of paths
    /// that *do* stat.
    pub(in crate::editor) fn enter_buffer(&mut self, target: BufferId) {
        if target != self.focused_buffer_id() {
            self.switch_to_buffer_with_jump(target);
        }
    }

    /// Open or refresh a read-only view buffer (`:messages`, `:ls`, `:plugin-status`).
    ///
    /// If a buffer with this label already exists, replaces its content in-place
    /// so repeated calls don't accumulate duplicates in `:ls`. Otherwise opens a
    /// fresh read-only buffer. Then switches the focused pane to it and positions
    /// the cursor at `cursor_line` (0-indexed, clamped to last content line).
    /// Returns the view buffer's id, e.g. for callers attaching decorations
    /// (`:messages`'s severity highlights) that must target this specific
    /// buffer rather than whatever ends up focused.
    pub(crate) fn open_read_only_view(
        &mut self,
        label: &'static str,
        content: &str,
        cursor_line: usize,
    ) -> BufferId {
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
        let text = self.state.buffers.get(bid).text();
        let target_line = cursor_line.min(text.last_content_line());
        let char_pos = text.line_to_char(target_line);
        self.state.panes.state[pid][bid].selections =
            SelectionSet::single(Selection::collapsed(char_pos));

        bid
    }
}
