//! Timer scheduling — moved out of `host.rs`'s per-capability split.

/// Timer scheduling — accessed through [`EditorHost::timers`](super::EditorHost::timers).
pub trait TimerHost {
    /// Schedules `thunk` — opaque to this trait, a raw Steel closure — to
    /// fire after `ms` milliseconds. Returns the new timer id, or `None` if
    /// this host has no timer wheel to schedule onto right now (`EditorHostImpl`
    /// only carries one at three call sites — command dispatch, hook fire,
    /// queued-call drain — not during init).
    fn schedule_timer(&mut self, ms: u64, thunk: steel::rvals::SteelVal) -> Option<u64>;

    /// Cancels a previously scheduled timer. A no-op if `id` already fired,
    /// was already cancelled, or this host has no timer wheel right now.
    fn cancel_timer(&mut self, id: u64);
}
