//! `(insert-key! pane key)` — the Steel builtin that runs an Insert-mode
//! key's default behaviour (bypassing the Insert keymap) for a command that
//! bound that same key and decided not to override it.

use super::*;
use hume_editing::tab_style::TabStyle;
use pretty_assertions::assert_eq;

/// Defines a one-arg Steel command whose body is `body` and binds it to
/// `key` in Insert mode — for a test that presses `key` and expects `body`
/// (typically an `insert-key!` call) to run instead of any built-in Insert
/// binding for that key.
fn bind_insert_key(ed: &mut Editor, tmp: &std::path::Path, key: &str, body: &str) {
    let source = format!(
        r#"(define-command! "insert-key-probe" "" (lambda (pane) {body}))
           (bind-key! 'insert "{key}" "insert-key-probe")"#
    );
    run(ed, tmp, &source);
}

/// `insert-key!` on Tab runs the same tab-style-aware insertion an unbound
/// Tab would (see `tabs.rs`'s `insert_tab_hard_default`) — Hard style
/// inserts a literal `\t`.
#[test]
fn insert_key_tab_hard_default() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    bind_insert_key(&mut ed, tmp.path(), "tab", r#"(insert-key! pane "tab")"#);
    ed.feed_key(key('i'));
    ed.feed_key(key_tab());
    assert_eq!(state(&ed), "\t-[h]>ello\n");
}

/// Same, Soft `tab-style`: spaces to the next tab stop, not a literal tab —
/// proving `insert-key!` reads the live setting rather than hardcoding Hard.
#[test]
fn insert_key_tab_soft_setting() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.settings.tab_style = TabStyle::Soft;
    bind_insert_key(&mut ed, tmp.path(), "tab", r#"(insert-key! pane "tab")"#);
    ed.feed_key(key('i'));
    ed.feed_key(key_tab());
    assert_eq!(state(&ed), "    -[h]>ello\n");
}

/// `insert-key!` on `(` before the structural newline runs the same
/// auto-pair logic an unbound `(` would (see `auto_pairs.rs`'s
/// `auto_pairs_auto_close`) — inserts `()`, cursor between them.
#[test]
fn insert_key_runs_auto_pairs() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("hello-[\n]>");
    bind_insert_key(&mut ed, tmp.path(), "(", r#"(insert-key! pane "(")"#);
    ed.feed_key(key('i'));
    ed.feed_key(key('('));
    assert_eq!(state(&ed), "hello(-[)]>\n");
}

/// Calling `insert-key!` from a command dispatched outside Insert mode
/// refuses instead of touching the buffer — it has no key's default
/// behaviour to fall back to when no Insert dispatch is in flight.
#[test]
fn insert_key_errors_outside_insert_mode() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "insert-key-probe" "" (lambda (pane) (insert-key! pane "tab")))"#,
    );
    ed.execute_keymap_command("insert-key-probe".into(), Some(1), false);
    assert_eq!(state(&ed), "-[h]>ello\n", "buffer must be unchanged");
    let msg = status(&ed);
    assert!(msg.contains("only in Insert mode"), "got: {msg}");
}

/// A key `insert_default_key` has no handling for (arrows, Esc, an
/// unhandled Ctrl-chord, …) errors instead of silently doing nothing — it
/// has no default Insert-mode behaviour to fall back to, unlike Tab/Enter/
/// Backspace/Delete/a plain char.
#[test]
fn insert_key_errors_on_key_with_no_default_behaviour() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    bind_insert_key(
        &mut ed,
        tmp.path(),
        "ctrl-x",
        r#"(insert-key! pane "left")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    assert_eq!(state(&ed), "-[h]>ello\n", "buffer must be unchanged");
    let msg = status(&ed);
    assert!(
        msg.contains("no default Insert-mode behaviour"),
        "got: {msg}"
    );
}

/// Calling `insert-key!` while Insert mode happens to be active, but not
/// from inside a command an Insert key's own keymap binding is dispatching
/// (a hook, a timer — simulated here the same way
/// `insert_key_errors_outside_insert_mode` simulates "no key dispatch in
/// flight": calling the command directly rather than through a keypress)
/// refuses instead of editing the buffer untracked for `.`.
#[test]
fn insert_key_errors_when_not_dispatched_from_a_bound_key() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "insert-key-probe" "" (lambda (pane) (insert-key! pane "tab")))"#,
    );
    ed.feed_key(key('i')); // now genuinely in Insert mode
    ed.execute_keymap_command("insert-key-probe".into(), Some(1), false);
    assert_eq!(state(&ed), "-[h]>ello\n", "buffer must be unchanged");
    let msg = status(&ed);
    assert!(msg.contains("only from a command bound"), "got: {msg}");
}
