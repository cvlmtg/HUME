//! The Steel timer surface: `(after! ms thunk)` / `(cancel-timer! id)`, and
//! the per-frame fire step that turns a due `TimerId` into either a queued
//! Steel call or a native action. `timers.rs`'s `TimerWheel` stays
//! payload-agnostic; the `TimerId -> TimerPayload` side table lives here
//! instead, alongside the glue that converts a wheel id to the plain
//! integer Steel sees.

use std::time::{Duration, Instant};

use hume_engine::pipeline::{BufferId, PaneId};

use super::Editor;
use super::async_source::AsyncSource;
use super::timers::{TimerId, TimerWheel};

/// What firing a `TimerId` actually does: a Steel closure (the `after!`
/// builtin) or a native Rust action (the viewport-change debounce, which has no Steel
/// closure to call: the fire site always reads the *current* visible range,
/// not whatever it was when the timer was scheduled; likewise the
/// diagnostics-pull debounce, which pulls for the buffer as it is when it
/// fires).
pub(super) enum TimerPayload {
    SteelThunk(hume_scripting::Rooted),
    /// A Steel thunk that came due and is queued, and has not run yet. The
    /// entry stays, off the wheel, so `cancel-timer!` can still drop the
    /// call; the call's turn or its drop removes it.
    QueuedThunk,
    ViewportDebounce(PaneId),
    DiagnosticPullDebounce(BufferId),
    BeforeSaveTimeout(BufferId),
}

/// Every scheduled timer with what firing it does. The wheel and the payload
/// table change together here and nowhere else, so no timer has a deadline
/// without a payload or a payload without a deadline.
pub(in crate::editor) struct Timers {
    wheel: TimerWheel,
    payloads: rustc_hash::FxHashMap<TimerId, TimerPayload>,
}

impl Timers {
    pub(in crate::editor) fn new() -> Self {
        Self {
            wheel: TimerWheel::new(),
            payloads: rustc_hash::FxHashMap::default(),
        }
    }

    pub(in crate::editor) fn schedule(
        &mut self,
        after: Duration,
        payload: TimerPayload,
    ) -> TimerId {
        let id = self.wheel.schedule(after);
        self.payloads.insert(id, payload);
        id
    }

    /// Cancels timer `id`: a scheduled timer never fires and a queued thunk
    /// never runs. Idempotent: an already-fired, already-cancelled or
    /// never-scheduled id is ignored.
    pub(in crate::editor) fn cancel(&mut self, id: TimerId) {
        match self.payloads.remove(&id) {
            Some(TimerPayload::QueuedThunk) | None => {}
            Some(_) => self.wheel.cancel(id),
        }
    }

    /// [`Self::cancel`] for an id a script names, which reaches only the
    /// timers `after!` made: the editor's own timers share the id space.
    pub(in crate::editor) fn cancel_thunk(&mut self, id: TimerId) {
        if matches!(
            self.payloads.get(&id),
            Some(TimerPayload::SteelThunk(_) | TimerPayload::QueuedThunk)
        ) {
            self.cancel(id);
        }
    }

    /// Whether `id` is a queued thunk not cancelled since it came due,
    /// removing its entry: the call's turn admits it once.
    pub(in crate::editor) fn take_queued(&mut self, id: TimerId) -> bool {
        let queued = matches!(self.payloads.get(&id), Some(TimerPayload::QueuedThunk));
        if queued {
            self.payloads.remove(&id);
        }
        queued
    }

    /// Cancels every timer whose payload matches `pred`.
    pub(in crate::editor) fn cancel_where(&mut self, pred: impl Fn(&TimerPayload) -> bool) {
        let ids: Vec<TimerId> = self
            .payloads
            .iter()
            .filter(|(_, payload)| pred(payload))
            .map(|(&id, _)| id)
            .collect();
        for id in ids {
            self.cancel(id);
        }
    }

    /// Whether no timer is scheduled or queued.
    #[cfg(test)]
    pub(in crate::editor) fn is_empty(&self) -> bool {
        self.payloads.is_empty()
    }

