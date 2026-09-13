//! `call!` and `register-hook!`/`fire_hook` tests.

use super::*;

// ── call! ─────────────────────────────────────────────────────────────────

/// `call_steel_cmd` forwards positional args by value into the invoked
/// lambda (direct `%dispatch-command` function call).  Independent oracle:
/// the expected dispatch name is derived from the input arg, not from
/// re-reading the implementation.
///
/// Verification validity: changing "hello" in the assert to "world" makes the test fail.
#[test]
fn call_bang_passes_args_to_command() {
    use steel::rvals::SteelVal;
    let mut h = host();
    let mut mock = MockHost::new();

    // Define a command that takes one arg x and calls (call! x).
    // The lambda receives x positionally, then dispatches x as a command name.
    h.eval_source(
        r#"(define-command! "echo-arg" "" (lambda (x) (call! x)))"#,
        &mut mock,
    )
    .unwrap();

    h.call_steel_cmd(
        "echo-arg",
        None,
        vec![SteelVal::StringV("hello".into())],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    )
    .expect("call should succeed");

    let msgs = h.take_pending_messages();
    assert!(
        msgs.iter().any(|(_, m)| m.contains("hello")),
        "call! must dispatch arg value as command name; got: {:?}",
        msgs
    );
}

#[test]
fn call_bang_forwards_multiple_args_to_lambda() {
    // Tests multi-arg forwarding beyond the first positional arg.
    // Oracle: each dispatched command name equals the corresponding input arg.
    // Verification: change "z" to "w" in the assert → test fails.
    use steel::rvals::SteelVal;
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "route-three" "" (lambda (a b c) (call! a) (call! b) (call! c)))"#,
        &mut mock,
    )
    .unwrap();

    h.call_steel_cmd(
        "route-three",
        None,
        vec![
            SteelVal::StringV("x".into()),
            SteelVal::StringV("y".into()),
            SteelVal::StringV("z".into()),
        ],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    )
    .expect("call should succeed");

    let msgs = h.take_pending_messages();
    let warned: Vec<&str> = msgs
        .iter()
        .filter_map(|(_, m)| {
            m.strip_prefix('\'')
                .and_then(|s| s.strip_suffix("' is not a native command"))
        })
        .collect();
    assert_eq!(
        warned,
        vec!["x", "y", "z"],
        "each arg must reach the corresponding lambda parameter; got: {:?}",
        msgs
    );
}

#[test]
fn call_bang_arity_mismatch_surfaces_steel_error() {
    // Passing fewer args than the lambda declares is a Steel VM arity error.
    // Verifies call_steel_cmd propagates it rather than silently misbehaving.
    use steel::rvals::SteelVal;
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "needs-two" "" (lambda (a b) (call! "move-right")))"#,
        &mut mock,
    )
    .unwrap();

    let err = h
        .call_steel_cmd(
            "needs-two",
            None,
            vec![SteelVal::StringV("only-one".into())],
            PaneId::default(),
            BufferId::default(),
            &mut mock,
        )
        .unwrap_err();

    assert!(
        !err.message.is_empty(),
        "expected a Steel arity error, got empty string"
    );
}

// ── register-hook! / fire_hook ────────────────────────────────────────────

use hume_scripting::SteelBufferId;

#[test]
fn register_hook_fires_on_buffer_open() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(register-hook! 'on-buffer-open (lambda (bid) (call! "move-right")))"#,
        &mut mock,
    )
    .unwrap();
    let bid = BufferId::default();
    let val = SteelBufferId::new(bid).into_steel_val();
    h.fire_hook("on-buffer-open", &[val], PaneId::default(), bid, &mut mock)
        .unwrap();
    let msgs = h.take_pending_messages();
    assert!(
        msgs.iter().any(|(_, m)| m.contains("move-right")),
        "hook handler must have dispatched move-right; got: {:?}",
        msgs
    );
}

#[test]
fn register_hook_fires_on_buffer_close() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(register-hook! 'on-buffer-close (lambda (bid) (call! "move-left")))"#,
        &mut mock,
    )
    .unwrap();
    let bid = BufferId::default();
    let val = SteelBufferId::new(bid).into_steel_val();
    h.fire_hook("on-buffer-close", &[val], PaneId::default(), bid, &mut mock)
        .unwrap();
    let msgs = h.take_pending_messages();
    assert!(
        msgs.iter().any(|(_, m)| m.contains("move-left")),
        "hook handler must have dispatched move-left; got: {:?}",
        msgs
    );
}

#[test]
fn register_hook_fires_on_buffer_save() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right")))"#,
        &mut mock,
    )
    .unwrap();
    let bid = BufferId::default();
    let val = SteelBufferId::new(bid).into_steel_val();
    h.fire_hook("on-buffer-save", &[val], PaneId::default(), bid, &mut mock)
        .unwrap();
    let msgs = h.take_pending_messages();
    assert!(
        msgs.iter().any(|(_, m)| m.contains("move-right")),
        "hook handler must have dispatched move-right; got: {:?}",
        msgs
    );
}

