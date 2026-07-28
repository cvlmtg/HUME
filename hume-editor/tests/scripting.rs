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
/// registered `open` (no eval-mode gate), so it must be callable from
/// `init.scm` too, not just from command bodies (unlike `current-buffer`
/// or other genuinely command-mode-only reads).
#[test]
fn get_option_works_during_init_eval() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(unless (equal? (get-option "tab-width") 4) (error "unexpected tab-width"))"#,
        &mut mock,
    )
    .unwrap();
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
    h.call_steel_cmd(
        "check",
        None,
        vec![],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    )
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
    h.call_steel_cmd(
        "check",
        None,
        vec![],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    )
    .expect("get-option must read back the default tab-style as a string");
}

#[test]
fn get_option_reads_back_whitespace_newline_as_string_and_round_trips() {
    // The plugin save/restore pattern from the bug report: read the value,
    // then feed it straight back into set-option! — must not error.
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "check" "" (lambda ()
             (set-option! "whitespace-newline" "all")
             (define saved (get-option "whitespace-newline"))
             (unless (equal? saved "all")
               (error "unexpected whitespace-newline"))
             (set-option! "whitespace-newline" saved)))"#,
        &mut mock,
    )
    .unwrap();
    h.call_steel_cmd(
        "check",
        None,
        vec![],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    )
    .expect("get-option must read back whitespace-newline as a string that set-option! accepts");
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
    h.call_steel_cmd(
        "check",
        None,
        vec![],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    )
    .expect("get-option must read back the default lsp.inlay-hints (false) as a bool");
}

#[test]
fn get_option_reads_back_statusline_mode_colors_as_bool() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "check" "" (lambda ()
             (unless (equal? (get-option "statusline.mode-colors") #t)
               (error "unexpected statusline.mode-colors"))))"#,
        &mut mock,
    )
    .unwrap();
    h.call_steel_cmd(
        "check",
        None,
        vec![],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    )
    .expect("get-option must read back the default statusline.mode-colors (true) as a bool");
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
        .call_steel_cmd(
            "check",
            None,
            vec![],
            PaneId::default(),
            BufferId::default(),
            &mut mock,
        )
        .unwrap_err();
    assert!(err.message.contains("unknown setting"), "got: {err:?}");
}

/// `(get-option bid key)` — the 2-arg form — reads back a value the same as
/// the 1-arg form (MockHost ignores `bid`, so this proves the wrapper's
/// arity dispatch and argument order, not bid-specific routing — that's
/// covered at the host layer by
/// `get_option_explicit_bid_reads_hook_target_not_focused_buffer` in
/// `hume-editor/src/editor/tests/settings_effects.rs`).
///
/// Fail oracle: swap `(cadr args)`/`(car args)` in the wrapper's 2-arity
/// branch (`bootstrap.scm`) → `key` and `bid` pass to `%get-option` in the
/// wrong order, and `%get-option`'s `key: String` param rejects the
/// `SteelBufferId` it receives instead with a conversion error, instead of
/// returning 4.
#[test]
fn get_option_explicit_bid_two_arg_form() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "check" "" (lambda ()
             (unless (equal? (get-option (current-buffer) "tab-width") 4)
               (error "unexpected tab-width"))))"#,
        &mut mock,
    )
    .unwrap();
    h.call_steel_cmd(
        "check",
        None,
        vec![],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    )
    .expect("get-option must accept (bid key) and read back tab-width");
}

