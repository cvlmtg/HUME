//! Nearest-deadline timer registry, integrated with the `AsyncSource` wake
//! logic. Rust-side machinery only — payload-agnostic on purpose: the wheel
//! hands back opaque [`TimerId`]s, and a side table adds the `TimerId ->
//! Steel thunk` mapping that gives them meaning, keeping Steel types out of the
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
    /// head-compacting walk `take_due` uses, since `AsyncSource::next_wake`
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
    // The wheel's own deadline bounds the event-loop's poll timeout — a
    // distant timer never forces the 8ms pending-poll cadence. Due-now
    // timers are caught by `take_due` in the async-source drain phase.
    fn next_wake(&self, _now: Instant) -> Option<Instant> {
        TimerWheel::next_deadline(self)
    }
}

#[cfg(test)]
mod tests;
