//! Backtrace capture reads `RUST_BACKTRACE` once per process, and the
//! workspace bans `set_var`, so the test re-runs itself in a child process
//! started with the variable set.

use hume_platform::worker_panic::WorkerPanics;
use std::process::Command;
use std::sync::Arc;
use std::thread;

const TEST: &str = "a_worker_panic_carries_a_backtrace_when_the_environment_asks_for_one";
const CHILD: &str = "HUME_WORKER_PANIC_BACKTRACE_CHILD";

#[test]
fn a_worker_panic_carries_a_backtrace_when_the_environment_asks_for_one() {
    if std::env::var_os(CHILD).is_none() {
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(CHILD, "1")
            .env("RUST_BACKTRACE", "1")
            .status()
            .unwrap();
        assert!(status.success(), "child run failed: {status}");
        return;
    }

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
