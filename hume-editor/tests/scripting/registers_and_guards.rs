//! `set-register-prefix!` and init-mode guards for buffer lifecycle
//! builtins.

use super::*;

// ── set-register-prefix! ─────────────────────────────────────────────────

#[test]
fn set_register_prefix_passed_to_dispatch() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "paste-ring" ""
             (lambda () (set-register-prefix! "k") (call! "paste-after" (focused-pane))))"#,
        &mut mock,
    )
    .unwrap();
    mock.native_names.insert("paste-after".to_string());
    h.call_steel_cmd("paste-ring", None, vec![], &mut mock)
        .unwrap();
    assert_eq!(
        mock.dispatched_native.len(),
        1,
        "paste-after must be dispatched once"
    );
    let (name, _, _, _, reg) = &mock.dispatched_native[0];
    assert_eq!(name, "paste-after");
    assert_eq!(
        *reg,
        Some('k'),
        "set-register-prefix! must pass register 'k' to dispatch"
    );
}

/// `(call! "paste-after" bid 0)` must decode to `count: None` at the
/// `run_command_sync` boundary: `0` is the Scheme spelling of "no count
/// typed" (`parse_count_extend`), distinct from `Some(1)` even though both
/// apply the command once.
///
/// Clamping `0` to `1` in `parse_count_extend` would make
/// `dispatched_native[0].2` come out `Some(1)`.
#[test]
fn call_native_zero_count_decodes_to_none() {
    let mut h = host();
    let mut mock = MockHost::new();

    h.eval_source(
        r#"(define-command! "paste-zero" ""
             (lambda () (call! "paste-after" (focused-pane) 0)))"#,
        &mut mock,
    )
    .unwrap();
    mock.native_names.insert("paste-after".to_string());
    h.call_steel_cmd("paste-zero", None, vec![], &mut mock)
        .unwrap();
    assert_eq!(mock.dispatched_native.len(), 1);
    let (name, _, count, _, _) = &mock.dispatched_native[0];
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
               (call! "yank" (focused-pane))
               (call! "delete" (focused-pane))))"#,
        &mut mock,
    )
    .unwrap();
    mock.native_names.insert("yank".to_string());
    mock.native_names.insert("delete".to_string());
    h.call_steel_cmd("multi", None, vec![], &mut mock).unwrap();
    assert_eq!(
        mock.dispatched_native.len(),
        2,
        "yank and delete must each be dispatched"
    );
    let (name0, _, _, _, reg0) = &mock.dispatched_native[0];
    let (name1, _, _, _, reg1) = &mock.dispatched_native[1];
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
               (call! "yank" (focused-pane))
               (set-register-prefix! "6")
               (call! "paste-after" (focused-pane))))"#,
        &mut mock,
    )
    .unwrap();
    mock.native_names.insert("yank".to_string());
    mock.native_names.insert("paste-after".to_string());
    h.call_steel_cmd("switch", None, vec![], &mut mock).unwrap();
    assert_eq!(mock.dispatched_native.len(), 2);
    let (name0, _, _, _, reg0) = &mock.dispatched_native[0];
    let (name1, _, _, _, reg1) = &mock.dispatched_native[1];
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
        .call_steel_cmd("bad-reg", None, vec![], &mut mock)
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
        .call_steel_cmd("bad-multi", None, vec![], &mut mock)
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
/// raise a Steel error rather than crashing. `bid` is a typed `LiveBid`
/// param, so steel-core decodes it (type only; liveness is a separate,
/// later check) before the registration wrapper's `cmd`-gate closure runs.
/// A call that is both wrong-mode and wrong-typed (as here: init.scm has no
/// way to construct a real pane, cmd-gated builtins like `focused-pane`
/// included) reports the type error, not the gate error.
/// The gate itself is covered directly in `hume-scripting`'s
/// `buffers::tests::close_buffer_blocked_in_init_mode`.
#[test]
fn close_buffer_errors_in_init_mode() {
    let mut h = host();
    let mut mock = MockHost::new();
    let err = h
        .eval_source("(close-buffer! (quote ()))", &mut mock)
        .unwrap_err();
    assert!(
        err.contains("expected pane"),
        "close-buffer! must raise a pane type error, got: {err}",
    );
}

/// `(switch-to-buffer! …)` called from init.scm with a malformed `bid` must
/// raise a Steel error rather than crashing.  Mirrors
/// `close_buffer_errors_in_init_mode`. See its doc for why this asserts
/// the type error, not the gate error.
#[test]
fn switch_to_buffer_errors_in_init_mode() {
    let mut h = host();
    let mut mock = MockHost::new();
    let err = h
        .eval_source("(switch-to-buffer! (quote ()) (quote ()))", &mut mock)
        .unwrap_err();
    assert!(
        err.contains("expected pane"),
        "switch-to-buffer! must raise a pane type error, got: {err}",
    );
}

/// `(get-buffer-option … "language")` / `(set-buffer-option! … "language" …)` on a stale buffer id must
/// raise a Steel error, not silently return `#f` or push a no-op. `bid`
/// (MockHost's `buffer_exists` returns false unconditionally) is a stale
/// handle from the builtins' point of view, exercising the guard path. Each
/// builtin relies on its own `buffer_exists` guard for its eval to fail.
#[test]
fn language_builtins_error_on_stale_buffer_id() {
    let mut h = host();
    let mut mock = MockHost::new();
    let bid = SteelPane::new(PaneHandle::buffer_only(BufferId::default())).into_steel_val();

    h.eval_source(
        r#"(define-command! "q-lang" "" (lambda (bid) (get-buffer-option bid "language")))
           (define-command! "set-lang" "" (lambda (bid) (set-buffer-option! bid "language" "rust")))"#,
        &mut mock,
    )
    .unwrap();

    let err = h
        .call_steel_cmd("q-lang", None, vec![bid.clone()], &mut mock)
        .unwrap_err();
    assert!(
        err.message.contains("get-buffer-option: invalid buffer id"),
        "get-buffer-option must reject a stale id; got: {err}"
    );

    let err = h
        .call_steel_cmd("set-lang", None, vec![bid], &mut mock)
        .unwrap_err();
    assert!(
        err.message
            .contains("set-buffer-option!: invalid buffer id"),
        "set-buffer-option! must reject a stale id; got: {err}"
    );
}