/// The 2-arg form works from inside a `(require "path.scm")`-loaded module
/// — the shape every real plugin command is defined in — not just from a
/// top-level `eval_source` call. Regression guard for the steel-core 0.8.2
/// mixed-fixed-plus-rest-list limitation documented in
/// `hume-scripting/src/builtins/io.rs`'s module doc: the wrapper avoids it
/// by using a rest-only parameter list, but that must be verified against a
/// required-module compilation unit, not assumed from the print-shim
/// precedent (a different global, shadowing the prelude).
///
/// Fail oracle: revert the wrapper to a mixed `(key #:buffer [bid #f])`
/// parameter list → this call, compiled inside the required module, fails
/// to invoke the shadowed global correctly (see `io.rs`'s doc for the exact
/// failure shape), while `get_option_explicit_bid_two_arg_form` above (a
/// top-level call) still passes — so only the required-module variant
/// catches a regression to the old parameter shape.
#[test]
fn get_option_explicit_bid_from_required_module() {
    let mut h = host();
    let mut mock = MockHost::new();

    let tmp = tempfile::tempdir().unwrap();
    let plugin_path = tmp.path().join("get_option_probe.scm");
    std::fs::write(
        &plugin_path,
        r#"
            (define-command! "probe-get-option"
              "doc"
              (lambda ()
                (unless (equal? (get-option (current-buffer) "tab-width") 4)
                  (error "unexpected tab-width"))))
        "#,
    )
    .unwrap();
    let escaped_path = plugin_path.to_string_lossy().replace('\\', "\\\\");
    h.eval_source(&format!(r#"(require "{escaped_path}")"#), &mut mock)
        .expect("requiring the plugin file must not error");

    h.call_steel_cmd(
        "probe-get-option",
        None,
        vec![],
        PaneId::default(),
        BufferId::default(),
        &mut mock,
    )
    .expect("get-option's 2-arg form must work from a required module");
}

/// Passing the key first and the bid second — the old `#:buffer`-era
/// argument order reversed — must error, not silently misinterpret the
/// buffer id as a settings key: `%get-option`'s `key: String` param rejects
/// the buffer-id `SteelVal` outright (a `ConversionError`, since `key` is
/// typed before `optional_bid_arg` ever runs on the second argument).
///
/// Fail oracle: swap the wrapper's `(car args)`/`(cadr args)` in the
/// 2-arity branch (making the *correct* call order the one that breaks) →
/// this call would instead succeed.
#[test]
fn get_option_swapped_args_errors() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "check" "" (lambda ()
             (get-option "tab-width" (current-buffer))))"#,
        &mut mock,
    )
    .unwrap();
    let err = h
        .call_steel_cmd(
            "check",
            None,
            vec![],
            PaneId::default(),
            BufferId::default(),
            &mut mock,
        )
        .unwrap_err();
    assert!(
        err.message.contains("Expected string") && err.message.contains("buffer-id"),
        "got: {err:?}"
    );
}

/// A 3-argument call — `(get-option "key" #:buffer bid)`, which desugars to
/// 3 positional args since `#:buffer` isn't a keyword param — hits the
/// wrapper's explicit arity-error arm rather than silently dropping the
/// extra argument.
///
/// Fail oracle: remove the wrapper's `else` arm (or replace it with a
/// permissive default) → this call would either error with an unrelated
/// message or succeed by ignoring the third argument.
#[test]
fn get_option_wrong_arity_errors() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "check" "" (lambda ()
             (get-option (current-buffer) "tab-width" "extra")))"#,
        &mut mock,
    )
    .unwrap();
    let err = h
        .call_steel_cmd(
            "check",
            None,
            vec![],
            PaneId::default(),
            BufferId::default(),
            &mut mock,
        )
        .unwrap_err();
    assert!(
        err.message
            .contains("expected (get-option key) or (get-option bid key)"),
        "got: {err:?}"
    );
}

// ── bind-key! ─────────────────────────────────────────────────────────────

#[test]
fn bind_key_queues_bind_effect() {
    let mut h = host();
    let mut mock = MockHost::new();

    let effects = h
        .eval_source("(bind-key! 'normal \"z\" \"move-right\")", &mut mock)
        .unwrap();

    use termina::event::{KeyCode, KeyEvent, Modifiers};
    let z_key = KeyEvent::new(KeyCode::Char('z'), Modifiers::NONE);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::BindKey { mode: BindMode::Normal, keys, cmd, force_extend: false }]
                if keys.as_slice() == [z_key] && cmd == "move-right"
        ),
        "expected one Effect::BindKey for 'z' → move-right; got: {effects:?}"
    );
}

#[test]
fn bind_key_multi_key_sequence_queues_full_sequence() {
    let mut h = host();
    let mut mock = MockHost::new();

    let effects = h
        .eval_source("(bind-key! 'normal \"g h\" \"move-right\")", &mut mock)
        .unwrap();

    use termina::event::{KeyCode, KeyEvent, Modifiers};
    let g_key = KeyEvent::new(KeyCode::Char('g'), Modifiers::NONE);
    let h_key = KeyEvent::new(KeyCode::Char('h'), Modifiers::NONE);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::BindKey { keys, cmd, .. }]
                if keys.as_slice() == [g_key, h_key] && cmd == "move-right"
        ),
        "the whole 'g h' sequence must reach the effect; got: {effects:?}"
    );
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

