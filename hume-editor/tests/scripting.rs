// Alias so mock_host.rs (included below via #[path]) can keep its `hume::` paths.
extern crate hume_editor as hume;

#[path = "../src/testing/mock_host.rs"]
mod mock_host;

use hume_engine::pipeline::{BufferId, PaneId};
use hume_scripting::EvalWatchdog;
use hume_scripting::*;
use mock_host::MockHost;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn host() -> ScriptingHost {
    ScriptingHost::new()
}

// ── set-option! ───────────────────────────────────────────────────────────

#[test]
fn set_option_tab_width_integer() {
    let mut h = host();
    let mut mock = MockHost::new();

    assert_eq!(mock.settings.tab_width, 4);
    h.eval_source("(set-option! \"tab-width\" 2)", &mut mock)
        .unwrap();
    assert_eq!(mock.settings.tab_width, 2);
}

#[test]
fn set_option_tab_width_string() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source("(set-option! \"tab-width\" \"8\")", &mut mock)
        .unwrap();
    assert_eq!(mock.settings.tab_width, 8);
}

#[test]
fn set_option_bool_as_bool() {
    let mut h = host();
    let mut mock = MockHost::new();

    assert!(mock.settings.mouse_enabled);
    h.eval_source("(set-option! \"mouse-enabled\" #f)", &mut mock)
        .unwrap();
    assert!(!mock.settings.mouse_enabled);
}

#[test]
fn set_option_unknown_key_errors() {
    let mut h = host();
    let mut mock = MockHost::new();

    let err = h
        .eval_source("(set-option! \"nonexistent\" \"val\")", &mut mock)
        .unwrap_err();
    assert!(err.contains("unknown setting"), "got: {err}");
}

// ── get-option ────────────────────────────────────────────────────────────

/// `eval_source` runs top-level code as an init eval — `get-option` is
/// command-mode only, so a bare top-level call must error the same way
/// `current-buffer` or any other command-mode read would.
#[test]
fn get_option_blocked_during_init_eval() {
    let mut h = host();
    let mut mock = MockHost::new();

    let err = h
        .eval_source("(get-option \"tab-width\")", &mut mock)
        .unwrap_err();
    assert!(err.contains("init"), "got: {err}");
}

#[test]
fn get_option_reads_back_tab_width_as_int() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "check" "" (lambda ()
             (unless (equal? (get-option "tab-width") 4)
               (error "unexpected tab-width"))))"#,
        &mut mock,
    )
    .unwrap();
    h.call_steel_cmd("check", None, vec![], PaneId::default(), BufferId::default(), &mut mock)
        .expect("get-option must read back the default tab-width as an int");
}

#[test]
fn get_option_reads_back_tab_style_as_string() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "check" "" (lambda ()
             (unless (equal? (get-option "tab-style") "hard")
               (error "unexpected tab-style"))))"#,
        &mut mock,
    )
    .unwrap();
    h.call_steel_cmd("check", None, vec![], PaneId::default(), BufferId::default(), &mut mock)
        .expect("get-option must read back the default tab-style as a string");
}

#[test]
fn get_option_reads_back_lsp_inlay_hints_as_bool() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "check" "" (lambda ()
             (unless (equal? (get-option "lsp.inlay-hints") #f)
               (error "unexpected lsp.inlay-hints"))))"#,
        &mut mock,
    )
    .unwrap();
    h.call_steel_cmd("check", None, vec![], PaneId::default(), BufferId::default(), &mut mock)
        .expect("get-option must read back the default lsp.inlay-hints (false, B10d) as a bool");
}

#[test]
fn get_option_unknown_key_errors() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "check" "" (lambda () (get-option "nonexistent")))"#,
        &mut mock,
    )
    .unwrap();
    let err = h
        .call_steel_cmd("check", None, vec![], PaneId::default(), BufferId::default(), &mut mock)
        .unwrap_err();
    assert!(err.contains("unknown setting"), "got: {err}");
}

// ── bind-key! ─────────────────────────────────────────────────────────────

#[test]
fn bind_key_does_not_error_on_valid_input() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source("(bind-key! 'normal \"z\" \"move-right\")", &mut mock)
        .unwrap();

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use hume_editor::KeymapBindMode as BindMode;
    let z_key = &[KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE)];
    let (name, _) = mock
        .keymap
        .lookup_command(BindMode::Normal, z_key)
        .expect("bind-key! must bind 'z' in the keymap");
    assert_eq!(name, "move-right", "z must be bound to move-right");
}

#[test]
fn bind_key_multi_key_sequence_no_error() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source("(bind-key! 'normal \"g h\" \"move-right\")", &mut mock)
        .unwrap();

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use hume_editor::KeymapBindMode as BindMode;
    let g_key = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE);
    let h_key = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE);
    let (name, _) = mock
        .keymap
        .lookup_command(BindMode::Normal, &[g_key, h_key])
        .expect("bind-key! must bind the 'g h' sequence");
    assert_eq!(name, "move-right", "g h must be bound to move-right");
}

#[test]
fn bind_key_invalid_mode_errors() {
    let mut h = host();
    let mut mock = MockHost::new();

    let err = h
        .eval_source("(bind-key! 'visual \"f\" \"cmd\")", &mut mock)
        .unwrap_err();
    assert!(err.contains("mode"), "got: {err}");
}

#[test]
fn bind_key_invalid_key_sequence_errors() {
    let mut h = host();
    let mut mock = MockHost::new();

    let err = h
        .eval_source("(bind-key! 'normal \"boguskey\" \"cmd\")", &mut mock)
        .unwrap_err();
    assert!(!err.is_empty(), "expected error for unknown key 'boguskey'");
}

// ── load-plugin path resolution ────────────────────────────────────────────

