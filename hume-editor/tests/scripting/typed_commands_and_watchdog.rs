//! `define-typed-command!` and `EvalWatchdog` tests.

use super::*;

// ── define-typed-command! ────────────────────────────────────────────────

#[test]
fn define_typed_command_registers_into_typed_table() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-typed-command! "greet" "doc" (lambda (arg) (+ 1 0)))"#,
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

/// `define-typed-command!` has no `#:repeatable` keyword — dot-repeat has
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
/// `define-typed-command!` sets it to `false` — the typed counterpart of
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
        inline.inline_output,
        "#:inline-output #t should set inline_output = true"
    );
    assert!(
        !plain.inline_output,
        "plain define-typed-command! should set inline_output = false"
    );
}

/// A name already claimed by `define-command!` (mappable) collides with
/// `define-typed-command!` for the same name, and vice versa — the two
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

/// The reverse direction of the collision above — asserted in that test's
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
    let flag = Arc::new(AtomicBool::new(false));
    let budget = std::time::Duration::from_secs(10);
    let start = std::time::Instant::now();
    let watchdog = EvalWatchdog::new();
    watchdog.arm(Arc::clone(&flag), budget);
    watchdog.cancel();
    // cancel() must return well within the budget; 500 ms is generous.
    assert!(
        start.elapsed() < std::time::Duration::from_millis(500),
        "cancel() took too long: {:?}",
        start.elapsed()
    );
    // Flag must not have been set (we cancelled before it fired).
    assert!(
        !flag.load(Ordering::Relaxed),
        "flag must stay false after cancel"
    );
}

/// A watchdog with a tiny budget fires and causes (hume/yield!) to abort.
#[test]
fn eval_source_watchdog_aborts_runaway() {
    let mut h = host();
    let mut mock = MockHost::new();

    let budget = std::time::Duration::from_millis(50);
    let start = std::time::Instant::now();

    let err = h
        .eval_source_watchdog(
            // This loop would run forever without the watchdog.
            "(let loop () (hume/yield!) (loop))",
            budget,
            &mut mock,
        )
        .unwrap_err();

    assert!(
        err.contains("interrupted"),
        "expected 'interrupted' in error, got: {err}"
    );
    // Must abort well within a second — if not, the watchdog didn't fire.
    assert!(
        start.elapsed() < std::time::Duration::from_secs(1),
        "eval took too long: {:?}",
        start.elapsed()
    );
    // Flag must be reset after eval_source_watchdog returns.
    assert!(
        !h.interrupt_flag_for_test().load(Ordering::Relaxed),
        "interrupt_flag must be false after eval returns"
    );
}

/// call_steel_cmd watchdog fires and aborts a runaway Steel command.
#[test]
fn call_steel_cmd_watchdog_aborts_runaway() {
    let mut h = host();
    let mut mock = MockHost::new();

    // Register a command whose body loops forever.
    h.eval_source(
        r#"(define-command! "spin" "spin forever" (lambda () (let loop () (hume/yield!) (loop))))"#,
        &mut mock,
    )
    .unwrap();
    let cmd_name = "spin".to_string();

    // Use a tight command budget.
    mock.settings.steel_command_budget_ms = 50;

    let start = std::time::Instant::now();
    let err = h
        .call_steel_cmd(
            &cmd_name,
            None,
            vec![],
            PaneId::default(),
            BufferId::default(),
            &mut mock,
        )
        .unwrap_err();

    assert!(
        err.message.contains("interrupted"),
        "expected 'interrupted', got: {err}"
    );
    assert!(
        start.elapsed() < std::time::Duration::from_secs(1),
        "call_steel_cmd took too long: {:?}",
        start.elapsed()
    );
    assert!(
        !h.interrupt_flag_for_test().load(Ordering::Relaxed),
        "interrupt_flag must be false after call_steel_cmd returns"
    );
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
        r#"(define-command! "looper" "loop" (lambda () (let loop () (hume/yield!) (loop))))"#,
        &mut mock,
    )
    .unwrap();
    let cmd_name = "looper".to_string();

    assert_eq!(mock.settings.tab_width, 4, "precondition");
    mock.settings.steel_command_budget_ms = 50;

    let err = h
        .call_steel_cmd(
            &cmd_name,
            None,
            vec![],
            PaneId::default(),
            BufferId::default(),
            &mut mock,
        )
        .unwrap_err();

    assert!(
        err.message.contains("interrupted"),
        "expected 'interrupted', got: {err}"
    );
    assert_eq!(
        mock.settings.tab_width, 4,
        "tab-width must be unchanged after interrupt"
    );
}

/// `set-option!` is registered `open` (no eval-mode gate) — calling it from
/// a Steel command body (`call_steel_cmd` runs with `EvalMode::Command`)
/// must actually apply the setting, not raise a gate error. A plugin-defined
/// command can now toggle a global setting at runtime — the gap this closes.
#[test]
fn call_steel_cmd_set_option_from_body_applies_the_setting() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "try-set" "" (lambda () (set-option! "tab-width" 8)))"#,
        &mut mock,
    )
    .unwrap();

    h.call_steel_cmd(
        "try-set",
        None,
        vec![],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    )
    .unwrap();

    assert_eq!(mock.settings.tab_width, 8, "tab-width must be applied");
}
