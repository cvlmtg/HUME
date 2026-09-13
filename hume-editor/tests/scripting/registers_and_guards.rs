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