#[test]
fn load_plugin_missing_plugin_declared_not_loaded() {
    let mut h = host();
    let mut mock = MockHost::new();

    // Eval #1: declare an absent plugin.
    h.eval_source("(load-plugin \"user/nonexistent-repo\")", &mut mock)
        .unwrap();

    // Persistence check: the host field should contain the declared name even
    // before eval #2 (direct, independent oracle).
    assert!(
        h.declared_plugins()
            .iter()
            .any(|d| d.eq_ignore_ascii_case("user/nonexistent-repo")),
        "declared_plugins field does not contain the declared name: {:?}",
        h.declared_plugins(),
    );

    // Eval #2 (separate eval, mimicking PLUM command-time read): verify the
    // (declared-plugins) builtin sees persisted data across the eval boundary.
    h.eval_source(
        r#"(if (member "user/nonexistent-repo" (declared-plugins))
               (log! 'info "PERSISTED")
               (log! 'info "MISSING"))"#,
        &mut mock,
    )
    .unwrap();
    assert!(
        h.peek_pending_messages()
            .iter()
            .any(|(_, msg)| msg == "PERSISTED"),
        "(declared-plugins) did not see persisted name across eval boundary; messages: {:?}",
        h.peek_pending_messages(),
    );
}

#[test]
fn load_plugin_malformed_name_errors() {
    let mut h = host();
    let mut mock = MockHost::new();

    let err = h
        .eval_source("(load-plugin \"just-a-name\")", &mut mock)
        .unwrap_err();
    assert!(!err.is_empty(), "expected error for malformed plugin name");
}

// ── configure-statusline! ─────────────────────────────────────────────────

#[test]
fn configure_statusline_sets_left_section() {
    use hume_editor::ui::statusline::StatusElement;
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(configure-statusline! '("Mode" "FileName") '() '("Position"))"#,
        &mut mock,
    )
    .unwrap();

    assert_eq!(
        mock.settings.statusline.left,
        vec![StatusElement::Mode, StatusElement::FileName]
    );
    assert_eq!(mock.settings.statusline.center, vec![]);
    assert_eq!(
        mock.settings.statusline.right,
        vec![StatusElement::Position]
    );
}

#[test]
fn configure_statusline_all_sections() {
    use hume_editor::ui::statusline::StatusElement;
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(configure-statusline!
             '("Position" "FileName" "DirtyIndicator")
             '("SearchMatches")
             '("Separator" "Mode"))"#,
        &mut mock,
    )
    .unwrap();

    assert_eq!(
        mock.settings.statusline.left,
        vec![
            StatusElement::Position,
            StatusElement::FileName,
            StatusElement::DirtyIndicator
        ]
    );
    assert_eq!(
        mock.settings.statusline.center,
        vec![StatusElement::SearchMatches]
    );
    assert_eq!(
        mock.settings.statusline.right,
        vec![StatusElement::Separator, StatusElement::Mode]
    );
}

#[test]
fn configure_statusline_empty_sections() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source("(configure-statusline! '() '() '())", &mut mock)
        .unwrap();

    assert!(mock.settings.statusline.left.is_empty());
    assert!(mock.settings.statusline.center.is_empty());
    assert!(mock.settings.statusline.right.is_empty());
}

#[test]
fn configure_statusline_unknown_element_errors() {
    let mut h = host();
    let mut mock = MockHost::new();

    let err = h
        .eval_source(
            r#"(configure-statusline! '("NotAnElement") '() '())"#,
            &mut mock,
        )
        .unwrap_err();
    assert!(err.contains("NotAnElement"), "got: {err}");
}

#[test]
fn configure_statusline_new_elements() {
    use hume_editor::ui::statusline::StatusElement;
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(configure-statusline! '("LineEnding") '() '("Cwd"))"#,
        &mut mock,
    )
    .unwrap();

    assert_eq!(
        mock.settings.statusline.left,
        vec![StatusElement::LineEnding]
    );
    assert_eq!(mock.settings.statusline.center, vec![]);
    assert_eq!(mock.settings.statusline.right, vec![StatusElement::Cwd]);
}

#[test]
fn configure_statusline_wrong_arity_errors() {
    let mut h = host();
    let mut mock = MockHost::new();

    let err = h
        .eval_source("(configure-statusline! '())", &mut mock)
        .unwrap_err();
    assert!(!err.is_empty(), "expected arity error");
}

// ── hume/yield! ───────────────────────────────────────────────────────────

#[test]
fn hume_yield_no_interrupt_is_noop() {
    let mut h = host();
    let mut mock = MockHost::new();

    // With no interrupt flag set, (hume/yield!) is a transparent no-op.
    h.eval_source("(hume/yield!)", &mut mock).unwrap();
}

#[test]
fn hume_yield_with_interrupt_errors() {
    let mut h = host();
    let mut mock = MockHost::new();

    // Pre-set the interrupt flag before the eval.
    h.interrupt_flag_for_test().store(true, Ordering::Relaxed);
    let err = h.eval_source("(hume/yield!)", &mut mock).unwrap_err();
    assert!(
        err.contains("interrupted"),
        "expected 'interrupted' in error, got: {err}"
    );

    // eval_source resets the flag after every call.
    assert!(
        !h.interrupt_flag_for_test().load(Ordering::Relaxed),
        "flag should be false after eval"
    );
}

#[test]
fn hume_yield_stops_loop_when_interrupted() {
    let mut h = host();
    let mut mock = MockHost::new();

    // Pre-set so the loop aborts on the very first yield call.
    h.interrupt_flag_for_test().store(true, Ordering::Relaxed);
    let err = h
        .eval_source(
            // Without the interrupt flag this loop would run forever.
            "(let loop () (hume/yield!) (loop))",
            &mut mock,
        )
        .unwrap_err();
    assert!(err.contains("interrupted"), "got: {err}");
}

#[test]
fn interrupt_flag_reset_after_eval() {
    let mut h = host();
    let mut mock = MockHost::new();

    // Pre-set the flag; after eval_source it must be cleared.
    h.interrupt_flag_for_test().store(true, Ordering::Relaxed);
    h.eval_source("(hume/yield!)", &mut mock).unwrap_err(); // interrupted via pre-set flag
    assert!(
        !h.interrupt_flag_for_test().load(Ordering::Relaxed),
        "interrupt_flag must be false after eval_source returns"
    );

    // Subsequent evals with no flag pre-set should succeed normally.
    h.eval_source("(hume/yield!)", &mut mock).unwrap();
}

// ── command-plugin ────────────────────────────────────────────────────────

/// Unknown (built-in) commands return "hume".
#[test]
fn command_plugin_unknown_returns_hume() {
    let h = host();

    // "move-right" is a Rust built-in — not in cmd_owners.
    assert!(!h.cmd_owners_for_test().contains_key("move-right"));
}

// ── define-command! — no extendable flag ─────────────────────────────────
// All Steel commands participate in Ctrl+key one-shot extend; the body
// receives `extend` as a lambda arg. There is no separate define-command-extend!.

/// `define-command!` registers the command; `define-command-extend!` has been
/// removed.  Verifies the old name is not a recognised builtin by checking that
/// calling it produces a Steel FreeIdentifier error.
#[test]
fn define_command_extend_builtin_removed() {
    let mut h = host();
    let mut mock = MockHost::new();

    let result = h.eval_source_returning_defs(
        r#"(define-command-extend! "ext-cmd" "doc" (lambda () (+ 1 0)))"#.to_owned(),
        Default::default(),
        &mut mock,
    );
    assert!(
        result.is_err(),
        "define-command-extend! must be gone; expected an error, got Ok"
    );
}

// ── define-command! keywords ──────────────────────────────────────────────

/// `#:inline-output #t` sets `inline_output: true`; plain `define-command!`
/// sets it to `false`.
#[test]
fn define_command_inline_output_sets_flag() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source_returning_defs(
        r#"(define-command! "inline-cmd" "doc" (lambda () (+ 1 0)) #:inline-output #t)
           (define-command! "plain-cmd"  "doc" (lambda () (+ 1 0)))"#
            .to_owned(),
        Default::default(),
        &mut mock,
    )
    .expect("eval should succeed");

    let inline = mock
        .registered_cmds
        .iter()
        .find(|d| d.name == "inline-cmd")
        .expect("inline-cmd not found");
    let plain = mock
        .registered_cmds
        .iter()
        .find(|d| d.name == "plain-cmd")
        .expect("plain-cmd not found");
    assert!(
        inline.inline_output,
        "#:inline-output #t should set inline_output = true"
    );
    assert!(
        !plain.inline_output,
        "plain define-command! should set inline_output = false"
    );
}

/// `#:repeatable #t` sets `repeatable: true`; plain `define-command!` does not.
#[test]
fn define_command_repeatable_sets_flag() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source_returning_defs(
        r#"(define-command! "rep-cmd"   "doc" (lambda () (+ 1 0)) #:repeatable #t)
           (define-command! "plain-cmd" "doc" (lambda () (+ 1 0)))"#
            .to_owned(),
        Default::default(),
        &mut mock,
    )
    .expect("eval should succeed");

    let rep = mock
        .registered_cmds
        .iter()
        .find(|d| d.name == "rep-cmd")
        .expect("rep-cmd not found");
    let plain = mock
        .registered_cmds
        .iter()
        .find(|d| d.name == "plain-cmd")
        .expect("plain-cmd not found");
    assert!(
        rep.repeatable,
        "#:repeatable #t should set repeatable = true"
    );
    assert!(
        !plain.repeatable,
        "plain define-command! should set repeatable = false"
    );
}

/// `#:repeatable #t` and `#:inline-output #t` together must raise a Steel error
/// and must not register the command.
///
/// Fail oracle: remove the mutual-exclusion guard in define_command_inner —
/// the eval would succeed and register the command with both flags set.
#[test]
fn repeatable_and_inline_output_mutually_exclusive() {
    let mut h = host();
    let mut mock = MockHost::new();

    let result = h.eval_source_returning_defs(
        r#"(define-command! "bad-cmd" "doc" (lambda () (+ 1 0)) #:repeatable #t #:inline-output #t)"#
            .to_owned(),
        Default::default(),
        &mut mock,
    );
    assert!(
        result.is_err(),
        "#:repeatable + #:inline-output must raise an error; got Ok"
    );
    assert!(
        mock.registered_cmds.is_empty(),
        "failed define-command! must not register the command"
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
fn eval_source_returning_defs_watchdog_aborts_runaway() {
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
    // Flag must be reset after eval_source_returning_defs returns.
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
        err.contains("interrupted"),
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

/// Command bodies cannot mutate settings/keymap (is_init = false during
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
        err.contains("interrupted"),
        "expected 'interrupted', got: {err}"
    );
    assert_eq!(
        mock.settings.tab_width, 4,
        "tab-width must be unchanged after interrupt"
    );
}

/// Calling an init-only builtin from a Steel command body must raise a Steel
/// error (not panic).  `is_init = false` during call_steel_cmd, and init-only
/// builtins check this flag.
#[test]
fn call_steel_cmd_set_option_from_body_returns_steel_error() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "try-set" "" (lambda () (set-option! "tab-width" 8)))"#,
        &mut mock,
    )
    .unwrap();

    let err = h
        .call_steel_cmd(
            "try-set",
            None,
            vec![],
            PaneId::default(),
            BufferId::default(),
            &mut mock,
        )
        .unwrap_err();

    assert!(
        err.contains("set-option!"),
        "error must name the failing builtin; got: {err}"
    );
    // Mutation never happened, so the setting is unchanged.
    assert_eq!(mock.settings.tab_width, 4, "tab-width must be untouched");
}

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
        !err.is_empty(),
        "expected a Steel arity error, got empty string"
    );
}

