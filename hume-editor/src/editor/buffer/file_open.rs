use std::io;

use hume_editing::text::BufferText;
use hume_engine::pipeline::BufferId;
use hume_platform::path::AbsolutePath;

use crate::editor::buffer::{Buffer, BufferFile, ViewLabel};
use crate::editor::commands::FocusedPane;
use crate::editor::event::EditorEvent;
use crate::editor::pipe_run::PipeRunning;

use super::lifecycle;
use crate::editor::{Editor, EditorState, Severity};

/// Where the text that replaces a buffer's content comes from, for
/// [`Editor::reload_buffer_in_place`].
pub(in crate::editor) enum ReplaceSource {
    /// A fresh read of the buffer's file. The buffer matches disk afterwards
    /// and takes `file`, the file the read resolved, so save-time permission
    /// and ownership checks see the current on-disk values.
    Disk { text: BufferText, file: BufferFile },
    /// The text of a crash dump. The buffer keeps its file metadata and disk
    /// state and reads as dirty.
    Dump(BufferText),
}

impl EditorState {
    /// Moves the working directory to `path`; a relative `path` resolves
    /// against the current one the way `:e` resolves it. Canonicalizes and
    /// rejects non-directories.
    ///
    /// `:cd` and `(set-cwd! …)` both land here. The process cwd moves too, so
    /// Steel's filesystem primitives and spawned programs resolve a relative
    /// path against the same directory as HUME; when it cannot move, neither
    /// does the editor's. Unit tests skip that call so they cannot race each
    /// other over it.
    pub(in crate::editor) fn set_cwd(
        &mut self,
        path: &std::path::Path,
    ) -> io::Result<AbsolutePath> {
        let canonical = AbsolutePath::canonicalize(&AbsolutePath::from_typed(path, &self.cwd)?)?;
        if !canonical.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "not a directory",
            ));
        }
        #[cfg(not(test))]
        std::env::set_current_dir(&canonical)?;
        self.cwd = canonical;
        Ok(self.cwd.clone())
    }
}

impl Editor {
    // ── Buffer choke-points ───────────────────────────────────────────────────

    /// Binds `bid` to `file`. A buffer whose resolved path changes is a
    /// different document to its language servers: the reconcile closes it
    /// under the old URI and reopens it under the new one, keeping each
    /// instance, so `didSave` afterwards names a document every attached
    /// server has open.
    pub(in crate::editor) fn set_buffer_file(&mut self, bid: BufferId, file: BufferFile) {
        let buf = self.state.buffers.get_mut(bid);
        let changed = buf.path() != Some(file.resolved());
        buf.set_file(file);
        if changed {
            self.state.lsp_reconcile_buffer(&self.view, bid);
        }
    }

