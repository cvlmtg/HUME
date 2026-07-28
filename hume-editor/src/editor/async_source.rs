//! Generalized event-loop wake: composes every asynchronous work source
//! (the LSP backend, the timer wheel) behind one `next_wake` predicate so
//! `run`'s wait primitive doesn't hard-code a single source.
//!
//! Sources report *real deadlines only* — a request timeout, a `$/progress`
//! spinner tick, a timer's fire time. Completion (an LSP response landing, a
//! parse finishing) is not a deadline this module tracks: the background
//! threads that produce it hold a `WakeCallback` and signal the event
//! loop's wait primitive directly the moment they post a result (the
//! waker wraps `termina::PlatformWaker::wake`, which interrupts a blocked
//! `EventReader::poll`), so there is nothing to poll for. The parse worker
//! accordingly contributes no `AsyncSource` — it has no deadline of its
//! own, only arrival-driven wakes. A picker's spawned line source follows
//! the same shape: `drain_picker_source` (`picker_source.rs`) has no
//! matching `AsyncSource` entry, only a drain call from
//! `drain_async_sources` below.

use std::time::{Duration, Instant};

use super::Editor;

/// One source of asynchronous work the event loop must wake for.
///
/// Implemented by the LSP backend and the timer wheel.
pub(crate) trait AsyncSource {
    /// Next instant the event loop should wake for this source, if any.
    /// `None` means this source needs no wake — the loop may block on input.
    fn next_wake(&self, now: Instant) -> Option<Instant>;
}

impl Editor {
    /// One place to enumerate async sources. Adding a source = one line here
    /// plus its `AsyncSource` impl.
    fn async_sources(&self) -> [&dyn AsyncSource; 2] {
        [&self.timer_wheel, &self.lsp]
    }

    /// `Some(timeout)` => wait with it; `None` => block indefinitely.
    ///
    /// `timeout` = time until the nearest source's wake instant. Idle (no
    /// source has a wake instant) is `None` — the wait primitive blocks
    /// until real input or a background-thread wake, so the editor never
    /// busy-polls at rest.
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
        self.drain_picker_source();
    }
}