    /// Every timer due at `now` with its payload, in deadline order. A
    /// Steel thunk leaves a [`TimerPayload::QueuedThunk`] entry behind.
    pub(in crate::editor) fn take_due(&mut self, now: Instant) -> Vec<(TimerId, TimerPayload)> {
        let due = self.wheel.take_due(now);
        due.into_iter()
            .filter_map(|id| {
                let payload = self.payloads.remove(&id)?;
                if matches!(payload, TimerPayload::SteelThunk(_)) {
                    self.payloads.insert(id, TimerPayload::QueuedThunk);
                }
                Some((id, payload))
            })
            .collect()
    }
}

impl AsyncSource for Timers {
    fn next_wake(&self, now: Instant) -> Option<Instant> {
        self.wheel.next_wake(now)
    }
}

impl Editor {
    /// Fires every due timer: a Steel thunk is queued (never evaluated
    /// inline; this runs from `drain_async_sources`, the per-frame
    /// chokepoint, same discipline as the LSP callbacks), a viewport
    /// debounce fires `OnViewportChange` directly with the pane's *current*
    /// bounds.
    pub(super) fn drain_due_timers(&mut self) {
        for (id, payload) in self.timers.take_due(Instant::now()) {
            match payload {
                TimerPayload::SteelThunk(thunk) => {
                    self.state
                        .queue_timer_thunk(id, hume_scripting::SteelCall::new(thunk, Vec::new()));
                }
                TimerPayload::QueuedThunk => unreachable!("a queued thunk is off the wheel"),
                TimerPayload::ViewportDebounce(pane_id) => {
                    self.viewport_debounce.remove(&pane_id);
                    self.queue_viewport_change(pane_id);
                }
                TimerPayload::DiagnosticPullDebounce(bid) => {
                    self.diagnostic_pull_debounce.remove(&bid);
                    self.state.lsp_pull_diagnostics(bid);
                }
                TimerPayload::BeforeSaveTimeout(bid) => self.before_save_timed_out(bid),
            }
        }
    }

    /// (Re)schedules `pane_id`'s viewport-change debounce, cancelling
    /// whichever timer from a previous call is still pending, so a scroll
    /// burst collapses to one fire, `hook-viewport-debounce-ms` after the
    /// burst settles.
    ///
    /// This coalescer is Rust, not the Scheme `debounce` builtin: that one
    /// wraps a user-supplied proc, but this guards a *built-in* fire site
    /// (every scroll step, every frame): a scroll burst must not queue
    /// hundreds of hook evals waiting for Steel to debounce them itself.
    pub(super) fn debounce_viewport_change(&mut self, pane_id: PaneId) {
        let after = Duration::from_millis(self.state.settings.hook_viewport_debounce_ms as u64);
        reschedule(
            &mut self.viewport_debounce,
            &mut self.timers,
            pane_id,
            after,
            TimerPayload::ViewportDebounce(pane_id),
        );
    }

    /// (Re)schedules `bid`'s diagnostics pull, cancelling whichever timer
    /// from a previous call is still pending, so a typing burst collapses to
    /// one pull, `lsp-diagnostics-pull-debounce-ms` after it settles.
    pub(super) fn debounce_diagnostic_pull(&mut self, bid: BufferId) {
        let after =
            Duration::from_millis(self.state.settings.lsp_diagnostics_pull_debounce_ms as u64);
        reschedule(
            &mut self.diagnostic_pull_debounce,
            &mut self.timers,
            bid,
            after,
            TimerPayload::DiagnosticPullDebounce(bid),
        );
    }
}

/// Schedules `payload` after `after` as `key`'s pending timer in `pending`,
/// cancelling the one a previous call left there.
fn reschedule<K: std::hash::Hash + Eq>(
    pending: &mut rustc_hash::FxHashMap<K, TimerId>,
    timers: &mut Timers,
    key: K,
    after: Duration,
    payload: TimerPayload,
) {
    if let Some(old_id) = pending.remove(&key) {
        timers.cancel(old_id);
    }
    pending.insert(key, timers.schedule(after, payload));
}
