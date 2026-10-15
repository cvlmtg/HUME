//! Writes held back for `on-before-buffer-save` handlers.
//!
//! `Editor::request_write` is the one entry every write command goes
//! through. With no handler registered it writes at once. Otherwise it parks
//! a [`PendingWrite`] in `EditorState::pending_writes`; the next drain pass
//! raises the event, and the write happens when every handler that took
//! `done` has called it, or `before-save-timeout-ms` runs out. A hold is open
//! while its write is parked with the hold in its set, so ending the write
//! ends its holds.

use std::time::Duration;

use hume_engine::pipeline::BufferId;
use hume_engine::pipeline::PaneId;
use hume_scripting::host::HostToken;
use rustc_hash::FxHashSet;

use super::buffer::Buffer;
use super::commands::{
    CommandPane, FocusedPane, WriteError, WriteKind, conclude_write, write_and_continue,
};
use super::error::CommandError;
use super::event::{BEFORE_BUFFER_SAVE, HeldEvent};
use super::host_token;
use super::timer_bridge::TimerPayload;
use super::timers::TimerId;
use super::{Editor, Severity};
use hume_scripting::PaneHandle;

/// Names the writes of one `:wa`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct WriteBatchId(u64);

/// What a write command does once its write has been attempted.
#[derive(Clone, Copy)]
pub(super) enum AfterWrite {
    Nothing,
    /// `:wq`: quit through the pane, if it is still the focused pane and
    /// still shows the buffer.
    Quit(PaneHandle),
    /// `:wa`: the outcome is recorded in the batch.
    Batch(WriteBatchId),
}

/// What the members of one `:wa` have done so far.
#[derive(Default)]
struct WriteBatch {
    /// Every member has been requested. A batch is not finished before
    /// then, even while none is parked.
    sealed: bool,
    written: usize,
    stale: Vec<BufferId>,
    in_progress: Vec<BufferId>,
}

pub(super) const SAVE_IN_PROGRESS: &str = "save in progress";

/// The holds a parked write waits on.
enum Holds {
    /// The event has not been raised yet.
    NotRaised,
    /// One hold per handler taking `done`; the write is ready once none is
    /// left. `timeout` fires `before-save-timeout-ms` after the event was
    /// raised, and exists only while a handler can still hold the write.
    Open {
        open: FxHashSet<HostToken>,
        timeout: Option<TimerId>,
    },
}

/// A write waiting for its buffer's `on-before-buffer-save` handlers.
pub(super) struct PendingWrite {
    /// Save-as target; `None` writes the buffer's own file.
    arg: Option<String>,
    /// The pane the write command ran in; the event names it while it shows
    /// the buffer.
    requester: PaneId,
    force: bool,
    after: AfterWrite,
    holds: Holds,
}

impl PendingWrite {
    /// Stops the write waiting on anything. Its timer, if any, stays for
    /// the commit to cancel.
    fn release(&mut self) {
        self.holds = Holds::Open {
            open: FxHashSet::default(),
            timeout: self.timeout(),
        };
    }

    /// The timer to cancel when the write commits.
    fn timeout(&self) -> Option<TimerId> {
        match self.holds {
            Holds::NotRaised => None,
            Holds::Open { timeout, .. } => timeout,
        }
    }
}

/// The parked writes in park order, at most one per buffer.
#[derive(Default)]
pub(crate) struct PendingWrites {
    writes: Vec<(BufferId, PendingWrite)>,
    batches: Vec<(WriteBatchId, WriteBatch)>,
    next_batch: u64,
}

impl PendingWrites {
    fn get_mut(&mut self, bid: BufferId) -> Option<&mut PendingWrite> {
        self.writes
            .iter_mut()
            .find_map(|(parked, write)| (*parked == bid).then_some(write))
    }

    fn contains(&self, bid: BufferId) -> bool {
        self.writes.iter().any(|(parked, _)| *parked == bid)
    }

    fn park(&mut self, bid: BufferId, write: PendingWrite) {
        debug_assert!(!self.contains(bid), "one parked write per buffer");
        self.writes.push((bid, write));
    }

    /// Whether no parked write is waiting for its event to be raised.
    pub(in crate::editor) fn is_settled(&self) -> bool {
        self.unraised().is_empty()
    }

    /// The buffers whose write still has its event to be raised.
    fn unraised(&self) -> Vec<BufferId> {
        self.writes
            .iter()
            .filter(|(_, write)| matches!(write.holds, Holds::NotRaised))
            .map(|(bid, _)| *bid)
            .collect()
    }

    /// The requester of `bid`'s write if its event is still to be raised.
    fn unraised_requester(&self, bid: BufferId) -> Option<PaneId> {
        self.writes
            .iter()
            .find(|(parked, _)| *parked == bid)
            .filter(|(_, write)| matches!(write.holds, Holds::NotRaised))
            .map(|(_, write)| write.requester)
    }

