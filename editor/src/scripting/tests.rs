use super::*;
use crate::editor::keymap::Keymap;
use crate::settings::EditorSettings;
use engine::pipeline::{BufferId, PaneId};

fn host() -> ScriptingHost {
    ScriptingHost::new()
}

/// Build a minimal `EditorSteelRefs` for tests that don't exercise
/// multi-buffer builtins (no `buffers` / `engine_view` / etc.).
fn test_refs<'a>(s: &'a mut EditorSettings, km: &'a mut Keymap) -> EditorSteelRefs<'a> {
    test_refs_with_bid(s, km, BufferId::default())
}

fn test_refs_with_bid<'a>(
    s: &'a mut EditorSettings,
    km: &'a mut Keymap,
    bid: BufferId,
) -> EditorSteelRefs<'a> {
    EditorSteelRefs {
        settings: s,
        keymap: km,
        focused_pane_id: PaneId::default(),
        focused_buffer_id: bid,
        buffers: None,
        engine_view: None,
        pane_state: None,
        pane_jumps: None,
    }
}

// ── set-option! ───────────────────────────────────────────────────────────

#[test]
fn set_option_tab_width_integer() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    assert_eq!(s.tab_width, 4);
    h.eval_source("(set-option! \"tab-width\" 2)", &mut s, &mut km)
        .unwrap();
    assert_eq!(s.tab_width, 2);
}

#[test]
fn set_option_tab_width_string() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    h.eval_source("(set-option! \"tab-width\" \"8\")", &mut s, &mut km)
        .unwrap();
    assert_eq!(s.tab_width, 8);
}

#[test]
fn set_option_bool_as_bool() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    assert!(s.mouse_enabled);
    h.eval_source("(set-option! \"mouse-enabled\" #f)", &mut s, &mut km)
        .unwrap();
    assert!(!s.mouse_enabled);
}

#[test]
fn set_option_unknown_key_errors() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let err = h
        .eval_source("(set-option! \"nonexistent\" \"val\")", &mut s, &mut km)
        .unwrap_err();
    assert!(err.contains("unknown setting"), "got: {err}");
}

// ── bind-key! ─────────────────────────────────────────────────────────────

#[test]
fn bind_key_does_not_error_on_valid_input() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    // A valid binding should succeed; the trie is verified via keymap's own tests.
    h.eval_source(
        "(bind-key! \"normal\" \"z\" \"move-right\")",
        &mut s,
        &mut km,
    )
    .unwrap();
}

#[test]
fn bind_key_multi_key_sequence_no_error() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    h.eval_source(
        "(bind-key! \"normal\" \"g h\" \"move-right\")",
        &mut s,
        &mut km,
    )
    .unwrap();
}

#[test]
fn bind_key_invalid_mode_errors() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let err = h
        .eval_source("(bind-key! \"visual\" \"f\" \"cmd\")", &mut s, &mut km)
        .unwrap_err();
    assert!(err.contains("mode"), "got: {err}");
}

#[test]
fn bind_key_invalid_key_sequence_errors() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let err = h
        .eval_source(
            "(bind-key! \"normal\" \"boguskey\" \"cmd\")",
            &mut s,
            &mut km,
        )
        .unwrap_err();
    assert!(!err.is_empty(), "expected error for unknown key 'boguskey'");
}

// ── load-plugin path resolution ────────────────────────────────────────────

#[test]
fn load_plugin_missing_plugin_declared_not_loaded() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    // Eval #1: declare an absent plugin.
    h.eval_source("(load-plugin \"user/nonexistent-repo\")", &mut s, &mut km)
        .unwrap();

    // Persistence check: the host field should contain the declared name even
    // before eval #2 (direct, independent oracle).
    assert!(
        h.declared_plugins
            .iter()
            .any(|d| d.eq_ignore_ascii_case("user/nonexistent-repo")),
        "declared_plugins field does not contain the declared name: {:?}",
        h.declared_plugins,
    );

    // Eval #2 (separate eval, mimicking PLUM command-time read): verify the
    // (declared-plugins) builtin sees persisted data across the eval boundary.
    h.eval_source(
        r#"(if (member "user/nonexistent-repo" (declared-plugins))
               (log! 'info "PERSISTED")
               (log! 'info "MISSING"))"#,
        &mut s,
        &mut km,
    )
    .unwrap();
    assert!(
        h.pending_messages
            .iter()
            .any(|(_, msg)| msg == "PERSISTED"),
        "(declared-plugins) did not see persisted name across eval boundary; messages: {:?}",
        h.pending_messages,
    );
}

#[test]
fn load_plugin_malformed_name_errors() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let err = h
        .eval_source("(load-plugin \"just-a-name\")", &mut s, &mut km)
        .unwrap_err();
    assert!(!err.is_empty(), "expected error for malformed plugin name");
}

// ── configure-statusline! ─────────────────────────────────────────────────

#[test]
fn configure_statusline_sets_left_section() {
    use crate::ui::statusline::StatusElement;
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    h.eval_source(
        r#"(configure-statusline! '("Mode" "FileName") '() '("Position"))"#,
        &mut s,
        &mut km,
    )
    .unwrap();

    assert_eq!(
        s.statusline.left,
        vec![StatusElement::Mode, StatusElement::FileName]
    );
    assert_eq!(s.statusline.center, vec![]);
    assert_eq!(s.statusline.right, vec![StatusElement::Position]);
}

#[test]
fn configure_statusline_all_sections() {
    use crate::ui::statusline::StatusElement;
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    h.eval_source(
        r#"(configure-statusline!
             '("Position" "FileName" "DirtyIndicator")
             '("SearchMatches")
             '("Separator" "Mode"))"#,
        &mut s,
        &mut km,
    )
    .unwrap();

    assert_eq!(
        s.statusline.left,
        vec![
            StatusElement::Position,
            StatusElement::FileName,
            StatusElement::DirtyIndicator
        ]
    );
    assert_eq!(s.statusline.center, vec![StatusElement::SearchMatches]);
    assert_eq!(
        s.statusline.right,
        vec![StatusElement::Separator, StatusElement::Mode]
    );
}

