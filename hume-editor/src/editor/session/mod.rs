//! The session of a run started with no files: its buffers, tabs, splits,
//! positions, registers and prompt history, saved under the session
//! directory at the `session-root` directory's path mirrored there with
//! `.session` appended, and restored by the next such run with that root.
//!
//! The file is a stamped file (`HUMESESS`) whose body is a postcard-encoded
//! [`SessionSnapshot`]. Buffer text is not saved: each buffer records its
//! text's MD5, and its positions are restored only onto that same text.

mod keys;
mod restore;
mod snapshot;

use std::io;
use std::path::PathBuf;

use hume_core::Severity;
use hume_platform::path::mirrored_under;

use super::{Editor, EditorState};
use crate::editor::commands::FocusedPane;
use crate::editor::error::CommandError;
use crate::editor::stamped_file::{self, Stamp};
use snapshot::SessionSnapshot;

const STAMP: Stamp = Stamp {
    magic: b"HUMESESS",
    format: 1,
    kind: "a session file",
};

impl EditorState {
    /// Where the session of `session-root` is saved. The error says why
    /// there is no such place.
    fn session_path(&self) -> Result<PathBuf, String> {
        let dir = self
            .settings
            .session_directory
            .location(&self.dirs, &self.cwd, "sessions")?;
        let root = self.settings.session_root.location(&self.cwd)?;
        mirrored_under(&dir, &root, ".session")
            .ok_or_else(|| "the session root has no place in the session directory".to_owned())
    }
}

impl Editor {
    /// Save the session when this run keeps one and `session-persist` is
    /// on: the one call every way out of the run loop but a panic reaches.
    pub(crate) fn save_session(&self) -> Result<(), String> {
        if !self.keeps_session || !self.state.settings.session_persist {
            return Ok(());
        }
        self.write_session()
    }

    fn write_session(&self) -> Result<(), String> {
        let path = self.state.session_path()?;
        let snapshot = SessionSnapshot::capture(&self.state, &self.view);
        let bytes = postcard::to_extend(&snapshot, STAMP.header()).map_err(|e| e.to_string())?;
        stamped_file::write(&path, &bytes)
    }
}

/// `:save-session`: save the session now, and once it is saved make this run
/// keep it, so quitting saves it again whatever files the run started with.
pub(in crate::editor) fn typed_save_session(
    ed: &mut Editor,
    _fp: FocusedPane,
) -> Result<(), CommandError> {
    if !ed.state.settings.session_persist {
        return Err(CommandError::new("save-session: session-persist is off"));
    }
    ed.write_session()
        .map_err(|e| CommandError::new(format!("save-session: {e}")))?;
    ed.keeps_session = true;
    ed.report(Severity::Info, "session saved");
    Ok(())
}

/// Write the outcome of [`Editor::save_session`] to `out` when it failed. A
/// failed write is skipped, as [`super::buffer::report_dumps`]' are.
pub(crate) fn report_unsaved(out: &mut impl io::Write, saved: &Result<(), String>) {
    if let Err(e) = saved {
        let _ = writeln!(out, "hume: session not saved: {e}");
    }
}

#[cfg(test)]
mod tests;
