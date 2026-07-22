//! The Steel timer surface: `(after ms thunk)` / `(cancel-timer! id)`, and
//! the per-frame fire step that turns a due `TimerId` into either a queued
//! Steel call or a native action. `timers.rs`'s `TimerWheel` stays
//! payload-agnostic; the `TimerId -> TimerPayload` side table lives here
//! instead, alongside the glue that converts a wheel id to the plain
//! integer Steel sees.

use std::time::Duration;

use hume_engine::pipeline::PaneId;
use steel::rvals::SteelVal;

use super::Editor;
use super::timers::TimerId;

/// What firing a `TimerId` actually does — a Steel closure (the `after`
/// builtin) or a native Rust action (the viewport-change debounce, which has no Steel
/// closure to call: the fire site always reads the *current* visible range,
/// not whatever it was when the timer was scheduled).
pub(super) enum TimerPayload {
    SteelThunk(SteelVal),
    ViewportDebounce(PaneId),
}

/// Disjoint-borrow handle over `Editor`'s timer wheel + payload table,
/// passed into `EditorHostImpl` the same way `&LspState` is passed — `Some`
/// only at the eval call sites that can reach a Steel builtin. Fields are
/// `pub(super)` (not a constructor) so callers build it from `&mut
/// self.timer_wheel` / `&mut self.timer_payloads` directly — going through a
/// `&mut self` method here would borrow all of `Editor`, defeating the
/// disjoint-field borrow the call sites need alongside `&mut self.state` /
/// `&mut self.scripting`.
pub(crate) struct TimerHandle<'a> {
    pub(super) wheel: &'a mut super::timers::TimerWheel,
    pub(super) payloads: &'a mut rustc_hash::FxHashMap<TimerId, TimerPayload>,
}

impl<'a> TimerHandle<'a> {
    pub(crate) fn schedule(&mut self, after: Duration, thunk: SteelVal) -> u64 {
        let id = self.wheel.schedule(after);
        self.payloads.insert(id, TimerPayload::SteelThunk(thunk));
        id.0
    }

    /// Idempotent: a already-fired or already-cancelled (or never-existed)
    /// raw id is silently ignored, matching `TimerWheel::cancel`'s contract.
    pub(crate) fn cancel(&mut self, raw_id: u64) {
        let id = TimerId(raw_id);
        self.wheel.cancel(id);
        self.payloads.remove(&id);
    }
}

impl Editor {
    /// Fires every due timer — a Steel thunk is queued (never evaluated
    /// inline; this runs from `drain_async_sources`, the per-frame
    /// chokepoint, same discipline as the LSP callbacks), a viewport
    /// debounce fires `OnViewportChange` directly with the pane's *current*
    /// bounds. An id with no matching payload (already cancelled) is
    /// silently skipped.
    pub(super) fn drain_due_timers(&mut self) {
        let due = self.timer_wheel.take_due(std::time::Instant::now());
        for id in due {
            match self.timer_payloads.remove(&id) {
                Some(TimerPayload::SteelThunk(thunk)) => self.queue_steel_call(thunk, Vec::new()),
                Some(TimerPayload::ViewportDebounce(pane_id)) => {
                    self.viewport_debounce.remove(&pane_id);
                    self.fire_hook_viewport_change(pane_id);
                }
                None => {}
            }
        }
    }

    /// (Re)schedules `pane_id`'s viewport-change debounce, cancelling
    /// whichever timer from a previous call is still pending — a scroll
    /// burst collapses to one fire, `lsp.viewport-debounce-ms` after the
    /// burst settles. Called from `prepare_frame`'s scroll step whenever a
    /// pane's visible range actually changed since the last frame —
    /// never from the render math itself, just this cheap follow-up.
    pub(super) fn debounce_viewport_change(&mut self, pane_id: PaneId) {
        if let Some(old_id) = self.viewport_debounce.remove(&pane_id) {
            TimerHandle {
                wheel: &mut self.timer_wheel,
                payloads: &mut self.timer_payloads,
            }
            .cancel(old_id.0);
        }
        let ms = self.state.settings.lsp_viewport_debounce_ms as u64;
        let id = self.timer_wheel.schedule(Duration::from_millis(ms));
        self.timer_payloads
            .insert(id, TimerPayload::ViewportDebounce(pane_id));
        self.viewport_debounce.insert(pane_id, id);
    }
}