#[test]
fn configure_statusline_empty_sections() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    h.eval_source("(configure-statusline! '() '() '())", &mut s, &mut km)
        .unwrap();

    assert!(s.statusline.left.is_empty());
    assert!(s.statusline.center.is_empty());
    assert!(s.statusline.right.is_empty());
}

#[test]
fn configure_statusline_unknown_element_errors() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    let err = h
        .eval_source(
            r#"(configure-statusline! '("NotAnElement") '() '())"#,
            &mut s,
            &mut km,
        )
        .unwrap_err();
    assert!(err.contains("NotAnElement"), "got: {err}");
}

#[test]
fn configure_statusline_new_elements() {
    use crate::ui::statusline::StatusElement;
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    h.eval_source(
        r#"(configure-statusline! '("LineEnding") '() '("Cwd"))"#,
        &mut s,
        &mut km,
    )
    .unwrap();

    assert_eq!(s.statusline.left, vec![StatusElement::LineEnding]);
    assert_eq!(s.statusline.center, vec![]);
    assert_eq!(s.statusline.right, vec![StatusElement::Cwd]);
}

#[test]
fn configure_statusline_wrong_arity_errors() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    let err = h
        .eval_source("(configure-statusline! '())", &mut s, &mut km)
        .unwrap_err();
    assert!(!err.is_empty(), "expected arity error");
}

// ── hume/yield! ───────────────────────────────────────────────────────────

#[test]
fn hume_yield_no_interrupt_is_noop() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    // With no interrupt flag set, (hume/yield!) is a transparent no-op.
    h.eval_source("(hume/yield!)", &mut s, &mut km).unwrap();
}

#[test]
fn hume_yield_with_interrupt_errors() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    // Pre-set the interrupt flag before the eval.
    h.interrupt_flag.store(true, Ordering::Relaxed);
    let err = h.eval_source("(hume/yield!)", &mut s, &mut km).unwrap_err();
    assert!(
        err.contains("interrupted"),
        "expected 'interrupted' in error, got: {err}"
    );

    // eval_source resets the flag after every call.
    assert!(
        !h.interrupt_flag.load(Ordering::Relaxed),
        "flag should be false after eval"
    );
}

#[test]
fn hume_yield_stops_loop_when_interrupted() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    // Pre-set so the loop aborts on the very first yield call.
    h.interrupt_flag.store(true, Ordering::Relaxed);
    let err = h
        .eval_source(
            // Without the interrupt flag this loop would run forever.
            "(let loop () (hume/yield!) (loop))",
            &mut s,
            &mut km,
        )
        .unwrap_err();
    assert!(err.contains("interrupted"), "got: {err}");
}

#[test]
fn interrupt_flag_reset_after_eval() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    // Pre-set the flag; after eval_source it must be cleared.
    h.interrupt_flag.store(true, Ordering::Relaxed);
    h.eval_source("(hume/yield!)", &mut s, &mut km).unwrap_err(); // interrupted via pre-set flag
    assert!(
        !h.interrupt_flag.load(Ordering::Relaxed),
        "interrupt_flag must be false after eval_source returns"
    );

    // Subsequent evals with no flag pre-set should succeed normally.
    h.eval_source("(hume/yield!)", &mut s, &mut km).unwrap();
}

// ── command-plugin ────────────────────────────────────────────────────────

/// `(command-plugin name)` returns the owning plugin id for a Steel command.
#[test]
fn command_plugin_returns_plugin_owner_during_eval() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    // Register a command attributed to a plugin.
    h.eval_source(
        r#"(push-current-plugin! "user/myplugin")
           (define-command! "my-cmd" "test cmd" (lambda () (+ 1 0)))
           (pop-current-plugin!)"#,
        &mut s,
        &mut km,
    )
    .unwrap();

    // Verify the owner is queryable during a subsequent eval.
    // We can't call (command-plugin) from Rust directly at exec-time in
    // these unit tests, but we CAN call it during eval_source.
    let result = h.eval_source(r#"(command-plugin "my-cmd")"#, &mut s, &mut km);
    assert!(
        result.is_ok(),
        "command-plugin should not error: {:?}",
        result
    );
    // The owner is recorded in cmd_owners; verify via the map directly.
    assert_eq!(
        h.cmd_owners.get("my-cmd").map(|s| s.as_str()),
        Some("user/myplugin")
    );
}

/// Unknown (built-in) commands return "hume".
#[test]
fn command_plugin_unknown_returns_hume() {
    let h = host();

    // "move-right" is a Rust built-in — not in cmd_owners.
    assert!(!h.cmd_owners.contains_key("move-right"));
}

// ── define-command-extend! ────────────────────────────────────────────────

/// `define-command-extend!` sets `extendable: true` on the returned SteelCmdDef;
/// plain `define-command!` sets it to `false`.
#[test]
fn define_command_extend_sets_extendable_flag() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    let defs = h
        .eval_source_raw(
            r#"(define-command-extend! "ext-cmd" "doc" (lambda () (+ 1 0)))
           (define-command!        "plain-cmd" "doc" (lambda () (+ 1 0)))"#
                .to_owned(),
            Default::default(),
            &mut s,
            &mut km,
        )
        .expect("eval should succeed");

    let ext = defs
        .iter()
        .find(|d| d.name == "ext-cmd")
        .expect("ext-cmd not found");
    let plain = defs
        .iter()
        .find(|d| d.name == "plain-cmd")
        .expect("plain-cmd not found");
    assert!(
        ext.extendable,
        "define-command-extend! should set extendable = true"
    );
    assert!(
        !plain.extendable,
        "define-command! should set extendable = false"
    );
}

// ── define-command-inline-output! ─────────────────────────────────────────