// ── register-hook! / fire_hook ────────────────────────────────────────────

use hume_scripting::SteelBufferId;
use hume_scripting::hooks::HookId;

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
    h.fire_hook(
        HookId::OnBufferOpen,
        &[val],
        PaneId::default(),
        bid,
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
    h.fire_hook(
        HookId::OnBufferClose,
        &[val],
        PaneId::default(),
        bid,
        &mut mock,
    )
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
    h.fire_hook(
        HookId::OnBufferSave,
        &[val],
        PaneId::default(),
        bid,
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
        HookId::OnModeChange,
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
        HookId::OnBufferOpen,
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
    h.fire_hook(
        HookId::OnBufferSave,
        &[val],
        PaneId::default(),
        bid,
        &mut mock,
    )
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
    assert!(err.contains("can only be called during init"), "got: {err}");
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
    assert!(err.contains("unknown hook"), "got: {err}");
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
        HookId::OnModeChange,
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
        HookId::OnModeChange,
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

// ── set-register-prefix! ─────────────────────────────────────────────────

#[test]
fn set_register_prefix_passed_to_dispatch() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "paste-ring" ""
             (lambda () (set-register-prefix! "k") (call! "paste-after")))"#,
        &mut mock,
    )
    .unwrap();
    mock.native_names.insert("paste-after".to_string());
    h.call_steel_cmd(
        "paste-ring",
        None,
        vec![],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    )
    .unwrap();
    assert_eq!(
        mock.dispatched_native.len(),
        1,
        "paste-after must be dispatched once"
    );
    let (name, _, _, reg) = &mock.dispatched_native[0];
    assert_eq!(name, "paste-after");
    assert_eq!(
        *reg,
        Some('k'),
        "set-register-prefix! must pass register 'k' to dispatch"
    );
}

/// `(call! "paste-after" 0)` must decode to `count: None` at the
/// `run_command_sync` boundary — `0` is the Scheme spelling of "no count
/// typed" (`parse_count_extend`), distinct from `Some(1)` even though both
/// apply the command once.
///
/// Fail oracle: before this change, `parse_count_extend` clamped `0` to `1`,
/// so `dispatched_native[0].1` would be `Some(1)`, not `None`.
#[test]
fn call_native_zero_count_decodes_to_none() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "paste-zero" ""
             (lambda () (call! "paste-after" 0)))"#,
        &mut mock,
    )
    .unwrap();
    mock.native_names.insert("paste-after".to_string());
    h.call_steel_cmd(
        "paste-zero",
        None,
        vec![],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    )
    .unwrap();
    assert_eq!(mock.dispatched_native.len(), 1);
    let (name, count, _, _) = &mock.dispatched_native[0];
    assert_eq!(name, "paste-after");
    assert_eq!(
        *count, None,
        "count 0 from Steel must decode to None, not Some(1)"
    );
}

#[test]
fn set_register_prefix_sticky_across_multiple_calls() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "multi" ""
             (lambda ()
               (set-register-prefix! "5")
               (call! "yank")
               (call! "delete")))"#,
        &mut mock,
    )
    .unwrap();
    mock.native_names.insert("yank".to_string());
    mock.native_names.insert("delete".to_string());
    h.call_steel_cmd(
        "multi",
        None,
        vec![],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    )
    .unwrap();
    assert_eq!(
        mock.dispatched_native.len(),
        2,
        "yank and delete must each be dispatched"
    );
    let (name0, _, _, reg0) = &mock.dispatched_native[0];
    let (name1, _, _, reg1) = &mock.dispatched_native[1];
    assert_eq!(name0, "yank");
    assert_eq!(*reg0, Some('5'), "prefix must be '5' for yank");
    assert_eq!(name1, "delete");
    assert_eq!(*reg1, Some('5'), "prefix must persist to delete");
}

#[test]
fn set_register_prefix_change_mid_body() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "switch" ""
             (lambda ()
               (set-register-prefix! "5")
               (call! "yank")
               (set-register-prefix! "6")
               (call! "paste-after")))"#,
        &mut mock,
    )
    .unwrap();
    mock.native_names.insert("yank".to_string());
    mock.native_names.insert("paste-after".to_string());
    h.call_steel_cmd(
        "switch",
        None,
        vec![],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    )
    .unwrap();
    assert_eq!(mock.dispatched_native.len(), 2);
    let (name0, _, _, reg0) = &mock.dispatched_native[0];
    let (name1, _, _, reg1) = &mock.dispatched_native[1];
    assert_eq!(name0, "yank");
    assert_eq!(*reg0, Some('5'), "first dispatch must use register '5'");
    assert_eq!(name1, "paste-after");
    assert_eq!(
        *reg1,
        Some('6'),
        "second dispatch must use changed register '6'"
    );
}

#[test]
fn set_register_prefix_invalid_name_errors() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "bad-reg" "" (lambda () (set-register-prefix! "z")))"#,
        &mut mock,
    )
    .unwrap();
    let err = h
        .call_steel_cmd(
            "bad-reg",
            None,
            vec![],
            PaneId::default(),
            BufferId::default(),
            &mut mock,
        )
        .unwrap_err();
    assert!(
        err.contains("invalid register"),
        "expected register-name error, got: {err}"
    );
}

#[test]
fn set_register_prefix_multichar_name_errors() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "bad-multi" "" (lambda () (set-register-prefix! "kk")))"#,
        &mut mock,
    )
    .unwrap();
    let err = h
        .call_steel_cmd(
            "bad-multi",
            None,
            vec![],
            PaneId::default(),
            BufferId::default(),
            &mut mock,
        )
        .unwrap_err();
    assert!(
        err.contains("single-character"),
        "expected single-char error, got: {err}"
    );
}

#[test]
fn set_register_prefix_at_init_errors() {
    let mut h = host();
    let mut mock = MockHost::new();

    let err = h
        .eval_source(r#"(set-register-prefix! "k")"#, &mut mock)
        .unwrap_err();
    assert!(err.contains("not available during init"), "got: {err}");
}

// ── init-mode guards for buffer lifecycle builtins ────────────────────────

/// `(close-buffer! …)` called from init.scm must raise a Steel error rather
/// than crashing.  The `require_cmd_ctx!` guard fires before the host method.
///
/// Flip: remove `require_cmd_ctx!` from `close_buffer` and the eval returns
/// Ok (or panics), not Err.
#[test]
fn close_buffer_errors_in_init_mode() {
    let mut h = host();
    let mut mock = MockHost::new();
    let err = h
        .eval_source("(close-buffer! (quote ()))", &mut mock)
        .unwrap_err();
    assert!(
        err.contains("not available during init"),
        "close-buffer! must raise init-guard error, got: {err}",
    );
}

/// `(switch-to-buffer! …)` called from init.scm must raise a Steel error
/// rather than crashing.  Mirrors `close_buffer_errors_in_init_mode`.
///
/// Flip: remove `require_cmd_ctx!` from `switch_to_buffer` and the eval
/// returns Ok (or panics), not Err.
#[test]
fn switch_to_buffer_errors_in_init_mode() {
    let mut h = host();
    let mut mock = MockHost::new();
    let err = h
        .eval_source("(switch-to-buffer! (quote ()))", &mut mock)
        .unwrap_err();
    assert!(
        err.contains("not available during init"),
        "switch-to-buffer! must raise init-guard error, got: {err}",
    );
}

/// `(buffer-language …)` / `(set-buffer-language! …)` on a stale buffer id must
/// raise a Steel error, not silently return `#f` or push a no-op. MockHost's
/// `buffer_exists` returns false unconditionally, so `(current-buffer)` is a
/// stale handle from the builtins' point of view — exercising the guard path.
///
/// Flip: remove the `buffer_exists` guard in either builtin and that eval
/// returns Ok instead of Err.
#[test]
fn language_builtins_error_on_stale_buffer_id() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "q-lang" "" (lambda () (buffer-language (current-buffer))))
           (define-command! "set-lang" "" (lambda () (set-buffer-language! (current-buffer) "rust")))"#,
        &mut mock,
    )
    .unwrap();

    let err = h
        .call_steel_cmd(
            "q-lang",
            None,
            vec![],
            PaneId::default(),
            BufferId::default(),
            &mut mock,
        )
        .unwrap_err();
    assert!(
        err.contains("buffer-language: invalid buffer id"),
        "buffer-language must reject a stale id; got: {err}"
    );

    let err = h
        .call_steel_cmd(
            "set-lang",
            None,
            vec![],
            PaneId::default(),
            BufferId::default(),
            &mut mock,
        )
        .unwrap_err();
    assert!(
        err.contains("set-buffer-language!: invalid buffer id"),
        "set-buffer-language! must reject a stale id; got: {err}"
    );
}

