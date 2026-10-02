//! Backtrace capture reads `RUST_BACKTRACE` once per process, so the variable
//! is set here before any panic, in a binary of its own.

use hume_platform::worker_panic::WorkerPanics;
use std::sync::Arc;
use std::thread;

#[test]
fn a_worker_panic_carries_a_backtrace_when_the_environment_asks_for_one() {
    // SAFETY: the only test in this binary, so no other thread reads the
    // environment.
    unsafe { std::env::set_var("RUST_BACKTRACE", "1") };
    let panics = WorkerPanics::install(Arc::new(|| {}));

    let _ = thread::spawn(|| panic!("boom")).join();

    let recorded = panics.take_unreported();
    assert_eq!(recorded.len(), 1);
    let backtrace = recorded[0]
        .backtrace
        .as_deref()
        .expect("backtrace captured");
    assert!(!backtrace.is_empty());
}
