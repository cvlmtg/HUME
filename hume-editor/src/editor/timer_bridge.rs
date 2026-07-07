//! B4's Steel timer surface: `(after ms thunk)` / `(cancel-timer! id)`, and
//! the per-frame fire step that turns a due `TimerId` into a queued Steel
//! call. `timers.rs`'s `TimerWheel` stays payload-agnostic; the
//! `TimerId -> SteelVal` side table lives here instead, alongside the glue
//! that converts a wheel id to the plain integer Steel sees.

use std::time::Duration;

use steel::rvals::SteelVal;

use super::Editor;
use super::timers::TimerId;

/// Disjoint-borrow handle over `Editor`'s timer wheel + thunk table, passed
/// into `EditorHostImpl` the same way B3 passes `&LspState` — `Some` only at
/// the eval call sites that can reach a Steel builtin. Fields are
/// `pub(super)` (not a constructor) so callers build it from `&mut
/// self.timer_wheel` / `&mut self.timer_thunks` directly — going through a
/// `&mut self` method here would borrow all of `Editor`, defeating the
/// disjoint-field borrow the call sites need alongside `&mut self.state` /
/// `&mut self.scripting`.
pub(crate) struct TimerHandle<'a> {
    pub(super) wheel: &'a mut super::timers::TimerWheel,
    pub(super) thunks: &'a mut std::collections::HashMap<TimerId, SteelVal>,
}

impl<'a> TimerHandle<'a> {
    pub(crate) fn schedule(&mut self, after: Duration, thunk: SteelVal) -> u64 {
        let id = self.wheel.schedule(after);
        self.thunks.insert(id, thunk);
        id.0
    }

    /// Idempotent: a already-fired or already-cancelled (or never-existed)
    /// raw id is silently ignored, matching `TimerWheel::cancel`'s contract.
    pub(crate) fn cancel(&mut self, raw_id: u64) {
        let id = TimerId(raw_id);
        self.wheel.cancel(id);
        self.thunks.remove(&id);
    }
}

impl Editor {
    /// Fires every due timer by queuing its thunk — never evaluated inline
    /// (this runs from `drain_async_sources`, the per-frame chokepoint,
    /// same discipline as B2's LSP callbacks). A thunk with no matching
    /// entry (already cancelled) is silently skipped.
    pub(super) fn drain_due_timers(&mut self) {
        let due = self.timer_wheel.take_due(std::time::Instant::now());
        for id in due {
            if let Some(thunk) = self.timer_thunks.remove(&id) {
                self.queue_steel_call(thunk, Vec::new());
            }
        }
    }
}
