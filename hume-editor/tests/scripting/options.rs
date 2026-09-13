//! `set-option!` / `get-option` tests.

use super::*;

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
