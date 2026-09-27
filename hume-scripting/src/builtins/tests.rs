// These two tests exercise a `cmd` and a `config` table entry through a
// real `ScriptingHost` (register_all → Steel dispatch → the wrapper
// closure's gate call) rather than calling `errors::require_cmd`/
// `require_config` directly — proving the registration table's kind tags
// actually wire a builtin to its gate, which the per-builtin unit tests
// (calling the gate primitive with the builtin's name as a string) can't
// catch on their own: a `cmd` entry mistyped as `open` would silently
// stop gating without failing any of those.

/// A `cmd`-gated builtin (`focused-pane`) called from init.scm — where
/// `register_all`'s wrapper closure is the only place the gate lives —
/// must still raise "not available during init".
#[test]
fn cmd_gated_builtin_rejected_from_init_through_real_registration() {
    let mut host = crate::ScriptingHost::new();
    let mut null_host = crate::null_host::NullHost;
    let err = host
        .eval_source("(focused-pane)", &mut null_host)
        .expect_err("focused-pane must be rejected during init.scm eval");
    assert!(err.contains("not available during init"), "got: {err}");
}

/// A `config`-gated builtin (`bind-key!`) called from inside a command body
/// (`EvalMode::Command`, dispatched via the real `call_steel_cmd` path) must
/// still raise "not from a Steel command body".
///
/// `set-option!` and `configure-statusline!` are `open` (callable from any
/// context — see `builtins/settings.rs`'s and `builtins/statusline.rs`'s
/// docs), so they can't exercise this rejection path; `bind-key!` is
/// genuinely `config`-gated.
#[test]
fn config_gated_builtin_rejected_from_command_body_through_real_registration() {
    let mut host = crate::ScriptingHost::new();
    let mut null_host = crate::null_host::NullHost;
    host.eval_source(
        r#"(define-command! "probe-bind-key" "doc" (lambda () (bind-key! 'normal "Q" "move-down")))"#,
        &mut null_host,
    )
    .expect("defining the probe command must not error");

    let err = host
        .call_steel_cmd("probe-bind-key", None, vec![], &mut null_host)
        .expect_err("bind-key! must be rejected from a command body");
    assert!(err.message.contains("command body"), "got: {err:?}");
}

