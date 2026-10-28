use hume_platform::dirs::Dirs;

// The tests below exercise table entries through a real `ScriptingHost`
// (register_all → Steel dispatch → the wrapper closure) rather than calling
// `errors::require_cmd`/`require_config` directly: only that shows the
// table's kind tags wire a builtin to its gate, and its name to its errors.
// A `cmd` entry mistyped as `open` stops gating, which a call to the gate
// function itself cannot show.

/// Every `cmd` builtin below refuses to run during init.scm, naming itself
/// by the name a plugin calls. The gate runs before any argument decodes,
/// so `#f` stands in for each argument.
#[test]
fn cmd_builtins_are_refused_during_init_through_real_registration() {
    const PROBES: &[(&str, &str)] = &[
        ("focused-pane", "(focused-pane)"),
        ("buffers", "(buffers)"),
        ("panes", "(panes)"),
        ("buffer-path", "(buffer-path #f)"),
        ("buffer-display-path", "(buffer-display-path #f)"),
        ("buffer-name", "(buffer-name #f)"),
        ("buffer-dirty?", "(buffer-dirty? #f)"),
        ("close-buffer!", "(close-buffer! #f)"),
        ("switch-to-buffer!", "(switch-to-buffer! #f #f)"),
        ("buffer-cursor-line", "(buffer-cursor-line #f)"),
        ("buffer-selections", "(buffer-selections #f)"),
        ("char-offset->line", "(char-offset->line #f #f)"),
        ("buffer-text", "(buffer-text #f)"),
        ("buffer-lines", "(buffer-lines #f)"),
        ("buffer-line-count", "(buffer-line-count #f)"),
        ("line->char-offset", "(line->char-offset #f #f)"),
        ("get-buffer-option", "(get-buffer-option #f #f)"),
        ("set-buffer-option!", "(set-buffer-option! #f #f #f)"),
        ("completion-emit!", "(completion-emit! #f #f)"),
        ("after!", "(after! #f #f)"),
        ("diff-lines", "(diff-lines #f #f)"),
        ("diff-buffer-lines", "(diff-buffer-lines #f #f)"),
        ("diff-words", "(diff-words #f #f)"),
        ("lsp-stop!", "(lsp-stop! #f)"),
        ("lsp-restart!", "(lsp-restart! #f)"),
        ("lsp-show-status!", "(lsp-show-status! #f)"),
    ];
    for (name, expr) in PROBES {
        let mut host = crate::ScriptingHost::new(&Dirs::none());
        let err = host
            .eval_source(expr, &mut crate::null_host::NullHost)
            .expect_err(expr);
        assert!(
            err.contains(&format!("{name}: not available during init evaluation")),
            "{expr}: {err}"
        );
    }
}

/// Every `config` builtin below refuses to run from a command body, naming
/// itself by the name a plugin calls.
#[test]
fn config_builtins_are_refused_in_a_command_body_through_real_registration() {
    const PROBES: &[(&str, &str)] = &[
        ("register-hook!", "(register-hook! #f #f)"),
        (
            "register-completion-source!",
            "(register-completion-source! #f #f #:target #f)",
        ),
        ("define-language!", "(%define-language! #f #f #f #f)"),
        ("configure-language!", "(%configure-language! #f #f)"),
    ];
    for (name, expr) in PROBES {
        let err = call_probe(expr).expect_err(expr);
        assert!(
            err.message.contains(&format!(
                "{name}: only valid during init.scm or plugin load, not from a Steel command body"
            )),
            "{expr}: {}",
            err.message
        );
    }
}

