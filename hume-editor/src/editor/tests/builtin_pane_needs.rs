// What each pane-taking builtin checks of its pane value, through a real
// editor: the shown/focused check runs only here, at the host method that
// acts through the pane.

use hume_scripting::pane_probes::{PaneNeed, pane_probes};

use super::*;

/// Runs `expr` from a command on an editor split into two panes on one
/// buffer, with `p` bound by `binding` (a `let*` clause list that may read
/// the command's own `pane`), and returns the editor.
fn run_on_split(binding: &str, expr: &str) -> Editor {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    vsplit_focused(&mut ed);
    run(
        &mut ed,
        tmp.path(),
        &format!(r#"(define-typed-command! "go" "" (lambda (pane) (let* ({binding}) {expr})))"#),
    );
    type_cmd(&mut ed, ":go");
    ed.settle();
    ed
}

/// `p` is the unfocused pane, still showing the command's buffer.
const UNFOCUSED: &str = "(p (cadr (buffer-panes pane)))";

/// `p` names the unfocused pane with the command's buffer, while that pane
/// shows another buffer.
const STALE: &str =
    r#"(p (cadr (buffer-panes pane))) (shown (show-read-only-buffer! p "probe" "x"))"#;

/// Every builtin that needs the focused pane refuses another pane showing
/// its buffer; no other builtin does.
#[test]
fn only_a_focused_need_refuses_an_unfocused_pane() {
    const REFUSAL: &str = "acts on the focused pane, which this pane isn't";
    for (name, need, expr) in pane_probes() {
        let ed = run_on_split(UNFOCUSED, expr);
        assert_eq!(
            has_error(&ed, &format!("{name}: {REFUSAL}")),
            need == PaneNeed::Focused,
            "{name}: {:?}",
            error_log(&ed)
        );
    }
}

/// Every builtin that acts through its pane refuses a pane that shows
/// another buffer; a builtin that needs only the buffer, or takes the pane as
/// a hint, does not.
#[test]
fn only_a_shown_or_focused_need_refuses_a_pane_on_another_buffer() {
    const REFUSAL: &str = "pane no longer shows this buffer";
    for (name, need, expr) in pane_probes() {
        let ed = run_on_split(STALE, expr);
        assert_eq!(
            has_error(&ed, &format!("{name}: {REFUSAL}")),
            matches!(need, PaneNeed::Shown | PaneNeed::Focused),
            "{name}: {:?}",
            error_log(&ed)
        );
    }
}

/// A malformed argument the builtin's body parses is reported ahead of the
/// pane refusal: the editor checks the pane only once the body hands it over.
#[test]
fn a_malformed_argument_is_reported_before_a_pane_refusal() {
    let cases = [
        (
            r#"(show-popup! p "x" #:kind 'bogus)"#,
            "show-popup!: #:kind: must be",
        ),
        (
            r#"(insert-key! p "a b")"#,
            "insert-key!: key: key spec 'a b' must name exactly one key",
        ),
        (
            r#"(goto-location! p (hash 'target "x.txt" 'char-offset 0))"#,
            "goto-location!: a path target takes 'line and 'char-col, not 'char-offset",
        ),
        (
            "(prompt! p 42 (lambda (s) s))",
            "prompt!: label: expected a string",
        ),
        (
            "(picker! p (list) (lambda (i) i) #:prompt 42)",
            "picker!: #:prompt: expected a string",
        ),
        (
            "(goto-revision! p -1)",
            "goto-revision!: expected a non-negative integer",
        ),
        (
            r#"(show-menu! p (list (cons "a" 'a)) 42)"#,
            "show-menu!: expected a callable, got 42",
        ),
    ];
    for (expr, message) in cases {
        let ed = run_on_split(UNFOCUSED, expr);
        assert!(has_error(&ed, message), "{expr}: {:?}", error_log(&ed));
    }
}