/// `define-command-inline-output!` sets `inline_output: true` on the returned
/// SteelCmdDef; plain `define-command!` sets it to `false`.
#[test]
fn define_command_inline_output_sets_flag() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    let defs = h
        .eval_source_raw(
            r#"(define-command-inline-output! "inline-cmd" "doc" (lambda () (+ 1 0)))
           (define-command! "plain-cmd" "doc" (lambda () (+ 1 0)))"#
                .to_owned(),
            Default::default(),
            &mut s,
            &mut km,
        )
        .expect("eval should succeed");

    let inline = defs
        .iter()
        .find(|d| d.name == "inline-cmd")
        .expect("inline-cmd not found");
    let plain = defs
        .iter()
        .find(|d| d.name == "plain-cmd")
        .expect("plain-cmd not found");
    assert!(
        inline.inline_output,
        "define-command-inline-output! should set inline_output = true"
    );
    assert!(
        !plain.inline_output,
        "define-command! should set inline_output = false"
    );
}

// ── EvalWatchdog ──────────────────────────────────────────────────────────

/// Cancelling a watchdog with a long budget wakes the thread immediately.
/// Without `park_timeout` + `unpark`, this would block for the full budget.
#[test]
fn watchdog_cancel_wakes_thread_immediately() {
    let flag = Arc::new(AtomicBool::new(false));
    let budget = std::time::Duration::from_secs(10);
    let start = std::time::Instant::now();
    let watchdog = EvalWatchdog::arm(Arc::clone(&flag), budget);
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
fn eval_source_raw_watchdog_aborts_runaway() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let budget = std::time::Duration::from_millis(50);
    let start = std::time::Instant::now();

    let err = h
        .eval_source_watchdog(
            // This loop would run forever without the watchdog.
            "(let loop () (hume/yield!) (loop))",
            budget,
            &mut s,
            &mut km,
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
    // Flag must be reset after eval_source_raw returns.
    assert!(
        !h.interrupt_flag.load(Ordering::Relaxed),
        "interrupt_flag must be false after eval returns"
    );
}

/// call_steel_cmd watchdog fires and aborts a runaway Steel command.
#[test]
fn call_steel_cmd_watchdog_aborts_runaway() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    // Register a command whose body loops forever.
    h.eval_source(
        r#"(define-command! "spin" "spin forever" (lambda () (let loop () (hume/yield!) (loop))))"#,
        &mut s,
        &mut km,
    )
    .unwrap();
    let steel_proc = "%hume-cmd-spin".to_string();

    // Use a tight command budget.
    s.steel_command_budget_ms = 50;

    let start = std::time::Instant::now();
    let err = h
        .call_steel_cmd(&steel_proc, None, vec![], test_refs(&mut s, &mut km))
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
        !h.interrupt_flag.load(Ordering::Relaxed),
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
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    h.eval_source(
        r#"(define-command! "looper" "loop" (lambda () (let loop () (hume/yield!) (loop))))"#,
        &mut s,
        &mut km,
    )
    .unwrap();
    let steel_proc = "%hume-cmd-looper".to_string();

    assert_eq!(s.tab_width, 4, "precondition");
    s.steel_command_budget_ms = 50;

    let err = h
        .call_steel_cmd(&steel_proc, None, vec![], test_refs(&mut s, &mut km))
        .unwrap_err();

    assert!(
        err.contains("interrupted"),
        "expected 'interrupted', got: {err}"
    );
    assert_eq!(
        s.tab_width, 4,
        "tab-width must be unchanged after interrupt"
    );
}

/// Calling an init-only builtin from a Steel command body must raise a Steel
/// error (not panic).  `is_init = false` during call_steel_cmd, and init-only
/// builtins check this flag.
#[test]
fn call_steel_cmd_set_option_from_body_returns_steel_error() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    h.eval_source(
        r#"(define-command! "try-set" "" (lambda () (set-option! "tab-width" 8)))"#,
        &mut s,
        &mut km,
    )
    .unwrap();

    let err = h
        .call_steel_cmd("%hume-cmd-try-set", None, vec![], test_refs(&mut s, &mut km))
        .unwrap_err();

    assert!(
        err.contains("set-option!"),
        "error must name the failing builtin; got: {err}"
    );
    // Mutation never happened, so the setting is unchanged.
    assert_eq!(s.tab_width, 4, "tab-width must be untouched");
}

// ── call! ─────────────────────────────────────────────────────────────────

/// The variadic `call!` macro desugars to `%call!` and correctly binds
/// positional args as `*hume.ca{i}*` globals, passing them into the invoked
/// lambda.  This is the independent oracle for the arg-binding splice in
/// `call_steel_cmd`: the expected `cmd_queue` is derived from the input args,
/// not from re-reading the implementation.
///
/// Verification validity: changing "hello" in the assert to "world" makes the test fail.
#[test]
fn call_bang_passes_args_to_command() {
    use steel::rvals::SteelVal;
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    // Define a command that takes one arg x and calls (call! x).
    // The lambda receives x as *hume.ca0*, then queues x as a command name.
    h.eval_source(
        r#"(define-command! "echo-arg" "" (lambda (x) (call! x)))"#,
        &mut s,
        &mut km,
    )
    .unwrap();

    let result = h
        .call_steel_cmd(
            "%hume-cmd-echo-arg",
            None,
            vec![SteelVal::StringV("hello".into())],
            test_refs(&mut s, &mut km),
        )
        .unwrap();

    assert_eq!(
        result.cmd_queue,
        vec![("hello".to_string(), vec![])],
        "call! should queue the arg value as a command name"
    );
}

#[test]
fn call_bang_forwards_multiple_args_to_lambda() {
    // Tests the multi-arg *hume.ca{i}* splice for i > 0.
    // Oracle: each queued command name equals the corresponding input arg.
    // Verification: change "z" to "w" in the assert → test fails.
    use steel::rvals::SteelVal;
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    h.eval_source(
        r#"(define-command! "route-three" "" (lambda (a b c) (call! a) (call! b) (call! c)))"#,
        &mut s,
        &mut km,
    )
    .unwrap();

    let result = h
        .call_steel_cmd(
            "%hume-cmd-route-three",
            None,
            vec![
                SteelVal::StringV("x".into()),
                SteelVal::StringV("y".into()),
                SteelVal::StringV("z".into()),
            ],
            test_refs(&mut s, &mut km),
        )
        .unwrap();

    assert_eq!(
        result.cmd_queue,
        vec![
            ("x".to_string(), vec![]),
            ("y".to_string(), vec![]),
            ("z".to_string(), vec![]),
        ],
        "each arg must reach the corresponding lambda parameter"
    );
}

