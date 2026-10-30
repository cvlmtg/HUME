use hume_platform::dirs::Dirs;

use crate::pane_probes::{PaneNeed, declared_pane_needs, pane_probes};

// The tests below exercise table entries through a real `ScriptingHost`
// (register_all → Steel dispatch → the wrapper closure) rather than calling
// `errors::require_cmd`/`require_config` directly: only that shows the
// table's kind tags wire a builtin to its gate, and its name to its errors.
// A `cmd` entry mistyped as `open` stops gating, which a call to the gate
// function itself cannot show.

/// Every `cmd` builtin below, and every one that takes a pane, refuses to
/// run during init.scm, naming itself by the name a plugin calls. The gate
/// runs before any argument decodes, so `#f` stands in for each argument
/// and for the pane.
#[test]
fn cmd_builtins_are_refused_during_init_through_real_registration() {
    const PROBES: &[(&str, &str)] = &[
        ("focused-pane", "(focused-pane)"),
        ("buffers", "(buffers)"),
        ("panes", "(panes)"),
        ("completion-emit!", "(completion-emit! #f #f)"),
        ("after!", "(after! #f #f)"),
    ];
    let pane_probes = pane_probes()
        .into_iter()
        .map(|(name, _, expr)| (name, format!("(let ((p #f)) {expr})")));
    let probes = PROBES
        .iter()
        .map(|&(name, expr)| (name, expr.to_string()))
        .chain(pane_probes);
    for (name, expr) in probes {
        let mut host = crate::ScriptingHost::for_testing(&Dirs::none());
        let err = host
            .eval_source(&expr, &mut crate::null_host::NullHost)
            .expect_err(&expr);
        assert!(
            err.contains(&format!("{name}: not available during init evaluation")),
            "{expr}: {err}"
        );
    }
}

/// `diff-lines` and `diff-words` read only their two strings, so they
/// answer during init, where every `cmd` builtin is refused.
#[test]
fn text_diffs_answer_during_init_through_real_registration() {
    let mut host = crate::ScriptingHost::for_testing(&Dirs::none());
    host.eval_source(
        r#"(define lines (diff-lines "a\n" "b\n")) (define words (diff-words "a" "b"))"#,
        &mut crate::null_host::NullHost,
    )
    .expect("both diffs answer during init");
    for name in ["lines", "words"] {
        let result = host
            .eval_source_for_value(name, &mut crate::null_host::NullHost)
            .expect(name);
        let steel::rvals::SteelVal::ListV(hunks) =
            crate::test_support::hash_field(&result, "hunks")
        else {
            panic!("{name}: 'hunks is not a list: {result:?}");
        };
        assert_eq!(hunks.len(), 1, "{name}: {result:?}");
    }
}

/// `lsp-label-offsets->text` reads the encoding off its own `offsets`
/// handle, so during init it reaches its argument check: a hand-built
/// handle names no server.
#[test]
fn lsp_label_offsets_to_text_checks_its_offsets_during_init() {
    let mut host = crate::ScriptingHost::for_testing(&Dirs::none());
    let err = host
        .eval_source(
            r#"(lsp-label-offsets->text "ab" (json-parse "[0,1]"))"#,
            &mut crate::null_host::NullHost,
        )
        .expect_err("a hand-built offsets handle raises");
    assert!(
        err.contains("lsp-label-offsets->text: offsets: not a value from an LSP server response"),
        "{err}"
    );
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
    let mut host = crate::ScriptingHost::for_testing(&Dirs::none());
    let mut store = crate::null_host::CommandStoreHost::default();
    host.eval_source(
        &format!(r#"(define-command! "probe" "doc" (lambda () {body}))"#),
        &mut store,
    )
    .expect("defining the probe command must not error");
    host.call_steel_cmd_by_name("probe", vec![], &mut store)
}

/// Every probe's pane value is `(focused-pane)`, whose buffer `NullHost`
/// reports closed, so each builtin raises the "invalid buffer id" wording at
/// argument-resolve time, naming itself.
#[test]
fn pane_builtins_raise_on_a_closed_buffer_through_real_registration() {
    for (name, _, expr) in pane_probes() {
        let err = call_probe(&format!("(let ((p (focused-pane))) {expr})"))
            .err()
            .unwrap_or_else(|| panic!("{name} must raise on a closed buffer, got Ok"));
        assert!(
            err.message
                .contains(&format!("{name}: pane: invalid buffer id"))
                || err
                    .message
                    .contains(&format!("{name}: target: invalid buffer id")),
            "{name}: got {err:?}"
        );
    }
}

/// The probes call every builtin the `builtins!` table declares with a
/// pane-checking argument type, and no other.
#[test]
fn pane_probes_cover_every_table_entry_that_checks_a_pane() {
    let probed: std::collections::BTreeSet<_> =
        pane_probes().into_iter().map(|(name, _, _)| name).collect();
    let declared: std::collections::BTreeSet<_> = declared_pane_needs().into_keys().collect();
    assert_eq!(probed, declared);
}

/// The manual's API tables mark a row's calls `shown` or `focused` in their
/// Pane column exactly when the builtin declares that need, so an unmarked
/// call works with a pane value that carries no pane.
#[test]
fn manual_marks_every_builtin_that_needs_a_shown_or_focused_pane() {
    let pages = [
        include_str!("../../../user-manual/docs/plugin-api.md"),
        include_str!("../../../user-manual/docs/lsp-api.md"),
    ];
    let mut marked = std::collections::BTreeMap::new();
    for page in pages {
        let mut in_pane_table = false;
        for line in page.lines() {
            if line == "| Call | Pane | Effect |" {
                in_pane_table = true;
                continue;
            }
            if !line.starts_with('|') {
                in_pane_table = false;
            }
            if !in_pane_table || line.starts_with("|--") {
                continue;
            }
            let mut cells = line[2..].splitn(3, " | ");
            let (names, pane) = (cells.next().unwrap_or(""), cells.next().unwrap_or(""));
            let need = match pane.trim() {
                "" => continue,
                "shown" => PaneNeed::Shown,
                "focused" => PaneNeed::Focused,
                other => panic!("unknown Pane value {other:?} in {line}"),
            };
            for name in names.split('`').skip(1).step_by(2) {
                marked.insert(name, need);
            }
        }
    }
    let declared: std::collections::BTreeMap<_, _> = declared_pane_needs()
        .into_iter()
        .filter(|(_, need)| matches!(need, PaneNeed::Shown | PaneNeed::Focused))
        .collect();
    assert_eq!(marked, declared);
}

/// `(lsp-stop! #f)` / `(lsp-restart! #f)`: there is no "focused buffer"
/// fallback left to decode `#f` into (`core:lsp-install`'s `register.scm`'s typed commands
/// supply the invoking buffer explicitly instead), so `#f` must raise a
/// type error through the real dispatch path, the same as passing any other
/// value neither a buffer-id nor a string/symbol names.
#[test]
fn lsp_stop_and_restart_reject_false_target_through_real_registration() {
    for expr in ["(lsp-stop! #f)", "(lsp-restart! #f)"] {
        let err = call_probe(expr)
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
        let err = crate::ScriptingHost::for_testing(&Dirs::none())
            .eval_source(&expr, &mut crate::null_host::NullHost)
            .expect_err(&expr);
        refused("register-completion-source!", &expr, &err);
    }
}
