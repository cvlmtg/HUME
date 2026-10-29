//! Keys bound with `(bind-key! 'command …)`: the command mode prompt's
//! keymap, which applies on the `:` line and in `prompt!` prompts.

use super::*;
use pretty_assertions::assert_eq;

/// Defines `put-w`, which inserts `W` where you're typing, plus whatever
/// `extra` holds, and installs the host.
fn with_put_w(ed: &mut Editor, tmp: &std::path::Path, extra: &str) {
    run(
        ed,
        tmp,
        &format!(
            r#"(define-command! "put-w" "" (lambda (pane) (paste-text! pane "W")))
               {extra}"#
        ),
    );
}

#[test]
fn command_binding_replaces_a_builtin_key() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    with_put_w(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'command "ctrl-w" "put-w")"#,
    );
    ed.feed_key(key(':'));
    type_chars(&mut ed, "ab");
    ed.feed_key(key_ctrl('w'));
    assert_eq!(minibuf_input(&ed), "abW");
}

#[test]
fn command_binding_runs_in_a_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    with_put_w(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'command "ctrl-w" "put-w")
           (define-command! "ask" "" (lambda (pane) (prompt! pane "q: " (lambda (t) (+ 1 0)))))"#,
    );
    ed.execute_keymap_command("ask", Count::typed(1), false);
    ed.feed_key(key('a'));
    ed.feed_key(key_ctrl('w'));
    assert_eq!(minibuf_input(&ed), "aW");
}

#[test]
fn command_bindings_do_not_apply_in_search() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello world\n");
    with_put_w(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'command "ctrl-w" "put-w")"#,
    );
    ed.feed_key(key('/'));
    type_chars(&mut ed, "ab");
    ed.feed_key(key_ctrl('w'));
    assert!(ed.state.minibuf().is_some());
    assert_eq!(minibuf_input(&ed), "");
}

#[test]
fn command_bindings_do_not_apply_in_sift() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[ab cd ab]>\n");
    with_put_w(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'command "ctrl-w" "put-w")"#,
    );
    ed.feed_key(key('s'));
    type_chars(&mut ed, "ab");
    ed.feed_key(key_ctrl('w'));
    assert!(ed.state.minibuf().is_some());
    assert_eq!(minibuf_input(&ed), "");
}

#[test]
fn unbinding_a_command_key_restores_the_builtin() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    with_put_w(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'command "ctrl-w" "put-w")
           (unbind-key! 'command "ctrl-w")"#,
    );
    ed.feed_key(key(':'));
    type_chars(&mut ed, "ab");
    ed.feed_key(key_ctrl('w'));
    assert!(ed.state.minibuf().is_some());
    assert_eq!(minibuf_input(&ed), "");
}

#[test]
fn command_binding_dismisses_the_candidate_list_and_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    with_put_w(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'command "ctrl-w" "put-w")"#,
    );
    ed.feed_key(key(':'));
    ed.feed_key(key('w'));
    ed.feed_key(key_tab());
    assert!(ed.state.input.minibuf_completion().is_some());
    let before = minibuf_input(&ed).to_string();

    ed.feed_key(key_ctrl('w'));

    assert!(ed.state.input.minibuf_completion().is_none());
    assert_eq!(minibuf_input(&ed), format!("{before}W"));
}

/// A repeatable command run from the `:` line doesn't become what `.`
/// repeats: `.` still repeats the delete made before.
#[test]
fn command_binding_leaves_dot_repeat_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "put-w" "" (lambda (pane) (paste-text! pane "W")) #:repeatable #t)
           (bind-key! 'command "ctrl-w" "put-w")"#,
    );
    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "bc\n");
    ed.feed_key(key(':'));
    ed.feed_key(key_ctrl('w'));
    ed.feed_key(key_esc());

    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "c\n");
}

const INSERT_READ_CHAR: &str = r#"(define-command! "insert-read-char" ""
    (lambda (pane) (on-next-char! (lambda (ch) (paste-text! pane (string ch))))))"#;

#[test]
fn command_binding_reads_a_char_and_inserts_it() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(r#"{INSERT_READ_CHAR} (bind-key! 'command "ctrl-r" "insert-read-char")"#),
    );
    ed.feed_key(key(':'));
    type_chars(&mut ed, "e ");
    ed.feed_key(key_ctrl('r'));
    ed.feed_key(key('x'));
    ed.settle();
    assert_eq!(minibuf_input(&ed), "e x");
}

#[test]
fn insert_binding_reads_a_char_and_inserts_it() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(r#"{INSERT_READ_CHAR} (bind-key! 'insert "ctrl-r" "insert-read-char")"#),
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('r'));
    ed.feed_key(key('x'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "xhello\n");
}
