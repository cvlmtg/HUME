// P3/P7 (docs/lsp/step-0.md) — generalized event-loop wake (`wake_timeout`)
// and the timer wheel's integration with it as a second real AsyncSource.

use std::time::{Duration, Instant};

use super::*;
use hume_treesitter::parse_worker::{ParseBackend, ParseDone, ParseRequest};

/// A `ParseBackend` double that reports work permanently in flight, without
/// spinning a real thread — for exercising `wake_timeout`'s "pending" branch
/// deterministically (the production `InlineParseBackend` always reports
/// `has_in_flight() == false`, so it can't reach that branch).
struct AlwaysPendingBackend;

impl ParseBackend for AlwaysPendingBackend {
    fn post(&mut self, _req: ParseRequest) {}
    fn drain_done(&mut self) -> Vec<ParseDone> {
        Vec::new()
    }
    fn is_in_flight(&self, _bid: hume_engine::pipeline::BufferId, _text_gen: u64) -> bool {
        false
    }
    fn remove_in_flight(&mut self, _bid: hume_engine::pipeline::BufferId) {}
    fn clear_in_flight_if_matches(
        &mut self,
        _bid: hume_engine::pipeline::BufferId,
        _text_gen: u64,
        _lang: &std::sync::Arc<hume_treesitter::registry::LanguageConfig>,
    ) {
    }
    fn has_in_flight(&self) -> bool {
        true
    }
    fn is_disconnected(&self) -> bool {
        false
    }
}

#[test]
fn wake_timeout_is_none_when_idle() {
    // The test harness's InlineParseBackend never reports in-flight work and
    // a freshly-constructed editor's timer wheel has nothing scheduled —
    // idle across every source must stay a blocking read.
    let ed = editor_from("-[w]>ord\n");
    assert_eq!(ed.wake_timeout(), None);
}

#[test]
fn wake_timeout_is_8ms_when_a_source_is_pending() {
    let mut ed = editor_from("-[w]>ord\n");
    ed.parse_worker = Box::new(AlwaysPendingBackend);
    assert_eq!(ed.wake_timeout(), Some(Duration::from_millis(8)));
}

#[test]
fn wake_timeout_bounded_by_nearer_timer_deadline() {
    // P3 deferred this case ("Some(<8ms) when a nearer deadline exists")
    // until a second real AsyncSource existed — the timer wheel is that
    // source.
    let mut ed = editor_from("-[w]>ord\n");
    ed.timer_wheel.schedule(Duration::from_millis(2));

    let timeout = ed.wake_timeout().expect("a timer is scheduled");
    assert!(
        timeout <= Duration::from_millis(2),
        "wake_timeout must be bounded by the nearer 2ms timer deadline, got {timeout:?}"
    );
}

#[test]
fn wake_timeout_distant_timer_bounds_without_busy_polling() {
    // A far-future deadline must neither collapse to the 8ms pending-poll
    // ceiling (TimerWheel::has_pending is always false) nor block forever
    // (None) — it bounds the timeout to roughly the real wait.
    let mut ed = editor_from("-[w]>ord\n");
    ed.timer_wheel.schedule(Duration::from_secs(10));

    let timeout = ed.wake_timeout().expect("a timer is scheduled");
    assert!(
        timeout > Duration::from_millis(8) && timeout <= Duration::from_secs(10),
        "a distant deadline must not trigger 8ms busy-polling, got {timeout:?}"
    );
}

#[test]
fn timer_wheel_end_to_end_tick_via_editor() {
    // Sleep-free: jump the query point 20ms past scheduling instead of
    // sleeping in the test (per the card's allowance).
    let mut ed = editor_from("-[w]>ord\n");
    let id = ed.timer_wheel.schedule(Duration::from_millis(10));

    let due = ed.timer_wheel.take_due(Instant::now() + Duration::from_millis(20));
    assert_eq!(due, vec![id]);
}
