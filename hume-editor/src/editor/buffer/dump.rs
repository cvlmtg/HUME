use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::{Path, PathBuf};

use hume_editing::text::BufferText;
use hume_engine::pipeline::BufferId;
use hume_platform::worker_panic::payload_message;

use super::{DiskCheckTrigger, ReplaceSource};
use crate::editor::input_stack::{ConfirmAction, ConfirmChoice, ConfirmLayer};
use crate::editor::{Editor, Severity};

/// The crash-dump file for the buffer backed by `path`: the same path with
/// `.dump` appended to the full file name.
pub(crate) fn dump_path_for(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".dump");
    PathBuf::from(name)
}

impl Editor {
    /// Write every dirty, writable buffer to its dump file. A buffer with a
    /// path goes to [`dump_path_for`]; a pathless one goes into
    /// `scratch_dir` (created on demand) under a per-process, per-buffer
    /// name, and fails when there is no such directory.
    ///
    /// Each buffer is written on its own, so one failure, including a panic,
    /// leaves the rest dumped. Returns each buffer's display name with where it was written.
    pub(crate) fn dump_dirty_buffers(
        &self,
        scratch_dir: Option<&Path>,
    ) -> Vec<(String, io::Result<PathBuf>)> {
        let mut scratch_count = 0;
        self.state
            .buffers
            .iter()
            .filter(|(_, buf)| buf.is_dirty() && !buf.is_read_only())
            .map(|(_, buf)| {
                let dest = match buf.path() {
                    Some(path) => Ok(dump_path_for(path)),
                    None => {
                        scratch_count += 1;
                        scratch_dump_path(scratch_dir, scratch_count)
                    }
                };
                let written = panic_to_error(|| {
                    let dest = dest?;
                    hume_platform::io::write_dump(&buf.serialized(), &dest)?;
                    Ok(dest)
                });
                (buf.display_name(), written)
            })
            .collect()
    }

    /// Dump the dirty buffers ([`Editor::dump_dirty_buffers`]) when `run`'s
    /// `result` shows the editor was stopped rather than quit: a signal asked
    /// it to (the terminate flag is set) or the terminal failed under it
    /// (`result` is an `Err`). Returns nothing after `:q!`/`:qa!`, which
    /// discard on purpose, even if a signal lands once the loop has ended.
    pub(crate) fn dump_if_abnormal_exit(
        &self,
        result: &io::Result<()>,
        scratch_dir: Option<&Path>,
    ) -> Vec<(String, io::Result<PathBuf>)> {
        let signalled = self
            .state
            .terminate_exit_code
            .load(std::sync::atomic::Ordering::Acquire)
            != 0;
        if self.state.should_quit || !(signalled || result.is_err()) {
            return Vec::new();
        }
        self.dump_dirty_buffers(scratch_dir)
    }

    /// Run `run` on this editor. If it panics, dump the dirty buffers
    /// ([`Editor::dump_dirty_buffers`]), name each result on stderr, then
    /// resume the panic.
    pub(crate) fn run_dumping_on_panic<R>(
        &mut self,
        scratch_dir: Option<&Path>,
        run: impl FnOnce(&mut Self) -> R,
    ) -> R {
        match catch_unwind(AssertUnwindSafe(|| run(self))) {
            Ok(result) => result,
            Err(payload) => {
                report_dumps(&self.dump_dirty_buffers(scratch_dir));
                resume_unwind(payload)
            }
        }
    }
}

/// Run `write`, turning a panic into an `Err` carrying the panic message.
/// The editor is already panicking when buffers are dumped, so a second
/// panic on one buffer must not cost the buffers after it.
fn panic_to_error<T>(write: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    catch_unwind(AssertUnwindSafe(write)).unwrap_or_else(|payload| {
        Err(io::Error::other(format!(
            "panicked: {}",
            payload_message(&*payload)
        )))
    })
}

/// Name each dump outcome on stderr.
pub(crate) fn report_dumps(outcomes: &[(String, io::Result<PathBuf>)]) {
    report_dumps_to(&mut io::stderr().lock(), outcomes);
}

/// Write one line per outcome to `out`. A failed write is skipped, not
/// raised: the dump runs while the editor is going down, possibly with
/// nothing left to write to (a closed terminal), and the outcomes after it
/// still get their turn.
fn report_dumps_to(out: &mut impl io::Write, outcomes: &[(String, io::Result<PathBuf>)]) {
    for (name, outcome) in outcomes {
        let _ = match outcome {
            Ok(path) => writeln!(out, "hume: unsaved {name} saved to {}", path.display()),
            Err(e) => writeln!(out, "hume: could not save unsaved {name}: {e}"),
        };
    }
}

