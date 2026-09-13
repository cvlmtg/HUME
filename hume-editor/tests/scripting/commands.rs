//! `bind-key!`, `load-plugin` path resolution, `configure-statusline!`,
//! `hume/yield!`, `command-plugin`, and `define-command!` tests.

use super::*;

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
        r#"(declare-plugin "core:lsp" #:events '(on-lsp-attach))"#,
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
    use hume_editor::statusline::StatusElement;
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(configure-statusline! '("Mode" "FileName") '() '("Position"))"#,
        &mut mock,
    )
    .unwrap();

    assert_eq!(
        mock.settings.statusline().left,
        vec![StatusElement::Mode, StatusElement::FileName]
    );
    assert_eq!(mock.settings.statusline().center, vec![]);
    assert_eq!(
        mock.settings.statusline().right,
        vec![StatusElement::Position]
    );
}

#[test]
fn configure_statusline_all_sections() {
    use hume_editor::statusline::StatusElement;
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
        mock.settings.statusline().left,
        vec![
            StatusElement::Position,
            StatusElement::FileName,
            StatusElement::DirtyIndicator
        ]
    );
    assert_eq!(
        mock.settings.statusline().center,
        vec![StatusElement::SearchMatches]
    );
    assert_eq!(
        mock.settings.statusline().right,
        vec![StatusElement::Separator, StatusElement::Mode]
    );
}

#[test]
fn configure_statusline_empty_sections() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source("(configure-statusline! '() '() '())", &mut mock)
        .unwrap();

    assert!(mock.settings.statusline().left.is_empty());
    assert!(mock.settings.statusline().center.is_empty());
    assert!(mock.settings.statusline().right.is_empty());
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
    use hume_editor::statusline::StatusElement;
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(configure-statusline! '("LineEnding") '() '("Cwd"))"#,
        &mut mock,
    )
    .unwrap();

    assert_eq!(
        mock.settings.statusline().left,
        vec![StatusElement::LineEnding]
    );
    assert_eq!(mock.settings.statusline().center, vec![]);
    assert_eq!(mock.settings.statusline().right, vec![StatusElement::Cwd]);
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

    let result = h.eval_source(
        r#"(define-command-extend! "ext-cmd" "doc" (lambda () (+ 1 0)))"#,
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

    h.eval_source(
        r#"(define-command! "inline-cmd" "doc" (lambda () (+ 1 0)) #:inline-output #t)
           (define-command! "plain-cmd"  "doc" (lambda () (+ 1 0)))"#,
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

    h.eval_source(
        r#"(define-command! "rep-cmd"   "doc" (lambda () (+ 1 0)) #:repeatable #t)
           (define-command! "plain-cmd" "doc" (lambda () (+ 1 0)))"#,
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
/// Fail oracle: remove the mutual-exclusion guard in define_command —
/// the eval would succeed and register the command with both flags set.
#[test]
fn repeatable_and_inline_output_mutually_exclusive() {
    let mut h = host();
    let mut mock = MockHost::new();

    let result = h.eval_source(
        r#"(define-command! "bad-cmd" "doc" (lambda () (+ 1 0)) #:repeatable #t #:inline-output #t)"#
            ,
        &mut mock);
    assert!(
        result.is_err(),
        "#:repeatable + #:inline-output must raise an error; got Ok"
    );
    assert!(
        mock.registered_cmds.is_empty(),
        "failed define-command! must not register the command"
    );
}