// ── bind-key-extend! ──────────────────────────────────────────────────────

#[test]
fn bind_key_extend_creates_force_extending_leaf() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(r#"(bind-key-extend! 'normal "z" "select-line")"#, &mut mock)
        .unwrap();
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use hume_editor::KeymapBindMode as BindMode;
    let z_key = &[KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE)];
    let (name, force_extend) = mock
        .keymap
        .lookup_command(BindMode::Normal, z_key)
        .expect("z must be bound after bind-key-extend!");
    assert_eq!(name, "select-line");
    assert!(
        force_extend,
        "bind-key-extend! must produce force_extend = true"
    );
}

#[test]
fn bind_key_does_not_force_extend() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(r#"(bind-key! 'normal "z" "select-line")"#, &mut mock)
        .unwrap();
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use hume_editor::KeymapBindMode as BindMode;
    let z_key = &[KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE)];
    let (_, force_extend) = mock
        .keymap
        .lookup_command(BindMode::Normal, z_key)
        .expect("z must be bound after bind-key!");
    assert!(!force_extend, "bind-key! must produce force_extend = false");
}

#[test]
fn bind_key_extend_invalid_mode_errors() {
    let mut h = host();
    let mut mock = MockHost::new();

    let err = h
        .eval_source(r#"(bind-key-extend! 'visual "f" "cmd")"#, &mut mock)
        .unwrap_err();
    assert!(err.contains("mode"), "got: {err}");
}

// ── unbind-key! ───────────────────────────────────────────────────────────

#[test]
fn unbind_key_removes_default_binding() {
    let mut h = host();
    let mut mock = MockHost::new();

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use hume_editor::KeymapBindMode as BindMode;
    let h_key = &[KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)];
    assert!(
        mock.keymap
            .lookup_command(BindMode::Normal, h_key)
            .is_some(),
        "'h' must be bound by default"
    );

    h.eval_source(r#"(unbind-key! 'normal "h")"#, &mut mock)
        .unwrap();

    assert!(
        mock.keymap
            .lookup_command(BindMode::Normal, h_key)
            .is_none(),
        "'h' must be unbound after unbind-key!"
    );
}

#[test]
fn unbind_key_noop_on_already_unbound() {
    let mut h = host();
    let mut mock = MockHost::new();

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use hume_editor::KeymapBindMode as BindMode;
    let q_key = &[KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE)];

    // 'Q' is not in the default keymap.
    assert!(
        mock.keymap
            .lookup_command(BindMode::Normal, q_key)
            .is_none(),
        "'Q' must not be bound before unbind-key! (baseline check)"
    );

    h.eval_source(r#"(unbind-key! 'normal "Q")"#, &mut mock)
        .unwrap();

    // The no-op must not corrupt the keymap — 'Q' remains absent.
    assert!(
        mock.keymap
            .lookup_command(BindMode::Normal, q_key)
            .is_none(),
        "'Q' must remain unbound after no-op unbind-key!"
    );
}

#[test]
fn unbind_key_invalid_mode_errors() {
    let mut h = host();
    let mut mock = MockHost::new();

    let err = h
        .eval_source(r#"(unbind-key! 'visual "h")"#, &mut mock)
        .unwrap_err();
    assert!(err.contains("mode"), "got: {err}");
}

// ── Steel file-module isolation + prelude macro visibility ────────────────
//
// Two properties of steel-core's module system required by the plugins branch:
//  1. Private helpers are isolated across modules (foundation of plan A).
//  2. A define-syntax macro defined globally (as the prelude does) is visible
//     inside a subsequently required module body.
//
// Not on Windows: path separators in Scheme string literals are not escaped.

#[test]
#[cfg(not(windows))]
fn file_module_private_helpers_are_isolated() {
    use steel::rvals::SteelVal;
    use steel::steel_vm::engine::Engine;

    let dir = tempfile::tempdir().unwrap();

    // Two modules with the same private helper name, different return values.
    std::fs::write(
        dir.path().join("a.scm"),
        "(define (helper) \"A\")\n(define (a-result) (helper))\n(provide a-result)\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("b.scm"),
        "(define (helper) \"B\")\n(define (b-result) (helper))\n(provide b-result)\n",
    )
    .unwrap();

    let a_abs = dir.path().join("a.scm").canonicalize().unwrap();
    let b_abs = dir.path().join("b.scm").canonicalize().unwrap();

    let mut engine = Engine::new();
    engine
        .compile_and_run_raw_program(format!("(require \"{}\")", a_abs.display()))
        .expect("require a.scm failed");
    // Loading B last: if helpers collide, a-result would return "B".
    engine
        .compile_and_run_raw_program(format!("(require \"{}\")", b_abs.display()))
        .expect("require b.scm failed");

    let a_vals = engine
        .compile_and_run_raw_program("(a-result)".to_owned())
        .expect("a-result failed");
    let b_vals = engine
        .compile_and_run_raw_program("(b-result)".to_owned())
        .expect("b-result failed");

    assert!(
        matches!(a_vals.last(), Some(SteelVal::StringV(s)) if s.as_str() == "A"),
        "a-result should use A's private helper (\"A\"); got {:?}",
        a_vals.last()
    );
    assert!(
        matches!(b_vals.last(), Some(SteelVal::StringV(s)) if s.as_str() == "B"),
        "b-result should use B's private helper (\"B\"); got {:?}",
        b_vals.last()
    );
}

#[test]
#[cfg(not(windows))]
fn file_module_relative_require_resolves_from_module_dir() {
    use steel::rvals::SteelVal;
    use steel::steel_vm::engine::Engine;

    let dir = tempfile::tempdir().unwrap();

    std::fs::write(
        dir.path().join("lib.scm"),
        "(define (lib-helper) \"from-lib\")\n(provide lib-helper)\n",
    )
    .unwrap();
    // plugin.scm uses a relative require — should resolve against its own dir,
    // not the process working directory.
    std::fs::write(
        dir.path().join("plugin.scm"),
        "(require \"lib.scm\")\n(define (plugin-result) (lib-helper))\n(provide plugin-result)\n",
    )
    .unwrap();

    let plugin_abs = dir.path().join("plugin.scm").canonicalize().unwrap();

    // Process CWD is the workspace root — NOT the plugin dir.  The require
    // must still succeed because Steel resolves relative paths from the
    // requiring module's own path, not from CWD.
    let mut engine = Engine::new();
    engine
        .compile_and_run_raw_program(format!("(require \"{}\")", plugin_abs.display()))
        .expect("require plugin.scm failed");

    let vals = engine
        .compile_and_run_raw_program("(plugin-result)".to_owned())
        .expect("plugin-result failed");

    assert!(
        matches!(vals.last(), Some(SteelVal::StringV(s)) if s.as_str() == "from-lib"),
        "plugin-result should return \"from-lib\" via relative sub-require; got {:?}",
        vals.last()
    );
}

/// De-risk test for the prelude concept: a `define-syntax` macro defined in
/// a global eval (as the prelude does) must be visible inside a subsequently
/// `(require)`d module.
///
/// If this test fails the prelude cannot serve plugin modules — only `init.scm`.
/// That would require documenting the limitation and NOT silently changing the
/// loader (HARD STOP per plan).
#[test]
#[cfg(not(windows))]
fn global_define_syntax_is_visible_inside_required_module() {
    use steel::rvals::SteelVal;
    use steel::steel_vm::engine::Engine;

    let dir = tempfile::tempdir().unwrap();

    let mut engine = Engine::new();

    // Define a macro globally, simulating what the prelude does.
    // id-macro! is the identity macro: (id-macro! x) => x.
    engine
        .compile_and_run_raw_program(
            "(define-syntax id-macro! (syntax-rules () ((_ x) x)))".to_owned(),
        )
        .expect("global macro definition must succeed");

    // Write a module whose top-level uses the globally-defined macro.
    // result is module-private; get-result wraps it so it can be called globally.
    std::fs::write(
        dir.path().join("mod.scm"),
        "(define result (id-macro! \"macro-expanded\"))\
         \n(define (get-result) result)\
         \n(provide get-result)\n",
    )
    .unwrap();
    let abs = dir.path().join("mod.scm").canonicalize().unwrap();

    engine
        .compile_and_run_raw_program(format!("(require \"{}\")", abs.display()))
        .expect("require failed — id-macro! not visible inside the module");

    let vals = engine
        .compile_and_run_raw_program("(get-result)".to_owned())
        .expect("get-result must be callable after require");

    assert!(
        matches!(vals.last(), Some(SteelVal::StringV(s)) if s.as_str() == "macro-expanded"),
        "id-macro! must have expanded inside the module; got {:?}",
        vals.last()
    );
}

// ── Prelude macro behavior ────────────────────────────────────────────────────
//
// The prelude defines bind-keys!, bind-keys-extend!, and unbind-keys! as
// syntax-rules batch wrappers over the underlying single-pair builtins.
// These tests load the three macros via eval_source (as init_scripting does via
// eval_init before init.scm), then exercise each macro.
//
// Independent oracle: the expected bindings come from the literal key/cmd pairs
// passed to the macro — not from re-reading the keymap.
// Zero-effect check: swap a cmd name in a pair; the assertion catches it because
// it compares against the literal name from the input, not "whatever the keymap says".

/// Macro source matching runtime/scheme/prelude.scm (inlined so tests do not
/// depend on the runtime dir being on disk relative to the test runner CWD).
const PRELUDE_MACROS: &str = r#"
(define-syntax bind-keys!
  (syntax-rules ()
    ((_ mode (key cmd) ...) (begin (bind-key! mode key cmd) ...))))
(define-syntax bind-keys-extend!
  (syntax-rules ()
    ((_ mode (key cmd) ...) (begin (bind-key-extend! mode key cmd) ...))))
(define-syntax unbind-keys!
  (syntax-rules ()
    ((_ mode key ...) (begin (unbind-key! mode key) ...))))
"#;

#[test]
fn prelude_bind_keys_batch_binds_multiple() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use hume_editor::KeymapBindMode as BindMode;

    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(PRELUDE_MACROS, &mut mock).unwrap();
    h.eval_source(
        r#"(bind-keys! 'normal
             ("z z" "move-left")
             ("z l" "move-right"))"#,
        &mut mock,
    )
    .unwrap();

    let z = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE);
    let l = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE);

    let (name, fe) = mock
        .keymap
        .lookup_command(BindMode::Normal, &[z, z])
        .expect("\"z z\" must be bound after bind-keys!");
    assert_eq!(name, "move-left");
    assert!(!fe, "bind-keys! must not force extend");

    let (name2, _) = mock
        .keymap
        .lookup_command(BindMode::Normal, &[z, l])
        .expect("\"z l\" must be bound after bind-keys!");
    assert_eq!(name2, "move-right");
}

