//! `define-typed-command!`, `EvalWatchdog` and session file-lock tests.

use super::*;

// ── define-typed-command! ────────────────────────────────────────────────

#[test]
fn define_typed_command_registers_into_typed_table() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-typed-command! "greet" "doc" (lambda (bid arg) (+ 1 0)))"#,
        &mut mock,
    )
    .expect("define-typed-command! must succeed");

    assert!(
        mock.registered_typed_cmds.iter().any(|d| d.name == "greet"),
        "greet must land in registered_typed_cmds, not registered_cmds"
    );
    assert!(
        !mock.registered_cmds.iter().any(|d| d.name == "greet"),
        "define-typed-command! must never register a mappable command"
    );
}

/// `define-typed-command!` has no `#:repeatable` keyword: dot-repeat has
/// no meaning for a `:` command. Steel's keyword-arg lambda syntax silently
/// ignores an unrecognized `#:key value` pair rather than erroring, so this
/// pins that outcome (registration still succeeds) rather than a rejection
/// no mechanism here actually produces.
#[test]
fn define_typed_command_ignores_unrecognized_repeatable_keyword() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-typed-command! "bad-cmd" "doc" (lambda () (+ 1 0)) #:repeatable #t)"#,
        &mut mock,
    )
    .expect("an unrecognized keyword is silently ignored, not rejected");
    assert!(
        mock.registered_typed_cmds
            .iter()
            .any(|d| d.name == "bad-cmd"),
        "the command must still register despite the ignored keyword"
    );
}

/// `#:inline-output #t` sets `SteelTypedCmdDef.inline_output: true`; plain
/// `define-typed-command!` sets it to `false`, the typed counterpart of
/// `define_command_inline_output_sets_flag`, asserted independently rather
/// than inferred from the mappable side: the two registries are strictly
/// separate and free to diverge.
#[test]
fn define_typed_command_inline_output_sets_flag() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-typed-command! "inline-typed" "doc" (lambda () (+ 1 0)) #:inline-output #t)
           (define-typed-command! "plain-typed"  "doc" (lambda () (+ 1 0)))"#,
        &mut mock,
    )
    .expect("eval should succeed");

    let inline = mock
        .registered_typed_cmds
        .iter()
        .find(|d| d.name == "inline-typed")
        .expect("inline-typed not found");
    let plain = mock
        .registered_typed_cmds
        .iter()
        .find(|d| d.name == "plain-typed")
        .expect("plain-typed not found");
    assert!(
        inline.body.inline_output,
        "#:inline-output #t should set inline_output = true"
    );
    assert!(
        !plain.body.inline_output,
        "plain define-typed-command! should set inline_output = false"
    );
}

/// A name already claimed by `define-command!` (mappable) collides with
/// `define-typed-command!` for the same name, and vice versa: the two
/// kinds share one namespace in the real `CommandRegistry`.
#[test]
fn define_typed_command_collides_with_existing_mappable_name() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "dup" "doc" (lambda () (+ 1 0)))"#,
        &mut mock,
    )
    .expect("first define-command! must succeed");

    let result = h.eval_source(
        r#"(define-typed-command! "dup" "doc" (lambda () (+ 1 0)))"#,
        &mut mock,
    );
    assert!(
        result.is_err(),
        "define-typed-command! must reject a name already claimed as mappable"
    );
}

/// The reverse direction of the collision above, asserted in that test's
/// own doc comment ("and vice versa") but never actually exercised until
/// this test.
#[test]
fn define_command_collides_with_existing_typed_name() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-typed-command! "dup" "doc" (lambda () (+ 1 0)))"#,
        &mut mock,
    )
    .expect("first define-typed-command! must succeed");

    let result = h.eval_source(
        r#"(define-command! "dup" "doc" (lambda () (+ 1 0)))"#,
        &mut mock,
    );
    assert!(
        result.is_err(),
        "define-command! must reject a name already claimed as typed"
    );
}

// ── EvalWatchdog ──────────────────────────────────────────────────────────

/// Cancelling a watchdog with a long budget wakes the thread immediately.
/// The channel-driven cancel must not block until the budget elapses.
#[test]
fn watchdog_cancel_wakes_thread_immediately() {
    let budget = std::time::Duration::from_secs(10);
    let vm = steel::steel_vm::engine::Engine::new().get_thread_state_controller();
    let start = std::time::Instant::now();
    let watchdog = EvalWatchdog::new();
    watchdog.arm(vm, budget);
    let fired = watchdog.cancel();
    // cancel() must return well within the budget; 500 ms is generous.
    assert!(
        start.elapsed() < std::time::Duration::from_millis(500),
        "cancel() took too long: {:?}",
        start.elapsed()
    );
    assert!(!fired, "cancelled before the budget expired");
}