#[test]
fn call_bang_arity_mismatch_surfaces_steel_error() {
    // Passing fewer args than the lambda declares is a Steel VM arity error.
    // Verifies call_steel_cmd propagates it rather than silently misbehaving.
    use steel::rvals::SteelVal;
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    h.eval_source(
        r#"(define-command! "needs-two" "" (lambda (a b) (call! "move-right")))"#,
        &mut s,
        &mut km,
    )
    .unwrap();

    let err = h
        .call_steel_cmd(
            "%hume-cmd-needs-two",
            None,
            vec![SteelVal::StringV("only-one".into())],
            test_refs(&mut s, &mut km),
        )
        .unwrap_err();

    assert!(!err.is_empty(), "expected a Steel arity error, got empty string");
}

// ── register-hook! / fire_hook ────────────────────────────────────────────

use crate::scripting::builtins::ids::SteelBufferId;
use crate::scripting::hooks::HookId;

#[test]
fn register_hook_fires_on_buffer_open() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    h.eval_source(
        r#"(register-hook! 'on-buffer-open (lambda (bid) (call! "move-right")))"#,
        &mut s,
        &mut km,
    )
    .unwrap();
    let bid = BufferId::default();
    let val = SteelBufferId(bid).into_steel_val();
    let queue = h
        .fire_hook(
            HookId::OnBufferOpen,
            &[val],
            test_refs_with_bid(&mut s, &mut km, bid),
        )
        .unwrap()
        .cmd_queue;
    assert_eq!(queue, vec![("move-right".to_string(), vec![])]);
}

#[test]
fn register_hook_fires_on_buffer_close() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    h.eval_source(
        r#"(register-hook! 'on-buffer-close (lambda (bid) (call! "move-left")))"#,
        &mut s,
        &mut km,
    )
    .unwrap();
    let bid = BufferId::default();
    let val = SteelBufferId(bid).into_steel_val();
    let queue = h
        .fire_hook(
            HookId::OnBufferClose,
            &[val],
            test_refs_with_bid(&mut s, &mut km, bid),
        )
        .unwrap()
        .cmd_queue;
    assert_eq!(queue, vec![("move-left".to_string(), vec![])]);
}

#[test]
fn register_hook_fires_on_buffer_save() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    h.eval_source(
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right")))"#,
        &mut s,
        &mut km,
    )
    .unwrap();
    let bid = BufferId::default();
    let val = SteelBufferId(bid).into_steel_val();
    let queue = h
        .fire_hook(
            HookId::OnBufferSave,
            &[val],
            test_refs_with_bid(&mut s, &mut km, bid),
        )
        .unwrap()
        .cmd_queue;
    assert_eq!(queue, vec![("move-right".to_string(), vec![])]);
}

#[test]
fn register_hook_fires_on_mode_change() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    h.eval_source(
        r#"(register-hook! 'on-mode-change
              (lambda (old new)
                (when (equal? new "insert") (call! "move-right"))))"#,
        &mut s,
        &mut km,
    )
    .unwrap();
    use steel::rvals::IntoSteelVal as _;
    let old_val = "normal".into_steelval().unwrap();
    let new_val = "insert".into_steelval().unwrap();
    let queue = h
        .fire_hook(
            HookId::OnModeChange,
            &[old_val, new_val],
            test_refs(&mut s, &mut km),
        )
        .unwrap()
        .cmd_queue;
    assert_eq!(queue, vec![("move-right".to_string(), vec![])]);
}

#[test]
fn register_hook_no_fire_if_no_handlers() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let queue = h
        .fire_hook(HookId::OnBufferOpen, &[], test_refs(&mut s, &mut km))
        .unwrap()
        .cmd_queue;
    assert!(queue.is_empty());
}

#[test]
fn register_hook_multiple_handlers_all_fire() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    h.eval_source(
        r#"
(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right")))
(register-hook! 'on-buffer-save (lambda (bid) (call! "move-left")))
"#,
        &mut s,
        &mut km,
    )
    .unwrap();
    let bid = BufferId::default();
    let val = SteelBufferId(bid).into_steel_val();
    let queue = h
        .fire_hook(
            HookId::OnBufferSave,
            &[val],
            test_refs_with_bid(&mut s, &mut km, bid),
        )
        .unwrap()
        .cmd_queue;
    assert_eq!(
        queue,
        vec![
            ("move-right".to_string(), vec![]),
            ("move-left".to_string(), vec![])
        ]
    );
}

#[test]
fn register_hook_errors_in_command_mode() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    // Define a command that tries to register a hook (not allowed in command mode).
    h.eval_source(
        r#"(define-command! "bad-cmd" "" (lambda ()
             (register-hook! 'on-buffer-open (lambda (bid) #f))))"#,
        &mut s,
        &mut km,
    )
    .unwrap();
    let err = h
        .call_steel_cmd("%hume-cmd-bad-cmd", None, vec![], test_refs(&mut s, &mut km))
        .unwrap_err();
    assert!(err.contains("can only be called during init"), "got: {err}");
}

#[test]
fn register_hook_unknown_name_errors() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let err = h
        .eval_source(
            r#"(register-hook! 'on-nonexistent (lambda () #f))"#,
            &mut s,
            &mut km,
        )
        .unwrap_err();
    assert!(err.contains("unknown hook"), "got: {err}");
}