#[test]
fn prelude_bind_keys_extend_creates_force_extend_leaves() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use hume_editor::KeymapBindMode as BindMode;

    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(PRELUDE_MACROS, &mut mock).unwrap();
    h.eval_source(
        r#"(bind-keys-extend! 'normal
             ("Q" "select-line")
             ("W" "select-to-end"))"#,
        &mut mock,
    )
    .unwrap();

    let q = &[KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE)];
    let w = &[KeyEvent::new(KeyCode::Char('W'), KeyModifiers::NONE)];

    let (name, fe) = mock
        .keymap
        .lookup_command(BindMode::Normal, q)
        .expect("\"Q\" must be bound after bind-keys-extend!");
    assert_eq!(name, "select-line");
    assert!(fe, "bind-keys-extend! must produce force_extend = true");

    let (name2, fe2) = mock
        .keymap
        .lookup_command(BindMode::Normal, w)
        .expect("\"W\" must be bound after bind-keys-extend!");
    assert_eq!(name2, "select-to-end");
    assert!(fe2, "bind-keys-extend! must produce force_extend = true");
}

#[test]
fn prelude_unbind_keys_batch_removes_bindings() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use hume_editor::KeymapBindMode as BindMode;

    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(PRELUDE_MACROS, &mut mock).unwrap();

    // 'h' and 'l' are default normal-mode bindings.
    let h_key = &[KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)];
    let l_key = &[KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE)];
    assert!(
        mock.keymap
            .lookup_command(BindMode::Normal, h_key)
            .is_some(),
        "'h' must be bound by default"
    );
    assert!(
        mock.keymap
            .lookup_command(BindMode::Normal, l_key)
            .is_some(),
        "'l' must be bound by default"
    );

    h.eval_source(r#"(unbind-keys! 'normal "h" "l")"#, &mut mock)
        .unwrap();

    assert!(
        mock.keymap
            .lookup_command(BindMode::Normal, h_key)
            .is_none(),
        "'h' must be unbound after unbind-keys!"
    );
    assert!(
        mock.keymap
            .lookup_command(BindMode::Normal, l_key)
            .is_none(),
        "'l' must be unbound after unbind-keys!"
    );
}

/// Verify the prelude eval_init → init.scm eval_init sequence: prelude macros
/// defined by the first eval_init are available in the second.
#[test]
fn prelude_eval_init_sequence_makes_macros_available_to_init_scm() {
    use std::io::Write as _;

    let mut h = host();
    let mut mock = MockHost::new();

    let builtin_names: std::collections::HashSet<String> = Default::default();

    // Stage a prelude file and an init.scm that uses its macros.
    let dir = tempfile::tempdir().unwrap();
    let prelude_path = dir.path().join("prelude.scm");
    let init_path = dir.path().join("init.scm");

    std::fs::write(&prelude_path, PRELUDE_MACROS).unwrap();
    let mut f = std::fs::File::create(&init_path).unwrap();
    writeln!(
        f,
        r#"(bind-keys! 'normal ("Q Q" "move-left") ("Q W" "move-right"))"#
    )
    .unwrap();

    // Load prelude first, then init.scm — mirroring init_scripting's sequence.
    h.eval_init(&prelude_path, 10_000, &mut mock, builtin_names.clone())
        .expect("prelude eval_init must succeed");
    assert!(
        mock.registered_cmds.is_empty(),
        "prelude must define no commands; got {:?}",
        mock.registered_cmds
            .iter()
            .map(|d| &d.name)
            .collect::<Vec<_>>()
    );

    h.eval_init(&init_path, 10_000, &mut mock, builtin_names)
        .expect("init.scm using bind-keys! must succeed after prelude is loaded");

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use hume_editor::KeymapBindMode as BindMode;
    let q = KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE);
    let w = KeyEvent::new(KeyCode::Char('W'), KeyModifiers::NONE);

    let (name1, _) = mock
        .keymap
        .lookup_command(BindMode::Normal, &[q, q])
        .expect("\"Q Q\" must be bound via bind-keys! from init.scm");
    assert_eq!(name1, "move-left");

    let (name2, _) = mock
        .keymap
        .lookup_command(BindMode::Normal, &[q, w])
        .expect("\"Q W\" must be bound via bind-keys! from init.scm");
    assert_eq!(name2, "move-right");
}

/// When init.scm uses bind-keys! but the prelude was never loaded, the eval
/// fails with a clear error (macro undefined) — not a panic.
#[test]
fn bind_keys_without_prelude_fails_gracefully() {
    let mut h = host();
    let mut mock = MockHost::new();

    // bind-keys! is NOT defined — init.scm uses it directly.
    let err = h
        .eval_source(r#"(bind-keys! 'normal ("z" "move-left"))"#, &mut mock)
        .unwrap_err();

    // Steel reports an unbound identifier or similar error; the editor survives.
    assert!(
        !err.is_empty(),
        "using bind-keys! without prelude must return an error"
    );
}

// ── Phase 0 lazy plugin loading ───────────────────────────────────────────
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals.

/// Helper: create a temp user plugin at `plugins/user/tp/plugin.scm` and
/// return `(TempDir, init.scm path)`.  Caller must keep TempDir alive.
#[cfg(not(windows))]
fn plugin_fixture(init_body: &str, plugin_body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), plugin_body).unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(&init_path, init_body).unwrap();
    (dir, init_path)
}