/// `(declared-plugins)` must include `core:*` names — PLUM's
/// never-install-core filter lives in Steel (`plum/missing-plugins`), not in
/// this builtin. No runtime dir is set, so the declare's own disk-resolution
/// logs an absent-core error and no-ops the activation — but `declared_plugins`
/// is recorded unconditionally before that check runs (see `declare_plugin`),
/// which is exactly the persistence this test locks in.
#[test]
fn declared_plugins_includes_core_plugins() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(declare-plugin "core:lsp" #:events '("on-lsp-attach"))"#,
        &mut mock,
    )
    .unwrap();

    assert!(
        h.declared_plugins()
            .iter()
            .any(|d| d.eq_ignore_ascii_case("core:lsp")),
        "declared_plugins field does not contain the declared core plugin: {:?}",
        h.declared_plugins(),
    );

    h.eval_source(
        r#"(if (member "core:lsp" (declared-plugins))
               (log! 'info "PERSISTED")
               (log! 'info "MISSING"))"#,
        &mut mock,
    )
    .unwrap();
    assert!(
        h.peek_pending_messages()
            .iter()
            .any(|(_, msg)| msg == "PERSISTED"),
        "(declared-plugins) did not include the declared core plugin across the eval boundary; messages: {:?}",
        h.peek_pending_messages(),
    );
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

/// `define-command!` registers the command; `define-command-extend!` does not
/// exist. Verifies calling it produces a Steel FreeIdentifier error, not a
/// recognised builtin.
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
/// Fail oracle: a `parse_count_extend` that clamps `0` to `1` would make
/// `dispatched_native[0].1` come out `Some(1)`, not `None`.
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
        err.message.contains("invalid register"),
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
        err.message.contains("single-character"),
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

/// `(close-buffer! …)` called from init.scm with a malformed `bid` must
/// raise a Steel error rather than crashing. `bid` is a typed `BidArg`
/// param, so steel-core decodes it before the registration wrapper's
/// `cmd`-gate closure runs — a call that is both wrong-mode and wrong-typed
/// (as here: init.scm has no way to construct a real buffer-id, cmd-gated
/// builtins like `current-buffer` included) reports the type error, not the
/// gate error. The gate itself is covered directly in `hume-scripting`'s
/// `buffers::tests::close_buffer_blocked_in_init_mode`.
///
/// Flip: accept any `SteelVal` (no `BidArg` decode) and the eval returns Ok
/// (or panics), not Err.
#[test]
fn close_buffer_errors_in_init_mode() {
    let mut h = host();
    let mut mock = MockHost::new();
    let err = h
        .eval_source("(close-buffer! (quote ()))", &mut mock)
        .unwrap_err();
    assert!(
        err.contains("expected buffer-id"),
        "close-buffer! must raise a buffer-id type error, got: {err}",
    );
}

/// `(switch-to-buffer! …)` called from init.scm with a malformed `bid` must
/// raise a Steel error rather than crashing.  Mirrors
/// `close_buffer_errors_in_init_mode` — see its doc for why this asserts
/// the type error, not the gate error.
///
/// Flip: accept any `SteelVal` (no `BidArg` decode) and the eval returns Ok
/// (or panics), not Err.
#[test]
fn switch_to_buffer_errors_in_init_mode() {
    let mut h = host();
    let mut mock = MockHost::new();
    let err = h
        .eval_source("(switch-to-buffer! (quote ()))", &mut mock)
        .unwrap_err();
    assert!(
        err.contains("expected buffer-id"),
        "switch-to-buffer! must raise a buffer-id type error, got: {err}",
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
        err.message.contains("buffer-language: invalid buffer id"),
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
        err.message
            .contains("set-buffer-language!: invalid buffer id"),
        "set-buffer-language! must reject a stale id; got: {err}"
    );
}

// ── bind-key-extend! ──────────────────────────────────────────────────────

#[test]
fn bind_key_extend_queues_force_extend_effect() {
    let mut h = host();
    let mut mock = MockHost::new();

    let effects = h
        .eval_source(r#"(bind-key-extend! 'normal "z" "select-line")"#, &mut mock)
        .unwrap();
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::BindKey { cmd, force_extend: true, .. }] if cmd == "select-line"
        ),
        "bind-key-extend! must produce force_extend = true; got: {effects:?}"
    );
}

