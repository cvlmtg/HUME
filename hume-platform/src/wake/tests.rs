use std::time::Duration;

use super::*;

#[test]
fn a_waking_thread_wakes_once_when_its_body_returns() {
    let (wake, woken) = counting_wake();
    let handle = spawn_waking("hume-test-waking", wake, |_| 7).expect("spawn");
    assert_eq!(handle.join().expect("no panic"), 7);
    assert_eq!(woken.try_iter().count(), 1);
}

#[test]
fn a_waking_thread_wakes_when_its_body_panics() {
    let (wake, woken) = counting_wake();
    let handle = spawn_waking::<(), _>("hume-test-waking", wake, |_| {
        panic!("simulated worker crash")
    })
    .expect("spawn");
    assert!(handle.join().is_err(), "the body panicked");
    woken
        .recv_timeout(Duration::from_secs(1))
        .expect("the unwind wakes");
}
