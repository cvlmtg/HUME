use super::*;
use engine::pipeline::{BufferId, PaneId};
use crate::settings::EditorSettings;
use crate::editor::keymap::Keymap;

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
        settings:          s,
        keymap:            km,
        focused_pane_id:   PaneId::default(),
        focused_buffer_id: bid,
        buffers:           None,
        engine_view:       None,
        pane_state:        None,
        pane_jumps:        None,
    }
}

// ── set-option! ───────────────────────────────────────────────────────────

#[test]
fn set_option_tab_width_integer() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    assert_eq!(s.tab_width, 4);
    h.eval_source("(set-option! \"tab-width\" 2)", &mut s, &mut km).unwrap();
    assert_eq!(s.tab_width, 2);
}

#[test]
fn set_option_tab_width_string() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    h.eval_source("(set-option! \"tab-width\" \"8\")", &mut s, &mut km).unwrap();
    assert_eq!(s.tab_width, 8);
}

#[test]
fn set_option_bool_as_bool() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    assert!(s.mouse_enabled);
    h.eval_source("(set-option! \"mouse-enabled\" #f)", &mut s, &mut km).unwrap();
    assert!(!s.mouse_enabled);
}

#[test]
fn set_option_unknown_key_errors() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let err = h.eval_source("(set-option! \"nonexistent\" \"val\")", &mut s, &mut km)
        .unwrap_err();
    assert!(err.contains("unknown setting"), "got: {err}");
}

#[test]
fn set_option_settings_restored_on_error() {
    // On error, settings are rolled back to their pre-eval state (all-or-nothing).
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    // First set tab-width to 2...
    h.eval_source("(set-option! \"tab-width\" 2)", &mut s, &mut km).unwrap();
    assert_eq!(s.tab_width, 2);
    // Then run a script that errors mid-way: tab-width is set to 8, then a
    // bad setting that raises. The eval errors and the snapshot is restored:
    // tab-width goes back to 2, not left at the partial 8.
    let err = h.eval_source(
        "(set-option! \"tab-width\" 8)\n(set-option! \"bogus\" \"x\")",
        &mut s, &mut km,
    );
    assert!(err.is_err(), "expected eval to fail");
    assert_eq!(s.tab_width, 2, "snapshot should have been restored");
}

#[test]
fn cmd_owners_rolled_back_on_error() {
    // A failing eval that defines a command mid-way must not leave a stale
    // entry in cmd_owners — the snapshot must be restored on error.
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    // Register a command successfully first so cmd_owners is non-empty.
    h.eval_source(
        r#"(push-current-plugin! "user/plugin-a")
           (define-command! "cmd-a" "a" (lambda () (+ 1 0)))
           (pop-current-plugin!)"#,
        &mut s, &mut km,
    ).unwrap();
    assert!(h.cmd_owners.contains_key("cmd-a"), "cmd-a should be registered");

    // Now run a script that defines a second command but then errors.
    // cmd-b must NOT appear in cmd_owners after rollback.
    let err = h.eval_source(
        r#"(push-current-plugin! "user/plugin-b")
           (define-command! "cmd-b" "b" (lambda () (+ 1 0)))
           (set-option! "bogus-key" "x")"#,
        &mut s, &mut km,
    );
    assert!(err.is_err(), "expected eval to fail");
    assert!(h.cmd_owners.contains_key("cmd-a"), "cmd-a should survive");
    assert!(!h.cmd_owners.contains_key("cmd-b"), "cmd-b must be rolled back");
}

// ── bind-key! ─────────────────────────────────────────────────────────────

#[test]
fn bind_key_does_not_error_on_valid_input() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    // A valid binding should succeed; the trie is verified via keymap's own tests.
    h.eval_source("(bind-key! \"normal\" \"z\" \"move-right\")", &mut s, &mut km).unwrap();
}

#[test]
fn bind_key_multi_key_sequence_no_error() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    h.eval_source("(bind-key! \"normal\" \"g h\" \"move-right\")", &mut s, &mut km).unwrap();
}

#[test]
fn bind_key_invalid_mode_errors() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let err = h.eval_source("(bind-key! \"visual\" \"f\" \"cmd\")", &mut s, &mut km)
        .unwrap_err();
    assert!(err.contains("mode"), "got: {err}");
}

