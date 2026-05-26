use std::io;
use std::path::PathBuf;

use engine::pipeline::BufferId;

use crate::editor::buffer::Buffer;
use crate::scripting::builtins::ids::SteelBufferId;
use crate::scripting::hooks::HookId;

use super::{Editor, Severity, ops};

impl Editor {
    // ── Working directory ─────────────────────────────────────────────────────

    /// Change the editor's working directory.
    ///
    /// Canonicalizes `path`, rejects non-directories, then updates both
    /// `self.cwd` and the process cwd so that relative paths in `:e` and
    /// subprocesses resolve consistently.
    pub(super) fn set_cwd(&mut self, path: &std::path::Path) -> io::Result<PathBuf> {
        let canonical = std::fs::canonicalize(path)?;
        if !canonical.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "not a directory",
            ));
        }
        std::env::set_current_dir(&canonical)?;
        self.cwd = canonical;
        Ok(self.cwd.clone())
    }

    // ── Buffer choke-points ───────────────────────────────────────────────────

    /// Dedup-open a canonicalized path: returns `(id, false)` if already open,
    /// `(id, true)` if newly opened (including `OnBufferOpen` hook fire).
    pub(super) fn open_or_dedup(
        &mut self,
        canonical: &std::path::Path,
    ) -> std::io::Result<(BufferId, bool)> {
        if let Some(existing) = self.buffers.find_by_path(canonical) {
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
        let expanded = crate::os::path::expand(&lossy);
        let canonical = std::fs::canonicalize(expanded.as_ref())?;
        // open_or_dedup handles dedup internally; it does not switch focus.
        self.open_or_dedup(&canonical)?;
        Ok(())
    }

    /// Allocate a new buffer slot (engine + BufferStore), seed the focused pane's
    /// `pane_state`, and return the allocated `BufferId`.
    pub(crate) fn open_buffer(&mut self, doc: Buffer) -> BufferId {
        let bid = ops::open_buffer(
            &mut self.engine_view,
            &mut self.buffers,
            &mut self.pane_state,
            self.focused_pane_id,
            doc,
        );
        self.detect_and_set_language(bid);
        let val = SteelBufferId(bid).into_steel_val();
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
            &mut self.engine_view,
            &mut self.buffers,
            &mut self.pane_state,
            &mut self.pane_jumps,
            self.focused_pane_id,
            id,
        );
        // Fire with the ID that was closed, not the new current buffer.
        let val = SteelBufferId(id).into_steel_val();
        self.fire_hook_silent(HookId::OnBufferClose, &[val]);
    }

    /// Replace buffer `id` with `new_doc` in-place, reseeding all pane state.
    ///
    /// Used by `:e!` reload. Caller contract: `new_doc.search_pattern` must be `None`
    /// (enforced by debug_assert — `Buffer::from_file` satisfies this by construction).
    pub(crate) fn replace_buffer_in_place(&mut self, id: BufferId, new_doc: Buffer) {
        ops::replace_buffer_in_place(
            &mut self.engine_view,
            &mut self.buffers,
            &mut self.pane_state,
            &mut self.pane_jumps,
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
        let pid = self.focused_pane_id;
        ops::switch_pane_to_buffer(
            &mut self.engine_view,
            &self.buffers,
            &mut self.pane_state,
            pid,
            target,
        );
    }

    /// Redirect the focused pane to `target`, recording the outgoing position
    /// in `pane_jumps[focused_pane]`.
    ///
    /// Caller contract: all fallible steps (path resolution, file read, etc.)
    /// must succeed before calling this — `push()` truncates forward history.
    pub(crate) fn switch_to_buffer_with_jump(&mut self, target: BufferId) {
        let current = self.focused_buffer_id();
        ops::switch_to_buffer_with_jump(
            &mut self.engine_view,
            &self.buffers,
            &mut self.pane_state,
            &mut self.pane_jumps,
            self.focused_pane_id,
            current,
            target,
        );
    }

    /// Snapshot the focused pane's current cursor as a `JumpEntry`.
    pub(crate) fn current_jump_entry(&self) -> crate::core::jump_list::JumpEntry {
        use crate::core::jump_list::JumpEntry;
        let pid = self.focused_pane_id;
        let bid = self.focused_buffer_id();
        let sels = self.pane_state[pid][bid].selections.clone();
        JumpEntry::new(sels, self.buffers.get(bid).text(), bid)
    }
}