/// A watchdog with a tiny budget stops a loop that would run forever.
#[test]
fn eval_source_watchdog_aborts_runaway() {
    let mut h = host();
    let mut mock = MockHost::new();

    let budget = std::time::Duration::from_millis(50);
    let start = std::time::Instant::now();

    let err = h
        .eval_source_watchdog(
            // This loop would run forever without the watchdog.
            "(let loop () (loop))",
            budget,
            &mut mock,
        )
        .unwrap_err();

    assert!(
        err.contains("interrupted"),
        "expected 'interrupted' in error, got: {err}"
    );
    // Must abort well within a second; if not, the watchdog didn't fire.
    assert!(
        start.elapsed() < std::time::Duration::from_secs(1),
        "eval took too long: {:?}",
        start.elapsed()
    );
    h.eval_source("(+ 1 2)", &mut mock)
        .expect("the next eval runs once the stopped one has returned");
}

/// Command bodies cannot mutate settings/keymap (`EvalMode::Command` during
/// call_steel_cmd; init-only builtins raise Steel errors).  This test verifies
/// that after a watchdog interrupt the settings remain at their pre-call values.
/// Also verifies the budget is read from settings at call time.
#[test]
fn call_steel_cmd_interrupt_leaves_settings_unchanged() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "looper" "loop" (lambda () (let loop () (loop))))"#,
        &mut mock,
    )
    .unwrap();
    let cmd_name = "looper".to_string();

    assert_eq!(mock.settings.indent_width, 4, "precondition");
    mock.settings.steel_command_budget_ms = 50;

    let err = h
        .call_steel_cmd_by_name(&cmd_name, vec![], &mut mock)
        .unwrap_err();

    assert!(
        err.message.contains("interrupted"),
        "expected 'interrupted', got: {err}"
    );
    assert_eq!(
        mock.settings.indent_width, 4,
        "indent-width must be unchanged after interrupt"
    );
}

/// `set-option!` is registered `open` (no eval-mode gate), so calling it from
/// a Steel command body (`call_steel_cmd` runs with `EvalMode::Command`)
/// must actually apply the setting, not raise a gate error. A plugin-defined
/// command can now toggle a global setting at runtime, the gap this closes.
#[test]
fn call_steel_cmd_set_option_from_body_applies_the_setting() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "try-set" "" (lambda () (set-option! "indent-width" 8)))"#,
        &mut mock,
    )
    .unwrap();

    h.call_steel_cmd_by_name("try-set", vec![], &mut mock)
        .unwrap();

    assert_eq!(
        mock.settings.indent_width, 8,
        "indent-width must be applied"
    );
}