#[test]
fn fire_hook_globals_cleared_between_fires() {
    // After each fire_hook call, *hume.ha0* / *hume.hp0* … must be Void.
    // Leaking them keeps Arc references alive (e.g. to a closed buffer)
    // and may surface stale data in subsequent fires with fewer args.
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    // Handler reads arg 0 and queues its string representation.
    h.eval_source(
        r#"(register-hook! 'on-mode-change (lambda (old new) (call! new)))"#,
        &mut s,
        &mut km,
    )
    .unwrap();
    use steel::rvals::IntoSteelVal as _;
    let old_val = "normal".into_steelval().unwrap();
    let new_val = "insert".into_steelval().unwrap();
    let q1 = h
        .fire_hook(
            HookId::OnModeChange,
            &[old_val.clone(), new_val],
            test_refs(&mut s, &mut km),
        )
        .unwrap()
        .cmd_queue;
    assert_eq!(q1, vec![("insert".to_string(), vec![])]);

    // Second fire with different args — stale *hume.ha1* would give wrong result.
    let new_val2 = "normal".into_steelval().unwrap();
    let q2 = h
        .fire_hook(
            HookId::OnModeChange,
            &[old_val, new_val2],
            test_refs(&mut s, &mut km),
        )
        .unwrap()
        .cmd_queue;
    assert_eq!(
        q2,
        vec![("normal".to_string(), vec![])],
        "second fire must not see stale globals from first"
    );
}

// ── bind-key-extend! ──────────────────────────────────────────────────────

#[test]
fn bind_key_extend_creates_force_extending_leaf() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    h.eval_source(
        r#"(bind-key-extend! "normal" "z" "select-line")"#,
        &mut s,
        &mut km,
    )
    .unwrap();
    use crate::editor::keymap::BindMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let z_key = &[KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE)];
    let (name, force_extend) = km
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
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    h.eval_source(r#"(bind-key! "normal" "z" "select-line")"#, &mut s, &mut km)
        .unwrap();
    use crate::editor::keymap::BindMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let z_key = &[KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE)];
    let (_, force_extend) = km
        .lookup_command(BindMode::Normal, z_key)
        .expect("z must be bound after bind-key!");
    assert!(!force_extend, "bind-key! must produce force_extend = false");
}

#[test]
fn bind_key_extend_invalid_mode_errors() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let err = h
        .eval_source(r#"(bind-key-extend! "visual" "f" "cmd")"#, &mut s, &mut km)
        .unwrap_err();
    assert!(err.contains("mode"), "got: {err}");
}

// ── unbind-key! ───────────────────────────────────────────────────────────

#[test]
fn unbind_key_removes_default_binding() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    use crate::editor::keymap::BindMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let h_key = &[KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)];
    assert!(
        km.lookup_command(BindMode::Normal, h_key).is_some(),
        "'h' must be bound by default"
    );

    h.eval_source(r#"(unbind-key! "normal" "h")"#, &mut s, &mut km)
        .unwrap();

    assert!(
        km.lookup_command(BindMode::Normal, h_key).is_none(),
        "'h' must be unbound after unbind-key!"
    );
}

#[test]
fn unbind_key_noop_on_already_unbound() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    // 'Q' is not in the default keymap.
    h.eval_source(r#"(unbind-key! "normal" "Q")"#, &mut s, &mut km)
        .unwrap();
}

#[test]
fn unbind_key_invalid_mode_errors() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let err = h
        .eval_source(r#"(unbind-key! "visual" "h")"#, &mut s, &mut km)
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
    use crate::editor::keymap::BindMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    h.eval_source(PRELUDE_MACROS, &mut s, &mut km).unwrap();
    h.eval_source(
        r#"(bind-keys! "normal"
             ("z z" "move-left")
             ("z l" "move-right"))"#,
        &mut s,
        &mut km,
    )
    .unwrap();

    let z = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE);
    let l = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE);

    let (name, fe) = km
        .lookup_command(BindMode::Normal, &[z, z])
        .expect("\"z z\" must be bound after bind-keys!");
    assert_eq!(name, "move-left");
    assert!(!fe, "bind-keys! must not force extend");

    let (name2, _) = km
        .lookup_command(BindMode::Normal, &[z, l])
        .expect("\"z l\" must be bound after bind-keys!");
    assert_eq!(name2, "move-right");
}

#[test]
fn prelude_bind_keys_extend_creates_force_extend_leaves() {
    use crate::editor::keymap::BindMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    h.eval_source(PRELUDE_MACROS, &mut s, &mut km).unwrap();
    h.eval_source(
        r#"(bind-keys-extend! "normal"
             ("Q" "select-line")
             ("W" "select-to-end"))"#,
        &mut s,
        &mut km,
    )
    .unwrap();

    let q = &[KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE)];
    let w = &[KeyEvent::new(KeyCode::Char('W'), KeyModifiers::NONE)];

    let (name, fe) = km
        .lookup_command(BindMode::Normal, q)
        .expect("\"Q\" must be bound after bind-keys-extend!");
    assert_eq!(name, "select-line");
    assert!(fe, "bind-keys-extend! must produce force_extend = true");

    let (name2, fe2) = km
        .lookup_command(BindMode::Normal, w)
        .expect("\"W\" must be bound after bind-keys-extend!");
    assert_eq!(name2, "select-to-end");
    assert!(fe2, "bind-keys-extend! must produce force_extend = true");
}

#[test]
fn prelude_unbind_keys_batch_removes_bindings() {
    use crate::editor::keymap::BindMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    h.eval_source(PRELUDE_MACROS, &mut s, &mut km).unwrap();

    // 'h' and 'l' are default normal-mode bindings.
    let h_key = &[KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)];
    let l_key = &[KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE)];
    assert!(
        km.lookup_command(BindMode::Normal, h_key).is_some(),
        "'h' must be bound by default"
    );
    assert!(
        km.lookup_command(BindMode::Normal, l_key).is_some(),
        "'l' must be bound by default"
    );

    h.eval_source(
        r#"(unbind-keys! "normal" "h" "l")"#,
        &mut s,
        &mut km,
    )
    .unwrap();

    assert!(
        km.lookup_command(BindMode::Normal, h_key).is_none(),
        "'h' must be unbound after unbind-keys!"
    );
    assert!(
        km.lookup_command(BindMode::Normal, l_key).is_none(),
        "'l' must be unbound after unbind-keys!"
    );
}