/// `(load-plugin "user/tp")` with no keywords → plugin activates eagerly,
/// reaches `Loaded`, and its command appears in the returned defs.
#[test]
#[cfg(not(windows))]
fn eager_load_no_keywords_reaches_loaded_state() {
    let (dir, init_path) = plugin_fixture(
        r#"(load-plugin "user/tp")"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("eager load must succeed");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Loaded)),
        "plugin must reach Loaded state; got {:?}",
        h.plugin_status(&id)
    );
    assert!(
        mock.registered_cmds.iter().any(|d| d.name == "tp-cmd"),
        "tp-cmd must be registered; got {:?}",
        mock.registered_cmds
            .iter()
            .map(|d| &d.name)
            .collect::<Vec<_>>()
    );
}

/// `(declare-plugin "user/tp" #:commands '("lazy-cmd"))` → plugin stays
/// `Declared`, body is NOT evaluated, and its commands are absent from init result.
#[test]
#[cfg(not(windows))]
fn lazy_load_stays_declared_body_not_evaluated() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:commands '("lazy-cmd"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("lazy load must not error during init");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Declared)),
        "plugin must stay Declared; got {:?}",
        h.plugin_status(&id)
    );
    assert!(
        !mock.registered_cmds.iter().any(|d| d.name == "tp-cmd"),
        "tp-cmd must NOT be registered for a lazy plugin"
    );
}

/// `(declare-plugin "user/tp" #:commands '("my-cmd"))` → plugin stays lazy,
/// `activation_commands["my-cmd"]` maps to the plugin, body not evaluated.
#[test]
#[cfg(not(windows))]
fn on_command_trigger_populates_registry_body_not_evaluated() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:commands '("my-cmd"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("#:commands declaration must not error during init");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Declared)),
        "plugin declared with #:commands must stay Declared; got {:?}",
        h.plugin_status(&id)
    );
    assert_eq!(
        h.activation_commands().get("my-cmd"),
        Some(&id),
        "activation_commands must map my-cmd to the plugin"
    );
    assert!(
        !mock.registered_cmds.iter().any(|d| d.name == "tp-cmd"),
        "tp-cmd must NOT be registered for a #:commands plugin"
    );
}

/// `activate_plugin` on a `Declared` lazy plugin → state transitions to
/// `Loaded`, returns the plugin's `SteelCmdDef`s.  Second call → idempotent
/// `Ok(vec![])`.
#[test]
#[cfg(not(windows))]
fn activate_plugin_idempotent_on_declared_lazy_plugin() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:commands '("lazy-cmd"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("init must succeed");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };

    // First activation: Declared → Loaded, registers the plugin's command.
    h.activate_plugin_inline(&id, 10_000, &mut mock, &Default::default())
        .expect("activate_plugin_inline must succeed");
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Loaded)),
        "plugin must be Loaded after activate_plugin_inline; got {:?}",
        h.plugin_status(&id)
    );
    assert!(
        mock.registered_cmds.iter().any(|d| d.name == "tp-cmd"),
        "tp-cmd must be registered after activation"
    );

    // Second activation: already Loaded → idempotent, no new registrations.
    let count_after_first = mock.registered_cmds.len();
    h.activate_plugin_inline(&id, 10_000, &mut mock, &Default::default())
        .expect("second activate_plugin_inline must succeed");
    assert_eq!(
        mock.registered_cmds.len(),
        count_after_first,
        "second activation must be idempotent (no new commands registered)"
    );
}

/// An eager plugin whose body raises an error causes `eval_init` to return
/// `Err` (fail-fast), and leaves the plugin in `Failed` state.
#[test]
#[cfg(not(windows))]
fn eager_plugin_body_error_aborts_init() {
    let (dir, init_path) = plugin_fixture(
        r#"(load-plugin "user/tp")"#,
        r#"(error "intentional plugin failure")"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    let result = h.eval_init(&init_path, 10_000, &mut mock, Default::default());
    assert!(
        result.is_err(),
        "init must fail when eager plugin body errors"
    );

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Failed)),
        "plugin must be Failed after body error; got {:?}",
        h.plugin_status(&id)
    );
}

// ── Phase 1 lazy plugin loading — command activations ────────────────────────
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals.

/// `#:commands '("move-right" "my-cmd")` — "move-right" clashes with a built-in →
/// colliding activation entry is dropped, a `Severity::Error` is logged, init continues with
/// the remaining valid activation entry "my-cmd".
///
/// Flip: a non-builtin name produces no Error and the activation entry is registered.
#[test]
#[cfg(not(windows))]
fn manifest_collision_with_builtin_logs_error_continues() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:commands '("move-right" "my-cmd"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    let builtin_names: std::collections::HashSet<String> =
        ["move-right".to_string()].into_iter().collect();
    h.eval_init(&init_path, 10_000, &mut mock, builtin_names)
        .expect("partial builtin collision must NOT abort init");

    // Error logged for the dropped activation entry.
    assert!(
        h.peek_pending_messages().iter().any(|(sev, msg)| {
            matches!(sev, hume_scripting::LogLevel::Error)
                && msg.contains("move-right")
                && msg.contains("built-in")
        }),
        "expected an Error about 'move-right' conflicting with a built-in; got: {:?}",
        h.peek_pending_messages()
    );
    // Colliding activation entry not written; valid one is.
    assert!(
        !h.activation_commands().contains_key("move-right"),
        "colliding activation entry must not appear in activation_commands"
    );
    assert!(
        h.activation_commands().contains_key("my-cmd"),
        "valid activation entry must appear in activation_commands"
    );
    // Plugin stays Declared (body not evaluated), with the remaining activation entry.
    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Declared)),
        "plugin must stay Declared after partial-collision #:commands list; got {:?}",
        h.plugin_status(&id)
    );

    // Flip: non-colliding entry produces no Error and is registered.
    let (dir2, init_path2) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:commands '("not-a-builtin"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut h2 = host();
    h2.set_data_dir(dir2.path().to_path_buf());
    let mut mock2 = MockHost::new();

    let builtin_names2: std::collections::HashSet<String> =
        ["move-right".to_string()].into_iter().collect();
    h2.eval_init(&init_path2, 10_000, &mut mock2, builtin_names2)
        .expect("non-colliding activation entry must not error");
    assert!(
        !h2.peek_pending_messages()
            .iter()
            .any(|(sev, _)| matches!(sev, hume_scripting::LogLevel::Error)),
        "non-colliding activation entry must not log any Error"
    );
    assert!(
        h2.activation_commands().contains_key("not-a-builtin"),
        "non-colliding activation entry must appear in activation_commands"
    );
}

/// Two plugins both declare `#:commands '("bar")` → second declaration's
/// activation entry is dropped, a `Severity::Error` is logged, first-writer-wins, init
/// continues.
///
/// Flip: both plugins are Declared; only the first plugin owns the activation entry.
#[test]
#[cfg(not(windows))]
fn manifest_collision_lazy_vs_lazy_logs_error_continues() {
    let dir = tempfile::tempdir().unwrap();
    let pa = dir.path().join("plugins").join("user").join("pa");
    let pb = dir.path().join("plugins").join("user").join("pb");
    std::fs::create_dir_all(&pa).unwrap();
    std::fs::create_dir_all(&pb).unwrap();
    std::fs::write(
        pa.join("plugin.scm"),
        r#"(define-command! "tp-a" "doc" (lambda () (+ 1 0)))"#,
    )
    .unwrap();
    std::fs::write(
        pb.join("plugin.scm"),
        r#"(define-command! "tp-b" "doc" (lambda () (+ 1 0)))"#,
    )
    .unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        r#"
(declare-plugin "user/pa" #:commands '("bar"))
(declare-plugin "user/pb" #:commands '("bar" "pb-only"))
"#,
    )
    .unwrap();

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("lazy-vs-lazy collision must NOT abort init");

    // Error logged for pb's duplicate activation entry.
    assert!(
        h.peek_pending_messages().iter().any(|(sev, msg)| {
            matches!(sev, hume_scripting::LogLevel::Error)
                && msg.contains("bar")
                && msg.contains("already claimed")
        }),
        "expected an Error about 'bar' already claimed; got: {:?}",
        h.peek_pending_messages()
    );
    // First-writer (pa) owns the activation entry.
    let pa_id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "pa".to_string(),
    };
    assert_eq!(
        h.activation_commands().get("bar"),
        Some(&pa_id),
        "activation_commands[\"bar\"] must point to pa (first-writer-wins)"
    );
    // Both plugins are Declared — pb stays declared even though its activation entry was dropped.
    let pb_id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "pb".to_string(),
    };
    assert!(
        matches!(h.plugin_status(&pa_id), Some(PluginStatus::Declared)),
        "pa must be Declared"
    );
    assert!(
        matches!(h.plugin_status(&pb_id), Some(PluginStatus::Declared)),
        "pb must be Declared even with its activation entry dropped"
    );
}

