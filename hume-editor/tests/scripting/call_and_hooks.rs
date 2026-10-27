//! `call!` and `register-hook!`/`fire_hook` tests.

use super::*;

/// A mock whose native commands are `names`, so a `call!` to one is
/// recorded in `dispatched_native`.
fn mock_with_natives(names: &[&str]) -> MockHost {
    let mut mock = MockHost::new();
    mock.native_names = names.iter().copied().collect();
    mock
}

/// The native commands `mock` was asked to run, in order.
fn dispatched(mock: &MockHost) -> Vec<&str> {
    mock.dispatched_native
        .iter()
        .map(|(name, _, _)| name.as_str())
        .collect()
}

// ── call! ─────────────────────────────────────────────────────────────────

/// `call_steel_cmd` forwards positional args by value into the invoked
/// lambda (direct `%dispatch-command!` function call). The expected dispatch
/// name is derived from the input arg, not from re-reading the
/// implementation.
#[test]
fn call_bang_passes_args_to_command() {
    use steel::rvals::SteelVal;
    let mut h = host();
    let mut mock = mock_with_natives(&["hello"]);

    // Define a command that takes one arg x and calls (call! x).
    // The lambda receives x positionally, then dispatches x as a command name.
    h.eval_source(
        r#"(define-command! "echo-arg" "" (lambda (x) (call! x (focused-pane))))"#,
        &mut mock,
    )
    .unwrap();

    h.call_steel_cmd_by_name(
        "echo-arg",
        vec![SteelVal::StringV("hello".into())],
        &mut mock,
    )
    .expect("call should succeed");

    assert_eq!(
        dispatched(&mock),
        ["hello"],
        "call! must dispatch arg value as command name"
    );
}

/// `call!` finds a native command in any case, and dispatches it under the
/// name as written.
#[test]
fn call_bang_finds_a_native_command_in_another_case() {
    let mut h = host();
    let mut mock = MockHost::new();
    h.eval_source(
        r#"(define-command! "yank-loud" "" (lambda () (call! "YANK" (focused-pane))))"#,
        &mut mock,
    )
    .unwrap();
    mock.native_names = ["yank"].into_iter().collect();

    h.call_steel_cmd_by_name("yank-loud", vec![], &mut mock)
        .unwrap();

    let names: Vec<&str> = mock
        .dispatched_native
        .iter()
        .map(|(name, _, _)| name.as_str())
        .collect();
    assert_eq!(names, ["YANK"]);
}

#[test]
fn call_bang_forwards_multiple_args_to_lambda() {
    // Tests multi-arg forwarding beyond the first positional arg.
    // Each dispatched command name equals the corresponding input arg.
    use steel::rvals::SteelVal;
    let mut h = host();
    let mut mock = mock_with_natives(&["x", "y", "z"]);

    h.eval_source(
        r#"(define-command! "route-three" "" (lambda (a b c)
             (call! a (focused-pane)) (call! b (focused-pane)) (call! c (focused-pane))))"#,
        &mut mock,
    )
    .unwrap();

    h.call_steel_cmd_by_name(
        "route-three",
        vec![
            SteelVal::StringV("x".into()),
            SteelVal::StringV("y".into()),
            SteelVal::StringV("z".into()),
        ],
        &mut mock,
    )
    .expect("call should succeed");

    assert_eq!(
        dispatched(&mock),
        ["x", "y", "z"],
        "each arg must reach the corresponding lambda parameter"
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
        .call_steel_cmd_by_name(
            "needs-two",
            vec![SteelVal::StringV("only-one".into())],
            &mut mock,
        )
        .unwrap_err();

    assert!(
        !err.message.is_empty(),
        "expected a Steel arity error, got empty string"
    );
}

// ── register-hook! / fire_hook ────────────────────────────────────────────

use hume_core::PaneHandle;
use hume_scripting::SteelPane;
use steel::rvals::SteelVal;

#[test]
fn register_hook_fires_on_buffer_open() {
    let mut h = host();
    let mut mock = mock_with_natives(&["move-right"]);

    h.eval_source(
        r#"(register-hook! 'on-buffer-open (lambda (bid) (call! "move-right" bid)))"#,
        &mut mock,
    )
    .unwrap();
    let bid = BufferId::default();
    let val = SteelPane::new(PaneHandle::buffer_only(bid)).into_steel_val();
    h.fire_hook("on-buffer-open", None, &[val], &mut mock)
        .unwrap();
    assert_eq!(
        dispatched(&mock),
        ["move-right"],
        "hook handler must have dispatched move-right"
    );
}

