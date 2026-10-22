use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use steel::steel_vm::ThreadStateController;
use steel::steel_vm::engine::Engine;

use super::EvalWatchdog;

/// A VM handle to arm with, for tests about the watchdog's own timing.
fn vm() -> ThreadStateController {
    Engine::new().get_thread_state_controller()
}

#[test]
fn expiry_sets_flag() {
    let flag = Arc::new(AtomicBool::new(false));
    let dog = EvalWatchdog::new();
    dog.arm(Arc::clone(&flag), vm(), Duration::from_millis(20));
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
    // Use a 30-second budget: the watchdog must not fire before cancel().
    dog.arm(Arc::clone(&flag), vm(), Duration::from_secs(30));
    dog.cancel(); // synchronous: the thread is disarmed when this returns
    assert!(
        !flag.load(Ordering::Relaxed),
        "flag must remain false after cancel"
    );
}

/// An unpaired second `arm` (no `cancel` in between) must be caught by the
/// caller-side `debug_assert` in `arm` itself, on the test's own thread,
/// rather than silently recovering inside the detached watchdog thread.
///
/// If the assert lived in `watchdog_loop`, it would run on the detached
/// thread and this test's `#[should_panic]` would never fire.
#[test]
#[should_panic(expected = "missing cancel() after the previous eval")]
#[cfg(debug_assertions)]
fn unpaired_arm_is_caught_on_callers_thread() {
    let flag = Arc::new(AtomicBool::new(false));
    let dog = EvalWatchdog::new();
    dog.arm(Arc::clone(&flag), vm(), Duration::from_secs(30));
    dog.arm(flag, vm(), Duration::from_secs(30)); // missing cancel(): must panic here
}

/// The persistent thread survives an arm/cancel cycle and fires on a
/// subsequent arm.
#[test]
fn rearming_after_cancel_still_fires() {
    let flag = Arc::new(AtomicBool::new(false));
    let dog = EvalWatchdog::new();
    dog.arm(Arc::clone(&flag), vm(), Duration::from_secs(30));
    dog.cancel();
    assert!(!flag.load(Ordering::Relaxed));

    dog.arm(Arc::clone(&flag), vm(), Duration::from_millis(20));
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
        dog.arm(Arc::clone(&flag), vm(), Duration::from_secs(30));
        // drop joins the thread; the armed wait sees Disconnected and exits
    }
    assert!(
        !flag.load(Ordering::Relaxed),
        "flag must remain false when dropped before the budget"
    );
}

/// Time spent waiting does not count: a budget that would have expired
/// during the wait has not when the wait ends.
#[test]
fn a_wait_pauses_the_budget() {
    let flag = Arc::new(AtomicBool::new(false));
    let dog = EvalWatchdog::new();
    let clock = dog.clock();
    dog.arm(Arc::clone(&flag), vm(), Duration::from_millis(50));
    clock.waiting(|| std::thread::sleep(Duration::from_millis(200)));
    assert!(
        !flag.load(Ordering::Relaxed),
        "the wait must not use up the budget"
    );
    dog.cancel();
}

/// The budget left when a wait began runs out after the wait ends.
#[test]
fn the_budget_left_runs_out_after_a_wait() {
    let flag = Arc::new(AtomicBool::new(false));
    let dog = EvalWatchdog::new();
    let clock = dog.clock();
    dog.arm(Arc::clone(&flag), vm(), Duration::from_millis(50));
    clock.waiting(|| std::thread::sleep(Duration::from_millis(100)));
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        flag.load(Ordering::Relaxed),
        "the remaining budget must expire"
    );
    dog.cancel();
}

/// Scheme time spent before a wait stays spent: the wait resumes with only
/// what was left when it began.
#[test]
fn time_before_a_wait_counts() {
    let flag = Arc::new(AtomicBool::new(false));
    let dog = EvalWatchdog::new();
    let clock = dog.clock();
    dog.arm(Arc::clone(&flag), vm(), Duration::from_millis(200));
    std::thread::sleep(Duration::from_millis(160));
    clock.waiting(|| std::thread::sleep(Duration::from_millis(20)));
    std::thread::sleep(Duration::from_millis(120));
    assert!(
        flag.load(Ordering::Relaxed),
        "40 ms were left when the wait began, so 120 ms after it the budget is spent"
    );
    dog.cancel();
}

/// A wait with nothing armed changes nothing for the next arm.
#[test]
fn a_wait_while_idle_is_ignored() {
    let flag = Arc::new(AtomicBool::new(false));
    let dog = EvalWatchdog::new();
    dog.clock().waiting(|| {});
    dog.arm(Arc::clone(&flag), vm(), Duration::from_millis(20));
    std::thread::sleep(Duration::from_millis(200));
    assert!(flag.load(Ordering::Relaxed));
    dog.cancel();
}
