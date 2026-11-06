use super::*;

#[test]
fn take_due_pops_in_deadline_order_not_insertion_order() {
    let mut wheel = TimerWheel::new();
    let now = Instant::now();
    // Insert far-then-near-then-mid: pop order must follow deadline,
    // not the order `schedule` was called in.
    let far = wheel.schedule(Duration::from_secs(10), now);
    let near = wheel.schedule(Duration::from_millis(1), now);
    let mid = wheel.schedule(Duration::from_secs(1), now);

    let due = wheel.take_due(now + Duration::from_secs(20));
    assert_eq!(due, vec![near, mid, far]);
}

#[test]
fn cancelled_entry_is_skipped_and_discarded() {
    let mut wheel = TimerWheel::new();
    let now = Instant::now();
    let a = wheel.schedule(Duration::from_millis(1), now);
    let b = wheel.schedule(Duration::from_millis(2), now);
    wheel.cancel(a);

    let due = wheel.take_due(now + Duration::from_secs(1));
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
    let now = Instant::now();
    wheel.schedule(Duration::from_secs(10), now);
    wheel.schedule(Duration::from_millis(1), now);

    assert_eq!(wheel.next_deadline(), Some(now + Duration::from_millis(1)));
}

#[test]
fn next_deadline_skips_a_cancelled_nearer_entry() {
    let mut wheel = TimerWheel::new();
    let now = Instant::now();
    let near = wheel.schedule(Duration::from_millis(1), now);
    wheel.schedule(Duration::from_secs(10), now);
    wheel.cancel(near);

    assert_eq!(wheel.next_deadline(), Some(now + Duration::from_secs(10)));
}

#[test]
fn take_due_respects_the_deadline_boundary() {
    let mut wheel = TimerWheel::new();
    let now = Instant::now();
    let id = wheel.schedule(Duration::from_millis(50), now);

    assert_eq!(wheel.take_due(now + Duration::from_millis(49)), Vec::new());
    // Inclusive: a deadline equal to `now` fires.
    assert_eq!(wheel.take_due(now + Duration::from_millis(50)), vec![id]);
}

#[test]
fn idle_wheel_has_no_wake() {
    let wheel = TimerWheel::new();
    assert_eq!(wheel.next_wake(Instant::now()), None);
}

#[test]
fn distant_timer_reports_its_own_deadline_not_a_pending_poll() {
    // AsyncSource::next_wake must return the timer's real deadline: a
    // distant timer must not collapse to a short poll cadence (see the
    // impl above).
    let mut wheel = TimerWheel::new();
    let now = Instant::now();
    wheel.schedule(Duration::from_secs(10), now);

    assert_eq!(wheel.next_wake(now), Some(now + Duration::from_secs(10)));
}

#[test]
fn many_cancelled_entries_do_not_need_a_compaction_pass() {
    // Cancel everything without ever calling take_due/next_deadline in
    // between. The cancelled set only shrinks when entries eventually
    // surface at the heap head, never needs an explicit sweep.
    let mut wheel = TimerWheel::new();
    let now = Instant::now();
    let ids: Vec<TimerId> = (0..50)
        .map(|i| wheel.schedule(Duration::from_millis(i), now))
        .collect();
    for id in &ids {
        wheel.cancel(*id);
    }

    let due = wheel.take_due(now + Duration::from_secs(1));
    assert!(due.is_empty());
    assert_eq!(wheel.next_deadline(), None);
}