/// Verify the prelude eval_init → init.scm eval_init sequence: prelude macros
/// defined by the first eval_init are available in the second.
#[test]
fn prelude_eval_init_sequence_makes_macros_available_to_init_scm() {
    use std::io::Write as _;

    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let builtin_names: std::collections::HashSet<String> = Default::default();

    // Stage a prelude file and an init.scm that uses its macros.
    let dir = tempfile::tempdir().unwrap();
    let prelude_path = dir.path().join("prelude.scm");
    let init_path = dir.path().join("init.scm");

    std::fs::write(&prelude_path, PRELUDE_MACROS).unwrap();
    let mut f = std::fs::File::create(&init_path).unwrap();
    writeln!(f, r#"(bind-keys! "normal" ("Q Q" "move-left") ("Q W" "move-right"))"#).unwrap();

    // Load prelude first, then init.scm — mirroring init_scripting's sequence.
    let prelude_cmds = h
        .eval_init(&prelude_path, &mut s, &mut km, builtin_names.clone())
        .expect("prelude eval_init must succeed");
    assert!(
        prelude_cmds.is_empty(),
        "prelude must define no commands; got {:?}",
        prelude_cmds.iter().map(|d| &d.name).collect::<Vec<_>>()
    );

    h.eval_init(&init_path, &mut s, &mut km, builtin_names)
        .expect("init.scm using bind-keys! must succeed after prelude is loaded");

    use crate::editor::keymap::BindMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let q = KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE);
    let w = KeyEvent::new(KeyCode::Char('W'), KeyModifiers::NONE);

    let (name1, _) = km
        .lookup_command(BindMode::Normal, &[q, q])
        .expect("\"Q Q\" must be bound via bind-keys! from init.scm");
    assert_eq!(name1, "move-left");

    let (name2, _) = km
        .lookup_command(BindMode::Normal, &[q, w])
        .expect("\"Q W\" must be bound via bind-keys! from init.scm");
    assert_eq!(name2, "move-right");
}

/// When init.scm uses bind-keys! but the prelude was never loaded, the eval
/// fails with a clear error (macro undefined) — not a panic.
#[test]
fn bind_keys_without_prelude_fails_gracefully() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    // bind-keys! is NOT defined — init.scm uses it directly.
    let err = h
        .eval_source(
            r#"(bind-keys! "normal" ("z" "move-left"))"#,
            &mut s,
            &mut km,
        )
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
    h.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    let cmds = h
        .eval_init(&init_path, &mut s, &mut km, Default::default())
        .expect("eager load must succeed");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(h.lazy_registry.plugins.get(&id), Some(lazy::PluginState::Loaded)),
        "plugin must reach Loaded state; got {:?}",
        h.lazy_registry.plugins.get(&id)
    );
    assert!(
        cmds.iter().any(|d| d.name == "tp-cmd"),
        "tp-cmd must be in returned defs; got {:?}",
        cmds.iter().map(|d| &d.name).collect::<Vec<_>>()
    );
}

/// `(declare-plugin "user/tp")` bare → plugin stays `Declared`, body is
/// NOT evaluated, and its commands are absent from the init result.
#[test]
#[cfg(not(windows))]
fn lazy_load_stays_declared_body_not_evaluated() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp")"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    let cmds = h
        .eval_init(&init_path, &mut s, &mut km, Default::default())
        .expect("lazy load must not error during init");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(h.lazy_registry.plugins.get(&id), Some(lazy::PluginState::Declared { .. })),
        "plugin must stay Declared; got {:?}",
        h.lazy_registry.plugins.get(&id)
    );
    assert!(
        !cmds.iter().any(|d| d.name == "tp-cmd"),
        "tp-cmd must NOT appear in init defs for a lazy plugin"
    );
}

/// `(declare-plugin "user/tp" #:on-command '("my-cmd"))` → plugin stays lazy,
/// `command_triggers["my-cmd"]` maps to the plugin, body not evaluated.
#[test]
#[cfg(not(windows))]
fn on_command_trigger_populates_registry_body_not_evaluated() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:on-command '("my-cmd"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    let cmds = h
        .eval_init(&init_path, &mut s, &mut km, Default::default())
        .expect("on-command declaration must not error during init");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(h.lazy_registry.plugins.get(&id), Some(lazy::PluginState::Declared { .. })),
        "plugin with on-command trigger must stay Declared; got {:?}",
        h.lazy_registry.plugins.get(&id)
    );
    assert_eq!(
        h.lazy_registry.command_triggers.get("my-cmd"),
        Some(&id),
        "command_triggers must map my-cmd to the plugin"
    );
    assert!(
        !cmds.iter().any(|d| d.name == "tp-cmd"),
        "tp-cmd must NOT appear in init defs for an on-command plugin"
    );
}

/// `activate_plugin` on a `Declared` lazy plugin → state transitions to
/// `Loaded`, returns the plugin's `SteelCmdDef`s.  Second call → idempotent
/// `Ok(vec![])`.
#[test]
#[cfg(not(windows))]
fn activate_plugin_idempotent_on_declared_lazy_plugin() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp")"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    h.eval_init(&init_path, &mut s, &mut km, Default::default())
        .expect("init must succeed");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };

    // First activation: Declared → Loaded, returns the plugin's command.
    let cmds = h
        .activate_plugin(&id, &mut s, &mut km, &Default::default(), 5_000)
        .expect("activate_plugin must succeed");
    assert!(
        matches!(h.lazy_registry.plugins.get(&id), Some(lazy::PluginState::Loaded)),
        "plugin must be Loaded after activate_plugin; got {:?}",
        h.lazy_registry.plugins.get(&id)
    );
    assert!(
        cmds.iter().any(|d| d.name == "tp-cmd"),
        "tp-cmd must be returned by activate_plugin"
    );

    // Second activation: already Loaded → idempotent Ok(vec![]).
    let cmds2 = h
        .activate_plugin(&id, &mut s, &mut km, &Default::default(), 5_000)
        .expect("second activate_plugin must succeed");
    assert!(
        cmds2.is_empty(),
        "second activate_plugin must return empty vec (idempotent)"
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
    h.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    let result = h.eval_init(&init_path, &mut s, &mut km, Default::default());
    assert!(result.is_err(), "init must fail when eager plugin body errors");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(h.lazy_registry.plugins.get(&id), Some(lazy::PluginState::Failed)),
        "plugin must be Failed after body error; got {:?}",
        h.lazy_registry.plugins.get(&id)
    );
}