#[test]
fn bind_key_does_not_force_extend() {
    let mut h = host();
    let mut mock = MockHost::new();

    let effects = h
        .eval_source(r#"(bind-key! 'normal "z" "select-line")"#, &mut mock)
        .unwrap();
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::BindKey {
                force_extend: false,
                ..
            }]
        ),
        "bind-key! must produce force_extend = false; got: {effects:?}"
    );
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
fn unbind_key_queues_unbind_effect() {
    let mut h = host();
    let mut mock = MockHost::new();

    let effects = h
        .eval_source(r#"(unbind-key! 'normal "h")"#, &mut mock)
        .unwrap();

    // Whether 'h' was bound is `Keymap`'s business, not the builtin's —
    // `remove_sequence_nonexistent_is_noop` (editor/keymap/mod.rs) owns that.
    use termina::event::{KeyCode, KeyEvent, Modifiers};
    let h_key = KeyEvent::new(KeyCode::Char('h'), Modifiers::NONE);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::UnbindKey { mode: BindMode::Normal, keys }] if keys.as_slice() == [h_key]
        ),
        "expected one Effect::UnbindKey for 'h'; got: {effects:?}"
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
    use termina::event::{KeyCode, KeyEvent, Modifiers};

    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(PRELUDE_MACROS, &mut mock).unwrap();
    let effects = h
        .eval_source(
            r#"(bind-keys! 'normal
             ("z z" "move-left")
             ("z l" "move-right"))"#,
            &mut mock,
        )
        .unwrap();

    let z = KeyEvent::new(KeyCode::Char('z'), Modifiers::NONE);
    let l = KeyEvent::new(KeyCode::Char('l'), Modifiers::NONE);

    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::BindKey { keys: k1, cmd: c1, force_extend: false, .. },
                Effect::BindKey { keys: k2, cmd: c2, force_extend: false, .. },
            ] if k1.as_slice() == [z, z] && c1 == "move-left"
                && k2.as_slice() == [z, l] && c2 == "move-right"
        ),
        "bind-keys! must expand to one non-force-extend bind per pair, in order; \
         got: {effects:?}"
    );
}

#[test]
fn prelude_bind_keys_extend_creates_force_extend_leaves() {
    use termina::event::{KeyCode, KeyEvent, Modifiers};

    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(PRELUDE_MACROS, &mut mock).unwrap();
    let effects = h
        .eval_source(
            r#"(bind-keys-extend! 'normal
             ("Q" "select-line")
             ("W" "select-to-end"))"#,
            &mut mock,
        )
        .unwrap();

    let q = KeyEvent::new(KeyCode::Char('Q'), Modifiers::NONE);
    let w = KeyEvent::new(KeyCode::Char('W'), Modifiers::NONE);

    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::BindKey { keys: k1, cmd: c1, force_extend: true, .. },
                Effect::BindKey { keys: k2, cmd: c2, force_extend: true, .. },
            ] if k1.as_slice() == [q] && c1 == "select-line"
                && k2.as_slice() == [w] && c2 == "select-to-end"
        ),
        "bind-keys-extend! must expand to force-extending binds, in order; got: {effects:?}"
    );
}

#[test]
fn prelude_unbind_keys_batch_removes_bindings() {
    use termina::event::{KeyCode, KeyEvent, Modifiers};

    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(PRELUDE_MACROS, &mut mock).unwrap();
    let effects = h
        .eval_source(r#"(unbind-keys! 'normal "h" "l")"#, &mut mock)
        .unwrap();

    let h_key = KeyEvent::new(KeyCode::Char('h'), Modifiers::NONE);
    let l_key = KeyEvent::new(KeyCode::Char('l'), Modifiers::NONE);

    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::UnbindKey { keys: k1, .. },
                Effect::UnbindKey { keys: k2, .. },
            ] if k1.as_slice() == [h_key] && k2.as_slice() == [l_key]
        ),
        "unbind-keys! must expand to one unbind per key, in order; got: {effects:?}"
    );
}

/// Verify the prelude eval_init → init.scm eval_init sequence: prelude macros
/// defined by the first eval_init are available in the second.
#[test]
fn prelude_eval_init_sequence_makes_macros_available_to_init_scm() {
    use std::io::Write as _;

    let mut h = host();
    let mut mock = MockHost::new();

    let builtin_names: rustc_hash::FxHashSet<String> = Default::default();

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

    let effects = h
        .eval_init(&init_path, 10_000, &mut mock, builtin_names)
        .expect("init.scm using bind-keys! must succeed after prelude is loaded");

    use termina::event::{KeyCode, KeyEvent, Modifiers};
    let q = KeyEvent::new(KeyCode::Char('Q'), Modifiers::NONE);
    let w = KeyEvent::new(KeyCode::Char('W'), Modifiers::NONE);

    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::BindKey { keys: k1, cmd: c1, .. },
                Effect::BindKey { keys: k2, cmd: c2, .. },
            ] if k1.as_slice() == [q, q] && c1 == "move-left"
                && k2.as_slice() == [q, w] && c2 == "move-right"
        ),
        "init.scm's bind-keys! must expand via the prelude macro; got: {effects:?}"
    );
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
        err.message.contains("requires a non-empty list"),
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
/// (`EvalMode::Command`) — the `ensure_top_level` gate rejects it.
///
/// Flip: remove `ensure_top_level` from `load_plugin` and the call returns `Ok`,
/// silently queuing a load request that is never drained.
#[test]
fn load_plugin_runtime_guard_fires() {
    // (load-plugin ...) from a command body (EvalMode::Command) must be rejected.
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
        err.message.contains("top level") || err.message.contains("init.scm"),
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