/// `bind-key!` without `#:keymap` changes the global keymap, which only init
/// and plugin loading may do.
#[test]
fn global_bind_key_rejected_from_command_body_through_real_registration() {
    let err = call_probe(r#"(bind-key! 'normal "Q" "move-down")"#)
        .expect_err("a global bind-key! must be rejected from a command body");
    assert!(err.message.contains("command body"), "got: {err:?}");
}

/// `bind-key!` with `#:keymap` binds into a named keymap, which a command
/// body may do.
#[test]
fn named_bind_key_allowed_from_command_body_through_real_registration() {
    let effects = call_probe(r#"(bind-key! 'normal "Q" "move-down" #:keymap "probe-keys")"#)
        .expect("a named bind-key! must be allowed from a command body");
    assert!(
        matches!(
            effects.as_slice(),
            [crate::Effect::BindKey { keymap: crate::host::KeymapTarget::Named(name), .. }]
                if name == "probe-keys"
        ),
        "got: {effects:?}"
    );
}

/// Each kind of builtin names itself once, in front of its error, by the
/// name a plugin calls: `cmd`, `open`, `plain`, a variadic one, a `%`
/// internal behind its wrapper, a decode failure, a closed buffer, and a
/// missing host capability.
#[test]
fn a_builtin_error_names_the_builtin_a_plugin_calls() {
    let cases = [
        ("(buffer-path 5)", "buffer-path", "expected pane"),
        ("(close-buffer! 5)", "close-buffer!", "expected pane"),
        ("(set-cwd! 5)", "set-cwd!", "expected a string path"),
        (
            "(json-list 5)",
            "json-list",
            "expected a JSON handle, got 5",
        ),
        ("(path-join)", "path-join", "expects at least 1 arg, got 0"),
        ("(call! 5)", "call!", "Expected string, found: 5"),
        (
            "(apply-text-edits! (focused-pane) (list))",
            "apply-text-edits!",
            "pane: invalid buffer id",
        ),
        (
            "(after! 1 (lambda () 1))",
            "after!",
            "not supported by this host",
        ),
        ("(after! 1 42)", "after!", "expected a callable, got 42"),
    ];
    for (body, name, message) in cases {
        let err = call_probe(body).expect_err(body).message;
        assert!(err.contains(&format!("{name}: {message}")), "{body}: {err}");
        assert!(!err.contains(&format!("{name}: {name}")), "{body}: {err}");
    }
}

/// Define a command whose body is `body`, then call it as a command.
fn call_probe(body: &str) -> Result<Vec<crate::Effect>, crate::EvalError> {
    let mut host = crate::ScriptingHost::new(&Dirs::none());
    let mut store = crate::null_host::CommandStoreHost::default();
    host.eval_source(
        &format!(r#"(define-command! "probe" "doc" (lambda () {body}))"#),
        &mut store,
    )
    .expect("defining the probe command must not error");
    host.call_steel_cmd_by_name("probe", vec![], &mut store)
}

/// Every explicit-`bid` builtin declared `args::LivePane` in the `builtins!`
/// table raises the same "invalid buffer id" wording on a stale `bid`,
/// checked at argument-resolve time (`args::BuiltinArg::resolve`, wired in
/// by the `builtins!` macro), before the builtin's own body ever runs. A
/// direct Rust call to the underlying function can't prove this: it skips
/// the resolve step entirely, so this goes through a real `ScriptingHost`
/// and Steel dispatch instead: the same real-registration discipline as
/// the two gate tests above.
///
/// Each probe passes `(focused-pane)` as `bid` (`NullHost::
/// buffer_exists` always answers `false`, so it's never live), with just
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
        ("buffer-char-count", "(buffer-char-count (focused-pane))"),
        ("buffer-lines", "(buffer-lines (focused-pane))"),
        ("buffer-generation", "(buffer-generation (focused-pane))"),
        ("buffer-undo-tree", "(buffer-undo-tree (focused-pane))"),
        ("goto-revision!", "(goto-revision! (focused-pane) 0)"),
        ("buffer-cursor-line", "(buffer-cursor-line (focused-pane))"),
        ("buffer-selections", "(buffer-selections (focused-pane))"),
        (
            "symbol-under-cursor",
            "(symbol-under-cursor (focused-pane))",
        ),
        (
            "selections-linewise?",
            "(selections-linewise? (focused-pane))",
        ),
        (
            "selections-charwise?",
            "(selections-charwise? (focused-pane))",
        ),
        ("viewport-range", "(viewport-range (focused-pane))"),
        (
            "lsp-document-params",
            "(lsp-document-params (focused-pane))",
        ),
        (
            "lsp-position-params",
            "(lsp-position-params (focused-pane))",
        ),
        (
            "lsp-primary-range-params",
            "(lsp-primary-range-params (focused-pane))",
        ),
        (
            "lsp-linewise-ranges-params",
            "(lsp-linewise-ranges-params (focused-pane))",
        ),
        ("close-buffer!", "(close-buffer! (focused-pane))"),
        (
            "switch-to-buffer!",
            "(switch-to-buffer! (focused-pane) (focused-pane))",
        ),
        (
            "show-read-only-buffer!",
            r#"(show-read-only-buffer! (focused-pane) "name" "text")"#,
        ),
        ("char-offset->line", "(char-offset->line (focused-pane) 0)"),
        ("line->char-offset", "(line->char-offset (focused-pane) 0)"),
        (
            "diff-buffer-lines",
            r#"(diff-buffer-lines (focused-pane) "")"#,
        ),
        (
            "apply-text-edits!",
            "(apply-text-edits! (focused-pane) (list))",
        ),
        (
            "lsp-request!",
            r#"(lsp-request! (focused-pane) "m" (hash) (lambda (e r) (begin)))"#,
        ),
        (
            "lsp-request-all!",
            r#"(lsp-request-all! (focused-pane) "m" (hash) (lambda (e r) (begin)))"#,
        ),
        ("lsp-notify!", r#"(lsp-notify! (focused-pane) "m" (hash))"#),
        ("lsp-servers", "(lsp-servers (focused-pane))"),
        (
            "lsp-position->char-offset",
            r#"(lsp-position->char-offset (focused-pane) (hash "line" 0 "character" 0))"#,
        ),
        (
            "lsp-range->char-range",
            r#"(lsp-range->char-range (focused-pane)
                 (hash "start" (hash "line" 0 "character" 0) "end" (hash "line" 0 "character" 0)))"#,
        ),
        (
            "set-inlay-hints!",
            r#"(set-inlay-hints! (focused-pane) "src" (list))"#,
        ),
        ("set-signs!", r#"(set-signs! (focused-pane) "src" (list))"#),
        (
            "set-virtual-lines!",
            r#"(set-virtual-lines! (focused-pane) "src" (list))"#,
        ),
        (
            "set-eol-text!",
            r#"(set-eol-text! (focused-pane) "src" (list))"#,
        ),
        (
            "set-extra-highlights!",
            r#"(set-extra-highlights! (focused-pane) "src" (list))"#,
        ),
        (
            "set-line-backgrounds!",
            r#"(set-line-backgrounds! (focused-pane) "src" (list))"#,
        ),
        (
            "set-statusline-text!",
            r#"(set-statusline-text! (focused-pane) "src" "text")"#,
        ),
        (
            "set-buffer-option!",
            r#"(set-buffer-option! (focused-pane) "indent-width" 4)"#,
        ),
        (
            "get-buffer-option",
            r#"(get-buffer-option (focused-pane) "indent-width")"#,
        ),
        ("lsp-stop!", "(lsp-stop! (focused-pane))"),
        ("lsp-restart!", "(lsp-restart! (focused-pane))"),
        (
            "register-sign-source!",
            r#"(register-sign-source! (focused-pane) "src" 0)"#,
        ),
    ];

    for (i, (name, expr)) in PROBES.iter().enumerate() {
        let mut host = crate::ScriptingHost::new(&Dirs::none());
        let mut store = crate::null_host::CommandStoreHost::default();
        let cmd_name = format!("probe-{i}");
        host.eval_source(
            &format!(r#"(define-command! "{cmd_name}" "" (lambda () {expr}))"#),
            &mut store,
        )
        .unwrap_or_else(|e| panic!("defining the probe for {name} must not error: {e}"));

        let err = host
            .call_steel_cmd_by_name(&cmd_name, vec![], &mut store)
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

/// `(lsp-stop! #f)` / `(lsp-restart! #f)`: there is no "focused buffer"
/// fallback left to decode `#f` into (`core:lsp-install`'s `register.scm`'s typed commands
/// supply the invoking buffer explicitly instead), so `#f` must raise a
/// type error through the real dispatch path, the same as passing any other
/// value neither a buffer-id nor a string/symbol names.
#[test]
fn lsp_stop_and_restart_reject_false_target_through_real_registration() {
    for expr in ["(lsp-stop! #f)", "(lsp-restart! #f)"] {
        let mut host = crate::ScriptingHost::new(&Dirs::none());
        let mut store = crate::null_host::CommandStoreHost::default();
        host.eval_source(
            &format!(r#"(define-command! "probe" "" (lambda () {expr}))"#),
            &mut store,
        )
        .unwrap();
        let err = host
            .call_steel_cmd_by_name("probe", vec![], &mut store)
            .err()
            .unwrap_or_else(|| panic!("{expr} must raise on a #f target, got Ok"));
        assert!(
            err.message
                .contains("expected a server, a pane or a server name"),
            "{expr}: got {err:?}"
        );
    }
}

/// Every builtin that takes a decoration, trigger or completion source name
/// refuses an empty or blank one, naming itself, before it checks the pane.
/// `{src}` stands for the name under test.
#[test]
fn every_source_name_refuses_an_empty_name_through_real_registration() {
    const COMMAND_PROBES: &[(&str, &str)] = &[
        (
            "set-inlay-hints!",
            "(set-inlay-hints! (focused-pane) {src} (list))",
        ),
        (
            "register-sign-source!",
            "(register-sign-source! (focused-pane) {src} 0)",
        ),
        ("set-signs!", "(set-signs! (focused-pane) {src} (list))"),
        (
            "set-virtual-lines!",
            "(set-virtual-lines! (focused-pane) {src} (list))",
        ),
        (
            "set-eol-text!",
            "(set-eol-text! (focused-pane) {src} (list))",
        ),
        (
            "set-extra-highlights!",
            "(set-extra-highlights! (focused-pane) {src} (list))",
        ),
        (
            "set-line-backgrounds!",
            "(set-line-backgrounds! (focused-pane) {src} (list))",
        ),
        (
            "set-statusline-text!",
            r#"(set-statusline-text! (focused-pane) {src} "text")"#,
        ),
        (
            "set-hook-triggers!",
            r#"(set-hook-triggers! "rust" {src} (list))"#,
        ),
        (
            "set-attachment-hook-triggers!",
            r#"(set-attachment-hook-triggers! (focused-pane) {src} "srv" 'completion (list))"#,
        ),
        (
            "set-completion-triggers!",
            r#"(set-completion-triggers! "rust" {src} (list))"#,
        ),
        (
            "set-attachment-completion-triggers!",
            r#"(set-attachment-completion-triggers! (focused-pane) {src} "srv" 'completion (list))"#,
        ),
    ];
    let refused = |name: &str, expr: &str, err: &str| {
        assert!(
            err.contains("source name must not be empty") && err.contains(name),
            "{expr}: {err}"
        );
    };
    for src in [r#""""#, r#""  ""#] {
        for (name, template) in COMMAND_PROBES {
            let expr = template.replace("{src}", src);
            let err = call_probe(&expr).expect_err(&expr);
            refused(name, &expr, &err.message);
        }
        let expr =
            format!("(register-completion-source! {src} (lambda (req) (void)) #:target 'buffer)");
        let err = crate::ScriptingHost::new(&Dirs::none())
            .eval_source(&expr, &mut crate::null_host::NullHost)
            .expect_err(&expr);
        refused("register-completion-source!", &expr, &err);
    }
}
