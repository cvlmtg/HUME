use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::{Path, PathBuf};

use hume_editing::text::BufferText;
use hume_engine::pipeline::BufferId;
use hume_platform::worker_panic::{WorkerPanics, payload_message};

use super::{Buffer, DiskCheckTrigger, ReplaceSource};
use crate::editor::input_stack::{ConfirmAction, ConfirmLayer};
use crate::editor::{Editor, Severity};

/// The crash-dump file for the buffer backed by `path`: the same path with
/// `.dump` appended to the full file name.
pub(crate) fn dump_path_for(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".dump");
    PathBuf::from(name)
}

impl Editor {
    /// Write every writable buffer with unsaved changes to its dump file.
    /// A buffer with a path goes to [`dump_path_for`], and to `scratch_dir`
    /// when that cannot be written; a pathless one goes straight to
    /// `scratch_dir` (created on demand) under a per-process, per-buffer
    /// name. Without a `scratch_dir` a pathless buffer fails, and a file
    /// buffer reports its error beside the file.
    ///
    /// Each buffer is written on its own, so one failure, including a panic,
    /// leaves the rest dumped. Returns each buffer's display name with where
    /// it was written.
    pub(crate) fn dump_dirty_buffers(
        &self,
        scratch_dir: Option<&Path>,
    ) -> Vec<(String, io::Result<PathBuf>)> {
        let mut scratch_count = 0;
        self.state
            .buffers
            .iter()
            .filter(|(id, buf)| self.state.has_unsaved_changes(*id) && !buf.is_read_only())
            .map(|(_, buf)| {
                let written = panic_to_error(|| dump_one(buf, scratch_dir, &mut scratch_count));
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
        if self.state.should_quit || !(self.state.terminate_requested() || result.is_err()) {
            return Vec::new();
        }
        self.dump_dirty_buffers(scratch_dir)
    }

    /// Dump the dirty buffers ([`Editor::dump_dirty_buffers`]) and write
    /// each result to `out`, then each worker-thread panic not yet printed.
    /// The default panic hook never printed those, so the main-thread panic
    /// that follows one would otherwise hide its cause.
    pub(crate) fn write_crash_report(&self, out: &mut impl io::Write, scratch_dir: Option<&Path>) {
        report_dumps(out, &self.dump_dirty_buffers(scratch_dir));
        print_worker_panics(out, &self.state.worker_panics);
    }

    /// Run `run` on this editor. If it panics, write the crash report
    /// ([`Editor::write_crash_report`]) to stderr, then resume the panic.
    pub(crate) fn run_dumping_on_panic<R>(
        &mut self,
        scratch_dir: Option<&Path>,
        run: impl FnOnce(&mut Self) -> R,
    ) -> R {
        match catch_unwind(AssertUnwindSafe(|| run(self))) {
            Ok(result) => result,
            Err(payload) => {
                self.write_crash_report(&mut io::stderr().lock(), scratch_dir);
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

/// Write one line per outcome to `out`. A failed write is skipped, not
/// raised: the dump runs while the editor is going down, possibly with
/// nothing left to write to (a closed terminal), and the outcomes after it
/// still get their turn.
pub(crate) fn report_dumps(out: &mut impl io::Write, outcomes: &[(String, io::Result<PathBuf>)]) {
    for (name, outcome) in outcomes {
        let _ = match outcome {
            Ok(path) => writeln!(out, "hume: unsaved {name} saved to {}", path.display()),
            Err(e) => writeln!(out, "hume: could not save unsaved {name}: {e}"),
        };
    }
}

/// Write each worker-thread panic not yet printed to `out`, with its
/// backtrace when it has one. Failed writes are skipped like
/// [`report_dumps`]'s.
pub(crate) fn print_worker_panics(out: &mut impl io::Write, panics: &WorkerPanics) {
    for panic in panics.take_unprinted() {
        let _ = writeln!(out, "hume: {panic}");
        if let Some(backtrace) = &panic.backtrace {
            let _ = writeln!(out, "{backtrace}");
        }
    }
}

impl Editor {
    /// Open the restore prompt for `bid` when a crash dump is waiting next to
    /// its file. Runs on every buffer-enter, so a prompt that could not open
    /// (another overlay was up) is offered again on the next one.
    pub(in crate::editor) fn offer_dump_restore(&mut self, bid: BufferId) {
        let Some(buf) = self.state.buffers.try_get(bid) else {
            return;
        };
        if buf.pending_dump.is_none() {
            return;
        }
        let Some(permit) = self.confirm_permit(bid, DiskCheckTrigger::BufferEnter) else {
            return;
        };
        let name = buf.display_name();
        let Some(dump) = buf.pending_dump.clone() else {
            return;
        };
        if let Err(e) = hume_platform::io::check_own_file(&dump) {
            self.take_pending_dump(bid);
            if e.kind() != io::ErrorKind::NotFound {
                self.report(
                    Severity::Warning,
                    format!("{name}: ignoring {}: {e}", dump.display()),
                );
            }
            return;
        }
        let dump_name = dump.file_name().unwrap_or_default().to_string_lossy();
        let prompt = format!("{name}: recovered unsaved changes found ({dump_name}).");
        self.state.push_layer(
            &self.view,
            ConfirmLayer::new(permit, prompt, ConfirmAction::RestoreDump(bid)),
        );
    }

    /// Replace `bid`'s text with its crash dump as one undoable edit, then
    /// delete the dump. The buffer keeps its path and file metadata and reads
    /// as dirty. Bails with a warning, leaving the question open, if `bid`
    /// lost focus before the answer.
    pub(in crate::editor) fn restore_dump(&mut self, bid: BufferId) {
        let Some(fp) = self.focused_answer_target(bid, "restoring") else {
            return;
        };
        let name = self.state.buffers.get(bid).display_name();
        let Some(dump) = self.take_pending_dump(bid) else {
            return;
        };
        match hume_platform::io::read_own_file(&dump) {
            Ok(content) => {
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
        let Some(dump) = self.take_pending_dump(bid) else {
            return;
        };
        self.remove_dump(&name, &dump);
    }

    /// Leave `bid`'s crash dump on disk and stop asking about it this
    /// session.
    pub(in crate::editor) fn keep_dump(&mut self, bid: BufferId) {
        self.take_pending_dump(bid);
    }

    /// Take `bid`'s pending dump path, so the prompt is not offered again.
    fn take_pending_dump(&mut self, bid: BufferId) -> Option<PathBuf> {
        self.state.buffers.try_get_mut(bid)?.pending_dump.take()
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

/// Write `buf`'s dump beside its file, or under `scratch_dir` when that
/// fails or the buffer has no path. `scratch_count` numbers the scratch
/// files so two buffers never share a name.
fn dump_one(
    buf: &Buffer,
    scratch_dir: Option<&Path>,
    scratch_count: &mut usize,
) -> io::Result<PathBuf> {
    let content = buf.serialized();
    let mut in_scratch_dir = |stem: &str| {
        *scratch_count += 1;
        let dest = data_dir_dump_path(scratch_dir, stem, *scratch_count)?;
        hume_platform::io::write_dump(&content, &dest)?;
        Ok(dest)
    };
    let Some(path) = buf.path() else {
        return in_scratch_dir("scratch");
    };
    let beside = dump_path_for(path);
    let beside_err = match hume_platform::io::write_dump(&content, &beside) {
        Ok(()) => return Ok(beside),
        Err(e) => e,
    };
    let stem = path.file_name().unwrap_or_default().to_string_lossy();
    in_scratch_dir(&stem).map_err(|_| beside_err)
}

fn data_dir_dump_path(dir: Option<&Path>, stem: &str, index: usize) -> io::Result<PathBuf> {
    let dir = dir.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no data directory for scratch dumps",
        )
    })?;
    std::fs::create_dir_all(dir)?;
    Ok(dir.join(format!("{stem}-{}-{index}.dump", std::process::id())))
}

#[cfg(test)]
mod tests;
