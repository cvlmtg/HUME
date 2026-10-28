// Every callback a script hands HUME is checked, when it is handed over,
// against the number of arguments HUME calls it with.

use super::*;

/// Runs `expr` from a typed command whose `pane` is the focused pane, and
/// returns the editor.
fn run_in_command(expr: &str) -> Editor {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abc\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(r#"(define-typed-command! "go" "" (lambda (pane) {expr}))"#),
    );
    type_cmd(&mut ed, ":go");
    ed.settle();
    ed
}

/// Evaluates `source` at the top level of a config, where registrations run,
/// and returns its error.
fn top_level_error(source: &str) -> String {
    let mut ed = editor_from("-[x]>abc\n");
    let mut host = hume_scripting::ScriptingHost::new(&Dirs::none());
    let mut init_host = crate::editor::host_impl::EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.eval_source(source, &mut init_host).expect_err(source)
}

/// A lambda of the wrong arity raises from the call that takes it, naming
/// the builtin and the parameter count it is called with.
#[test]
fn a_callback_of_the_wrong_arity_raises_where_it_is_handed_over() {
    let cases = [
        (
            "(on-next-char! (lambda () 1))",
            "on-next-char!: expected a procedure of 1 parameter",
        ),
        (
            "(after! 1 (lambda (x) 1))",
            "after!: expected a procedure of 0 parameters",
        ),
        (
            r#"(prompt! pane "label" (lambda () 1))"#,
            "prompt!: expected a procedure of 1 parameter",
        ),
        (
            r#"(show-menu! pane (list (cons "a" 1)) (lambda () 1))"#,
            "show-menu!: expected a procedure of 1 parameter",
        ),
        (
            r#"(show-drawer-list! pane (list (cons "a" 1)) (lambda (x) 1))"#,
            "show-drawer-list!: expected a procedure of 2 parameters",
        ),
        (
            r#"(show-drawer-list! pane (list 1) (lambda (x t) 1) #:render (lambda (s) 1))"#,
            "show-drawer-list!: #:render: expected a procedure of 2 parameters",
        ),
        (
            r#"(update-drawer-list! #f (list (cons "a" 1)) (lambda (x) 1) 0)"#,
            "update-drawer-list!: expected a procedure of 2 parameters",
        ),
        (
            r#"(picker! pane (list (cons "a" 1)) (lambda () 1))"#,
            "picker!: expected a procedure of 1 parameter",
        ),
        (
            r#"(picker! pane (list (cons "a" 1)) (lambda (x) 1)
                 #:actions (list (cons "ctrl-t" (lambda () 1))))"#,
            "picker!: #:actions: expected a procedure of 1 parameter",
        ),
        (
            "(live-picker! pane (lambda () 1) #:source (lambda (t q) 1))",
            "live-picker!: expected a procedure of 1 parameter",
        ),
        (
            "(live-picker! pane (lambda (x) 1) #:source (lambda (q) 1))",
            "live-picker!: #:source must be a procedure of two arguments",
        ),
        (
            r#"(spawn-async! "true" (list) (lambda () 1))"#,
            "spawn-async!: expected a procedure of 1 parameter",
        ),
        (
            r#"(path-programs "x" (list) (lambda (names) 1))"#,
            "path-programs: expected a procedure of 2 parameters",
        ),
        (
            r#"(lsp-request! pane "m" (hash) (lambda (result) 1))"#,
            "lsp-request!: expected a procedure of 2 parameters",
        ),
        (
            r#"(lsp-request-all! pane "m" (hash) (lambda (results) 1))"#,
            "lsp-request-all!: expected a procedure of 2 parameters",
        ),
    ];
    for (expr, message) in cases {
        let ed = run_in_command(expr);
        assert!(has_error(&ed, message), "{expr}: {:?}", error_log(&ed));
    }

    let top_level = [
        (
            r#"(register-completion-source! "s" (lambda (id pane) 1) #:target 'buffer)"#,
            "register-completion-source!: expected a procedure of 3 parameters",
        ),
        (
            r#"(register-completion-source! "s" (lambda (id pane prefix) 1) #:target 'buffer
                 #:cancel (lambda () 1))"#,
            "register-completion-source!: #:cancel: expected a procedure of 1 parameter",
        ),
        (
            "(picker-command-source (lambda () 1))",
            "picker-command-source: argv-for must be a procedure of one argument",
        ),
    ];
    for (source, message) in top_level {
        let err = top_level_error(source);
        assert!(err.contains(message), "{source}: {err}");
    }
}

/// A lambda with a rest parameter, or a builtin, is handed over whatever its
/// count: the first can take the arguments, the second's count can't be read.
#[test]
fn a_variadic_lambda_or_a_builtin_is_accepted() {
    let ed = run_in_command(
        "(begin (on-next-char! (lambda (c . rest) 1)) (after! 1 (lambda rest 1)) (after! 1 list))",
    );
    assert!(error_log(&ed).is_empty(), "{:?}", error_log(&ed));
}