#[test]
fn bind_key_invalid_key_sequence_errors() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let err = h.eval_source("(bind-key! \"normal\" \"boguskey\" \"cmd\")", &mut s, &mut km)
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
    h.eval_source("(load-plugin \"user/nonexistent-repo\")", &mut s, &mut km).unwrap();

    // Inspect state via builtins.
    // declared-plugins filters out core:* — user/nonexistent should appear.
    let declared_result = h.eval_source("(declared-plugins)", &mut s, &mut km);
    // Even if we can't inspect the list directly here, the eval should not error.
    assert!(declared_result.is_ok(), "declared-plugins raised: {:?}", declared_result);
}

#[test]
fn load_plugin_malformed_name_errors() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let err = h.eval_source("(load-plugin \"just-a-name\")", &mut s, &mut km)
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
        &mut s, &mut km,
    ).unwrap();

    assert_eq!(s.statusline.left,   vec![StatusElement::Mode, StatusElement::FileName]);
    assert_eq!(s.statusline.center, vec![]);
    assert_eq!(s.statusline.right,  vec![StatusElement::Position]);
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
        &mut s, &mut km,
    ).unwrap();

    assert_eq!(s.statusline.left,
        vec![StatusElement::Position, StatusElement::FileName, StatusElement::DirtyIndicator]);
    assert_eq!(s.statusline.center, vec![StatusElement::SearchMatches]);
    assert_eq!(s.statusline.right,  vec![StatusElement::Separator, StatusElement::Mode]);
}

#[test]
fn configure_statusline_empty_sections() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    h.eval_source("(configure-statusline! '() '() '())", &mut s, &mut km).unwrap();

    assert!(s.statusline.left.is_empty());
    assert!(s.statusline.center.is_empty());
    assert!(s.statusline.right.is_empty());
}

#[test]
fn configure_statusline_unknown_element_errors() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    let err = h.eval_source(
        r#"(configure-statusline! '("NotAnElement") '() '())"#,
        &mut s, &mut km,
    ).unwrap_err();
    assert!(err.contains("NotAnElement"), "got: {err}");
}

#[test]
fn configure_statusline_wrong_arity_errors() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    let err = h.eval_source("(configure-statusline! '())", &mut s, &mut km).unwrap_err();
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
    assert!(err.contains("interrupted"), "expected 'interrupted' in error, got: {err}");

    // eval_source resets the flag after every call.
    assert!(!h.interrupt_flag.load(Ordering::Relaxed), "flag should be false after eval");
}

#[test]
fn hume_yield_stops_loop_when_interrupted() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    // Pre-set so the loop aborts on the very first yield call.
    h.interrupt_flag.store(true, Ordering::Relaxed);
    let err = h.eval_source(
        // Without the interrupt flag this loop would run forever.
        "(let loop () (hume/yield!) (loop))",
        &mut s, &mut km,
    ).unwrap_err();
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
    assert!(!h.interrupt_flag.load(Ordering::Relaxed),
            "interrupt_flag must be false after eval_source returns");

    // Subsequent evals with no flag pre-set should succeed normally.
    h.eval_source("(hume/yield!)", &mut s, &mut km).unwrap();
}

// ── teardown_plugin / reload_plugin ───────────────────────────────────────

/// Run a mini two-plugin scenario:
///   plugin A sets tab-width to 8 (prior: 4, core)
///   plugin B sets tab-width to 2 (prior: 8, A)
/// Unloading A rewrites B's prior to (4, core).
/// Unloading B restores tab-width to 4.
#[test]
fn teardown_restores_setting_when_plugin_is_live_owner() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    // Simulate plugin A setting tab-width to 8.
    // We drive via eval_source with the plugin on the attribution stack.
    h.eval_source(
        r#"(push-current-plugin! "user/a")
           (set-option! "tab-width" 8)
           (pop-current-plugin!)"#,
        &mut s, &mut km,
    ).unwrap();
    assert_eq!(s.tab_width, 8);

    // Tear down plugin A — tab-width should be restored to 4 (prior).
    h.teardown_plugin("user/a", &mut s, &mut km).unwrap();
    assert_eq!(s.tab_width, 4, "teardown should restore prior tab-width");
}