    /// Opens `holds` on `bid`'s write, one per handler taking `done`, with
    /// `timeout` to end the wait.
    fn open_holds(&mut self, bid: BufferId, holds: &[HostToken], timeout: Option<TimerId>) {
        if let Some(write) = self.get_mut(bid) {
            write.holds = Holds::Open {
                open: holds.iter().copied().collect(),
                timeout,
            };
        }
    }

    /// Releases `hold` from whichever write waits on it.
    pub(in crate::editor) fn finish(&mut self, hold: HostToken) {
        for (_, write) in &mut self.writes {
            if let Holds::Open { open, .. } = &mut write.holds {
                open.remove(&hold);
            }
        }
    }

    /// Stops `bid`'s write waiting on anything: no handler will run for it,
    /// or none can still answer.
    pub(in crate::editor) fn release(&mut self, bid: BufferId) {
        if let Some(write) = self.get_mut(bid) {
            write.release();
        }
    }

    pub(in crate::editor) fn release_all(&mut self) {
        self.writes
            .iter_mut()
            .for_each(|(_, write)| write.release());
    }

    /// The buffers whose writes are not waiting on anything.
    fn ready(&self) -> Vec<BufferId> {
        self.writes
            .iter()
            .filter(
                |(_, write)| matches!(&write.holds, Holds::Open { open, .. } if open.is_empty()),
            )
            .map(|(bid, _)| *bid)
            .collect()
    }

    fn take(&mut self, bid: BufferId) -> Option<PendingWrite> {
        let index = self.writes.iter().position(|(parked, _)| *parked == bid)?;
        Some(self.writes.remove(index).1)
    }

    /// Drops `bid`'s write: the buffer is closed, so there is nothing to
    /// write. Its timer fires later and finds no write.
    pub(in crate::editor) fn forget_buffer(&mut self, bid: BufferId) {
        self.writes.retain(|(parked, _)| *parked != bid);
    }

    pub(in crate::editor) fn open_batch(&mut self) -> WriteBatchId {
        let id = WriteBatchId(self.next_batch);
        self.next_batch += 1;
        self.batches.push((id, WriteBatch::default()));
        id
    }

    /// Marks every member of `id` as requested.
    pub(in crate::editor) fn seal(&mut self, id: WriteBatchId) {
        if let Some(batch) = self.batch_mut(id) {
            batch.sealed = true;
        }
    }

    /// `None` once the batch ended on a failure.
    fn batch_mut(&mut self, id: WriteBatchId) -> Option<&mut WriteBatch> {
        self.batches
            .iter_mut()
            .find_map(|(batch, record)| (*batch == id).then_some(record))
    }

    /// Ends `id` on a failure: its record goes, and so do its writes still
    /// parked. Returns their timers for the caller to cancel.
    fn drop_batch(&mut self, id: WriteBatchId) -> Vec<TimerId> {
        self.batches.retain(|(batch, _)| *batch != id);
        let mut timers = Vec::new();
        self.writes.retain(|(_, write)| {
            let member = matches!(write.after, AfterWrite::Batch(batch) if batch == id);
            if member && let Some(timer) = write.timeout() {
                timers.push(timer);
            }
            !member
        });
        timers
    }

    /// The batches that are sealed and have no write left parked.
    fn take_finished(&mut self) -> Vec<WriteBatch> {
        let (finished, running): (Vec<_>, Vec<_>) = std::mem::take(&mut self.batches)
            .into_iter()
            .partition(|(id, batch)| {
                batch.sealed
                    && !self
                        .writes
                        .iter()
                        .any(|(_, write)| matches!(write.after, AfterWrite::Batch(b) if b == *id))
            });
        self.batches = running;
        finished.into_iter().map(|(_, batch)| batch).collect()
    }
}

impl Editor {
    /// Writes `bid` now, or once its `on-before-buffer-save` handlers are
    /// done. `pane` is the pane the write command ran in. The hook does not
    /// apply to a write that cannot reach the buffer's own file (an export or
    /// a refusal, see [`WriteKind`]). Those go straight to
    /// `write_and_continue`.
    pub(super) fn request_write(
        &mut self,
        bid: BufferId,
        pane: FocusedPane,
        arg: Option<&str>,
        force: bool,
        after: AfterWrite,
    ) -> Result<(), CommandError> {
        let kind = WriteKind::classify(self.state.buffers.get(bid), arg);
        if !(kind.reaches_own_file() && self.has_event_handlers(BEFORE_BUFFER_SAVE, None)) {
            return write_and_continue(self, bid, kind, force, after);
        }
        if self.state.pending_writes.contains(bid) {
            return conclude_write(self, bid, Err(WriteError::InProgress), force, after);
        }
        self.state.pending_writes.park(
            bid,
            PendingWrite {
                arg: arg.map(str::to_owned),
                requester: pane.pid(),
                force,
                after,
                holds: Holds::NotRaised,
            },
        );
        Ok(())
    }

