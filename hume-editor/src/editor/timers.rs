//! Nearest-deadline timer registry, integrated with the P3 `AsyncSource` wake
//! logic. Rust-side machinery only — payload-agnostic on purpose: the wheel
//! hands back opaque [`TimerId`]s, and B4 adds the `TimerId -> Steel thunk`
//! side table that gives them meaning, keeping Steel types out of the
//! editor core.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::time::{Duration, Instant};

use super::async_source::AsyncSource;

/// Opaque handle to a scheduled timer. The inner `u64` is `pub(crate)` (not
/// exposed via a method) so `timer_bridge.rs` can convert to/from the plain
/// integer Steel's `(after ms thunk)` returns — this module itself stays
/// Steel-agnostic (see the module doc).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) struct TimerId(pub(crate) u64);

/// Min-heap of `(deadline, id)`, plus a lazily-drained cancellation set.
///
/// `cancel` only records the id — no heap search. The entry is skipped (and
/// forgotten) the next time it reaches the heap's head via [`Self::take_due`].
/// No separate compaction pass is needed: every cancelled id that was ever
/// pushed eventually surfaces at the head as earlier entries pop, at which
/// point `take_due` drops it — the cancelled set can only shrink over time,
/// never needs a sweep.
pub(crate) struct TimerWheel {
    heap: BinaryHeap<Reverse<(Instant, TimerId)>>,
    cancelled: HashSet<TimerId>,
    next_id: u64,
}

impl TimerWheel {
    pub(crate) fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            cancelled: HashSet::new(),
            next_id: 0,
        }
    }

    /// Schedule a timer to fire `after` from now. Returns a handle usable
    /// with [`Self::cancel`]. Production caller: `timer_bridge::TimerHandle`.
    pub(crate) fn schedule(&mut self, after: Duration) -> TimerId {
        let id = TimerId(self.next_id);
        self.next_id += 1;
        self.heap.push(Reverse((Instant::now() + after, id)));
        id
    }

    /// Cancel a previously scheduled timer. A no-op if `id` already fired or
    /// was already cancelled.
    pub(crate) fn cancel(&mut self, id: TimerId) {
        self.cancelled.insert(id);
    }

    /// The nearest still-pending deadline, ignoring cancelled entries.
    ///
    /// Immutable — a full O(heap size) scan rather than the mutating,
    /// head-compacting walk `take_due` uses, since `AsyncSource::next_deadline`
    /// is queried every event-loop iteration and must not disturb timer state
    /// mid-command.
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.heap
            .iter()
            .filter(|Reverse((_, id))| !self.cancelled.contains(id))
            .map(|Reverse((deadline, _))| *deadline)
            .min()
    }

    /// Pop every timer with deadline `<= now`, skipping (and discarding)
    /// cancelled entries as they're encountered.
    pub(crate) fn take_due(&mut self, now: Instant) -> Vec<TimerId> {
        let mut due = Vec::new();
        loop {
            self.drop_cancelled_head();
            match self.heap.peek() {
                Some(Reverse((deadline, _))) if *deadline <= now => {
                    let Reverse((_, id)) = self.heap.pop().expect("peeked Some above");
                    due.push(id);
                }
                _ => break,
            }
        }
        due
    }

    /// Pop cancelled entries sitting at the heap's head, regardless of their
    /// deadline — once cancelled an entry can never be due, so there is no
    /// reason to wait for its deadline to reclaim the slot.
    fn drop_cancelled_head(&mut self) {
        while let Some(Reverse((_, id))) = self.heap.peek() {
            if self.cancelled.remove(id) {
                self.heap.pop();
            } else {
                break;
            }
        }
    }
}

impl AsyncSource for TimerWheel {
    // A distant deadline must not cause 8ms busy-polling — `next_deadline`
    // bounds the event-loop's poll timeout instead. Due-now timers are
    // caught by `take_due` in the P3 drain phase, not by this flag.
    fn has_pending(&self) -> bool {
        false
    }