// ── Phase 1 lazy plugin loading — command triggers ────────────────────────
//
// Not on Windows: Scheme require strings embed OS paths; backslashes are not
// escaped in Steel string literals.

/// `#:on-command '("move-right")` — name clashes with a built-in → colliding
/// trigger is dropped, a `Severity::Error` is logged, init continues.
///
/// Flip: a non-builtin name produces no Error and the trigger is registered.
#[test]
#[cfg(not(windows))]
fn manifest_collision_with_builtin_logs_error_continues() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:on-command '("move-right"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut h = host();
    h.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let builtin_names: std::collections::HashSet<String> =
        ["move-right".to_string()].into_iter().collect();
    h.eval_init(&init_path, &mut s, &mut km, builtin_names)
        .expect("builtin collision must NOT abort init");

    // Error logged for the dropped trigger.
    assert!(
        h.pending_messages.iter().any(|(sev, msg)| {
            matches!(sev, crate::editor::Severity::Error)
                && msg.contains("move-right")
                && msg.contains("built-in")
        }),
        "expected an Error about 'move-right' conflicting with a built-in; got: {:?}",
        h.pending_messages
    );
    // Trigger not written (no cmd_owners pollution, no command_triggers entry).
    assert!(
        !h.lazy_registry.command_triggers.contains_key("move-right"),
        "colliding trigger must not appear in command_triggers"
    );
    // Plugin stays dead-lazy — Declared, body not evaluated.
    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(
            h.lazy_registry.plugins.get(&id),
            Some(lazy::PluginState::Declared { .. })
        ),
        "plugin must stay Declared (dead-lazy) after all-colliding on-command list"
    );

    // Flip: non-colliding trigger produces no Error and is registered.
    let (dir2, init_path2) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:on-command '("not-a-builtin"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut h2 = host();
    h2.data_dir = Some(dir2.path().to_path_buf());
    let mut s2 = EditorSettings::default();
    let mut km2 = Keymap::default();
    let builtin_names2: std::collections::HashSet<String> =
        ["move-right".to_string()].into_iter().collect();
    h2.eval_init(&init_path2, &mut s2, &mut km2, builtin_names2)
        .expect("non-colliding trigger must not error");
    assert!(
        !h2.pending_messages
            .iter()
            .any(|(sev, _)| matches!(sev, crate::editor::Severity::Error)),
        "non-colliding trigger must not log any Error"
    );
    assert!(
        h2.lazy_registry.command_triggers.contains_key("not-a-builtin"),
        "non-colliding trigger must appear in command_triggers"
    );
}

