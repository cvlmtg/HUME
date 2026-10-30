use std::time::Duration;

use steel::steel_vm::ThreadStateController;
use steel::steel_vm::engine::Engine;

use super::EvalWatchdog;

/// A VM handle to arm with, for tests about the watchdog's own timing.
fn vm() -> ThreadStateController {
    Engine::new().get_thread_state_controller()
}

#[test]
fn expiry_is_reported_by_cancel() {
    let dog = EvalWatchdog::new();
    dog.arm(vm(), Duration::from_millis(20));
    // Sleep 10× the budget to rule out timing races on slow CI.
    std::thread::sleep(Duration::from_millis(200));
    // cancel-after-expiry must ack from idle, not hang
    assert!(dog.cancel(), "the budget must have expired");
}

#[test]
fn cancel_before_expiry_reports_no_fire() {
    let dog = EvalWatchdog::new();
    // Use a 30-second budget: the watchdog must not fire before cancel().
    dog.arm(vm(), Duration::from_secs(30));
    assert!(!dog.cancel(), "the budget must not have expired");
}

/// An unpaired second `arm` (no `cancel` in between) must be caught by the
/// caller-side `debug_assert` in `arm` itself, on the test's own thread,
/// rather than silently recovering inside the detached watchdog thread.
#[test]
#[should_panic(expected = "missing cancel() after the previous eval")]
#[cfg(debug_assertions)]
fn unpaired_arm_is_caught_on_callers_thread() {
    let dog = EvalWatchdog::new();
    dog.arm(vm(), Duration::from_secs(30));
    dog.arm(vm(), Duration::from_secs(30)); // missing cancel(): must panic here
}

/// The persistent thread survives an arm/cancel cycle and fires on a
/// subsequent arm.
#[test]
fn rearming_after_cancel_still_fires() {
    let dog = EvalWatchdog::new();
    dog.arm(vm(), Duration::from_secs(30));
    assert!(!dog.cancel());

    dog.arm(vm(), Duration::from_millis(20));
    std::thread::sleep(Duration::from_millis(200));
    assert!(dog.cancel(), "second arm must fire after its budget");
}

/// A fire is reported to the cancel that follows it, not to the next arm's.
#[test]
fn a_fire_is_reported_once() {
    let dog = EvalWatchdog::new();
    dog.arm(vm(), Duration::from_millis(20));
    std::thread::sleep(Duration::from_millis(200));
    assert!(dog.cancel());

    dog.arm(vm(), Duration::from_secs(30));
    assert!(!dog.cancel(), "the previous arm's fire must not carry over");
}

/// Dropping the watchdog while armed shuts the thread down without
/// interrupting the VM.
#[test]
fn drop_while_armed_does_not_fire() {
    let mut engine = Engine::new();
    {
        let dog = EvalWatchdog::new();
        dog.arm(
            engine.get_thread_state_controller(),
            Duration::from_secs(30),
        );
        // drop joins the thread; the armed wait sees Shutdown and exits
    }
    engine
        .compile_and_run_raw_program("(+ 1 2)")
        .expect("the VM must not have been interrupted");
}

/// Time spent waiting does not count: a budget that would have expired
/// during the wait has not when the wait ends.
#[test]
fn a_wait_pauses_the_budget() {
    let dog = EvalWatchdog::new();
    let clock = dog.clock();
    dog.arm(vm(), Duration::from_millis(200));
    clock.waiting(|| std::thread::sleep(Duration::from_millis(500)));
    assert!(!dog.cancel(), "the wait must not use up the budget");
}

/// The budget left when a wait began runs out after the wait ends.
#[test]
fn the_budget_left_runs_out_after_a_wait() {
    let dog = EvalWatchdog::new();
    let clock = dog.clock();
    dog.arm(vm(), Duration::from_millis(50));
    clock.waiting(|| std::thread::sleep(Duration::from_millis(100)));
    std::thread::sleep(Duration::from_millis(300));
    assert!(dog.cancel(), "the remaining budget must expire");
}

/// Scheme time spent before a wait stays spent: the wait resumes with only
/// what was left when it began.
#[test]
fn time_before_a_wait_counts() {
    let dog = EvalWatchdog::new();
    let clock = dog.clock();
    dog.arm(vm(), Duration::from_millis(600));
    std::thread::sleep(Duration::from_millis(500));
    clock.waiting(|| std::thread::sleep(Duration::from_millis(20)));
    std::thread::sleep(Duration::from_millis(350));
    assert!(
        dog.cancel(),
        "100 ms were left when the wait began, so 350 ms after it the budget is spent"
    );
}

/// A wait with nothing armed changes nothing for the next arm.
#[test]
fn a_wait_while_idle_is_ignored() {
    let dog = EvalWatchdog::new();
    dog.clock().waiting(|| {});
    dog.arm(vm(), Duration::from_millis(20));
    std::thread::sleep(Duration::from_millis(200));
    assert!(dog.cancel());
}

/// An `OsWait` that outlives its watchdog runs its wait, with no budget left
/// to stop.
#[test]
fn an_os_wait_runs_its_wait_after_the_watchdog_is_gone() {
    let dog = EvalWatchdog::new();
    let wait = hume_core::OsWait::default();
    dog.connect(&wait);
    drop(dog);
    assert_eq!(wait.waiting(|| 7), 7);
}