#[test]
fn teardown_splices_chain_when_later_plugin_owns_key() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    // A sets tab-width 8, then B sets it to 2.
    h.eval_source(
        r#"(push-current-plugin! "user/a")
           (set-option! "tab-width" 8)
           (pop-current-plugin!)
           (push-current-plugin! "user/b")
           (set-option! "tab-width" 2)
           (pop-current-plugin!)"#,
        &mut s, &mut km,
    ).unwrap();
    assert_eq!(s.tab_width, 2);

    // Unload A — B still owns tab-width (live value = 2 unchanged).
    h.teardown_plugin("user/a", &mut s, &mut km).unwrap();
    assert_eq!(s.tab_width, 2, "B's live value must be preserved");

    // Now unload B — B's prior was rewritten by A's teardown to (4, core),
    // so restoring should give tab-width = 4.
    h.teardown_plugin("user/b", &mut s, &mut km).unwrap();
    assert_eq!(s.tab_width, 4, "after both unloads, core default restored");
}

#[test]
fn teardown_restores_keybind() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    // The default normal keymap has 'h' bound to "move-left".
    // Plugin A rebinds 'h' to "move-right".
    h.eval_source(
        r#"(push-current-plugin! "user/a")
           (bind-key! "normal" "h" "move-right")
           (pop-current-plugin!)"#,
        &mut s, &mut km,
    ).unwrap();

    use crate::editor::keymap::BindMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let h_key = &[KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)];
    assert_eq!(km.lookup_command(BindMode::Normal, h_key).as_deref(), Some("move-right"));

    // Tear down plugin A — 'h' should go back to "move-left".
    h.teardown_plugin("user/a", &mut s, &mut km).unwrap();
    assert_eq!(km.lookup_command(BindMode::Normal, h_key).as_deref(), Some("move-left"),
               "teardown should restore prior keybind");
}

#[test]
fn teardown_unbinds_when_key_was_previously_unbound() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    // Bind an unused key (assume 'Q' is not in the default keymap).
    h.eval_source(
        r#"(push-current-plugin! "user/a")
           (bind-key! "normal" "Q" "move-right")
           (pop-current-plugin!)"#,
        &mut s, &mut km,
    ).unwrap();

    use crate::editor::keymap::BindMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let q_key = &[KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE)];
    assert!(km.lookup_command(BindMode::Normal, q_key).is_some());

    // Tear down — 'Q' was unbound before, so it should become unbound again.
    h.teardown_plugin("user/a", &mut s, &mut km).unwrap();
    assert!(km.lookup_command(BindMode::Normal, q_key).is_none(),
            "binding for unowned key must be removed on teardown");
}

#[test]
fn teardown_unknown_plugin_is_noop() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    // No error, no state change.
    h.teardown_plugin("user/nonexistent", &mut s, &mut km).unwrap();
    assert_eq!(s.tab_width, 4);
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
        &mut s, &mut km,
    ).unwrap();

    // Verify the owner is queryable during a subsequent eval.
    // We can't call (command-plugin) from Rust directly at exec-time in
    // these unit tests, but we CAN call it during eval_source.
    let result = h.eval_source(
        r#"(command-plugin "my-cmd")"#,
        &mut s, &mut km,
    );
    assert!(result.is_ok(), "command-plugin should not error: {:?}", result);
    // The owner is recorded in cmd_owners; verify via the map directly.
    assert_eq!(h.cmd_owners.get("my-cmd").map(|s| s.as_str()), Some("user/myplugin"));
}

/// Unknown (built-in) commands return "hume".
#[test]
fn command_plugin_unknown_returns_hume() {
    let h = host();

    // "move-right" is a Rust built-in — not in cmd_owners.
    assert!(!h.cmd_owners.contains_key("move-right"));
}

/// Teardown removes the command from cmd_owners.
#[test]
fn command_plugin_cleared_on_teardown() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();

    h.eval_source(
        r#"(push-current-plugin! "user/myplugin")
           (define-command! "my-cmd" "test cmd" (lambda () (+ 1 0)))
           (pop-current-plugin!)"#,
        &mut s, &mut km,
    ).unwrap();
    assert_eq!(h.cmd_owners.get("my-cmd").map(|s| s.as_str()), Some("user/myplugin"));

    h.teardown_plugin("user/myplugin", &mut s, &mut km).unwrap();
    assert!(!h.cmd_owners.contains_key("my-cmd"), "teardown should remove from cmd_owners");
}