/// The budget stops a command whose loop never calls a builtin.
#[test]
fn call_steel_cmd_budget_stops_a_loop_that_calls_nothing() {
    let mut h = host();
    let mut mock = MockHost::new();
    h.eval_source(
        r#"(define-command! "count-up" "doc"
             (lambda () (let loop ((n 0)) (if (< n 50000000) (loop (+ n 1)) n))))
           (define-command! "quick" "doc" (lambda () (log! 'info "quick ran")))"#,
        &mut mock,
    )
    .unwrap();
    mock.settings.steel_command_budget_ms = 50;

    let start = std::time::Instant::now();
    let err = h
        .call_steel_cmd_by_name("count-up", vec![], &mut mock)
        .unwrap_err();

    assert!(
        err.message.contains("interrupted"),
        "expected 'interrupted', got: {err}"
    );
    assert!(
        start.elapsed() < std::time::Duration::from_secs(1),
        "the budget took {:?} to stop the loop",
        start.elapsed()
    );
    h.call_steel_cmd_by_name("quick", vec![], &mut mock)
        .expect("the next command runs once the stopped one has returned");
}

/// A command `"wait-then-log"` that waits on Steel's own `wait` for a child
/// sleeping well past a 50 ms budget, then logs `"after the wait"`.
const STEEL_WAIT_THEN_LOG: &str = r#"(require "steel/result")
(define-command! "wait-then-log" "doc"
  (lambda ()
    (wait (unwrap-ok (spawn-process (command "sleep" (list "0.3")))))
    (log! 'info "after the wait")))"#;

/// Runs `"wait-then-log"` under a 50 ms budget and asserts it ran past its
/// wait.
fn assert_wait_then_log_outlives_the_budget(h: &mut ScriptingHost, mock: &mut MockHost) {
    mock.settings.steel_command_budget_ms = 50;

    h.call_steel_cmd_by_name("wait-then-log", vec![], mock)
        .expect("a wait on a child must not use up the budget");
    assert!(
        logged(h).iter().any(|msg| msg == "after the wait"),
        "the command must run past its wait"
    );
}

/// Time spent waiting on a child process does not count against the budget.
#[test]
fn call_steel_cmd_budget_does_not_count_a_wait_on_a_child() {
    let mut h = host();
    let mut mock = MockHost::new();
    h.eval_source(
        r#"(define-command! "wait-then-log" "doc"
             (lambda ()
               (run-capture! "sleep" '("0.3"))
               (log! 'info "after the wait")))"#,
        &mut mock,
    )
    .unwrap();

    assert_wait_then_log_outlives_the_budget(&mut h, &mut mock);
}

/// Steel's own `wait` on a child does not count against the budget either.
#[test]
fn call_steel_cmd_budget_does_not_count_a_steel_wait() {
    let mut h = host();
    let mut mock = MockHost::new();
    h.eval_source(STEEL_WAIT_THEN_LOG, &mut mock).unwrap();

    assert_wait_then_log_outlives_the_budget(&mut h, &mut mock);
}

/// Steel's `wait->stdout` waits without counting, and still answers the
/// child's output.
#[test]
fn call_steel_cmd_budget_does_not_count_a_steel_wait_for_stdout() {
    let mut h = host();
    let mut mock = MockHost::new();
    h.eval_source(
        r#"(require "steel/result")
           (define-command! "capture" "doc"
             (lambda ()
               (log! 'info
                 (unwrap-ok
                   (wait->stdout
                     (unwrap-ok
                       (spawn-process
                         (with-stdout-piped (command "sh" (list "-c" "sleep 0.3; echo out"))))))))))"#,
        &mut mock,
    )
    .unwrap();
    mock.settings.steel_command_budget_ms = 50;

    h.call_steel_cmd_by_name("capture", vec![], &mut mock)
        .expect("a wait on a child must not use up the budget");
    assert_eq!(logged(&mut h), ["out\n"]);
}

/// A plugin file is compiled as its own module, so its `wait` resolves
/// through the prelude rather than the top level: it does not count either.
#[test]
fn call_steel_cmd_budget_does_not_count_a_steel_wait_in_a_required_module() {
    let tmp = tempfile::tempdir().unwrap();
    let module = tmp.path().join("waits.scm");
    std::fs::write(&module, STEEL_WAIT_THEN_LOG).unwrap();
    let mut h = host();
    let mut mock = MockHost::new();
    h.eval_source(
        &format!("(require {})", steel_path_literal(&module).unwrap()),
        &mut mock,
    )
    .unwrap();

    assert_wait_then_log_outlives_the_budget(&mut h, &mut mock);
}

/// Steel's `delete-directory!` on a large tree is waiting on the OS, which
/// the budget does not count.
#[test]
fn steel_delete_directory_does_not_count_the_delete_against_the_budget() {
    let tree = tempfile::tempdir().unwrap();
    let root = tree.path().join("tree");
    for d in 0..50 {
        let dir = root.join(format!("d{d}"));
        std::fs::create_dir_all(&dir).unwrap();
        for f in 0..100 {
            std::fs::write(dir.join(format!("f{f}")), "x").unwrap();
        }
    }
    let mut h = host();
    let mut mock = MockHost::new();

    let result = h.eval_source_watchdog(
        &format!("(delete-directory! {})", steel_path_literal(&root).unwrap()),
        std::time::Duration::from_millis(10),
        &mut mock,
    );

    assert!(result.is_ok(), "{result:?}");
    assert!(!root.exists(), "the tree must be gone");
}

/// `delete-directory!` raises on a missing directory, as Steel's own does.
#[test]
fn steel_delete_directory_raises_on_a_missing_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("missing");
    let mut h = host();
    let mut mock = MockHost::new();

    let result = h.eval_source(
        &format!(
            "(delete-directory! {})",
            steel_path_literal(&missing).unwrap()
        ),
        &mut mock,
    );

    assert!(result.is_err(), "a missing directory must raise");
}

// ── try-lock-file! / unlock-file! ────────────────────────────────────────

/// Whether another holder could take the lock on `path` right now.
fn lock_is_free(path: &std::path::Path) -> bool {
    let file = std::fs::File::open(path).expect("the lock file exists");
    match file.try_lock() {
        Ok(()) => true,
        Err(std::fs::TryLockError::WouldBlock) => false,
        Err(std::fs::TryLockError::Error(e)) => panic!("try_lock failed: {e}"),
    }
}

/// The messages `h` logged, in order, without their levels.
fn logged(h: &mut ScriptingHost) -> Vec<String> {
    h.take_pending_messages()
        .into_iter()
        .map(|(_, msg)| msg)
        .collect()
}

/// A lock the session still holds when the budget stops it is released with
/// the session, so nothing in this process keeps it.
#[test]
fn a_file_lock_is_released_when_the_budget_stops_its_session() {
    let tmp = tempfile::tempdir().unwrap();
    let lock = tmp.path().join("lock");
    let mut h = host();
    let mut mock = MockHost::new();
    define_lock_command(
        &mut h,
        &mut mock,
        "lock-and-spin",
        &lock,
        "(try-lock-file! path) (let loop () (loop))",
    );
    mock.settings.steel_command_budget_ms = 50;

    let err = h
        .call_steel_cmd_by_name("lock-and-spin", vec![], &mut mock)
        .unwrap_err();

    assert!(err.message.contains("interrupted"), "{err}");
    assert!(
        lock_is_free(&lock),
        "the stopped session must not keep the lock"
    );
}

/// Defines `name` as a command running `body` with `path` bound to the
/// string `lock`.
fn define_lock_command(
    h: &mut ScriptingHost,
    mock: &mut MockHost,
    name: &str,
    lock: &std::path::Path,
    body: &str,
) {
    h.eval_source(
        &format!(
            r#"(define-command! "{name}" "doc" (lambda () (let ((path {path:?})) {body})))"#,
            path = lock.to_str().expect("temp paths are UTF-8")
        ),
        mock,
    )
    .expect("the command defines");
}

/// A lock not released by `unlock-file!` is released when its session
/// returns.
#[test]
fn a_file_lock_is_released_when_its_session_returns() {
    let tmp = tempfile::tempdir().unwrap();
    let lock = tmp.path().join("lock");
    let mut h = host();
    let mut mock = MockHost::new();
    define_lock_command(
        &mut h,
        &mut mock,
        "take",
        &lock,
        r#"(log! 'info (if (try-lock-file! path) "taken" "held elsewhere"))"#,
    );

    h.call_steel_cmd_by_name("take", vec![], &mut mock).unwrap();

    assert_eq!(logged(&mut h), ["taken"]);
    assert!(lock_is_free(&lock), "the lock must not outlive its session");
}

/// A lock another holder has answers `#f` and leaves that holder's lock in
/// place.
#[test]
fn a_file_lock_another_holder_has_answers_false() {
    let tmp = tempfile::tempdir().unwrap();
    let lock = tmp.path().join("lock");
    let other = std::fs::File::create(&lock).unwrap();
    other.try_lock().unwrap();
    let mut h = host();
    let mut mock = MockHost::new();
    define_lock_command(
        &mut h,
        &mut mock,
        "take",
        &lock,
        r#"(log! 'info (if (try-lock-file! path) "taken" "held elsewhere"))"#,
    );

    h.call_steel_cmd_by_name("take", vec![], &mut mock).unwrap();

    assert_eq!(logged(&mut h), ["held elsewhere"]);
    other.unlock().unwrap();
}

/// `unlock-file!` releases the lock at once: a fresh take in the same
/// session gets it.
#[test]
fn unlock_file_releases_the_lock_within_the_session() {
    let tmp = tempfile::tempdir().unwrap();
    let lock = tmp.path().join("lock");
    let mut h = host();
    let mut mock = MockHost::new();
    define_lock_command(
        &mut h,
        &mut mock,
        "retake",
        &lock,
        r#"(try-lock-file! path) (unlock-file! path) (log! 'info (if (try-lock-file! path) "taken" "held elsewhere"))"#,
    );

    h.call_steel_cmd_by_name("retake", vec![], &mut mock)
        .unwrap();

    assert_eq!(logged(&mut h), ["taken"]);
}

/// Taking a lock the session already holds is an error, not a second hold.
#[test]
fn taking_a_held_file_lock_again_raises() {
    let tmp = tempfile::tempdir().unwrap();
    let lock = tmp.path().join("lock");
    let mut h = host();
    let mut mock = MockHost::new();
    define_lock_command(
        &mut h,
        &mut mock,
        "twice",
        &lock,
        "(try-lock-file! path) (try-lock-file! path)",
    );

    let err = h
        .call_steel_cmd_by_name("twice", vec![], &mut mock)
        .unwrap_err();

    assert!(err.message.contains("already holds"), "{err}");
}

/// Releasing a lock the session does not hold is an error.
#[test]
fn unlocking_a_file_lock_not_held_raises() {
    let tmp = tempfile::tempdir().unwrap();
    let lock = tmp.path().join("lock");
    let mut h = host();
    let mut mock = MockHost::new();
    define_lock_command(&mut h, &mut mock, "release", &lock, "(unlock-file! path)");

    let err = h
        .call_steel_cmd_by_name("release", vec![], &mut mock)
        .unwrap_err();

    assert!(err.message.contains("does not hold"), "{err}");
}