impl Editor {
    /// Open the restore prompt for `bid` when a crash dump is waiting next to
    /// its file. Runs on every buffer-enter, so a prompt that could not open
    /// (another overlay was up) is offered again on the next one.
    pub(in crate::editor) fn offer_dump_restore(&mut self, bid: BufferId) {
        let buf = self.state.buffers.get(bid);
        if !buf.dump_pending
            || bid != self.focused_buffer_id()
            || !self.can_open_confirm(DiskCheckTrigger::BufferEnter)
        {
            return;
        }
        let Some(dump) = buf.path().map(dump_path_for) else {
            return;
        };
        let dump_name = dump.file_name().unwrap_or_default().to_string_lossy();
        let prompt = format!(
            "{}: recovered unsaved changes found ({dump_name}).",
            buf.display_name()
        );
        self.state.push_layer(
            &self.view,
            ConfirmLayer {
                prompt,
                choices: vec![
                    ConfirmChoice {
                        key: 'r',
                        label: "restore",
                    },
                    ConfirmChoice {
                        key: 'd',
                        label: "discard",
                    },
                    ConfirmChoice {
                        key: 'k',
                        label: "keep",
                    },
                ],
                action: ConfirmAction::RestoreDump(bid),
            },
        );
    }

    /// Replace `bid`'s text with its crash dump as one undoable edit, then
    /// delete the dump. The buffer keeps its path and file metadata and reads
    /// as dirty. Bails with a warning, leaving the question open, if `bid`
    /// lost focus before the answer (same guard as `reload_buffer_from_disk`).
    pub(in crate::editor) fn restore_dump(&mut self, bid: BufferId) {
        let Some(buf) = self.state.buffers.try_get(bid) else {
            return;
        };
        let name = buf.display_name();
        let fp = crate::editor::commands::FocusedPane::current(&self.state);
        if bid != fp.bid(&self.view) {
            self.report(
                Severity::Warning,
                format!("{name}: no longer focused, not restoring"),
            );
            return;
        }
        let Some(dump) = buf.path().map(dump_path_for) else {
            return;
        };
        self.clear_dump_pending(bid);
        match hume_platform::io::read_file(&dump) {
            Ok((content, _)) => {
                let text = BufferText::from(content.as_str());
                self.reload_buffer_in_place(fp, ReplaceSource::Dump(text));
                self.remove_dump(&name, &dump);
                self.report(Severity::Info, format!("Restored {name} from its dump"));
            }
            Err(e) => self.report(
                Severity::Warning,
                format!("{name}: cannot read {}: {e}", dump.display()),
            ),
        }
    }

    /// Delete `bid`'s crash dump without applying it.
    pub(in crate::editor) fn discard_dump(&mut self, bid: BufferId) {
        let Some(buf) = self.state.buffers.try_get(bid) else {
            return;
        };
        let name = buf.display_name();
        let Some(dump) = buf.path().map(dump_path_for) else {
            return;
        };
        self.clear_dump_pending(bid);
        self.remove_dump(&name, &dump);
    }

    /// Leave `bid`'s crash dump on disk and stop asking about it this
    /// session.
    pub(in crate::editor) fn keep_dump(&mut self, bid: BufferId) {
        self.clear_dump_pending(bid);
    }

    fn clear_dump_pending(&mut self, bid: BufferId) {
        if let Some(buf) = self.state.buffers.try_get_mut(bid) {
            buf.dump_pending = false;
        }
    }

    /// Delete `dump`; a dump that is already gone is the wanted end state.
    fn remove_dump(&mut self, name: &str, dump: &Path) {
        match std::fs::remove_file(dump) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => self.report(
                Severity::Warning,
                format!("{name}: cannot remove {}: {e}", dump.display()),
            ),
        }
    }
}

fn scratch_dump_path(scratch_dir: Option<&Path>, index: usize) -> io::Result<PathBuf> {
    let dir = scratch_dir.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no data directory for scratch dumps",
        )
    })?;
    std::fs::create_dir_all(dir)?;
    Ok(dir.join(format!("scratch-{}-{index}.dump", std::process::id())))
}

#[cfg(test)]
mod tests;