    fn next_deadline(&self) -> Option<Instant> {
        TimerWheel::next_deadline(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_due_pops_in_deadline_order_not_insertion_order() {
        let mut wheel = TimerWheel::new();
        // Insert far-then-near-then-mid — pop order must follow deadline,
        // not the order `schedule` was called in.
        let far = wheel.schedule(Duration::from_secs(10));
        let near = wheel.schedule(Duration::from_millis(1));
        let mid = wheel.schedule(Duration::from_secs(1));

        let due = wheel.take_due(Instant::now() + Duration::from_secs(20));
        assert_eq!(due, vec![near, mid, far]);
    }

    #[test]
    fn cancelled_entry_is_skipped_and_discarded() {
        let mut wheel = TimerWheel::new();
        let a = wheel.schedule(Duration::from_millis(1));
        let b = wheel.schedule(Duration::from_millis(2));
        wheel.cancel(a);

        let due = wheel.take_due(Instant::now() + Duration::from_secs(1));
        assert_eq!(due, vec![b]);
    }

    #[test]
    fn next_deadline_is_none_for_an_idle_wheel() {
        let wheel = TimerWheel::new();
        assert_eq!(wheel.next_deadline(), None);
    }

    #[test]
    fn next_deadline_picks_the_nearer_of_two_pending() {
        let mut wheel = TimerWheel::new();
        wheel.schedule(Duration::from_secs(10));
        wheel.schedule(Duration::from_millis(1));

        let deadline = wheel.next_deadline().expect("a timer is scheduled");
        assert!(
            deadline < Instant::now() + Duration::from_secs(1),
            "next_deadline must report the near (1ms) timer, not the far (10s) one"
        );
    }

    #[test]
    fn next_deadline_skips_a_cancelled_nearer_entry() {
        let mut wheel = TimerWheel::new();
        let near = wheel.schedule(Duration::from_millis(1));
        wheel.schedule(Duration::from_secs(10));
        wheel.cancel(near);

        let deadline = wheel.next_deadline().expect("a timer is scheduled");
        assert!(
            deadline >= Instant::now() + Duration::from_secs(1),
            "next_deadline must skip the cancelled near timer and report the far one"
        );
    }

    #[test]
    fn take_due_respects_the_deadline_boundary() {
        let mut wheel = TimerWheel::new();
        let before = Instant::now();
        let id = wheel.schedule(Duration::from_millis(50));
        // `before` predates `schedule`'s own `Instant::now()` read, so the
        // real deadline is >= before + 50ms — but the two reads can tie on
        // some clocks, and `take_due`'s `<=` is deliberately inclusive (a
        // deadline exactly at `now` must fire), so an exact `before + 50ms`
        // query isn't safely "not yet due". Query below the guaranteed
        // minimum deadline instead, with margin to absorb scheduling jitter
        // between the two reads.
        assert_eq!(
            wheel.take_due(before + Duration::from_millis(10)),
            Vec::new()
        );
        // A point comfortably past any scheduling jitter must fire.
        assert_eq!(wheel.take_due(before + Duration::from_secs(1)), vec![id]);
    }

    #[test]
    fn idle_wheel_never_reports_pending() {
        // The wheel's AsyncSource::has_pending is always false by design — a
        // distant deadline bounds the poll timeout instead of triggering
        // 8ms busy-polling (see the impl above).
        let mut wheel = TimerWheel::new();
        assert!(!wheel.has_pending());
        wheel.schedule(Duration::from_secs(10));
        assert!(!wheel.has_pending());
    }

    #[test]
    fn many_cancelled_entries_do_not_need_a_compaction_pass() {
        // Cancel everything without ever calling take_due/next_deadline in
        // between — the cancelled set only shrinks when entries eventually
        // surface at the heap head, never needs an explicit sweep.
        let mut wheel = TimerWheel::new();
        let ids: Vec<TimerId> = (0..50)
            .map(|i| wheel.schedule(Duration::from_millis(i)))
            .collect();
        for id in &ids {
            wheel.cancel(*id);
        }

        let due = wheel.take_due(Instant::now() + Duration::from_secs(1));
        assert!(due.is_empty());
        assert_eq!(wheel.next_deadline(), None);
    }
}
