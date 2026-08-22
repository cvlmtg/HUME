//! Regression tests for `TestGlobals`, the reentrant lock guarding the
//! suite's process globals. It replaced two plain `std::sync::Mutex`es that
//! self-deadlocked twice — `docs/LESSONS.md` L7, and again in
//! `bad_config_value_fails_plugin_load_with_prefixed_error`
//! (`unix/git_diff_plugin.rs`) — with no panic and no assertion failure, just
//! the test runner's generic "running for over 60s" notice on a process-wide
//! lock that then starved every other concurrently-running test too.
//!
//! Fail oracle: swap `TestGlobals::inner`'s `parking_lot::ReentrantMutex` for
//! a plain `std::sync::Mutex` and the first two tests below hang instead of
//! completing — there is no timeout to assert against directly, since a hang
//! is exactly the failure mode this type exists to make impossible.

use super::*;

#[test]
fn safe_tempdir_does_not_deadlock_under_a_held_env_claim() {
    let _claim = TEST_GLOBALS.claim(Global::Env);
    // Would hang forever under the old non-reentrant `HUME_RUNTIME_MUTEX` —
    // this is the exact call shape that caused both real hangs.
    let dir = safe_tempdir();
    assert!(dir.path().is_dir());
}

#[test]
#[should_panic(expected = "already holds a Env claim")]
fn claiming_the_same_global_twice_on_one_thread_panics_instead_of_hanging() {
    let _outer = TEST_GLOBALS.claim(Global::Env);
    let _inner = TEST_GLOBALS.claim(Global::Env);
}

#[test]
fn claiming_a_different_global_while_holding_one_succeeds() {
    let _env = TEST_GLOBALS.claim(Global::Env);
    // Different resource — legitimate nesting (e.g. `CwdSandbox` constructed
    // inside a live `HumeRuntimeGuard` in `unix/pickers_plugin.rs`) — must
    // neither panic nor hang.
    let _cwd = TEST_GLOBALS.claim(Global::Cwd);
}
