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
fn idle_wheel_has_no_wake() {
    let wheel = TimerWheel::new();
    assert_eq!(wheel.next_wake(Instant::now()), None);
}

#[test]
fn distant_timer_reports_its_own_deadline_not_a_pending_poll() {
    // AsyncSource::next_wake must return the timer's real deadline — a
    // distant timer must not collapse to a short poll cadence (see the
    // impl above).
    let mut wheel = TimerWheel::new();
    let now = Instant::now();
    wheel.schedule(Duration::from_secs(10));

    let wake = wheel.next_wake(now).expect("a timer is scheduled");
    assert!(
        wake >= now + Duration::from_secs(9),
        "expected the wheel's own ~10s deadline, got {:?} from now",
        wake.saturating_duration_since(now)
    );
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
