use hume_core::{BufferId, OnUnsaved, Severity};
use hume_platform::io::FileMeta;
use hume_platform::path::AbsolutePath;

use super::super::Editor;
use super::super::before_save::{AfterWrite, SAVE_IN_PROGRESS};
use super::{CloseRefused, FocusedPane};
use crate::editor::buffer::lifecycle::{self, CloseBufferRefused};
use crate::editor::buffer::{Buffer, BufferFile, TextRefusal};
use crate::editor::error::CommandError;

const STALE_WRITE_MSG: &str = "file has changed on disk (add ! to override)";

/// Why a write command's write did not land.
pub(in crate::editor) enum WriteError {
    /// The buffer's own file changed on disk since it was read or written;
    /// `!` writes anyway.
    Stale,
    /// Not attempted: the buffer already has a write waiting on its
    /// handlers.
    InProgress,
    Failed(CommandError),
}

impl From<CommandError> for WriteError {
    fn from(e: CommandError) -> Self {
        Self::Failed(e)
    }
}

impl From<WriteError> for CommandError {
    fn from(e: WriteError) -> Self {
        match e {
            WriteError::Stale => CommandError::transient(STALE_WRITE_MSG),
            WriteError::InProgress => CommandError::transient(SAVE_IN_PROGRESS),
            WriteError::Failed(e) => e,
        }
    }
}

// ── Typed file commands ───────────────────────────────────────────────────────

pub(in crate::editor) fn typed_quit(
    ed: &mut Editor,
    fp: FocusedPane,
    force: bool,
) -> Result<(), CommandError> {
    // When the active tab has more than one pane, `:q` closes the focused
    // pane, not the tab or the editor. On the tab's last pane, with other
    // tabs open, it closes the tab (Vim's `:q` on a window closes it; the
    // last window of a tab closes the tab, not the editor). The buffer stays
    // open either way (no edits lost), so no dirty check. That guard belongs
    // to the steps below, which actually close something the user can't get
    // back without saving. A pipe run a closed pane started would stop with
    // it, so that needs `!`.
    let closed = ed.run_stopping_pipes(force, |ed| {
        match super::close_focused_pane(&mut ed.state, &mut ed.view) {
            Err(CloseRefused::Last) => super::close_tab(&mut ed.state, &mut ed.view),
            closed => closed,
        }
    });
    match closed {
        Ok(()) => return Ok(()),
        Err(CloseRefused::PipeRunning(running)) => {
            return Err(super::pipe_running_refusal(
                &ed.state, &ed.view, running, ":q!",
            ));
        }
        Err(CloseRefused::Last) => {}
    }

    let current = fp.bid(&ed.view);
    // Stay only for a buffer worth returning to: a real editable file, or any
    // buffer with unsaved edits (rescues a scratch the user has typed into).
    // Empty scratch buffers and read-only views (e.g. [messages]) are disposable:
    // :q exits rather than parking on them.
    let stays = ed
        .state
        .buffers
        .iter()
        .filter(|(id, _)| *id != current)
        .any(|(id, buf)| {
            (buf.path().is_some() && !buf.is_read_only()) || ed.state.has_unsaved_changes(id)
        });
    let closed = if stays {
        let on_unsaved = OnUnsaved::from_force(force);
        ed.run_stopping_pipes(force, |ed| {
            lifecycle::close_buffer_and_notify(&mut ed.view, &mut ed.state, current, on_unsaved)
        })
    } else {
        ed.state.quit(current, force)
    };
    closed.map_err(|refused| super::close_refusal(&ed.state, &ed.view, refused, ":q!"))
}

pub(in crate::editor) fn typed_quit_all(
    ed: &mut Editor,
    fp: FocusedPane,
    force: bool,
) -> Result<(), CommandError> {
    ed.state
        .quit(fp.bid(&ed.view), force)
        .map_err(|refused| match refused {
            CloseBufferRefused::Unsaved(unsaved) => {
                // The focused buffer is named first, so this jumps only when it
                // is clean: a user already on an unsaved buffer stays there, and
                // a save + :qa cycle walks through dirty buffers one at a time.
                ed.enter_buffer(fp, unsaved.0);
                // Stays Error, not transient: `EditorState::message_logged_this_input`
                // (input.rs) keys off `message_log.totals()` moving, and
                // `Editor::confirm_permit` (input_stack/confirm.rs) reads that flag to refuse a
                // disk-change reload confirm from popping over this exact message
                // right after :qa's focus-move names the dirty buffer; a
                // transient report wouldn't move totals() and would let the
                // confirm steal the next keystroke.
                CommandError::new(unsaved.message(&ed.state, ":qa!"))
            }
            CloseBufferRefused::PipeRunning(running) => {
                super::pipe_running_refusal(&ed.state, &ed.view, running, ":qa!")
            }
        })
}

