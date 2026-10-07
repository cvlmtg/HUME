//! Regression tests for `TestGlobals`, the reentrant lock guarding the
//! suite's process globals. Nesting a different `Global` must neither panic
//! nor hang; claiming the same one twice, or taking `PATH_USERS` after the
//! mutex, must panic. A hang is the failure this type exists to rule out, so
//! no test here asserts against a timeout.

use super::*;

#[test]
#[should_panic(expected = "already holds a Env claim")]
fn claiming_the_same_global_twice_on_one_thread_panics_instead_of_hanging() {
    let _outer = TEST_GLOBALS.claim(Global::Env);
    let _inner = TEST_GLOBALS.claim(Global::Env);
}

#[test]
fn claiming_a_different_global_while_holding_one_succeeds() {
    let _env = TEST_GLOBALS.claim(Global::Env);
    // Different resource: legitimate nesting (e.g. `CwdSandbox` constructed
    // inside a live `RuntimeDirs` in `unix/pickers_plugin.rs`) must
    // neither panic nor hang.
    let _cwd = TEST_GLOBALS.claim(Global::Cwd);
}

#[test]
#[should_panic(expected = "holds a Cwd claim and claims `Global::Env`")]
fn env_claim_inside_a_cwd_claim_panics_instead_of_risking_a_deadlock() {
    let _cwd = TEST_GLOBALS.claim(Global::Cwd);
    let _env = TEST_GLOBALS.claim(Global::Env);
}

#[test]
#[should_panic(expected = "holds a Cwd claim and builds a `Dirs` fixture")]
fn path_reader_inside_a_cwd_claim_panics_instead_of_risking_a_deadlock() {
    let _cwd = TEST_GLOBALS.claim(Global::Cwd);
    let _path = path_reader();
}

#[test]
fn nested_path_readers_on_one_thread_succeed() {
    let _outer = path_reader();
    let _inner = path_reader();
}
