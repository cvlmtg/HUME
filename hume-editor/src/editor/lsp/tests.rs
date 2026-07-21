use super::*;

#[test]
fn spinner_clock_advances_only_after_the_interval_elapses() {
    let mut clock = SpinnerClock::default();
    let t0 = Instant::now();

    clock.maybe_advance(t0);
    assert_eq!(clock.frame, 1, "first call always advances");

    clock.maybe_advance(t0 + Duration::from_millis(50));
    assert_eq!(clock.frame, 1, "below the interval — no advance");

    clock.maybe_advance(t0 + SPINNER_INTERVAL);
    assert_eq!(clock.frame, 2, "at/past the interval — advances by one");
}
