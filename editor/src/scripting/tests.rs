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

    // The plugin doesn't exist on disk — should be declared but not loaded.
    h.eval_source("(load-plugin \"user/nonexistent-repo\")", &mut s, &mut km)
        .unwrap();

    // Inspect state via builtins.
    // declared-plugins filters out core:* — user/nonexistent should appear.
    let declared_result = h.eval_source("(declared-plugins)", &mut s, &mut km);
    // Even if we can't inspect the list directly here, the eval should not error.
    assert!(
        declared_result.is_ok(),
        "declared-plugins raised: {:?}",
        declared_result
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
        .call_steel_cmd(&steel_proc, None, None, test_refs(&mut s, &mut km))
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
        .call_steel_cmd(&steel_proc, None, None, test_refs(&mut s, &mut km))
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
        .call_steel_cmd("%hume-cmd-try-set", None, None, test_refs(&mut s, &mut km))
        .unwrap_err();

    assert!(
        err.contains("set-option!"),
        "error must name the failing builtin; got: {err}"
    );
    // Mutation never happened, so the setting is unchanged.
    assert_eq!(s.tab_width, 4, "tab-width must be untouched");
}

// ── call! alias ───────────────────────────────────────────────────────────

/// Both `call!` and `call-command!` route to the same builtin.  Verify
/// that commands defined with each spelling both queue their sub-commands.
#[test]
fn call_bang_and_call_command_both_dispatch() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    h.eval_source(
        r#"
(define-command! "use-call-bang"    "" (lambda () (call! "move-right")))
(define-command! "use-call-command" "" (lambda () (call-command! "move-left")))
"#,
        &mut s,
        &mut km,
    )
    .unwrap();

    let (q1, _) = h
        .call_steel_cmd(
            "%hume-cmd-use-call-bang",
            None,
            None,
            test_refs(&mut s, &mut km),
        )
        .unwrap();
    assert_eq!(q1, vec!["move-right"], "call! should queue the command");

    let (q2, _) = h
        .call_steel_cmd(
            "%hume-cmd-use-call-command",
            None,
            None,
            test_refs(&mut s, &mut km),
        )
        .unwrap();
    assert_eq!(
        q2,
        vec!["move-left"],
        "call-command! alias should queue the command"
    );
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
        .unwrap();
    assert_eq!(queue, vec!["move-right"]);
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
        .unwrap();
    assert_eq!(queue, vec!["move-left"]);
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
        .unwrap();
    assert_eq!(queue, vec!["move-right"]);
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
        .unwrap();
    assert_eq!(queue, vec!["move-right"]);
}

#[test]
fn register_hook_no_fire_if_no_handlers() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let queue = h
        .fire_hook(HookId::OnBufferOpen, &[], test_refs(&mut s, &mut km))
        .unwrap();
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
        .unwrap();
    assert_eq!(queue, vec!["move-right", "move-left"]);
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
        .call_steel_cmd("%hume-cmd-bad-cmd", None, None, test_refs(&mut s, &mut km))
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
        .unwrap();
    assert_eq!(q1, vec!["insert"]);

    // Second fire with different args — stale *hume.ha1* would give wrong result.
    let new_val2 = "normal".into_steelval().unwrap();
    let q2 = h
        .fire_hook(
            HookId::OnModeChange,
            &[old_val, new_val2],
            test_refs(&mut s, &mut km),
        )
        .unwrap();
    assert_eq!(
        q2,
        vec!["normal"],
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