#[test]
fn register_hook_fires_on_buffer_close() {
    let mut h = host();
    let mut mock = mock_with_natives(&["move-left"]);

    h.eval_source(
        r#"(register-hook! 'on-buffer-close (lambda (bid) (call! "move-left" bid)))"#,
        &mut mock,
    )
    .unwrap();
    let bid = BufferId::default();
    let val = SteelPane::new(PaneHandle::buffer_only(bid)).into_steel_val();
    h.fire_hook("on-buffer-close", None, &[val], &mut mock)
        .unwrap();
    assert_eq!(
        dispatched(&mock),
        ["move-left"],
        "hook handler must have dispatched move-left"
    );
}

#[test]
fn register_hook_fires_on_buffer_save() {
    let mut h = host();
    let mut mock = mock_with_natives(&["move-right"]);

    h.eval_source(
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right" bid)))"#,
        &mut mock,
    )
    .unwrap();
    let bid = BufferId::default();
    let val = SteelPane::new(PaneHandle::buffer_only(bid)).into_steel_val();
    h.fire_hook("on-buffer-save", None, &[val], &mut mock)
        .unwrap();
    assert_eq!(
        dispatched(&mock),
        ["move-right"],
        "hook handler must have dispatched move-right"
    );
}

#[test]
fn register_hook_fires_on_mode_change() {
    let mut h = host();
    let mut mock = mock_with_natives(&["move-right"]);

    h.eval_source(
        r#"(register-hook! 'on-mode-change
              (lambda (old new)
                (when (equal? new 'insert) (call! "move-right" (focused-pane)))))"#,
        &mut mock,
    )
    .unwrap();
    let old_val = SteelVal::SymbolV("normal".into());
    let new_val = SteelVal::SymbolV("insert".into());
    h.fire_hook("on-mode-change", None, &[old_val, new_val], &mut mock)
        .unwrap();
    assert_eq!(
        dispatched(&mock),
        ["move-right"],
        "hook handler must have dispatched move-right"
    );
}

#[test]
fn register_hook_no_fire_if_no_handlers() {
    let mut h = host();
    let mut mock = MockHost::new();

    // No handlers registered: fire_hook must succeed without dispatching anything.
    h.fire_hook("on-buffer-open", None, &[], &mut mock).unwrap();

    // Proves no native dispatch occurred (would have been recorded in dispatched_native).
    assert!(
        mock.dispatched_native.is_empty(),
        "no handlers → fire_hook must not dispatch any native commands"
    );
}

#[test]
fn register_hook_multiple_handlers_all_fire() {
    let mut h = host();
    let mut mock = mock_with_natives(&["move-right", "move-left"]);

    h.eval_source(
        r#"
(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right" bid)))
(register-hook! 'on-buffer-save (lambda (bid) (call! "move-left" bid)))
"#,
        &mut mock,
    )
    .unwrap();
    let bid = BufferId::default();
    let val = SteelPane::new(PaneHandle::buffer_only(bid)).into_steel_val();
    h.fire_hook("on-buffer-save", None, &[val], &mut mock)
        .unwrap();
    assert_eq!(
        dispatched(&mock),
        ["move-right", "move-left"],
        "both handlers must have fired"
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
        .call_steel_cmd_by_name("bad-cmd", vec![], &mut mock)
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
    // Each fire must see exactly its own args; stale values from a prior
    // fire (e.g. Arc references to a closed buffer) must never leak into a
    // subsequent fire with different args.
    let mut h = host();
    let mut mock = mock_with_natives(&["insert", "normal"]);

    // Handler reads arg 1 (new mode) and dispatches it as a command name.
    h.eval_source(
        r#"(register-hook! 'on-mode-change (lambda (old new) (call! (symbol->string new) (focused-pane))))"#,
        &mut mock,
    )
    .unwrap();
    let old_val = SteelVal::SymbolV("normal".into());
    let new_val = SteelVal::SymbolV("insert".into());
    h.fire_hook(
        "on-mode-change",
        None,
        &[old_val.clone(), new_val],
        &mut mock,
    )
    .unwrap();
    assert_eq!(
        dispatched(&mock),
        ["insert"],
        "first fire must dispatch 'insert'"
    );

    // Second fire with different args: any stale first-fire arg would give a wrong result.
    let new_val2 = SteelVal::SymbolV("normal".into());
    h.fire_hook("on-mode-change", None, &[old_val, new_val2], &mut mock)
        .unwrap();
    assert_eq!(
        dispatched(&mock),
        ["insert", "normal"],
        "second fire must dispatch 'normal', not a stale 'insert' from the first"
    );
}