    /// `{name} [new file]`, the message reported when a newly opened `buf`
    /// has nothing on disk yet. `None` for a file read from disk.
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
    pub(in crate::editor) fn open_extra_file(
        &mut self,
        path: &std::path::Path,
    ) -> Option<BufferId> {
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
    /// [`Buffer::new_file`] bound to the path if it doesn't. The argument is
    /// `expand`ed, then opened through [`lifecycle::open_or_dedup_and_notify`].
    /// Errors propagate as raw
    /// `io::Error`; callers format with whichever path string suits their
    /// reporting.
    pub(in crate::editor) fn resolve_open_path(
        &mut self,
        path_str: &str,
    ) -> io::Result<(BufferId, bool)> {
        let expanded = hume_platform::path::expand(path_str);
        lifecycle::open_or_dedup_and_notify(
            &mut self.view,
            &mut self.state,
            std::path::Path::new(expanded.as_ref()),
        )
    }

    fn try_open_extra(&mut self, path: &std::path::Path) -> io::Result<(BufferId, bool)> {
        self.resolve_open_path(&path.to_string_lossy())
    }

    /// Tests' entry point to [`lifecycle::open_buffer_and_notify`], the
    /// production open chokepoint: allocates the buffer slot and queues its
    /// open announcement, which the next `settle` delivers. See
    /// `lifecycle::open_buffer`'s doc for why no pane is seeded yet.
    #[cfg(test)]
    pub(in crate::editor) fn open_buffer(&mut self, doc: Buffer) -> BufferId {
        lifecycle::open_buffer_and_notify(&mut self.view, &mut self.state, doc)
    }

    /// Tests' entry point to [`lifecycle::close_buffer_and_notify`], the
    /// production close chokepoint, refusing unsaved changes and a running
    /// pipe.
    #[cfg(test)]
    pub(in crate::editor) fn close_buffer(
        &mut self,
        id: BufferId,
    ) -> Result<(), lifecycle::CloseBufferRefused> {
        lifecycle::close_buffer_and_notify(
            &mut self.view,
            &mut self.state,
            id,
            super::OnUnsaved::Refuse,
        )
    }

    /// Replace `fp`'s buffer text with `source`'s in place, preserving the
    /// undo tree.
    ///
    /// Unlike `set_view_content` (which discards `History` on a full
    /// `Buffer` swap), this records the replacement as one revision; see
    /// [`Buffer::replace_text_recorded`] for the history/undo mechanics.
    /// [`ReplaceSource`] decides what else the swap touches: a disk read
    /// leaves the buffer matching its file, a dump leaves it dirty.
    ///
    /// The replacement is an edit: it carries every stored position (every
    /// pane's selections for the buffer, jump lists, prompt snapshots)
    /// through its line-diff `ChangeSet`, so each follows its text. Only
    /// `fp`'s pre/post selections are written into the history revision
    /// (undo/redo restore its cursor).
    ///
    /// Survives the replacement: per-buffer search state (match cache
    /// rebuilds lazily) and the syntax tree, which the edit shifts like any
    /// other. Dropped as stale: in-progress edit groups/paste sessions and
    /// saved scrolls.
    ///
    /// `Err`, with nothing changed, while a pipe runs in the buffer.
    pub(in crate::editor) fn reload_buffer_in_place(
        &mut self,
        fp: FocusedPane,
        source: ReplaceSource,
    ) -> Result<(), PipeRunning> {
        let id = fp.bid(&self.view);
        if self.state.buffers.get(id).is_busy() {
            return Err(PipeRunning(id));
        }
        // End any open Insert/paste session the same way every other
        // buffer/focus-invalidating path does (`switch_pane_to_buffer`,
        // `reset_config_state`), before the replacement invalidates the text
        // it was snapshotted against. Leaving it open would keep a session
        // whose group no longer matches the buffer.
        crate::editor::focus::end_focus_sessions(&mut self.state, &self.view);

        let (buffers, mut stores) = self.state.edit_parts();
        let buf = buffers.get_mut(id);
        let (mutated, read_file) = match source {
            ReplaceSource::Disk { text, file } => {
                let mutated = buf.replace_text_recorded(id, &mut stores, text, fp.pid());
                buf.mark_saved();
                (mutated, Some(file))
            }
            ReplaceSource::Dump(text) => (
                buf.replace_text_recorded(id, &mut stores, text, fp.pid()),
                None,
            ),
        };
        if let Some(file) = read_file {
            self.set_buffer_file(id, file);
        }
        // The replacement went through `install` as a line-diff change, so
        // the buffer's servers get it as an ordinary queued `didChange`.
        // Everything below discards state computed against the replaced
        // text. A no-op replacement (`mutated == false`) keeps the content
        // and the text generation, so that state is still valid against the
        // unchanged content and is kept. It does adopt the new text's line
        // ending.
        if mutated {
            // Diagnostics and LSP-sourced decorations describe the replaced
            // content, so they are dropped, and so is what each server was
            // last told in a diagnostics pull. The servers report again
            // after seeing the change; nothing republishes decorations on
            // its own, so they stay cleared until a plugin sets them again.
            if self.state.buffer_positions.diagnostics.remove_buffer(id) {
                self.state
                    .queue_event(EditorEvent::OnDiagnosticsChanged { buffer: id });
            }
            self.state.buffer_positions.lsp.forget_pulls(id);
            self.state.config.decorations.remove_buffer(id);
        }
        // `detect_and_set_language` handles a genuine language change
        // (shebang/extension) regardless of `mutated`, re-running setup via
        // `set_buffer_language` itself.
        self.detect_and_set_language(id);

        // Drop stale saved scrolls for the reloaded buffer on every pane:
        // `Pane::show_buffer` clamps the top's line to the buffer's current last
        // line, but a saved top slot/`horizontal_offset` for a
        // scroll position that no longer exists is still worth discarding
        // outright rather than recalling a clamped-but-arbitrary spot.
        // Every pane, active tab or not.
        lifecycle::forget_saved_views(&mut self.view, id);
        Ok(())
    }

    /// Redirect `fp` to `target` without recording a jump.
    #[cfg(test)]
    pub(in crate::editor) fn switch_to_buffer_without_jump(
        &mut self,
        fp: FocusedPane,
        target: BufferId,
    ) {
        crate::editor::focus::switch_pane_to_buffer(
            &mut self.state,
            &mut self.view,
            fp.pid(),
            target,
        );
    }

    /// Redirect `fp` to `target`, recording the outgoing position in
    /// `panes.entries[fp].jumps`.
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
    /// already-focused buffer runs no disk stat at all: it's a
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

    /// Show `content` in the read-only view labelled `label` on `fp`; see
    /// [`lifecycle::show_read_only_view`]. Returns the view buffer's id, e.g.
    /// for callers attaching decorations (`:messages`'s severity highlights)
    /// that must target this specific buffer rather than whatever ends up
    /// focused.
    pub(in crate::editor) fn open_read_only_view(
        &mut self,
        fp: FocusedPane,
        label: &ViewLabel,
        content: &str,
        cursor_line: Option<hume_rope::line::ContentLine>,
    ) -> BufferId {
        lifecycle::show_read_only_view(
            &mut self.state,
            &mut self.view,
            fp.pane(),
            label,
            content,
            cursor_line,
        )
    }
}