/// Every explicit-`bid` builtin declared `args::LivePane` in the `builtins!`
/// table raises the same "invalid buffer id" wording on a stale `bid` —
/// checked at argument-resolve time (`args::BuiltinArg::resolve`, wired in
/// by the `builtins!` macro), before the builtin's own body ever runs. A
/// direct Rust call to the underlying function can't prove this: it skips
/// the resolve step entirely, so this goes through a real `ScriptingHost`
/// and Steel dispatch instead — the same real-registration discipline as
/// the two gate tests above.
///
/// Each probe passes `(focused-pane)` as `bid` — `NullHost::
/// buffer_exists` always answers `false`, so it's never live — with just
/// enough well-formed sibling arguments to reach `bid`'s own resolve.
///
/// A `builtins!` table entry using `args::ArgPane` in place of
/// `args::LivePane` would make its probe return `Ok`/`#f` instead of
/// raising here.
#[test]
fn live_pane_builtins_raise_on_a_closed_buffer_through_real_registration() {
    const PROBES: &[(&str, &str)] = &[
        ("buffer-path", "(buffer-path (focused-pane))"),
        (
            "buffer-display-path",
            "(buffer-display-path (focused-pane))",
        ),
        ("buffer-name", "(buffer-name (focused-pane))"),
        ("buffer-dirty?", "(buffer-dirty? (focused-pane))"),
        ("buffer-text", "(buffer-text (focused-pane))"),
        ("buffer-line-count", "(buffer-line-count (focused-pane))"),
        ("buffer-lines", "(buffer-lines (focused-pane))"),
        ("buffer-generation", "(buffer-generation (focused-pane))"),
        ("close-buffer!", "(close-buffer! (focused-pane))"),
        (
            "switch-to-buffer!",
            "(switch-to-buffer! (focused-pane) (focused-pane))",
        ),
        ("buffer-language", "(buffer-language (focused-pane))"),
        (
            "set-buffer-language!",
            r#"(set-buffer-language! (focused-pane) "rust")"#,
        ),
        ("offset->line", "(offset->line (focused-pane) 0)"),
        ("line->offset", "(line->offset (focused-pane) 0)"),
        (
            "diff-buffer-lines",
            r#"(diff-buffer-lines (focused-pane) "")"#,
        ),
        (
            "apply-text-edits!",
            "(apply-text-edits! (focused-pane) (list))",
        ),
        (
            "lsp-request",
            r#"(lsp-request (focused-pane) "m" (hash) (lambda (e r) (begin)))"#,
        ),
        ("lsp-notify", r#"(lsp-notify (focused-pane) "m" (hash))"#),
        (
            "lsp-position->offset",
            r#"(lsp-position->offset (focused-pane) (hash "line" 0 "character" 0))"#,
        ),
        (
            "lsp-range->offsets",
            r#"(lsp-range->offsets (focused-pane)
                 (hash "start" (hash "line" 0 "character" 0) "end" (hash "line" 0 "character" 0)))"#,
        ),
        (
            "set-inlay-hints!",
            r#"(set-inlay-hints! "src" (focused-pane) (list))"#,
        ),
        ("set-signs!", r#"(set-signs! "src" (focused-pane) (list))"#),
        (
            "set-virtual-lines!",
            r#"(set-virtual-lines! "src" (focused-pane) (list))"#,
        ),
        (
            "set-eol-text!",
            r#"(set-eol-text! "src" (focused-pane) (list))"#,
        ),
        (
            "set-extra-highlights!",
            r#"(set-extra-highlights! "src" (focused-pane) (list))"#,
        ),
        (
            "set-line-backgrounds!",
            r#"(set-line-backgrounds! "src" (focused-pane) (list))"#,
        ),
        (
            "set-statusline-text!",
            r#"(set-statusline-text! "src" (focused-pane) "text")"#,
        ),
        (
            "set-buffer-option!",
            r#"(set-buffer-option! (focused-pane) "tab-width" 4)"#,
        ),
        (
            "get-buffer-option",
            r#"(get-buffer-option (focused-pane) "tab-width")"#,
        ),
        ("lsp-stop!", "(lsp-stop! (focused-pane))"),
        ("lsp-restart!", "(lsp-restart! (focused-pane))"),
        (
            "register-sign-source!",
            r#"(register-sign-source! "src" (focused-pane) 0)"#,
        ),
    ];

    for (i, (name, expr)) in PROBES.iter().enumerate() {
        let mut host = crate::ScriptingHost::new();
        let mut null_host = crate::null_host::NullHost;
        let cmd_name = format!("probe-{i}");
        host.eval_source(
            &format!(r#"(define-command! "{cmd_name}" "" (lambda () {expr}))"#),
            &mut null_host,
        )
        .unwrap_or_else(|e| panic!("defining the probe for {name} must not error: {e}"));

        let err = host
            .call_steel_cmd(&cmd_name, None, vec![], &mut null_host)
            .err()
            .unwrap_or_else(|| panic!("{name} must raise on a closed buffer, got Ok"));
        assert!(
            err.message.contains("invalid buffer id"),
            "{name}: got {err:?}"
        );
        assert!(
            err.message.contains(name),
            "{name}: error must name the builtin, got {err:?}"
        );
    }
}

/// `(lsp-stop! #f)` / `(lsp-restart! #f)` — there is no "focused buffer"
/// fallback left to decode `#f` into (`registration.scm`'s typed commands
/// supply the invoking buffer explicitly instead), so `#f` must raise a
/// type error through the real dispatch path, the same as passing any other
/// value neither a buffer-id nor a string/symbol names.
#[test]
fn lsp_stop_and_restart_reject_false_target_through_real_registration() {
    for expr in ["(lsp-stop! #f)", "(lsp-restart! #f)"] {
        let mut host = crate::ScriptingHost::new();
        let mut null_host = crate::null_host::NullHost;
        host.eval_source(
            &format!(r#"(define-command! "probe" "" (lambda () {expr}))"#),
            &mut null_host,
        )
        .unwrap();
        let err = host
            .call_steel_cmd("probe", None, vec![], &mut null_host)
            .err()
            .unwrap_or_else(|| panic!("{expr} must raise on a #f target, got Ok"));
        assert!(
            err.message.contains("expected a pane or a language name"),
            "{expr}: got {err:?}"
        );
    }
}