#[test]
fn register_hook_fires_on_mode_change() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(register-hook! 'on-mode-change
              (lambda (old new)
                (when (equal? new "insert") (call! "move-right"))))"#,
        &mut mock,
    )
    .unwrap();
    use steel::rvals::IntoSteelVal as _;
    let old_val = "normal".into_steelval().unwrap();
    let new_val = "insert".into_steelval().unwrap();
    h.fire_hook(
        "on-mode-change",
        &[old_val, new_val],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    )
    .unwrap();
    let msgs = h.take_pending_messages();
    assert!(
        msgs.iter().any(|(_, m)| m.contains("move-right")),
        "hook handler must have dispatched move-right; got: {:?}",
        msgs
    );
}

#[test]
fn register_hook_no_fire_if_no_handlers() {
    let mut h = host();
    let mut mock = MockHost::new();

    // No handlers registered — fire_hook must succeed without dispatching anything.
    h.fire_hook(
        "on-buffer-open",
        &[],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    )
    .unwrap();

    // Proves no native dispatch occurred (would have been recorded in dispatched_native).
    assert!(
        mock.dispatched_native.is_empty(),
        "no handlers → fire_hook must not dispatch any native commands"
    );
}

#[test]
fn register_hook_multiple_handlers_all_fire() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"
(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right")))
(register-hook! 'on-buffer-save (lambda (bid) (call! "move-left")))
"#,
        &mut mock,
    )
    .unwrap();
    let bid = BufferId::default();
    let val = SteelBufferId::new(bid).into_steel_val();
    h.fire_hook("on-buffer-save", &[val], PaneId::default(), bid, &mut mock)
        .unwrap();
    let msgs = h.take_pending_messages();
    let warned: Vec<&str> = msgs
        .iter()
        .filter_map(|(_, m)| {
            m.strip_prefix('\'')
                .and_then(|s| s.strip_suffix("' is not a native command"))
        })
        .collect();
    assert_eq!(
        warned,
        vec!["move-right", "move-left"],
        "both handlers must have fired; got: {:?}",
        msgs
    );
}

#[test]
fn register_hook_errors_in_command_mode() {
    let mut h = host();
    let mut mock = MockHost::new();

    // Define a command that tries to register a hook (not allowed in command mode).
    h.eval_source(
        r#"(define-command! "bad-cmd" "" (lambda ()
             (register-hook! 'on-buffer-open (lambda (bid) #f))))"#,
        &mut mock,
    )
    .unwrap();
    let err = h
        .call_steel_cmd(
            "bad-cmd",
            None,
            vec![],
            PaneId::default(),
            BufferId::default(),
            &mut mock,
        )
        .unwrap_err();
    assert!(
        err.message
            .contains("only valid during init.scm or plugin load"),
        "got: {err}"
    );
}

#[test]
fn register_hook_unknown_name_errors() {
    let mut h = host();
    let mut mock = MockHost::new();

    let err = h
        .eval_source(
            r#"(register-hook! 'on-nonexistent (lambda () #f))"#,
            &mut mock,
        )
        .unwrap_err();
    assert!(err.contains("unknown event"), "got: {err}");
}

#[test]
fn fire_hook_globals_cleared_between_fires() {
    // Each fire must see exactly its own args — stale values from a prior
    // fire (e.g. Arc references to a closed buffer) must never leak into a
    // subsequent fire with different args.
    let mut h = host();
    let mut mock = MockHost::new();

    // Handler reads arg 1 (new mode) and dispatches it as a command name.
    h.eval_source(
        r#"(register-hook! 'on-mode-change (lambda (old new) (call! new)))"#,
        &mut mock,
    )
    .unwrap();
    use steel::rvals::IntoSteelVal as _;
    let old_val = "normal".into_steelval().unwrap();
    let new_val = "insert".into_steelval().unwrap();
    h.fire_hook(
        "on-mode-change",
        &[old_val.clone(), new_val],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    )
    .unwrap();
    let msgs1 = h.take_pending_messages();
    assert!(
        msgs1.iter().any(|(_, m)| m.contains("insert")),
        "first fire must dispatch 'insert'; got: {:?}",
        msgs1
    );

    // Second fire with different args — any stale first-fire arg would give a wrong result.
    let new_val2 = "normal".into_steelval().unwrap();
    h.fire_hook(
        "on-mode-change",
        &[old_val, new_val2],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    )
    .unwrap();
    let msgs2 = h.take_pending_messages();
    assert!(
        msgs2.iter().any(|(_, m)| m.contains("normal")),
        "second fire must dispatch 'normal'; got: {:?}",
        msgs2
    );
    assert!(
        !msgs2.iter().any(|(_, m)| m.contains("insert")),
        "second fire must NOT see stale 'insert' from first; got: {:?}",
        msgs2
    );
}
