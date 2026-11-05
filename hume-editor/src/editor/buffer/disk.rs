//! External file-change detection: stat-on-trigger, mtime+size comparison.
//!
//! A stat at a few trigger points (terminal focus, buffer-enter,
//! `:checktime`) has no background cost and behaves the same on every
//! platform, where filesystem watchers disagree on rename semantics and
//! coalescing.

use crate::editor::Editor;
use crate::editor::error::CommandError;
use crate::editor::input_stack::{ConfirmAction, ConfirmLayer, ConfirmPermit};
use crate::editor::pipe_run::{PipeRefusal, PipeRunning};
use crate::editor::syntax::BufferFate;
use hume_core::{BufferId, OnUnsaved, Severity};

use super::{Buffer, BufferFile, ReplaceSource, TextRefusal, Unsaved};

/// Result of comparing a buffer's stored file signature against a fresh stat.
/// `Changed` carries the freshly-read signature so the caller can store it
/// back without a second stat.
pub(in crate::editor::buffer::disk) enum DiskChange {
    /// The fresh stat matches the stored signature: the buffer is
    /// caught up with disk, whatever its prior `DiskState` was.
    Unchanged,
    Changed(hume_platform::io::FileSignature),
    Vanished,
    /// Nothing to compare (no backing file), or the stat failed for a reason
    /// other than `NotFound`. Unlike `Unchanged`, it leaves a reported
    /// `Changed`/`Vanished` state in place.
    Indeterminate,
}

/// A buffer's disk state as of the last check: whether the state on disk
/// has already been reported. `Changed` carries the reported signature, so a
/// later check can tell the change already warned about from a new one;
/// `Declined` keeps a "keep" answer from coming back on the next
/// `BufferEnter`.
///
/// Never written into `FileMeta::signature`, which stays the baseline
/// `disk_change_for` compares against, so a further external change is
/// still detected. A fresh stat that matches the baseline again (an
/// external revert) resets this to `InSync`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) enum DiskState {
    /// Matches what the editor last read or wrote: nothing to guard against.
    InSync,
    /// Changed externally; carries the signature that was reported.
    Changed(hume_platform::io::FileSignature),
    /// Changed externally, and the user answered the reload confirm with
    /// "keep"; carries the declined signature. Still stale: `:w` re-reads
    /// the file and keeps refusing until a reload or `!`. `Ambient` and
    /// `BufferEnter` checks stay silent for this signature, `Explicit` still
    /// warns, and a different signature is a fresh `Changed`.
    Declined(hume_platform::io::FileSignature),
    /// The backing file no longer exists.
    Vanished,
}

/// Which trigger ran a disk check. Decides whether a `Changed` state that
/// was already reported should re-fire.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) enum DiskCheckTrigger {
    /// Terminal focus, or return from an inline shell command. A state
    /// already reported stays silent.
    Ambient,
    /// Switching the focused pane onto this buffer. A change that only got a
    /// warning earlier (the buffer was not focused, or `file-reload-prompt`
    /// was off) prompts now, though nothing changed on disk since.
    BufferEnter,
    /// `:checktime`. Like `Ambient`, except that a `Declined` change still
    /// warns: a decline silences the automatic checks, not one the user
    /// asked for.
    Explicit,
}

