//! Writes held back for `on-before-buffer-save` handlers.
//!
//! `Editor::request_write` is the one entry every write command goes
//! through. With no handler registered it writes at once. Otherwise it parks
//! a [`PendingWrite`] in `EditorState::pending_writes` and raises the event;
//! the write happens when every handler that took `done` has called it, or
//! `before-save-timeout-ms` runs out. A hold is open while its write
//! is parked with the hold in its set, so ending the write ends its holds.

use std::time::Duration;

use hume_engine::pipeline::BufferId;
use hume_scripting::host::HostToken;
use rustc_hash::{FxHashMap, FxHashSet};

use super::commands::{CommandPane, FocusedPane, WriteKind, write_and_continue};
use super::error::CommandError;
use super::event::{EditorEvent, PendingWork};
use super::host_token;
use super::timer_bridge::TimerPayload;
use super::timers::TimerId;
use super::{Editor, Severity};
use hume_scripting::PaneHandle;

/// What a write command does once its write has been attempted.
#[derive(Clone, Copy)]
pub(super) enum AfterWrite {
    Nothing,
    /// `:wq`: quit through the pane, if it is still the focused pane and
    /// still shows the buffer.
    Quit(PaneHandle),
}

/// What `request_write` did with a write.
pub(super) enum WriteRequest {
    Written,
    Held,
    /// The buffer already has a write waiting on its handlers; nothing was
    /// started.
    AlreadyPending,
}

pub(super) const SAVE_IN_PROGRESS: &str = "save in progress";

impl WriteRequest {
    /// For a command that writes one buffer and has no use for the
    /// difference between written and held.
    pub(super) fn into_command_result(self) -> Result<(), CommandError> {
        match self {
            WriteRequest::Written | WriteRequest::Held => Ok(()),
            WriteRequest::AlreadyPending => Err(CommandError::transient(SAVE_IN_PROGRESS)),
        }
    }
}

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
    force: bool,
    after: AfterWrite,
    holds: Holds,
}

impl PendingWrite {
    /// Stops the write waiting on anything. Its timer, if any, stays for
    /// the commit to cancel.
    fn release(&mut self) {
        let timeout = match self.holds {
            Holds::NotRaised => None,
            Holds::Open { timeout, .. } => timeout,
        };
        self.holds = Holds::Open {
            open: FxHashSet::default(),
            timeout,
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

/// The parked writes, at most one per buffer.
#[derive(Default)]
pub(crate) struct PendingWrites(FxHashMap<BufferId, PendingWrite>);

impl PendingWrites {
    fn contains(&self, bid: BufferId) -> bool {
        self.0.contains_key(&bid)
    }

    fn park(&mut self, bid: BufferId, write: PendingWrite) {
        self.0.insert(bid, write);
    }

    /// Whether `bid` has a parked write whose event is still to be raised.
    fn awaits_raise(&self, bid: BufferId) -> bool {
        self.0
            .get(&bid)
            .is_some_and(|write| matches!(write.holds, Holds::NotRaised))
    }

    /// Opens `holds` on `bid`'s write, one per handler taking `done`, with
    /// `timeout` to end the wait.
    fn open_holds(&mut self, bid: BufferId, holds: &[HostToken], timeout: Option<TimerId>) {
        if let Some(write) = self.0.get_mut(&bid) {
            write.holds = Holds::Open {
                open: holds.iter().copied().collect(),
                timeout,
            };
        }
    }

    /// Releases `hold` from whichever write waits on it.
    pub(in crate::editor) fn finish(&mut self, hold: HostToken) {
        for write in self.0.values_mut() {
            if let Holds::Open { open, .. } = &mut write.holds {
                open.remove(&hold);
            }
        }
    }

    /// Stops `bid`'s write waiting on anything: no handler will run for it,
    /// or none can still answer.
    pub(in crate::editor) fn release(&mut self, bid: BufferId) {
        if let Some(write) = self.0.get_mut(&bid) {
            write.release();
        }
    }

    pub(in crate::editor) fn release_all(&mut self) {
        self.0.values_mut().for_each(PendingWrite::release);
    }

    /// The buffers whose writes are not waiting on anything.
    fn ready(&self) -> Vec<BufferId> {
        self.0
            .iter()
            .filter(
                |(_, write)| matches!(&write.holds, Holds::Open { open, .. } if open.is_empty()),
            )
            .map(|(bid, _)| *bid)
            .collect()
    }

    fn take(&mut self, bid: BufferId) -> Option<PendingWrite> {
        self.0.remove(&bid)
    }

    /// Drops `bid`'s write: the buffer is closed, so there is nothing to
    /// write. Its timer fires later and finds no write.
    pub(in crate::editor) fn forget_buffer(&mut self, bid: BufferId) {
        self.0.remove(&bid);
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
    ) -> Result<WriteRequest, CommandError> {
        let target = if pane.bid(&self.view) == bid {
            pane.handle(&self.view)
        } else {
            PaneHandle::buffer_only(bid)
        };
        let event = EditorEvent::OnBeforeBufferSave { target };
        let kind = WriteKind::classify(self.state.buffers.get(bid), arg);
        let applies = kind.reaches_own_file();
        if applies {
            self.activate_lazy_event_plugins(event.name());
        }
        let handled = applies
            && self
                .scripting
                .as_ref()
                .is_some_and(|host| host.has_hook_handlers(event.name(), None));
        if !handled {
            return write_and_continue(self, bid, kind, force, after)
                .map(|()| WriteRequest::Written);
        }
        if self.state.pending_writes.contains(bid) {
            return Ok(WriteRequest::AlreadyPending);
        }
        self.state.pending_writes.park(
            bid,
            PendingWrite {
                arg: arg.map(str::to_owned),
                force,
                after,
                holds: Holds::NotRaised,
            },
        );
        self.state
            .config
            .pending_work
            .push_back(PendingWork::BeforeSave(target));
        Ok(WriteRequest::Held)
    }

    fn before_save_timeout(&self) -> Duration {
        Duration::from_millis(self.state.settings.before_save_timeout_ms as u64)
    }

    /// Raises `on-before-buffer-save` for the write parked on `bid`. A
    /// pending write that is gone (timed out first) leaves nothing to hold.
    /// `target`'s pane is dropped from the handlers' argument once it does
    /// not show the buffer.
    pub(super) fn fire_before_save(&mut self, target: PaneHandle) {
        let bid = target.buffer();
        if !self.state.pending_writes.awaits_raise(bid) {
            return;
        }
        let target = if CommandPane::resolve(&self.state, &self.view, target).is_ok() {
            target
        } else {
            PaneHandle::buffer_only(bid)
        };
        let event = EditorEvent::OnBeforeBufferSave { target };
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
            self.timer_wheel.cancel(timeout);
            self.timer_payloads.remove(&timeout);
        }
        let kind = WriteKind::classify(self.state.buffers.get(bid), pending.arg.as_deref());
        if let Err(e) = write_and_continue(self, bid, kind, pending.force, pending.after) {
            self.report_error(&e);
        }
    }
}
