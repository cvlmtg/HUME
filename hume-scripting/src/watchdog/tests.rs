use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use super::EvalWatchdog;

#[test]
fn expiry_sets_flag() {
    let flag = Arc::new(AtomicBool::new(false));
    let dog = EvalWatchdog::new();
    dog.arm(Arc::clone(&flag), Duration::from_millis(20));
    // Sleep 10× the budget to rule out timing races on slow CI.
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        flag.load(Ordering::Relaxed),
        "flag must be set after budget expires"
    );
    dog.cancel(); // cancel-after-expiry must ack from idle, not hang
}

#[test]
fn cancel_prevents_flag() {
    let flag = Arc::new(AtomicBool::new(false));
    let dog = EvalWatchdog::new();
    // Use a 30-second budget — the watchdog must not fire before cancel().
    dog.arm(Arc::clone(&flag), Duration::from_secs(30));
    dog.cancel(); // synchronous: the thread is disarmed when this returns
    assert!(
        !flag.load(Ordering::Relaxed),
        "flag must remain false after cancel"
    );
}

/// An unpaired second `arm` (no `cancel` in between) must be caught by the
/// caller-side `debug_assert` in `arm` itself — on the test's own thread —
/// rather than silently recovering inside the detached watchdog thread.
///
/// Fail oracle: move the assert into `watchdog_loop` (checked on the
/// detached thread) → this test's `#[should_panic]` never fires because
/// nothing on the *caller's* stack panics.
#[test]
#[should_panic(expected = "missing cancel() after the previous eval")]
#[cfg(debug_assertions)]
fn unpaired_arm_is_caught_on_callers_thread() {
    let flag = Arc::new(AtomicBool::new(false));
    let dog = EvalWatchdog::new();
    dog.arm(Arc::clone(&flag), Duration::from_secs(30));
    dog.arm(flag, Duration::from_secs(30)); // missing cancel() — must panic here
}

/// The persistent thread survives an arm/cancel cycle and fires on a
/// subsequent arm.
///
/// Fail oracle: make the thread exit after the first cancel → the second
/// arm never fires (or `arm` panics on a dead channel).
#[test]
fn rearming_after_cancel_still_fires() {
    let flag = Arc::new(AtomicBool::new(false));
    let dog = EvalWatchdog::new();
    dog.arm(Arc::clone(&flag), Duration::from_secs(30));
    dog.cancel();
    assert!(!flag.load(Ordering::Relaxed));

    dog.arm(Arc::clone(&flag), Duration::from_millis(20));
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        flag.load(Ordering::Relaxed),
        "second arm must fire after its budget"
    );
    dog.cancel();
}

/// Dropping the watchdog while armed shuts the thread down without firing.
#[test]
fn drop_while_armed_does_not_fire() {
    let flag = Arc::new(AtomicBool::new(false));
    {
        let dog = EvalWatchdog::new();
        dog.arm(Arc::clone(&flag), Duration::from_secs(30));
        // drop joins the thread; the armed wait sees Disconnected and exits
    }
    assert!(
        !flag.load(Ordering::Relaxed),
        "flag must remain false when dropped before the budget"
    );
}