impl Editor {
    /// Stat the file at `bid`'s path and compare it with the file the buffer
    /// last read or wrote ([`BufferFile::describes`]): another file there,
    /// as behind a retargeted symlink, is a change too. A buffer with no
    /// file on disk, or a stat error other than `NotFound`, reads
    /// `Indeterminate`.
    fn disk_change_for(&self, bid: BufferId) -> DiskChange {
        let Some(file) = self
            .state
            .buffers
            .try_get(bid)
            .and_then(Buffer::file)
            .filter(|file| file.file_meta().is_some())
        else {
            return DiskChange::Indeterminate;
        };
        match hume_platform::io::read_file_meta(file.typed()) {
            Ok(fresh) if file.describes(&fresh) => DiskChange::Unchanged,
            Ok(fresh) => DiskChange::Changed(fresh.signature()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => DiskChange::Vanished,
            Err(_) => DiskChange::Indeterminate,
        }
    }

    /// Check one buffer's disk state and act on it.
    ///
    /// `Vanished` warns once and never prompts: there is nothing to reload
    /// from. `Changed` on the focused buffer opens a reload confirm when its
    /// `file-reload-prompt` is on and [`Self::confirm_permit`] allows one;
    /// otherwise it only warns.
    ///
    /// A state already reported stays silent on a further `Ambient` or
    /// `Explicit` check. A `BufferEnter` check on the focused buffer with
    /// `file-reload-prompt` on always prompts a pending `Changed`, or warns
    /// when the confirm is blocked: landing on a stale buffer is never
    /// silent. A `Declined` change warns only on `Explicit`.
    pub(in crate::editor) fn check_buffer_disk_state(
        &mut self,
        bid: BufferId,
        trigger: DiskCheckTrigger,
    ) {
        match self.disk_change_for(bid) {
            // A match resets `disk_state`, so a change after an external
            // revert is detected again. `Indeterminate` says nothing about
            // sync state.
            DiskChange::Unchanged => {
                self.state.buffers.get_mut(bid).disk_state = DiskState::InSync;
            }
            DiskChange::Indeterminate => {}
            DiskChange::Vanished => {
                let buf = self.state.buffers.get_mut(bid);
                let already_reported = buf.disk_state == DiskState::Vanished;
                buf.disk_state = DiskState::Vanished;
                if !already_reported {
                    let name = self.state.buffers.get(bid).display_name();
                    self.report_disk_state(format!("{name}: file no longer exists on disk"));
                }
            }
            DiskChange::Changed(sig) => {
                let buf = self.state.buffers.get_mut(bid);
                let declined = matches!(buf.disk_state, DiskState::Declined(prev) if prev == sig);
                if declined {
                    // Already answered "keep" for this signature. `Declined`
                    // stays, so a later check still sees it answered.
                    if trigger == DiskCheckTrigger::Explicit {
                        let name = self.state.buffers.get(bid).display_name();
                        self.report_disk_state(format!("{name}: file has changed on disk"));
                    }
                    return;
                }
                let already_reported =
                    matches!(buf.disk_state, DiskState::Changed(prev) if prev == sig);
                buf.disk_state = DiskState::Changed(sig);

                let buf = self.state.buffers.get(bid);
                let name = buf.display_name();
                let dirty = self.state.has_unsaved_changes(bid);
                let auto_read = buf.overrides.file_reload_prompt(&self.state.settings);
                let focused = bid == self.focused_buffer_id();
                let promptable = focused && auto_read;
                let is_buffer_enter = trigger == DiskCheckTrigger::BufferEnter;

                if promptable
                    && (is_buffer_enter || !already_reported)
                    && let Some(permit) = self.confirm_permit(bid, trigger)
                {
                    self.open_disk_change_confirm(permit, bid, &name, dirty);
                } else if !already_reported || (is_buffer_enter && promptable) {
                    self.report_disk_state(format!("{name}: file has changed on disk"));
                }
            }
        }
    }

    /// Report a disk-state warning. When this input already logged its own
    /// warning or error (`:qa` naming the first dirty buffer right before the
    /// focus move that lands on it), that message keeps the status line and
    /// this goes to `:messages` only.
    fn report_disk_state(&mut self, text: String) {
        if self.state.message_logged_this_input {
            self.state.message_log.push(Severity::Warning, text);
        } else {
            self.report(Severity::Warning, text);
        }
    }

    /// Check every open buffer against `trigger`, `Ambient` or `Explicit`.
    /// Never `BufferEnter`: no buffer among many is the one being entered.
    pub(in crate::editor) fn check_all_disk_state(&mut self, trigger: DiskCheckTrigger) {
        debug_assert_ne!(
            trigger,
            DiskCheckTrigger::BufferEnter,
            "check_all_disk_state has no single buffer to call BufferEnter for; \
             use check_buffer_disk_state or enter_buffer_disk_check instead"
        );
        let ids: Vec<BufferId> = self.state.buffers.iter().map(|(id, _)| id).collect();
        for id in ids {
            self.check_buffer_disk_state(id, trigger);
        }
    }

    /// The disk check for `entered`, the Rust reaction to
    /// `EditorEvent::OnBufferEnter`. That event is a diff against
    /// `EditorState::last_entered_buffer`, so every focus-changing path
    /// reaches this with no per-command wiring; re-targeting the buffer
    /// already focused raises nothing.
    ///
    /// Also retires a confirm that no longer targets `entered`: an async
    /// handler can switch buffers mid-`settle`, with no input for the
    /// confirm's dismiss arm to catch. Left open, the confirm would be
    /// unanswerable and would block `entered`'s own prompt. Retiring, not
    /// declining, leaves the old buffer `Changed`, so it is asked about
    /// again when focus returns.
    pub(in crate::editor) fn enter_buffer_disk_check(&mut self, entered: BufferId) {
        self.state
            .retire_stale_confirm(&self.view, |c| !c.targets_buffer(entered));
        self.check_buffer_disk_state(entered, DiskCheckTrigger::BufferEnter);
        self.offer_dump_restore(entered);
    }

    /// Record the confirm's `[k]eep` answer for the change pending on `bid`.
    /// `Esc` or any other key dismisses the confirm without answering,
    /// leaving the question for the next `BufferEnter`. Only a `Changed`
    /// state is declined: one that moved on before the answer has nothing to
    /// decline. A `bid` that is gone is a no-op.
    pub(in crate::editor) fn decline_disk_change(&mut self, bid: BufferId) {
        let Some(buf) = self.state.buffers.try_get_mut(bid) else {
            return;
        };
        if let DiskState::Changed(sig) = buf.disk_state {
            buf.disk_state = DiskState::Declined(sig);
        }
    }

    /// Open the reload confirm for `bid`. A dirty buffer's prompt adds that
    /// the reload replaces unsaved edits as one undoable step.
    fn open_disk_change_confirm(
        &mut self,
        permit: ConfirmPermit,
        bid: BufferId,
        name: &str,
        dirty: bool,
    ) {
        let prompt = if dirty {
            format!("{name} has changed on disk (unsaved edits will be replaced, undo with u).")
        } else {
            format!("{name} has changed on disk.")
        };
        self.state.push_layer(
            &self.view,
            ConfirmLayer::new(permit, prompt, ConfirmAction::ReloadBuffer(bid)),
        );
    }

    /// Re-read `bid` from disk and reload it in place: the confirm's
    /// `[r]eload` answer.
    ///
    /// The confirm opens only for the focused buffer, but async Steel work
    /// drained in `prepare_frame` can move focus before the user answers,
    /// without the input dispatch that dismisses the confirm. This then warns
    /// instead of reloading a buffer that is no longer focused, and does
    /// nothing for a `bid` that is gone.
    pub(in crate::editor) fn reload_buffer_from_disk(&mut self, bid: BufferId) {
        let Some(fp) = self.focused_answer_target(bid, "reloading") else {
            return;
        };
        // Answering `[r]eload` is the consent: the prompt says unsaved
        // edits will be replaced.
        if let Err(error) = self.reload_from_file(fp, OnUnsaved::Discard) {
            let message = match error {
                ReloadError::NoFile => return,
                ReloadError::PipeRunning(PipeRunning(bid)) => {
                    TextRefusal::Busy.message(self.state.buffers.get(bid))
                }
                ReloadError::Read(e) => e.message().to_string(),
                ReloadError::Unsaved(_) => {
                    unreachable!("a reload that discards unsaved changes is not refused for them")
                }
            };
            self.report(Severity::Warning, message);
        }
    }

    /// The focused pane, when it views `bid`, for a confirm answer that
    /// replaces the buffer's text. Warns and returns `None` when focus moved
    /// off `bid` before the answer, naming `verb` as what was not done;
    /// `None` without a warning when `bid` is gone.
    pub(in crate::editor::buffer) fn focused_answer_target(
        &mut self,
        bid: BufferId,
        verb: &str,
    ) -> Option<crate::editor::commands::FocusedPane> {
        let name = self.state.buffers.try_get(bid)?.display_name();
        let fp = crate::editor::commands::FocusedPane::current(&self.view);
        if bid != fp.bid(&self.view) {
            self.report(
                Severity::Warning,
                format!("{name}: no longer focused, not {verb}"),
            );
            return None;
        }
        Some(fp)
    }

    /// Read the buffer's file fresh, swap it into `fp`'s buffer in place, and
    /// report success; each caller reports a failure at its own severity.
    /// The read follows the buffer's typed path, so a symlink retargeted
    /// since the last read is followed and the buffer's file becomes the one
    /// it resolved to. A buffer with no file, with unsaved changes
    /// `on_unsaved` does not consent to dropping, or with a pipe running is
    /// refused before the read.
    pub(in crate::editor) fn reload_from_file(
        &mut self,
        fp: crate::editor::commands::FocusedPane,
        on_unsaved: OnUnsaved,
    ) -> Result<(), ReloadError> {
        let bid = fp.bid(&self.view);
        let Some(file) = self.state.buffers.get(bid).file() else {
            return Err(ReloadError::NoFile);
        };
        let typed = file.typed().clone();
        let display = file.display().to_owned();
        self.state
            .refuse_unsaved(bid, on_unsaved)
            .map_err(ReloadError::Unsaved)?;
        if self.state.buffers.get(bid).is_busy() {
            return Err(ReloadError::PipeRunning(PipeRunning(bid)));
        }
        let read_error = |e| ReloadError::Read(CommandError::new(format!("{display}: {e}")));
        let (text, file) = BufferFile::read(typed).map_err(read_error)?;
        let source = ReplaceSource::Disk { text, file };
        let fate = self
            .reload_buffer_in_place(fp, source)
            .map_err(ReloadError::PipeRunning)?;
        // A plugin the reloaded text's language activated closed the buffer:
        // there is no buffer left to report reloaded.
        if fate == BufferFate::Closed {
            return Ok(());
        }
        let name = self.state.buffers.get(bid).display_name();
        self.report(Severity::Info, format!("Reloaded {name}"));
        Ok(())
    }
}

/// Why [`Editor::reload_from_file`] changed nothing.
#[derive(Debug)]
pub(in crate::editor) enum ReloadError {
    /// The buffer has no file.
    NoFile,
    /// The buffer has unsaved changes the caller did not consent to drop.
    Unsaved(Unsaved),
    /// A pipe runs in the buffer.
    PipeRunning(PipeRunning),
    /// The file could not be read.
    Read(CommandError),
}

impl PipeRefusal for ReloadError {
    fn pipe_running(&self) -> Option<PipeRunning> {
        match self {
            Self::PipeRunning(running) => Some(*running),
            Self::NoFile | Self::Unsaved(_) | Self::Read(_) => None,
        }
    }
}