/// After a lazy declare, `cmd_owners["bar"]` maps to the plugin id — not to
/// `"hume"` — even before the plugin body is evaluated.
///
/// Flip: assert it is NOT `"hume"` after the lazy declare.
#[test]
#[cfg(not(windows))]
fn cmd_owners_pre_seeded_before_activation() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:commands '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("init must succeed");

    // Plugin has NOT been activated yet — body was not evaluated.
    let owner = h.cmd_owners_for_test().get("bar").map(|s| s.as_str());
    assert!(
        owner != Some("hume"),
        "cmd_owners must be pre-seeded with the plugin id, not 'hume'; got: {:?}",
        owner
    );
    assert_eq!(
        owner,
        Some("user/tp"),
        "cmd_owners must map 'bar' to 'user/tp' before activation"
    );
}

/// `activate_plugin` drops the plugin's `activation_commands` entry after the
/// plugin body is evaluated successfully.
#[test]
#[cfg(not(windows))]
fn activate_plugin_drops_command_trigger_on_loaded() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:commands '("my-cmd"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("init must succeed");

    // Activation entry is present before activation.
    assert!(
        h.activation_commands().contains_key("my-cmd"),
        "activation entry must be present before activation"
    );

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    h.activate_plugin_inline(&id, 10_000, &mut mock, &Default::default())
        .expect("activate_plugin_inline must succeed");

    // Activation entry is removed after activation.
    assert!(
        !h.activation_commands().contains_key("my-cmd"),
        "activation entry must be removed after activation"
    );
}

/// `(declare-plugin "user/tp" #:languages '("rust"))` → plugin stays lazy,
/// `activation_languages["rust"]` contains the plugin, body not evaluated.
///
/// Flip: if the `#:languages` list were not threaded through `%declare-plugin!`, the
/// plugin would stay Declared but with an empty activation_languages map.
#[test]
#[cfg(not(windows))]
fn on_language_trigger_populates_registry_body_not_evaluated() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:languages '("rust"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("#:languages declaration must not error during init");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Declared)),
        "plugin declared with #:languages must stay Declared; got {:?}",
        h.plugin_status(&id)
    );
    assert!(
        h.activation_language_plugins("rust").contains(&id),
        "activation_languages must map \"rust\" to the plugin"
    );
    assert!(
        !mock.registered_cmds.iter().any(|d| d.name == "tp-cmd"),
        "tp-cmd must NOT be registered for a #:languages plugin"
    );
}

/// `activate_plugin` on a language-matched plugin drops the activation entry on success.
///
/// Flip: without the `activation_languages.retain` in the Ok branch, the activation
/// entry would survive and falsely appear pending on subsequent language sets.
#[test]
#[cfg(not(windows))]
fn activate_plugin_drops_language_activation_on_loaded() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:languages '("rust"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("init must succeed");

    assert!(
        !h.activation_language_plugins("rust").is_empty(),
        "activation entry must be present before activation"
    );

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    h.activate_plugin_inline(&id, 10_000, &mut mock, &Default::default())
        .expect("activate_plugin_inline must succeed");

    assert!(
        h.activation_language_plugins("rust").is_empty(),
        "activation entry must be removed after activation"
    );
}

/// `(load-plugin "x")` after `(declare-plugin "x" #:commands …)` force-activates
/// the plugin: state transitions to `Loaded` and the activation command entry is cleared.
///
/// Flip: without the %activate-plugin-inline call in the load-plugin wrapper,
/// the plugin would stay `Declared` and the activation entry would remain.
#[test]
#[cfg(not(windows))]
fn declare_then_load_activates_and_logs_soft_error() {
    let (dir, init_path) = plugin_fixture(
        "(declare-plugin \"user/tp\" #:commands '(\"my-cmd\"))\n(load-plugin \"user/tp\")",
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("declare-then-load must succeed (soft error, not hard)");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Loaded)),
        "plugin must be Loaded after explicit load-plugin; got {:?}",
        h.plugin_status(&id)
    );
    assert!(
        !h.activation_commands().contains_key("my-cmd"),
        "activation command entry must be cleared after activation"
    );
    assert!(
        mock.registered_cmds.iter().any(|d| d.name == "tp-cmd"),
        "tp-cmd must be registered after activation"
    );
    // Soft error: declare-then-load is contradictory and must be logged.
    assert!(
        h.peek_pending_messages().iter().any(|(sev, msg)| {
            matches!(sev, hume_scripting::LogLevel::Error)
                && msg.contains("user/tp")
                && msg.contains("declared lazily")
        }),
        "expected a soft error about declare-then-load contradiction; got: {:?}",
        h.peek_pending_messages()
    );
}

/// `(load-plugin "foo")` then `(declare-plugin "foo" …)` — load runs first,
/// plugin is `Loaded`; the declare is ignored with a soft error.
///
/// Flip: remove the load-then-declare guard in declare_plugin and the declare
/// silently no-ops (via the existing PluginState::Declared duplicate guard) without
/// logging an error.
#[test]
#[cfg(not(windows))]
fn load_then_declare_ignored_with_soft_error() {
    let (dir, init_path) = plugin_fixture(
        "(load-plugin \"user/tp\")\n(declare-plugin \"user/tp\" #:commands '(\"my-cmd\"))",
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("load-then-declare must succeed (soft error, not hard)");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(h.plugin_status(&id), Some(PluginStatus::Loaded)),
        "plugin must remain Loaded; got {:?}",
        h.plugin_status(&id)
    );
    // Soft error: the declare after load is contradictory and must be logged.
    assert!(
        h.peek_pending_messages().iter().any(|(sev, msg)| {
            matches!(sev, hume_scripting::LogLevel::Error)
                && msg.contains("user/tp")
                && msg.contains("already loaded")
        }),
        "expected a soft error about load-then-declare contradiction; got: {:?}",
        h.peek_pending_messages()
    );
    // The declare was ignored: no activation entry for "my-cmd" should be registered.
    assert!(
        !h.activation_commands().contains_key("my-cmd"),
        "my-cmd must not be registered as an activation entry — declare was ignored"
    );
}

/// `(load-plugin …)` inside an eager plugin body is rejected unconditionally —
/// even when the dep is present on disk, the gate fires before path resolution.
///
/// Flip: weaken the gate back to `!ctx.is_init` and the eager in-body call
/// succeeds (is_init=true inside an eager body) instead of erroring.
#[test]
#[cfg(not(windows))]
fn load_plugin_in_plugin_body_rejected() {
    // Plugin pb calls (load-plugin "user/dep") in its body; dep IS present on
    // disk so a missing-file error cannot mask the gate.
    let dir = tempfile::tempdir().unwrap();
    let pb_dir = dir.path().join("plugins").join("user").join("pb");
    let dep_dir = dir.path().join("plugins").join("user").join("dep");
    std::fs::create_dir_all(&pb_dir).unwrap();
    std::fs::create_dir_all(&dep_dir).unwrap();
    std::fs::write(pb_dir.join("plugin.scm"), r#"(load-plugin "user/dep")"#).unwrap();
    std::fs::write(dep_dir.join("plugin.scm"), r#"(+ 1 0)"#).unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(&init_path, r#"(load-plugin "user/pb")"#).unwrap();

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    let Err(msg) = h.eval_init(&init_path, 10_000, &mut mock, Default::default()) else {
        panic!("load-plugin inside a plugin body must be rejected");
    };
    assert!(
        msg.contains("top level") || msg.contains("init.scm"),
        "error must mention top-level restriction; got: {msg}"
    );
}

/// `(declare-plugin …)` inside an eager plugin body is rejected — plugins
/// cannot register other plugins; both registration verbs are top-level only.
///
/// Flip: remove the `ensure_top_level` gate from `declare_plugin` and the call
/// succeeds, silently registering a plugin from inside a plugin body.
#[test]
#[cfg(not(windows))]
fn declare_plugin_in_plugin_body_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let pb_dir = dir.path().join("plugins").join("user").join("pb");
    std::fs::create_dir_all(&pb_dir).unwrap();
    std::fs::write(
        pb_dir.join("plugin.scm"),
        r#"(declare-plugin "user/other" #:commands '("other-cmd"))"#,
    )
    .unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(&init_path, r#"(load-plugin "user/pb")"#).unwrap();

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    let Err(msg) = h.eval_init(&init_path, 10_000, &mut mock, Default::default()) else {
        panic!("declare-plugin inside a plugin body must be rejected");
    };
    assert!(
        msg.contains("top level") || msg.contains("init.scm"),
        "error must mention top-level restriction; got: {msg}"
    );
}

// ── zero-entry / duplicate no-op regressions ─────────────────────────────────

/// `(declare-plugin "foo")` with no activation entries is a hard error even in
/// the hume-scripting unit-test harness (no editor needed).
///
/// Flip: remove the zero-entry guard in declare_plugin and eval_source succeeds.
#[test]
#[cfg(not(windows))]
fn declare_plugin_no_triggers_hard_error_scripting_level() {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), r#"(+ 1 0)"#).unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(&init_path, r#"(declare-plugin "user/tp")"#).unwrap();

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    let result = h.eval_init(&init_path, 10_000, &mut mock, Default::default());
    assert!(
        result.is_err(),
        "declare-plugin with no activation entries must hard-error"
    );
    let msg = result.unwrap_err();
    assert!(
        msg.contains("no activation entries") || msg.contains("never be activated"),
        "error must describe the zero-entry problem; got: {msg}"
    );
}

/// `#:commands` names that ALL collide with builtins leave zero effective
/// activation entries → hard error (the post-filter zero-entry check fires).
///
/// Flip: check entry emptiness before collision filtering (pre-filter) and
/// this test passes with a misleading success.
#[test]
#[cfg(not(windows))]
fn declare_plugin_all_commands_collide_is_hard_error() {
    let dir = tempfile::tempdir().unwrap();
    let plugin_dir = dir.path().join("plugins").join("user").join("tp");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), r#"(+ 1 0)"#).unwrap();
    let init_path = dir.path().join("init.scm");
    // "move-right" is a built-in — collision filter drops it, leaving zero activation entries.
    std::fs::write(
        &init_path,
        r#"(declare-plugin "user/tp" #:commands '("move-right"))"#,
    )
    .unwrap();

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    let builtin_names: std::collections::HashSet<String> =
        ["move-right".to_string()].into_iter().collect();
    let result = h.eval_init(&init_path, 10_000, &mut mock, builtin_names);
    assert!(
        result.is_err(),
        "all-collide #:commands with no other activation entry must hard-error"
    );
}

