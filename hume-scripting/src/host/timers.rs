//! Timer scheduling.

/// A timer `after!` scheduled, as Steel holds it: an opaque value only
/// `cancel-timer!` accepts, so a job's handle can never name a timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimerHandle(u64);

impl TimerHandle {
    /// Wraps an id the host's timer wheel minted.
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The id the host's timer wheel minted.
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// Timer scheduling, accessed through [`EditorHost::timers`](super::EditorHost::timers).
pub trait TimerHost {
    /// Schedules `thunk` (opaque to this trait, a Steel closure) to
    /// fire after `ms` milliseconds. Returns the new timer, or `None` if
    /// this host has no timer wheel to schedule onto right now (`EditorHostImpl`
    /// only carries one at three call sites: command dispatch, hook fire,
    /// queued-call drain; not during init).
    fn schedule_timer(&mut self, ms: u64, thunk: crate::Callback) -> Option<TimerHandle>;

    /// Cancels `timer` from [`Self::schedule_timer`]: its thunk never
    /// runs, even one already due and queued. A no-op once the thunk has
    /// run, after an earlier cancel, or when this host has no timer wheel
    /// right now.
    fn cancel_timer(&mut self, timer: TimerHandle);
}