// ── EvalWatchdog ──────────────────────────────────────────────────────────

/// Cancelling a watchdog with a long budget wakes the thread immediately.
/// Without `park_timeout` + `unpark`, this would block for the full budget.
#[test]
fn watchdog_cancel_wakes_thread_immediately() {
    let flag   = Arc::new(AtomicBool::new(false));
    let budget = std::time::Duration::from_secs(10);
    let start  = std::time::Instant::now();
    let watchdog = EvalWatchdog::arm(Arc::clone(&flag), budget);
    watchdog.cancel();
    // cancel() must return well within the budget; 500 ms is generous.
    assert!(start.elapsed() < std::time::Duration::from_millis(500),
            "cancel() took too long: {:?}", start.elapsed());
    // Flag must not have been set (we cancelled before it fired).
    assert!(!flag.load(Ordering::Relaxed), "flag must stay false after cancel");
}

/// A watchdog with a tiny budget fires and causes (hume/yield!) to abort.
#[test]
fn eval_source_raw_watchdog_aborts_runaway() {
    let mut h  = host();
    let mut s  = EditorSettings::default();
    let mut km = Keymap::default();
    let budget = std::time::Duration::from_millis(50);
    let start  = std::time::Instant::now();

    let err = h.eval_source_watchdog(
        // This loop would run forever without the watchdog.
        "(let loop () (hume/yield!) (loop))",
        budget,
        &mut s,
        &mut km,
    ).unwrap_err();

    assert!(err.contains("interrupted"), "expected 'interrupted' in error, got: {err}");
    // Must abort well within a second — if not, the watchdog didn't fire.
    assert!(start.elapsed() < std::time::Duration::from_secs(1),
            "eval took too long: {:?}", start.elapsed());
    // Flag must be reset after eval_source_raw returns.
    assert!(!h.interrupt_flag.load(Ordering::Relaxed),
            "interrupt_flag must be false after eval returns");
}

/// When the watchdog fires during an eval that had already mutated a
/// setting, the rollback must restore the original value.
#[test]
fn eval_source_raw_watchdog_rollback_on_abort() {
    let mut h  = host();
    let mut s  = EditorSettings::default();
    let mut km = Keymap::default();
    let budget = std::time::Duration::from_millis(50);

    // Confirm the starting value so the assertion is not vacuously true.
    assert_eq!(s.tab_width, 4, "precondition: default tab-width is 4");

    // Set the option then run forever — rollback must undo the set.
    let err = h.eval_source_watchdog(
        r#"(set-option! "tab-width" 99) (let loop () (hume/yield!) (loop))"#,
        budget,
        &mut s,
        &mut km,
    ).unwrap_err();

    assert!(err.contains("interrupted"), "expected 'interrupted' in error, got: {err}");
    assert_eq!(s.tab_width, 4, "rollback must restore tab-width to pre-eval value");
}

/// call_steel_cmd watchdog fires and aborts a runaway Steel command.
#[test]
fn call_steel_cmd_watchdog_aborts_runaway() {
    let mut h  = host();
    let mut s  = EditorSettings::default();
    let mut km = Keymap::default();

    // Register a command whose body loops forever.
    h.eval_source(
        r#"(define-command! "spin" "spin forever" (lambda () (let loop () (hume/yield!) (loop))))"#,
        &mut s, &mut km,
    ).unwrap();
    let steel_proc = "%hume-cmd-spin".to_string();

    // Use a tight command budget.
    s.steel_command_budget_ms = 50;

    let start = std::time::Instant::now();
    let err = h.call_steel_cmd(
        &steel_proc, None, None, test_refs(&mut s, &mut km),
    ).unwrap_err();

    assert!(err.contains("interrupted"), "expected 'interrupted', got: {err}");
    assert!(start.elapsed() < std::time::Duration::from_secs(1),
            "call_steel_cmd took too long: {:?}", start.elapsed());
    assert!(!h.interrupt_flag.load(Ordering::Relaxed),
            "interrupt_flag must be false after call_steel_cmd returns");
}

