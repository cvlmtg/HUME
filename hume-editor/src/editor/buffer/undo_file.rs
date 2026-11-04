//! A buffer's undo history, saved under the undo directory at its file's
//! resolved path mirrored there with `.undo` appended, so every name of one
//! file shares one history.
//!
//! The file is the bytes `HUMEUNDO`, a little-endian `u32` format version,
//! the MD5 of the text the history's current revision holds, then a
//! postcard-encoded [`HistorySnapshot`]. A file of another format version
//! reads as no history, so a format change needs no migration. The MD5 comes
//! first so a file changed elsewhere skips reading the history.

use std::io::{self, Read};
use std::path::{Path, PathBuf};

use hume_core::{BufferId, PaneId, Severity};
use hume_editing::history::HistorySnapshot;
use hume_editing::text::BufferText;
use hume_platform::path::mirrored_under;
use md5::{Digest, Md5};

use super::Buffer;
use crate::editor::EditorState;

const MAGIC: &[u8; 8] = b"HUMEUNDO";
const FORMAT: u32 = 1;
const HEADER_LEN: usize = MAGIC.len() + size_of::<u32>() + 16;

/// MD5 here only tells whether the file still holds the text the history
/// ends on; nothing relies on it resisting a deliberate collision.
fn text_md5(text: &BufferText) -> [u8; 16] {
    let mut hasher = Md5::new();
    for chunk in text.rope().chunks() {
        hasher.update(chunk.as_bytes());
    }
    hasher.finalize().into()
}

impl Buffer {
    /// Write this buffer's undo history to `path`, its text being the one
    /// its file holds. Does nothing for a history with no revision past the
    /// root.
    fn save_undo(&self, path: &Path) -> Result<(), String> {
        if self.history.len() == 1 {
            return Ok(());
        }
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&FORMAT.to_le_bytes());
        bytes.extend_from_slice(&text_md5(&self.text));
        let bytes =
            postcard::to_extend(&self.history.snapshot(), bytes).map_err(|e| e.to_string())?;
        let write = || {
            if let Some(parent) = path.parent() {
                hume_platform::io::create_private_dir_all(parent)?;
            }
            hume_platform::io::write_private_file(&bytes, path)
        };
        write().map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Replace this buffer's untouched history with the one saved at `path`,
    /// when that history ends on the text the buffer holds. `Ok` when there
    /// is none to restore; an error names the undo file that could not be
    /// used.
    fn restore_undo(&mut self, path: &Path) -> Result<(), String> {
        if self.history.len() != 1 {
            return Ok(());
        }
        let fail = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
        let mut file = match hume_platform::io::open_own_file(path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(fail(&e)),
        };
        let mut bytes = Vec::new();
        file.by_ref()
            .take(HEADER_LEN as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| fail(&e))?;
        let Some(rest) = bytes.strip_prefix(MAGIC.as_slice()) else {
            return Err(fail(&"not an undo history file"));
        };
        let Some((format, rest)) = rest.split_first_chunk::<4>() else {
            return Err(fail(&"the file ends inside its header"));
        };
        if u32::from_le_bytes(*format) != FORMAT {
            return Ok(());
        }
        let Some((md5, _)) = rest.split_first_chunk::<16>() else {
            return Err(fail(&"the file ends inside its header"));
        };
        if *md5 != text_md5(&self.text) {
            return Ok(());
        }
        file.read_to_end(&mut bytes).map_err(|e| fail(&e))?;
        let encoded = &bytes[HEADER_LEN..];
        let history = match postcard::take_from_bytes::<HistorySnapshot<'_>>(encoded) {
            Ok((history, [])) => history,
            Ok(_) => return Err(fail(&"trailing bytes after the history")),
            Err(e) => return Err(fail(&e)),
        };
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
            .location(&self.dirs, &self.cwd)?;
        mirrored_under(&dir, file.resolved(), ".undo")
            .map(Some)
            .ok_or_else(|| "the path has no place in the undo directory".to_owned())
    }
}
