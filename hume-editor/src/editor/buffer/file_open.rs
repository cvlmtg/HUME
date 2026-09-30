use std::io;
use std::path::PathBuf;

use hume_engine::pipeline::BufferId;

use crate::editor::buffer::Buffer;
use crate::editor::commands::FocusedPane;

use super::lifecycle;
use crate::editor::position_stores::PositionStores;
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
    /// doesn't, canonicalize the parent instead and re-append the basename:
    /// this still resolves symlinks in the parent chain (e.g. `/tmp` →
    /// `/private/tmp` on macOS), so a new-file buffer opened via
    /// `/tmp/x.txt` keys identically to one opened via its canonical form,
    /// and to the `FileMeta` `:w` produces once the file is written. Falls
    /// back to the lexically-normalized path only when the parent doesn't
    /// exist either (nested missing directories); `open_or_dedup`'s
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
    /// basename appended lexically). `find_by_path` compares whichever form
    /// was stored, so dedup still works once the file is later created and
    /// reopened via its now-canonicalizable path.
    ///
    /// Thin wrapper over [`lifecycle::open_or_dedup_and_notify`]; the actual
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
            // Steel eval capability only `&mut Editor` has; see
            // `open_buffer_and_notify`'s doc for why detection can't live there.
            self.detect_pending_languages();
        }
        Ok((bid, is_new))
    }

    /// `{name} [new file]`, the message reported when a newly opened `buf`
    /// has nothing on disk yet. `None` for a genuinely read file.
    pub(in crate::editor) fn new_file_open_msg(buf: &Buffer) -> Option<String> {
        buf.is_new_file()
            .then(|| format!("{} [new file]", buf.display_name()))
    }

    /// Open an additional file without switching focus; an error is logged
    /// as a warning and yields `None`. A path that doesn't exist opens a
    /// new-file buffer instead of erroring (see `resolve_open_path`) and
    /// reports Info `[new file]`, matching `:e`. Otherwise a mistyped
    /// trailing CLI argument would silently open an empty buffer with no
    /// feedback at all.
    pub(crate) fn open_extra_file(&mut self, path: &std::path::Path) -> Option<BufferId> {
        match self.try_open_extra(path) {
            Ok((bid, is_new)) => {
                if is_new && let Some(msg) = Self::new_file_open_msg(self.state.buffers.get(bid)) {
                    self.report(Severity::Info, msg);
                }
                Some(bid)
            }
            Err(e) => {
                self.report(
                    Severity::Warning,
                    format!("Failed to open {}: {e}", path.display()),
                );
                None
            }
        }
    }

    /// Resolve a path argument to an open buffer, opening the file if it isn't
    /// already open: reading it if it exists, or opening an empty
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

    /// Allocate a new buffer slot (engine + BufferStore) and return the
    /// allocated `BufferId`; see `lifecycle::open_buffer`'s own doc for why
    /// no pane is seeded yet.
    pub(in crate::editor) fn open_buffer(&mut self, doc: Buffer) -> BufferId {
        let bid = lifecycle::open_buffer_and_notify(&mut self.view, &mut self.state, doc);
        // Steel eval capability only `&mut Editor` has; see
        // `open_buffer_and_notify`'s doc for why detection can't live there.
        self.detect_pending_languages();
        bid
    }

    /// Remove buffer `id`; see [`lifecycle::close_buffer`]'s own doc for
    /// the last-buffer case (a fresh scratch buffer, not `id` reused).
    pub(in crate::editor) fn close_buffer(&mut self, id: BufferId) {
        lifecycle::close_buffer_and_notify(
            &mut self.view,
            &mut self.state,
            Some(&mut self.lsp),
            id,
        );
        // Mirrors `open_buffer`'s own call, right above: the last-buffer
        // case queues a fresh scratch buffer for language detection and
        // `OnBufferOpen` (`close_buffer_and_notify`'s `queue_open_announcement`
        // call): this is `(close-buffer! id)`'s Steel path's own capability
        // (`apply_script_effects` calls this unconditionally at the end of
        // every command dispatch), but a direct `&mut Editor` caller like
        // this one has no such funnel to fall back on.
        self.detect_pending_languages();
    }

    /// Reload `fp`'s buffer with `new_doc`'s content in place, preserving the
    /// undo tree.
    ///
    /// Unlike `set_view_content` (which discards `History` on a full
    /// `Buffer` swap), this delegates to [`Buffer::reload_from_text`]; see
    /// its doc for the history/undo mechanics.
    ///
    /// The reload is an edit: `Buffer::reload_from_text` carries every stored
    /// position (every pane's selections for the buffer, jump lists, prompt
    /// snapshots) through its line-diff `ChangeSet`, so each follows its text.
    /// Only `fp`'s pre/post selections are written into the history revision
    /// (undo/redo restore its cursor).
    ///
    /// Survives the reload: per-buffer search state (match cache rebuilds
    /// lazily) and the syntax tree, which the reload's edit shifts like any
    /// other. Dropped as stale: in-progress edit groups/paste sessions and
    /// saved scrolls.
    pub(in crate::editor) fn reload_buffer_in_place(
        &mut self,
        fp: FocusedPane,
        mut new_doc: Buffer,
    ) {
        let id = fp.bid(&self.view);
        // End any open Insert/paste session the same way every other
        // buffer/focus-invalidating path does (`switch_pane_to_buffer`,
        // `reset_config_state`), before the reload invalidates the text it
        // was snapshotted against. Leaving it open would keep
        // `state.active_session` and the `Insert` mode layer pointing at a
        // session whose group no longer matches the buffer.
        crate::editor::focus::end_focus_sessions(&mut self.state, &self.view);

        // History-preserving reload.
        // Refresh `file_meta` so save-time permission/ownership checks see
        // the current on-disk metadata: `reload_from_text` only replaces
        // the buffer's text, not its `file_meta`, so this must be set
        // explicitly.
        let new_text = new_doc.text().clone();
        let new_file_meta = std::mem::take(&mut new_doc.file_meta);
        drop(new_doc);

        let mutated = self.state.buffers.get_mut(id).reload_from_text(
            id,
            &mut PositionStores::new(
                &mut self.state.panes,
                &mut self.state.input,
                &mut self.state.buffer_positions,
                &mut self.state.config.decorations,
            ),
            new_text,
            fp.pid(),
        );
        self.state.buffers.get_mut(id).file_meta = new_file_meta;
        // Flush any didChange already queued for this buffer *before* the
        // whole-document one below. Otherwise, under macro replay (an edit
        // followed by `:e!` in the same drain window), the server would see
        // the full reloaded text at the new version first and the queued
        // incremental change (computed against the pre-reload text, at an
        // *older* version) after it: a version regression the server can't
        // recover from, permanently desyncing its copy of the document.
        self.flush_lsp_pending_changes();
        // Everything below discards state computed against the pre-reload
        // text: diagnostics/decorations char offsets, and sends a
        // whole-document didChange at a fresh version. A no-op
        // reload (`mutated == false`) never touched `self.text` or
        // the text version, so that state is still valid against the
        // unchanged content and is kept.
        if mutated {
            // `reload_from_text` changed the text version but produced no
            // *queued incremental* change the LSP pending-queue mechanism can
            // consume, so send the reload as a whole-document didChange instead.
            self.lsp_did_change_whole_document(id);
            // Diagnostics and LSP-sourced decorations were computed against the
            // pre-reload text; their char offsets are meaningless (and
            // potentially out-of-bounds, e.g. after a shrink) against the new
            // content. The server republishes diagnostics shortly after seeing
            // the didChange above; nothing republishes decorations on its own,
            // so they simply stay cleared until a plugin sets them again.
            if self.state.buffer_positions.diagnostics.remove_buffer(id) {
                self.queue_diagnostics_changed(id);
            }
            self.state.config.decorations.remove_buffer(id);
        }
        // `detect_and_set_language` handles a genuine language change
        // (shebang/extension) regardless of `mutated`, re-running setup via
        // `set_buffer_language` itself.
        self.detect_and_set_language(id);

        // Drop stale saved scrolls for the reloaded buffer on every pane:
        // `recall_scroll` clamps the top's line to the buffer's current last
        // line, but a saved top slot/`horizontal_offset` for a
        // scroll position that no longer exists is still worth discarding
        // outright rather than recalling a clamped-but-arbitrary spot.
        // Every pane, active tab or not.
        lifecycle::forget_saved_views(&mut self.view, id);
    }

    /// Redirect `fp` to `target` without recording a jump.
    pub(in crate::editor) fn switch_to_buffer_without_jump(
        &mut self,
        fp: FocusedPane,
        target: BufferId,
    ) {
        lifecycle::switch_pane_to_buffer(&mut self.state, &mut self.view, fp.pid(), target);
    }

    /// Redirect `fp` to `target`, recording the outgoing position in
    /// `panes.jumps[fp]`.
    ///
    /// Caller contract: all fallible steps (path resolution, file read, etc.)
    /// must succeed before calling this: `push()` truncates forward history.
    pub(in crate::editor) fn switch_to_buffer_with_jump(
        &mut self,
        fp: FocusedPane,
        target: BufferId,
    ) {
        lifecycle::switch_to_buffer_with_jump(&mut self.state, &mut self.view, fp.pid(), target);
    }

    /// Switch `fp` to `target`, or no-op if it already shows it: the
    /// `:e`/`:b` entry point. Unlike `switch_to_buffer_with_jump`, safe
    /// to call with a target that might already be the focused buffer: that
    /// primitive's `push()` truncates forward jump history unconditionally,
    /// so a same-buffer call would corrupt it for nothing.
    ///
    /// External-change detection does not run here: every genuine switch
    /// this produces raises `EditorEvent::OnBufferEnter`, observed by
    /// `Editor::settle`'s diff regardless of caller, interactive or not. A
    /// no-op call raises nothing, matching Vim's `BufEnter`, which doesn't
    /// re-fire for re-entering the buffer you're already viewing.
    ///
    /// Accepted cost of that parity: `:e`/`:b` re-targeting the
    /// already-focused buffer runs no disk stat at all: it's genuinely a
    /// no-op, not a deferred one. An external change to that file still
    /// surfaces the moment any of terminal `FocusIn`, a genuine buffer-enter
    /// (switch away and back), or `:checktime` runs; see
    /// `Editor::enter_buffer_disk_check`'s doc for the full list of paths
    /// that *do* stat.
    pub(in crate::editor) fn enter_buffer(&mut self, fp: FocusedPane, target: BufferId) {
        if target != fp.bid(&self.view) {
            self.switch_to_buffer_with_jump(fp, target);
        }
    }

    /// Open or refresh a read-only view buffer (`:messages`, `:ls`, `:plugin-status`).
    ///
    /// If a buffer with this label already exists, replaces its content in-place
    /// so repeated calls don't accumulate duplicates in `:ls`. Otherwise opens a
    /// fresh read-only buffer. Then switches `fp` to it and positions
    /// the cursor at `cursor_line` (clamped to last content line), or the last
    /// content line itself when `cursor_line` is `None`: `:messages` wants the
    /// bottom (most recent entry) without needing a sentinel value to name it.
    /// Returns the view buffer's id, e.g. for callers attaching decorations
    /// (`:messages`'s severity highlights) that must target this specific
    /// buffer rather than whatever ends up focused.
    pub(in crate::editor) fn open_read_only_view(
        &mut self,
        fp: FocusedPane,
        label: &'static str,
        content: &str,
        cursor_line: Option<hume_rope::line::ContentLine>,
    ) -> BufferId {
        let bid = if let Some(existing) = self.state.buffers.find_by_label(label) {
            // `set_view_content` resets history and every position stored
            // for the buffer: a regenerated view buffer (`[messages]`,
            // `[buffers]`) shares nothing but its id with the old content.
            self.state.buffers.get_mut(existing).set_view_content(
                existing,
                &mut PositionStores::new(
                    &mut self.state.panes,
                    &mut self.state.input,
                    &mut self.state.buffer_positions,
                    &mut self.state.config.decorations,
                ),
                content,
            );
            lifecycle::forget_saved_views(&mut self.view, existing);
            existing
        } else {
            let doc = Buffer::read_only_view(
                hume_editing::text::BufferText::from(content),
                label.to_owned(),
            );
            self.open_buffer(doc)
        };

        if fp.bid(&self.view) != bid {
            self.switch_to_buffer_without_jump(fp, bid);
        }

        // Position cursor at the requested line (clamped to last content
        // line), or the bottom when the caller didn't ask for a specific one.
        let cursor_line =
            cursor_line.unwrap_or_else(|| self.state.buffers.get(bid).text().last_content_line());
        crate::editor::pane_state::park_cursor_at(
            &mut self.state.panes.state,
            &self.state.buffers,
            &self.view.panes,
            fp.pid(),
            bid,
            cursor_line,
            hume_rope::column::GraphemeCol::new(0),
        );

        bid
    }
}