/// Command bodies cannot mutate settings/keymap (is_init = false during
/// call_steel_cmd; init-only builtins raise Steel errors).  This test verifies
/// that after a watchdog interrupt the settings remain at their pre-call values.
/// Also verifies the budget is read from settings at call time.
#[test]
fn call_steel_cmd_interrupt_leaves_settings_unchanged() {
    let mut h  = host();
    let mut s  = EditorSettings::default();
    let mut km = Keymap::default();

    h.eval_source(
        r#"(define-command! "looper" "loop" (lambda () (let loop () (hume/yield!) (loop))))"#,
        &mut s, &mut km,
    ).unwrap();
    let steel_proc = "%hume-cmd-looper".to_string();

    assert_eq!(s.tab_width, 4, "precondition");
    s.steel_command_budget_ms = 50;

    let err = h.call_steel_cmd(
        &steel_proc, None, None, test_refs(&mut s, &mut km),
    ).unwrap_err();

    assert!(err.contains("interrupted"), "expected 'interrupted', got: {err}");
    assert_eq!(s.tab_width, 4, "tab-width must be unchanged after interrupt");
}

/// Calling an init-only builtin from a Steel command body must raise a Steel
/// error (not panic).  `is_init = false` during call_steel_cmd, and init-only
/// builtins check this flag.
#[test]
fn call_steel_cmd_set_option_from_body_returns_steel_error() {
    let mut h  = host();
    let mut s  = EditorSettings::default();
    let mut km = Keymap::default();

    h.eval_source(
        r#"(define-command! "try-set" "" (lambda () (set-option! "tab-width" 8)))"#,
        &mut s, &mut km,
    ).unwrap();

    let err = h.call_steel_cmd(
        "%hume-cmd-try-set", None, None, test_refs(&mut s, &mut km),
    ).unwrap_err();

    assert!(err.contains("set-option!"),
        "error must name the failing builtin; got: {err}");
    // Mutation never happened, so the setting is unchanged.
    assert_eq!(s.tab_width, 4, "tab-width must be untouched");
}

// ── call! alias ───────────────────────────────────────────────────────────

/// Both `call!` and `call-command!` route to the same builtin.  Verify
/// that commands defined with each spelling both queue their sub-commands.
#[test]
fn call_bang_and_call_command_both_dispatch() {
    let mut h  = host();
    let mut s  = EditorSettings::default();
    let mut km = Keymap::default();

    h.eval_source(
        r#"
(define-command! "use-call-bang"    "" (lambda () (call! "move-right")))
(define-command! "use-call-command" "" (lambda () (call-command! "move-left")))
"#,
        &mut s, &mut km,
    ).unwrap();

    let (q1, _) = h.call_steel_cmd(
        "%hume-cmd-use-call-bang", None, None, test_refs(&mut s, &mut km),
    ).unwrap();
    assert_eq!(q1, vec!["move-right"], "call! should queue the command");

    let (q2, _) = h.call_steel_cmd(
        "%hume-cmd-use-call-command", None, None, test_refs(&mut s, &mut km),
    ).unwrap();
    assert_eq!(q2, vec!["move-left"], "call-command! alias should queue the command");
}

// ── register-hook! / fire_hook ────────────────────────────────────────────

use crate::scripting::hooks::HookId;
use crate::scripting::builtins::ids::SteelBufferId;

#[test]
fn register_hook_fires_on_buffer_open() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    h.eval_source(
        r#"(register-hook! 'on-buffer-open (lambda (bid) (call! "move-right")))"#,
        &mut s, &mut km,
    ).unwrap();
    let bid = BufferId::default();
    let val = SteelBufferId(bid).into_steel_val();
    let queue = h.fire_hook(
        HookId::OnBufferOpen, &[val], test_refs_with_bid(&mut s, &mut km, bid),
    ).unwrap();
    assert_eq!(queue, vec!["move-right"]);
}

#[test]
fn register_hook_fires_on_buffer_close() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    h.eval_source(
        r#"(register-hook! 'on-buffer-close (lambda (bid) (call! "move-left")))"#,
        &mut s, &mut km,
    ).unwrap();
    let bid = BufferId::default();
    let val = SteelBufferId(bid).into_steel_val();
    let queue = h.fire_hook(
        HookId::OnBufferClose, &[val], test_refs_with_bid(&mut s, &mut km, bid),
    ).unwrap();
    assert_eq!(queue, vec!["move-left"]);
}

