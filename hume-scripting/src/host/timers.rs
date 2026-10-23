//! Timer scheduling.

/// Timer scheduling, accessed through [`EditorHost::timers`](super::EditorHost::timers).
pub trait TimerHost {
    /// Schedules `thunk` (opaque to this trait, a Steel closure) to
    /// fire after `ms` milliseconds. Returns the new timer id, or `None` if
    /// this host has no timer wheel to schedule onto right now (`EditorHostImpl`
    /// only carries one at three call sites: command dispatch, hook fire,
    /// queued-call drain; not during init).
    fn schedule_timer(&mut self, ms: u64, thunk: crate::Rooted) -> Option<u64>;

    /// Cancels timer `id` from [`Self::schedule_timer`]: its thunk never
    /// runs, even one already due and queued. A no-op once the thunk has
    /// run, after an earlier cancel, for an id `schedule_timer` never
    /// returned, or when this host has no timer wheel right now.
    fn cancel_timer(&mut self, id: u64);
}
