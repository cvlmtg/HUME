//! `set-option!` / `get-option` tests.

use super::*;

// ── set-option! ───────────────────────────────────────────────────────────

#[test]
fn set_option_tab_width_integer() {
    let mut h = host();
    let mut mock = MockHost::new();

    assert_eq!(mock.settings.indent_width, 4);
    h.eval_source("(set-option! \"indent-width\" 2)", &mut mock)
        .unwrap();
    assert_eq!(mock.settings.indent_width, 2);
}

#[test]
fn set_option_tab_width_string_raises() {
    let mut h = host();
    let mut mock = MockHost::new();

    let err = h
        .eval_source("(set-option! \"indent-width\" \"8\")", &mut mock)
        .unwrap_err();
    assert!(
        err.contains("'indent-width' takes an integer"),
        "got: {err}"
    );
    assert_eq!(mock.settings.indent_width, 4);
}

#[test]
fn set_option_bool_as_bool() {
    let mut h = host();
    let mut mock = MockHost::new();

    assert!(mock.settings.mouse);
    h.eval_source("(set-option! \"mouse\" #f)", &mut mock)
        .unwrap();
    assert!(!mock.settings.mouse);
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

/// `eval_source` runs top-level code as an init eval. `get-option` is
/// registered `open` (no eval-mode gate), so it must be callable from
/// `init.scm` too, not just from command bodies (unlike `focused-pane`
/// or other command-mode-only reads).
#[test]
fn get_option_works_during_init_eval() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(unless (equal? (get-option "indent-width") 4) (error "unexpected indent-width"))"#,
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
             (unless (equal? (get-option "indent-width") 4)
               (error "unexpected indent-width"))))"#,
        &mut mock,
    )
    .unwrap();
    h.call_steel_cmd_by_name("check", vec![], &mut mock)
        .expect("get-option must read back the default indent-width as an int");
}

#[test]
fn get_option_reads_back_tab_style_as_its_name() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "check" "" (lambda ()
             (unless (equal? (get-option "indent-style") "hard")
               (error "unexpected indent-style"))))"#,
        &mut mock,
    )
    .unwrap();
    h.call_steel_cmd_by_name("check", vec![], &mut mock)
        .expect("get-option must read back the default indent-style as a string");
}

#[test]
fn get_option_reads_back_whitespace_newline_as_its_name_and_round_trips() {
    // The plugin save/restore pattern from the bug report: read the value,
    // then feed it straight back into set-option!. It must not error.
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
    h.call_steel_cmd_by_name("check", vec![], &mut mock).expect(
        "get-option must read back whitespace-newline as a string that set-option! accepts",
    );
}

#[test]
fn get_option_reads_back_lsp_diagnostics_on_insert_line_as_bool() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "check" "" (lambda ()
             (unless (equal? (get-option "lsp-diagnostics-on-insert-line") #f)
               (error "unexpected lsp-diagnostics-on-insert-line"))))"#,
        &mut mock,
    )
    .unwrap();
    h.call_steel_cmd_by_name("check", vec![], &mut mock).expect(
        "get-option must read back the default lsp-diagnostics-on-insert-line (false) as a bool",
    );
}

#[test]
fn get_option_reads_back_statusline_mode_colors_as_bool() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "check" "" (lambda ()
             (unless (equal? (get-option "statusline-mode-colors") #t)
               (error "unexpected statusline-mode-colors"))))"#,
        &mut mock,
    )
    .unwrap();
    h.call_steel_cmd_by_name("check", vec![], &mut mock)
        .expect("get-option must read back the default statusline-mode-colors (true) as a bool");
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
        .call_steel_cmd_by_name("check", vec![], &mut mock)
        .unwrap_err();
    assert!(err.message.contains("unknown setting"), "got: {err:?}");
}

/// `(get-buffer-option bid key)` reads back a value the same as `get-option`
/// (MockHost ignores `bid`, so this proves the call reaches the host at
/// all, not bid-specific routing; that's covered at the host layer by
/// `get_buffer_option_explicit_bid_reads_hook_target_not_focused_buffer` in
/// `hume-editor/src/editor/tests/settings_effects.rs`).
///
/// `get-buffer-option` is a direct 2-arg Rust builtin registration: Steel's
/// own arg-count and per-position type checking on that registration covers
/// a swapped-argument or wrong-arity call, so there is no separate wrapper
/// logic here needing its own test coverage.
#[test]
fn get_buffer_option_reads_back_a_value() {
    let mut h = host();
    let mut mock = MockHost::new();
    mock.live_buffer_ids.insert(BufferId::default());
    let bid = SteelPane::new(PaneHandle::buffer_only(BufferId::default())).into_steel_val();

    h.eval_source(
        r#"(define-command! "check" "" (lambda (bid)
             (unless (equal? (get-buffer-option bid "indent-width") 4)
               (error "unexpected indent-width"))))"#,
        &mut mock,
    )
    .unwrap();
    h.call_steel_cmd_by_name("check", vec![bid], &mut mock)
        .expect("get-buffer-option must accept (bid key) and read back indent-width");
}
