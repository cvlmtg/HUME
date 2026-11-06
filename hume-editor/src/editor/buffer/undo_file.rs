//! A buffer's undo history, saved under the undo directory at its file's
//! resolved path mirrored there with `.undo` appended, so every name of one
//! file shares one history.
//!
//! The file is a stamped file (`HUMEUNDO`) whose body is the MD5 of the
//! text the history's current revision holds, then a postcard-encoded
//! [`HistorySnapshot`]. The MD5 comes first so a file changed elsewhere
//! skips reading the history.

use std::io::Read;
use std::path::{Path, PathBuf};

use hume_core::{BufferId, PaneId, Severity};
use hume_editing::history::HistorySnapshot;
use hume_platform::path::mirrored_under;

use super::Buffer;
use crate::editor::EditorState;
use crate::editor::stamped_file::{self, Stamp};

const STAMP: Stamp = Stamp {
    magic: b"HUMEUNDO",
    format: 1,
    kind: "an undo history file",
};

impl Buffer {
    /// Write this buffer's undo history to `path`, its text being the one
    /// its file holds. Does nothing for a history with no revision past the
    /// root.
    fn save_undo(&self, path: &Path) -> Result<(), String> {
        if self.history.len() == 1 {
            return Ok(());
        }
        let mut bytes = STAMP.header();
        bytes.extend_from_slice(&self.text_md5());
        let bytes =
            postcard::to_extend(&self.history.snapshot(), bytes).map_err(|e| e.to_string())?;
        stamped_file::write(path, &bytes)
    }

    /// Replace this buffer's untouched history with the one saved at `path`,
    /// when that history ends on the text the buffer holds. `Ok` when there
    /// is none to restore; an error names the undo file that could not be
    /// used.
    fn restore_undo(&mut self, path: &Path) -> Result<(), String> {
        if self.history.len() != 1 {
            return Ok(());
        }
        let Some(mut file) = STAMP.open(path)? else {
            return Ok(());
        };
        let fail = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
        let mut md5 = [0; 16];
        file.read_exact(&mut md5).map_err(|e| match e.kind() {
            std::io::ErrorKind::UnexpectedEof => fail(&"the file ends inside its header"),
            _ => fail(&e),
        })?;
        if md5 != self.text_md5() {
            return Ok(());
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|e| fail(&e))?;
        let history =
            stamped_file::decode_whole::<HistorySnapshot<'_>>(&bytes).map_err(|e| fail(&e))?;
        self.history
            .restore(history, &self.text)
            .map_err(|e| fail(&e))?;
        self.saved_revision = Some(self.history.current_id());
        Ok(())
    }

    /// Record `pane` as showing this buffer: the first such pane owns the
    /// revisions restored as the last editing pane's, now or later.
    pub(in crate::editor) fn claim_restored_history(&mut self, pane: PaneId) {
        self.history.claim_restored(pane);
    }
}

impl EditorState {
    /// Save `bid`'s undo history, its text being the one its file holds,
    /// when its `undo-persist` is on. A failure is a warning.
    pub(in crate::editor::buffer) fn save_undo_history(&mut self, bid: BufferId) {
        if !self.buffers.get(bid).overrides.undo_persist(&self.settings) {
            return;
        }
        let saved = self
            .undo_path(bid)
            .and_then(|path| path.map_or(Ok(()), |path| self.buffers.get(bid).save_undo(&path)));
        if let Err(e) = saved {
            self.report(Severity::Warning, format!("undo history not saved: {e}"));
        }
    }

    /// Restore the undo history saved for `bid`'s file, whatever its
    /// `undo-persist`, when it ends on the text the buffer holds. Only a
    /// damaged history file warns: one that can't be located is none to
    /// restore.
    pub(in crate::editor::buffer) fn restore_undo(&mut self, bid: BufferId) {
        let Ok(Some(path)) = self.undo_path(bid) else {
            return;
        };
        if let Err(e) = self.buffers.get_mut(bid).restore_undo(&path) {
            self.report(Severity::Warning, format!("undo history not restored: {e}"));
        }
    }

    /// Where the history of `bid`'s file is saved, `None` for a buffer with
    /// no file. The error says why there is no such place.
    fn undo_path(&self, bid: BufferId) -> Result<Option<PathBuf>, String> {
        let Some(file) = self.buffers.get(bid).file() else {
            return Ok(None);
        };
        let dir = self
            .settings
            .undo_directory
            .location(&self.dirs, &self.cwd, "undo")?;
        mirrored_under(&dir, file.resolved(), ".undo")
            .map(Some)
            .ok_or_else(|| "the path has no place in the undo directory".to_owned())
    }
}