pub(in crate::editor) fn typed_write(
    ed: &mut Editor,
    fp: FocusedPane,
    arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    ed.request_write(fp.bid(&ed.view), fp, arg, force, AfterWrite::Nothing)
}

pub(in crate::editor) fn typed_write_quit(
    ed: &mut Editor,
    fp: FocusedPane,
    arg: Option<&str>,
    force: bool,
) -> Result<(), CommandError> {
    let quit = AfterWrite::Quit(fp.handle(&ed.view));
    ed.request_write(fp.bid(&ed.view), fp, arg, force, quit)
}

/// Writes `bid`, then carries out `after`. The commit step behind
/// `Editor::request_write`, run at once or when a parked write is released.
pub(in crate::editor) fn write_and_continue(
    ed: &mut Editor,
    bid: BufferId,
    arg: Option<&str>,
    force: bool,
    after: AfterWrite,
) -> Result<(), CommandError> {
    let written = write_file(ed, bid, arg, force);
    conclude_write(ed, bid, written, force, after)
}

/// Carries out `after` for a write attempt that ended in `written`.
///
/// `force` applies to the quit too (proceed with the quit even if the write
/// fails). A quit whose pane is not the focused pane showing `bid` any more
/// (the write was parked while focus moved) is cancelled: the write still
/// happened. A `:wa` write is recorded in its batch, and a hard failure ends
/// the batch.
pub(in crate::editor) fn conclude_write(
    ed: &mut Editor,
    bid: BufferId,
    written: Result<(), WriteError>,
    force: bool,
    after: AfterWrite,
) -> Result<(), CommandError> {
    let handle = match after {
        AfterWrite::Nothing => return written.map_err(Into::into),
        AfterWrite::Batch(batch) => return ed.record_batch_write(batch, bid, written),
        AfterWrite::Quit(handle) => handle,
    };
    let Ok(pane) = FocusedPane::resolve(&ed.state, &ed.view, handle) else {
        if written.is_ok() {
            ed.report(Severity::Info, "Written; quit cancelled (focus moved)");
        }
        return written.map_err(Into::into);
    };
    match written {
        Ok(()) => typed_quit(ed, pane, force),
        Err(WriteError::InProgress) => Err(WriteError::InProgress.into()),
        Err(e) if force => {
            typed_quit(ed, pane, true)
                .expect("force quit cannot fail: it skips the unsaved-changes and pipe checks");
            Err(e.into())
        }
        Err(e) => Err(e.into()),
    }
}

pub(in crate::editor) fn typed_toggle_soft_wrap(
    ed: &mut Editor,
    fp: FocusedPane,
) -> Result<(), CommandError> {
    let now_wrapping = ed.toggle_wrap(fp);
    let state = if now_wrapping { "on" } else { "off" };
    ed.report(Severity::Info, format!("Soft wrap {state}"));
    Ok(())
}

/// Writes `content` to the file at `target`, as it resolves now, creating it
/// when it does not exist. Returns the written file's metadata and whether
/// the write was retried (see [`hume_platform::io::write_file_atomic`]).
///
/// Unless `force`, an existing file must be the one `baseline` describes,
/// unchanged (see [`BufferFile::describes`]): a fresh read at write time, not
/// a cached flag, since nothing else checks the disk right before a write. A
/// file that vanished is recreated whatever `baseline` says: that writes back
/// the user's own work and clobbers nobody's.
fn write_to(
    target: &AbsolutePath,
    content: &str,
    baseline: Option<&BufferFile>,
    force: bool,
) -> Result<(FileMeta, bool), WriteError> {
    let failed = |e: std::io::Error| WriteError::Failed(CommandError::new(e.to_string()));
    match hume_platform::io::read_file_meta(target) {
        Ok(mut fresh) => {
            if !force && baseline.is_some_and(|file| !file.describes(&fresh)) {
                return Err(WriteError::Stale);
            }
            let retried =
                hume_platform::io::write_file_atomic(content, &mut fresh, force).map_err(failed)?;
            Ok((fresh, retried))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            hume_platform::io::write_file_new(content, target)
                .map(|meta| (meta, false))
                .map_err(failed)
        }
        Err(e) => Err(failed(e)),
    }
}

