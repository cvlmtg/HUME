//! Generalized event-loop wake: composes every asynchronous work source
//! (parse worker, LSP backend, timer wheel) behind one `next_wake`
//! predicate so `run`'s event-poll timeout doesn't hard-code a single
//! source.

use std::time::{Duration, Instant};

use hume_treesitter::parse_worker::ParseBackend;

use super::Editor;

/// Poll cadence used while a source's work may complete any moment (a parse
/// or LSP response in flight) — not a real deadline, just a short recheck.
pub(crate) const PENDING_POLL: Duration = Duration::from_millis(8);

/// One source of asynchronous work the event loop must wake for.
///
/// Implemented by the parse worker, the LSP backend, and the timer wheel.
pub(crate) trait AsyncSource {
    /// Next instant the event loop should wake for this source, if any.
    /// `None` means this source needs no wake — the loop may block on input.
    fn next_wake(&self, now: Instant) -> Option<Instant>;
}

impl AsyncSource for Box<dyn ParseBackend> {
    fn next_wake(&self, now: Instant) -> Option<Instant> {
        self.has_in_flight().then(|| now + PENDING_POLL)
    }
}

impl Editor {
    /// One place to enumerate async sources. Adding a source = one line here
    /// plus its `AsyncSource` impl.
    fn async_sources(&self) -> [&dyn AsyncSource; 3] {
        [&self.parse_worker, &self.timer_wheel, &self.lsp]
    }

    /// `Some(timeout)` => poll with it; `None` => block on `event::read()`.
    ///
    /// `timeout` = time until the nearest source's wake instant. Idle (no
    /// source has a wake instant) is `None`, a genuinely blocking read, so
    /// the editor never busy-polls at rest.
    pub(crate) fn wake_timeout(&self) -> Option<Duration> {
        let now = Instant::now();
        self.async_sources()
            .iter()
            .filter_map(|s| s.next_wake(now))
            .min()
            .map(|wake| wake.saturating_duration_since(now))
    }

    /// Named drain phase for completed async work, called once per frame from
    /// `prepare_frame`.
    pub(super) fn drain_async_sources(&mut self) {
        self.reparse_stale_buffers();
        self.drain_due_timers();
        self.drain_lsp();
    }
}