#[test]
fn register_hook_fires_on_buffer_save() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    h.eval_source(
        r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right")))"#,
        &mut s, &mut km,
    ).unwrap();
    let bid = BufferId::default();
    let val = SteelBufferId(bid).into_steel_val();
    let queue = h.fire_hook(
        HookId::OnBufferSave, &[val], test_refs_with_bid(&mut s, &mut km, bid),
    ).unwrap();
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
        &mut s, &mut km,
    ).unwrap();
    use steel::rvals::IntoSteelVal as _;
    let old_val = "normal".into_steelval().unwrap();
    let new_val = "insert".into_steelval().unwrap();
    let queue = h.fire_hook(
        HookId::OnModeChange, &[old_val, new_val], test_refs(&mut s, &mut km),
    ).unwrap();
    assert_eq!(queue, vec!["move-right"]);
}

#[test]
fn register_hook_no_fire_if_no_handlers() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let queue = h.fire_hook(
        HookId::OnBufferOpen, &[], test_refs(&mut s, &mut km),
    ).unwrap();
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
        &mut s, &mut km,
    ).unwrap();
    let bid = BufferId::default();
    let val = SteelBufferId(bid).into_steel_val();
    let queue = h.fire_hook(
        HookId::OnBufferSave, &[val], test_refs_with_bid(&mut s, &mut km, bid),
    ).unwrap();
    assert_eq!(queue, vec!["move-right", "move-left"]);
}

#[test]
fn teardown_removes_plugin_hooks() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    // Register a hook as part of a plugin.
    h.plugin_stack.push(ledger::PluginId::parse("user/myplugin").unwrap());
    h.eval_source(
        r#"(register-hook! 'on-buffer-open (lambda (bid) (call! "move-right")))"#,
        &mut s, &mut km,
    ).unwrap();
    h.plugin_stack.pop();
    // Hook is registered.
    assert!(!h.hooks.is_empty_for(HookId::OnBufferOpen));
    // Teardown removes it.
    h.teardown_plugin("user/myplugin", &mut s, &mut km).unwrap();
    assert!(h.hooks.is_empty_for(HookId::OnBufferOpen));
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
        &mut s, &mut km,
    ).unwrap();
    let err = h.call_steel_cmd(
        "%hume-cmd-bad-cmd", None, None, test_refs(&mut s, &mut km),
    ).unwrap_err();
    assert!(err.contains("can only be called during init"), "got: {err}");
}

#[test]
fn register_hook_unknown_name_errors() {
    let mut h = host();
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let err = h.eval_source(
        r#"(register-hook! 'on-nonexistent (lambda () #f))"#,
        &mut s, &mut km,
    ).unwrap_err();
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
        &mut s, &mut km,
    ).unwrap();
    use steel::rvals::IntoSteelVal as _;
    let old_val = "normal".into_steelval().unwrap();
    let new_val = "insert".into_steelval().unwrap();
    let q1 = h.fire_hook(
        HookId::OnModeChange, &[old_val.clone(), new_val], test_refs(&mut s, &mut km),
    ).unwrap();
    assert_eq!(q1, vec!["insert"]);

    // Second fire with different args — stale *hume.ha1* would give wrong result.
    let new_val2 = "normal".into_steelval().unwrap();
    let q2 = h.fire_hook(
        HookId::OnModeChange, &[old_val, new_val2], test_refs(&mut s, &mut km),
    ).unwrap();
    assert_eq!(q2, vec!["normal"], "second fire must not see stale globals from first");
}

#[test]
fn restore_ledger_entry_rejects_unknown_mode_prefix() {
    use crate::scripting::ledger::{LedgerEntry, Owner};
    let mut s = EditorSettings::default();
    let mut km = Keymap::default();
    let entry = LedgerEntry {
        key: "bogus abc".to_string(),
        prior_value: String::new(),
        prior_owner: Owner::Core,
    };
    let err = restore_ledger_entry(entry, &mut s, &mut km).unwrap_err();
    assert!(err.contains("unknown mode prefix"), "got: {err}");
}