/// What a write command does with a buffer.
pub(in crate::editor) enum WriteKind<'a> {
    /// `:w`: the buffer's own file, created when it is not on disk yet. A
    /// read-only buffer or one with no file is `Refused` instead.
    Save(&'a BufferFile),
    /// `:w <path>` on a writable buffer: it becomes the file at the path.
    SaveAs(&'a str),
    /// `:w <path>` on a read-only or synthetic buffer (e.g. `[messages]`): the
    /// content is written to the path, but the buffer's file and dirty state
    /// are left alone, since it did not become that file.
    Export(&'a str),
    /// No write happens; the message says why.
    Refused(String),
}

impl<'a> WriteKind<'a> {
    pub(in crate::editor) fn classify(buf: &'a Buffer, arg: Option<&'a str>) -> Self {
        let read_only = buf.is_read_only();
        match arg {
            Some(path) if read_only || buf.is_synthetic() => Self::Export(path),
            Some(path) => Self::SaveAs(path),
            None if read_only => Self::Refused(TextRefusal::ReadOnly.message(buf)),
            None => buf
                .file()
                .map_or_else(|| Self::Refused("no file name".to_string()), Self::Save),
        }
    }

    /// Whether the write lands in the buffer's own file, once it is written.
    pub(in crate::editor) fn reaches_own_file(&self) -> bool {
        matches!(self, Self::Save(_) | Self::SaveAs(_))
    }
}

/// Serialize the buffer and write it to disk.
///
/// What the write does is `arg` classified against the buffer now (see
/// [`WriteKind`]): a save or save-as makes the written file the buffer's
/// file, so that a later `:w` targets the same path, an export leaves the
/// buffer untouched, and a refusal is the error. A save, and a save-as or
/// export whose path names the buffer's own file, refuses a file changed on
/// disk unless `force` (see [`write_to`]).
///
/// When `force` is `true`, a `PermissionDenied` rename error triggers a
/// chmod-retry: the target is made writable, the rename is retried, and the
/// status message includes "(forced)".
fn write_file(
    ed: &mut Editor,
    bid: BufferId,
    arg: Option<&str>,
    force: bool,
) -> Result<(), WriteError> {
    let buf = ed.state.buffers.get(bid);
    let (content, line_count) = (buf.serialized(), buf.text().content_line_count());
    let (target, baseline, becomes_file) = match WriteKind::classify(buf, arg) {
        WriteKind::Refused(msg) => return Err(CommandError::transient(msg).into()),
        WriteKind::Save(file) => (file.typed().clone(), Some(file), true),
        kind @ (WriteKind::SaveAs(path) | WriteKind::Export(path)) => {
            // A relative path resolves against editor.cwd, not the process
            // cwd, so `:w relpath` is stable regardless of how the process
            // cwd drifts, and the way `:e` resolves it, so the file written
            // is the one `:e` opens.
            let target = AbsolutePath::from_typed(path, &ed.state.cwd)
                .map_err(|e| CommandError::new(e.to_string()))?;
            let baseline = buf
                .file()
                .filter(|own| target.resolve().is_ok_and(|named| named == *own.resolved()));
            (target, baseline, kind.reaches_own_file())
        }
    };
    let (meta, retried) = write_to(&target, &content, baseline, force)?;
    if becomes_file {
        let file =
            BufferFile::on_disk(meta, target).map_err(|e| CommandError::new(e.to_string()))?;
        ed.mark_synced(bid, file);
        ed.report(write_severity(retried), write_msg(line_count, retried));
        ed.queue_buffer_save(bid);
        // Flush any didChange already queued for this buffer first: a
        // save-triggered server action (e.g. lint-on-save) must see a
        // document state at least as current as the file just written,
        // not one edit behind (didSave itself carries no text).
        ed.state.lsp_flush_pending();
        ed.state.lsp_did_save(bid);
        ed.state.lsp_repull_diagnostics_all(bid);
    } else {
        ed.report(write_severity(retried), write_msg(line_count, retried));
    }
    Ok(())
}

pub(in crate::editor) fn typed_write_all(
    ed: &mut Editor,
    fp: FocusedPane,
    force: bool,
) -> Result<(), CommandError> {
    let dirty_savable: Vec<BufferId> = ed
        .state
        .buffers
        .iter()
        // A buffer `:w` would refuse is skipped: erroring would abort the
        // batch after partial saves.
        .filter(|(id, buf)| {
            ed.state.has_unsaved_changes(*id)
                && matches!(WriteKind::classify(buf, None), WriteKind::Save(_))
        })
        .map(|(id, _)| id)
        .collect();

    if dirty_savable.is_empty() {
        return Ok(());
    }

    // The writes form one batch, reported once every member has finished,
    // which may be later than this command when handlers hold some. A buffer
    // whose file changed on disk, or whose write is already waiting, is
    // skipped: one stale buffer among several dirty ones shouldn't block
    // saving the rest. `force` (`:wa!`) writes through the stale ones
    // instead, same as a per-buffer `:w!`. Any other write failure ends the
    // batch.
    let batch = ed.state.pending_writes.open_batch();
    for bid in dirty_savable {
        ed.request_write(bid, fp, None, force, AfterWrite::Batch(batch))?;
    }
    ed.state.pending_writes.seal(batch);
    ed.report_finished_write_batches();
    Ok(())
}

fn write_severity(forced: bool) -> Severity {
    if forced {
        Severity::Warning
    } else {
        Severity::Info
    }
}

fn write_msg(line_count: hume_rope::line::ContentLineCount, forced: bool) -> String {
    let line_count = line_count.get();
    if forced {
        format!("Written {line_count} lines (forced)")
    } else {
        format!("Written {line_count} lines")
    }
}