    /// Records a `:wa` member's outcome in its batch. A hard failure ends
    /// the batch: its writes still waiting are dropped, and the error is the
    /// caller's to report.
    pub(super) fn record_batch_write(
        &mut self,
        id: WriteBatchId,
        bid: BufferId,
        written: Result<(), WriteError>,
    ) -> Result<(), CommandError> {
        if let Err(WriteError::Failed(e)) = written {
            for timer in self.state.pending_writes.drop_batch(id) {
                self.cancel_timeout(timer);
            }
            return Err(e);
        }
        if let Some(batch) = self.state.pending_writes.batch_mut(id) {
            match written {
                Ok(()) => batch.written += 1,
                Err(WriteError::Stale) => batch.stale.push(bid),
                Err(WriteError::InProgress) => batch.in_progress.push(bid),
                Err(WriteError::Failed(_)) => unreachable!("returned above"),
            }
        }
        Ok(())
    }

    /// Reports each `:wa` whose writes have all finished.
    pub(in crate::editor) fn report_finished_write_batches(&mut self) {
        for batch in self.state.pending_writes.take_finished() {
            if batch.written > 0 {
                self.report(Severity::Info, format!("Written {} file(s)", batch.written));
            }
            for (reason, skipped) in [
                ("changed on disk", batch.stale),
                (SAVE_IN_PROGRESS, batch.in_progress),
            ] {
                let names: Vec<String> = skipped
                    .into_iter()
                    .filter_map(|bid| self.state.buffers.try_get(bid))
                    .map(Buffer::display_name)
                    .collect();
                if !names.is_empty() {
                    self.report(
                        Severity::Warning,
                        format!("Skipped ({reason}): {}", names.join(", ")),
                    );
                }
            }
        }
    }

    fn cancel_timeout(&mut self, timeout: TimerId) {
        self.timer_wheel.cancel(timeout);
        self.timer_payloads.remove(&timeout);
    }

    fn before_save_timeout(&self) -> Duration {
        Duration::from_millis(self.state.settings.before_save_timeout_ms as u64)
    }

    /// Raises `on-before-buffer-save` for every parked write still waiting
    /// for it. Returns how many were raised, for the drain cap.
    pub(super) fn raise_parked_writes(&mut self) -> usize {
        let unraised = self.state.pending_writes.unraised();
        for &bid in &unraised {
            self.fire_before_save(bid);
        }
        unraised.len()
    }

    /// Raises the event for the write parked on `bid`. A write that is gone
    /// (timed out or closed by an earlier handler) leaves nothing to hold.
    /// The handlers' pane argument is the requester while it shows the
    /// buffer, and no pane otherwise.
    fn fire_before_save(&mut self, bid: BufferId) {
        let Some(requester) = self.state.pending_writes.unraised_requester(bid) else {
            return;
        };
        let target = PaneHandle::with_pane(bid, requester);
        let target = if CommandPane::resolve(&self.state, &self.view, target).is_ok() {
            target
        } else {
            PaneHandle::buffer_only(bid)
        };
        let event = HeldEvent::OnBeforeBufferSave { target };
        let name = event.name();
        let args = event.into_steel_args();
        let Some(handlers) = self.scripting.as_ref().map(|s| s.held_handlers(name)) else {
            self.state.pending_writes.release(bid);
            self.commit_ready_writes();
            return;
        };
        let holds: Vec<HostToken> = (0..handlers.holds_needed())
            .map(|_| host_token::mint())
            .collect();
        let timeout = (!holds.is_empty()).then(|| {
            let timeout = self.timer_wheel.schedule(self.before_save_timeout());
            self.timer_payloads
                .insert(timeout, TimerPayload::BeforeSaveTimeout(bid));
            timeout
        });
        self.state.pending_writes.open_holds(bid, &holds, timeout);
        let fired = {
            let (host_scr, mut impl_host) = self.scripting_with_host().expect("checked above");
            host_scr.fire_held_hook(handlers, &args, holds, &mut impl_host)
        };
        self.flush_script_messages();
        self.apply_script_result(fired, "hook error: ");
        self.commit_ready_writes();
    }

    /// Commits every parked write with no hold left. Run each pass of
    /// `drain_pending_work`, since a `done` can come from any Steel session.
    pub(super) fn commit_ready_writes(&mut self) {
        for bid in self.state.pending_writes.ready() {
            self.commit_pending_write(bid);
        }
        self.report_finished_write_batches();
    }

    /// The `before-save-timeout-ms` timer for `bid`'s pending write fired.
    pub(super) fn before_save_timed_out(&mut self, bid: BufferId) {
        if !self.state.pending_writes.contains(bid) {
            return;
        }
        self.report(
            Severity::Warning,
            format!(
                "on-before-buffer-save handlers did not finish within {} ms; writing anyway",
                self.before_save_timeout().as_millis()
            ),
        );
        self.commit_pending_write(bid);
    }

    fn commit_pending_write(&mut self, bid: BufferId) {
        let Some(pending) = self.state.pending_writes.take(bid) else {
            return;
        };
        if let Some(timeout) = pending.timeout() {
            self.cancel_timeout(timeout);
        }
        let kind = WriteKind::classify(self.state.buffers.get(bid), pending.arg.as_deref());
        if let Err(e) = write_and_continue(self, bid, kind, pending.force, pending.after) {
            self.report_error(&e);
        }
    }
}