/// Duplicate `(declare-plugin …)` for the same name stays a silent no-op.
///
/// Flip: add a duplicate-declare error in LazyRegistry::declare and this errors.
#[test]
#[cfg(not(windows))]
fn duplicate_declare_remains_silent_noop() {
    let (dir, init_path) = plugin_fixture(
        "(declare-plugin \"user/tp\" #:commands '(\"tp-cmd\"))\n\
         (declare-plugin \"user/tp\" #:commands '(\"tp-cmd\"))",
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("duplicate declare must be a silent no-op, not an error");

    // No error-level message about the duplicate declare.
    assert!(
        !h.peek_pending_messages().iter().any(|(sev, msg)| {
            matches!(sev, hume_scripting::LogLevel::Error) && msg.contains("user/tp")
        }),
        "duplicate declare must not log an error; got: {:?}",
        h.peek_pending_messages()
    );
}

/// Duplicate `(load-plugin …)` for the same name stays a silent no-op.
///
/// Flip: add a duplicate-load error and this panics on the second load.
#[test]
#[cfg(not(windows))]
fn duplicate_load_remains_silent_noop() {
    let (dir, init_path) = plugin_fixture(
        "(load-plugin \"user/tp\")\n(load-plugin \"user/tp\")",
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.set_data_dir(dir.path().to_path_buf());
    let mut mock = MockHost::new();

    h.eval_init(&init_path, 10_000, &mut mock, Default::default())
        .expect("duplicate load must be a silent no-op, not an error");

    // No error-level message about the duplicate load.
    assert!(
        !h.peek_pending_messages().iter().any(|(sev, msg)| {
            matches!(sev, hume_scripting::LogLevel::Error) && msg.contains("user/tp")
        }),
        "duplicate load must not log an error; got: {:?}",
        h.peek_pending_messages()
    );
}

// ── arity-1 command list-arg validation (plum-ensure-grammars pattern) ───────

/// An arity-1 command that validates its arg is a non-empty list must error
/// when called with no args (#f from minibuffer path).
///
/// Flip: change `unless` guard to `(when #t ...)` and the error disappears.
#[test]
fn arity1_list_command_rejects_false_arg() {
    use steel::rvals::SteelVal;
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "needs-list" ""
             (lambda (items)
               (unless (and (list? items) (not (null? items)))
                 (error "needs-list: requires a non-empty list"))
               (call! "move-right")))"#,
        &mut mock,
    )
    .unwrap();
    let err = h
        .call_steel_cmd(
            "needs-list",
            None,
            vec![SteelVal::BoolV(false)],
            PaneId::default(),
            BufferId::default(),
            &mut mock,
        )
        .unwrap_err();
    assert!(
        err.contains("requires a non-empty list"),
        "expected list-required error, got: {err}"
    );
}

/// An arity-1 command that validates its arg is a non-empty list must succeed
/// when passed a real list.
///
/// Flip: change the guard to always error and the Ok below becomes Err.
#[test]
fn arity1_list_command_accepts_list_arg() {
    use steel::rvals::IntoSteelVal as _;
    use steel::rvals::SteelVal;
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "needs-list" ""
             (lambda (items)
               (unless (and (list? items) (not (null? items)))
                 (error "needs-list: requires a non-empty list"))
               (call! "move-right")))"#,
        &mut mock,
    )
    .unwrap();
    let items: Vec<SteelVal> = vec!["rust".into_steelval().unwrap()];
    let list_val = items.into_steelval().unwrap();
    let result = h.call_steel_cmd(
        "needs-list",
        None,
        vec![list_val],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    );
    assert!(
        result.is_ok(),
        "expected Ok for valid list arg, got: {:?}",
        result.err()
    );
}

/// `(load-plugin …)` raises a Steel error when called from a command body
/// (`is_init = false`, `plugin_stack` empty) — the `ensure_top_level` gate rejects it.
///
/// Flip: remove `ensure_top_level` from `load_plugin` and the call returns `Ok`,
/// silently queuing a load request that is never drained.
#[test]
fn load_plugin_runtime_guard_fires() {
    // (load-plugin ...) from a command body (is_init=false) must be rejected.
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "try-load" "" (lambda () (load-plugin "user/tp")))"#,
        &mut mock,
    )
    .unwrap();

    let err = h
        .call_steel_cmd(
            "try-load",
            None,
            vec![],
            PaneId::default(),
            BufferId::default(),
            &mut mock,
        )
        .unwrap_err();
    assert!(
        err.contains("top level") || err.contains("init.scm"),
        "error must mention top-level restriction; got: {err}"
    );
}

/// Test that core:plum grammars.scm has balanced parentheses.
/// This plugin is loaded via `(load-plugin "core:plum")` in init.scm.
/// An earlier imbalance caused "Parse: Unexpected EOF" on startup.
#[test]
fn plum_grammars_scm_balanced() {
    let src = include_str!("../../runtime/plugins/core/plum/grammars.scm");

    // Count structural parens only. Parens inside string literals, `;` line
    // comments, `#| |#` block comments, and `#\(` char literals are not
    // structural and must be skipped, or the oracle is not independent of the
    // file's prose (a comment with an unbalanced paren would mask or fake an
    // imbalance in the actual code).
    let mut opens = 0usize;
    let mut closes = 0usize;
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                // String literal — consume to the closing quote, honoring `\` escapes.
                while let Some(s) = chars.next() {
                    match s {
                        '\\' => {
                            chars.next();
                        }
                        '"' => break,
                        _ => {}
                    }
                }
            }
            ';' => {
                // Line comment — consume to end of line.
                while chars.next_if(|&s| s != '\n').is_some() {}
            }
            '#' if chars.peek() == Some(&'\\') => {
                // Char literal `#\x` (e.g. `#\(`) — skip the `\` and the char.
                chars.next();
                chars.next();
            }
            '#' if chars.peek() == Some(&'|') => {
                // Block comment `#| ... |#` — consume to the closing `|#`.
                chars.next();
                while let Some(s) = chars.next() {
                    if s == '|' && chars.next_if(|&t| t == '#').is_some() {
                        break;
                    }
                }
            }
            '(' => opens += 1,
            ')' => closes += 1,
            _ => {}
        }
    }

    assert_eq!(
        opens, closes,
        "grammars.scm: {opens} opens vs {closes} closes — unbalanced parens",
    );
}