/// Two plugins both declare `#:on-command '("bar")` → second declaration's
/// trigger is dropped, a `Severity::Error` is logged, first-writer-wins, init
/// continues.
///
/// Flip: both plugins are Declared; only the first plugin owns the trigger.
#[test]
#[cfg(not(windows))]
fn manifest_collision_lazy_vs_lazy_logs_error_continues() {
    let dir = tempfile::tempdir().unwrap();
    let pa = dir.path().join("plugins").join("user").join("pa");
    let pb = dir.path().join("plugins").join("user").join("pb");
    std::fs::create_dir_all(&pa).unwrap();
    std::fs::create_dir_all(&pb).unwrap();
    std::fs::write(pa.join("plugin.scm"), r#"(define-command! "tp-a" "doc" (lambda () (+ 1 0)))"#).unwrap();
    std::fs::write(pb.join("plugin.scm"), r#"(define-command! "tp-b" "doc" (lambda () (+ 1 0)))"#).unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(&init_path, r#"
(declare-plugin "user/pa" #:on-command '("bar"))
(declare-plugin "user/pb" #:on-command '("bar"))
"#).unwrap();

    let mut h = host();
    h.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    h.eval_init(&init_path, &mut s, &mut km, Default::default())
        .expect("lazy-vs-lazy collision must NOT abort init");

    // Error logged for pb's duplicate trigger.
    assert!(
        h.pending_messages.iter().any(|(sev, msg)| {
            matches!(sev, crate::editor::Severity::Error)
                && msg.contains("bar")
                && msg.contains("already claimed")
        }),
        "expected an Error about 'bar' already claimed; got: {:?}",
        h.pending_messages
    );
    // First-writer (pa) owns the trigger.
    let pa_id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "pa".to_string(),
    };
    assert_eq!(
        h.lazy_registry.command_triggers.get("bar"),
        Some(&pa_id),
        "command_triggers['bar'] must point to pa (first-writer-wins)"
    );
    // Both plugins are Declared — pb stays declared even though its trigger was dropped.
    let pb_id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "pb".to_string(),
    };
    assert!(
        matches!(
            h.lazy_registry.plugins.get(&pa_id),
            Some(lazy::PluginState::Declared { .. })
        ),
        "pa must be Declared"
    );
    assert!(
        matches!(
            h.lazy_registry.plugins.get(&pb_id),
            Some(lazy::PluginState::Declared { .. })
        ),
        "pb must be Declared even with its trigger dropped"
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
        r#"(declare-plugin "user/tp" #:on-command '("bar"))"#,
        r#"(define-command! "bar" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut h = host();
    h.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    h.eval_init(&init_path, &mut s, &mut km, Default::default())
        .expect("init must succeed");

    // Plugin has NOT been activated yet — body was not evaluated.
    let owner = h.cmd_owners.get("bar").map(|s| s.as_str());
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

/// `activate_plugin` drops the plugin's `command_triggers` entry after the
/// plugin body is evaluated successfully.
#[test]
#[cfg(not(windows))]
fn activate_plugin_drops_command_trigger_on_loaded() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:on-command '("my-cmd"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut h = host();
    h.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    h.eval_init(&init_path, &mut s, &mut km, Default::default())
        .expect("init must succeed");

    // Trigger is present before activation.
    assert!(
        h.lazy_registry.command_triggers.contains_key("my-cmd"),
        "trigger must be present before activation"
    );

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    h.activate_plugin(&id, &mut s, &mut km, &Default::default(), 5_000)
        .expect("activate_plugin must succeed");

    // Trigger is removed after activation.
    assert!(
        !h.lazy_registry.command_triggers.contains_key("my-cmd"),
        "trigger must be removed after activation"
    );
}


/// `(declare-plugin "user/tp" #:on-language '("rust"))` → plugin stays lazy,
/// `language_triggers["rust"]` contains the plugin, body not evaluated.
///
/// Flip: if on-language were not threaded through `%declare-plugin!`, the
/// plugin would stay Declared but with an empty language_triggers map.
#[test]
#[cfg(not(windows))]
fn on_language_trigger_populates_registry_body_not_evaluated() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:on-language '("rust"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    let cmds = h
        .eval_init(&init_path, &mut s, &mut km, Default::default())
        .expect("on-language declaration must not error during init");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(h.lazy_registry.plugins.get(&id), Some(lazy::PluginState::Declared { .. })),
        "plugin with on-language trigger must stay Declared; got {:?}",
        h.lazy_registry.plugins.get(&id)
    );
    assert!(
        h.lazy_registry
            .language_triggers
            .get("rust")
            .is_some_and(|v| v.contains(&id)),
        "language_triggers must map \"rust\" to the plugin"
    );
    assert!(
        !cmds.iter().any(|d| d.name == "tp-cmd"),
        "tp-cmd must NOT appear in init defs for an on-language plugin"
    );
}

/// `activate_plugin` on a language-triggered plugin drops the trigger on success.
///
/// Flip: without the `language_triggers.retain` in the Ok branch, the trigger
/// would survive activation and falsely appear pending on subsequent language sets.
#[test]
#[cfg(not(windows))]
fn activate_plugin_drops_language_trigger_on_loaded() {
    let (dir, init_path) = plugin_fixture(
        r#"(declare-plugin "user/tp" #:on-language '("rust"))"#,
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );
    let mut h = host();
    h.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    h.eval_init(&init_path, &mut s, &mut km, Default::default())
        .expect("init must succeed");

    assert!(
        h.lazy_registry.language_triggers.contains_key("rust"),
        "trigger must be present before activation"
    );

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    h.activate_plugin(&id, &mut s, &mut km, &Default::default(), 5_000)
        .expect("activate_plugin must succeed");

    assert!(
        !h.lazy_registry.language_triggers.contains_key("rust"),
        "trigger must be removed after activation"
    );
}

/// `(load-plugin "x")` after `(declare-plugin "x" #:on-command …)` force-activates
/// the plugin: state transitions to `Loaded` and the command trigger is cleared.
///
/// Flip: without the pending_plugin_loads path in `load_plugin`, the plugin would
/// stay `Declared` and the trigger would remain.
#[test]
#[cfg(not(windows))]
fn load_plugin_force_activates_declared_plugin() {
    let (dir, init_path) = plugin_fixture(
        "(declare-plugin \"user/tp\" #:on-command '(\"my-cmd\"))\n(load-plugin \"user/tp\")",
        r#"(define-command! "tp-cmd" "doc" (lambda () (+ 1 0)))"#,
    );

    let mut h = host();
    h.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    let cmds = h
        .eval_init(&init_path, &mut s, &mut km, Default::default())
        .expect("force-activate must succeed");

    let id = attribution::PluginId::User {
        user: "user".to_string(),
        repo: "tp".to_string(),
    };
    assert!(
        matches!(h.lazy_registry.plugins.get(&id), Some(lazy::PluginState::Loaded)),
        "plugin must be Loaded after explicit load-plugin; got {:?}",
        h.lazy_registry.plugins.get(&id)
    );
    assert!(
        !h.lazy_registry.command_triggers.contains_key("my-cmd"),
        "command trigger must be cleared after activation"
    );
    assert!(
        cmds.iter().any(|d| d.name == "tp-cmd"),
        "tp-cmd must be in returned defs after force-activate"
    );
}

/// `(load-plugin "absent-dep")` inside a plugin body (non-top-level) → hard error.
///
/// Flip: with top-level silent-skip applied everywhere, this would return `Ok`
/// and the dependency would silently not load, breaking the dependent plugin.
#[test]
#[cfg(not(windows))]
fn load_plugin_in_body_absent_dep_errors() {
    // Plugin B calls (load-plugin "user/dep") in its body but dep doesn't exist.
    let dir = tempfile::tempdir().unwrap();
    let plugin_b_dir = dir.path().join("plugins").join("user").join("pb");
    std::fs::create_dir_all(&plugin_b_dir).unwrap();
    std::fs::write(
        plugin_b_dir.join("plugin.scm"),
        r#"(load-plugin "user/dep-absent")"#,
    )
    .unwrap();
    let init_path = dir.path().join("init.scm");
    std::fs::write(&init_path, r#"(load-plugin "user/pb")"#).unwrap();

    let mut h = host();
    h.data_dir = Some(dir.path().to_path_buf());
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let Err(msg) = h.eval_init(&init_path, &mut s, &mut km, Default::default()) else {
        panic!("load-plugin for absent dep inside plugin body must error");
    };
    assert!(
        msg.contains("not found on disk") || msg.contains("dep-absent"),
        "error must mention the absent dependency; got: {msg}"
    );
}

/// `(load-plugin …)` raises a Steel error when called from a command body
/// (`is_init = false`), mirroring the `register-hook!` guard.
///
/// Flip: removing the `if !ctx.is_init` guard in `load_plugin` would let
/// this return `Ok`, silently queuing a request that is never drained.
#[test]
fn load_plugin_runtime_guard_fires() {
    use crate::scripting::SteelCtxTestHarness;
    use crate::scripting::builtins::plugins::load_plugin;

    // SteelCtxTestHarness builds an is_init=false (command) context.
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = load_plugin(&mut ctx, "user/tp".to_string());
    assert!(
        result.is_err(),
        "load-plugin must error when called outside init/plugin load (is_init=false)"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("init/plugin load"),
        "error must mention init/plugin load context; got: {msg}"
    );
}
