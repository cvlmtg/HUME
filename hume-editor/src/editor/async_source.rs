//! Generalized event-loop wake: composes every asynchronous work source
//! (parse worker now; LSP backend and timer wheel later) behind one
//! `has_pending` / `next_deadline` predicate so `run`'s event-poll timeout
//! doesn't hard-code a single source.

use std::time::{Duration, Instant};

use hume_treesitter::parse_worker::ParseBackend;

use super::Editor;

/// One source of asynchronous work the event loop must wake for.
///
/// Implemented by the parse worker (now), the LSP backend (C6) and the
/// timer wheel (P7).
pub(crate) trait AsyncSource {
    /// Work may complete soon — poll instead of blocking on input.
    fn has_pending(&self) -> bool;
    /// Absolute wake deadline, if this source schedules timed work (P7).
    fn next_deadline(&self) -> Option<Instant> {
        None
    }
}

impl AsyncSource for Box<dyn ParseBackend> {
    fn has_pending(&self) -> bool {
        self.has_in_flight()
    }
}

impl Editor {
    /// One place to enumerate async sources. Adding a source = one line here
    /// plus its `AsyncSource` impl.
    fn async_sources(&self) -> [&dyn AsyncSource; 2] {
        [&self.parse_worker, &self.timer_wheel]
    }

    /// `Some(timeout)` => poll with it; `None` => block on `event::read()`.
    ///
    /// `timeout` = the smaller of "8ms because some source has pending work"
    /// and "time until the nearest scheduled deadline" — whichever bounds
    /// apply. Idle (no source pending, no deadline scheduled) is `None`, a
    /// genuinely blocking read, so the editor never busy-polls at rest.
    pub(crate) fn wake_timeout(&self) -> Option<Duration> {
        let sources = self.async_sources();
        let pending = sources.iter().any(|s| s.has_pending());
        let deadline = sources.iter().filter_map(|s| s.next_deadline()).min();

        let pending_timeout = pending.then_some(Duration::from_millis(8));
        match (pending_timeout, deadline) {
            (None, None) => None,
            (Some(t), None) => Some(t),
            (None, Some(d)) => Some(d.saturating_duration_since(Instant::now())),
            (Some(t), Some(d)) => Some(t.min(d.saturating_duration_since(Instant::now()))),
        }
    }

    /// Named drain phase for completed async work, called once per frame from
    /// `prepare_frame`. New sources (LSP responses) add their drain call here.
    pub(super) fn drain_async_sources(&mut self) {
        self.reparse_stale_buffers();
        // Collected but not yet dispatched — B4 adds the TimerId -> Steel
        // thunk side table that gives these ids a payload.
        let _ = self.timer_wheel.take_due(Instant::now());
    }
}
