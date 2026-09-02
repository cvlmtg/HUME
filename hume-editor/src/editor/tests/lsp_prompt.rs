// Steel minibuffer prompt: (prompt! label
// on-confirm #:prefill text), (symbol-under-cursor bid).

use super::*;
use crate::editor::commands::open_pane;
use hume_scripting::ScriptingHost;

#[test]
fn prompt_confirm_calls_callback_with_typed_text() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (prompt! "Name: " (lambda (s) (log! 'info (to-string s))))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Command);

    ed.feed_key(key('h'));
    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    ed.settle();

    assert_eq!(ed.state.status_msg.clone().unwrap(), "hi");
    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Normal);
    assert!(ed.state.minibuf.is_none());
}

#[test]
fn prompt_esc_calls_callback_with_false_exactly_once() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (prompt! "Name: " (lambda (s) (log! 'info (to-string s))))))"#,
    );
    type_cmd(&mut ed, ":go");

    ed.feed_key(key('h'));
    ed.feed_key(key_esc());
    ed.settle();

    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");
    assert!(ed.state.minibuf.is_none());
}

#[test]
fn prompt_prefill_is_visible_and_editable() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (prompt! "Name: " (lambda (s) (log! 'info (to-string s))) #:prefill "old")))"#,
    );
    type_cmd(&mut ed, ":go");

    let mb = ed.state.minibuf.as_ref().unwrap();
    assert_eq!(mb.input, "old");
    assert_eq!(
        mb.cursor,
        "old".len(),
        "cursor must start at the end of the prefill"
    );

    // Editable: Backspace removes the trailing char, then typing appends.
    ed.feed_key(key_backspace());
    ed.feed_key(key('a'));
    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "ola");
}

#[test]
fn second_prompt_while_one_is_open_errors() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (prompt! "a" (lambda (s) (log! 'info "cb1")))
             (prompt! "b" (lambda (s) (log! 'info "cb2")))))"#,
    );
    type_cmd(&mut ed, ":go");

    // The first prompt! already took effect (Steel errors don't roll back
    // prior host mutations within the same command body).
    assert_eq!(ed.state.minibuf.as_ref().unwrap().prompt, "a");
    // The command's overall failure (from the second prompt!'s error) is
    // reported rather than silently swallowed.
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("already open") || msg.to_lowercase().contains("error"),
        "expected an error message, got {msg:?}"
    );
}

#[test]
fn prompt_mode_round_trips_and_fires_on_mode_change() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdefghij\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (prompt! "x: " (lambda (s) (void)))))
           (register-hook! 'on-mode-change (lambda (old new) (call! "move-right")))"#,
    );

    let before = state(&ed);
    type_cmd(&mut ed, ":go");
    ed.settle();
    assert_eq!(
        ed.state.mode(),
        hume_engine::types::EditorMode::Command,
        "prompt! must reuse Command mode, not a new EditorMode"
    );
    let after_enter = state(&ed);
    assert_ne!(
        before, after_enter,
        "on-mode-change must fire entering the prompt"
    );

    ed.feed_key(key_esc());
    ed.settle();
    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Normal);
    assert_ne!(
        state(&ed),
        after_enter,
        "on-mode-change must fire again leaving the prompt"
    );
}

// ── symbol-under-cursor ──────────────────────────────────────────────────────

#[test]
fn symbol_under_cursor_on_a_word_char_returns_the_whole_word() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("foo -[b]>ar baz\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "check" "" (lambda ()
             (log! 'info (symbol-under-cursor (current-buffer)))))"#,
    );
    type_cmd(&mut ed, ":check");
    assert_eq!(ed.state.status_msg.clone().unwrap(), "bar");
}

#[test]
fn symbol_under_cursor_on_whitespace_returns_empty() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("foo-[ ]>bar\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "check" "" (lambda ()
             (log! 'info (to-string "[" (symbol-under-cursor (current-buffer)) "]"))))"#,
    );
    type_cmd(&mut ed, ":check");
    assert_eq!(ed.state.status_msg.clone().unwrap(), "[  ]");
}

#[test]
fn symbol_under_cursor_on_punctuation_returns_empty() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("foo-[.]>bar\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "check" "" (lambda ()
             (log! 'info (to-string "[" (symbol-under-cursor (current-buffer)) "]"))))"#,
    );
    type_cmd(&mut ed, ":check");
    assert_eq!(ed.state.status_msg.clone().unwrap(), "[  ]");
}

#[test]
fn symbol_under_cursor_finds_a_word_in_a_non_focused_pane() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("foo -[b]>ar baz\n");

    // Open a second buffer in a second pane, then focus that pane — `bid`
    // stays open (with its "bar" cursor) in the now-unfocused first pane.
    let extra = tmp.path().join("other.rs");
    std::fs::write(&extra, "fn other() {}\n").unwrap();
    ed.open_extra_file(&extra);
    let other_bid = ed
        .state
        .buffers
        .find_by_path(&std::fs::canonicalize(&extra).unwrap())
        .expect("extra file must be open in the buffer list");
    let other_pid = open_pane(&mut ed.state, &mut ed.view, other_bid);
    ed.state.focused_pane_id = other_pid;

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(let ((hidden (car (filter (lambda (b) (not (equal? b (current-buffer)))) (buffers)))))
             (equal? (symbol-under-cursor hidden) "bar"))"#,
    );
    assert!(
        fired,
        "symbol-under-cursor must resolve bid in whichever pane currently shows it, not just the focused one"
    );
}

#[test]
fn symbol_under_cursor_is_empty_once_no_pane_shows_the_buffer() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("foo -[b]>ar baz\n");

    // Redirect the sole pane to a different buffer. `bid`'s PaneBufferState
    // stays seeded (stale) in that pane's map — `pane_state::ensure` never
    // removes an entry — but the pane's live `buffer_id` no longer points
    // at it, so no pane currently shows it.
    let extra = tmp.path().join("other.rs");
    std::fs::write(&extra, "fn other() {}\n").unwrap();
    ed.open_extra_file(&extra);
    let other_bid = ed
        .state
        .buffers
        .find_by_path(&std::fs::canonicalize(&extra).unwrap())
        .expect("extra file must be open in the buffer list");
    ed.switch_to_buffer_with_jump(other_bid);

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(let ((hidden (car (filter (lambda (b) (not (equal? b (current-buffer)))) (buffers)))))
             (equal? (symbol-under-cursor hidden) ""))"#,
    );
    assert!(
        fired,
        "symbol-under-cursor must return \"\" once the buffer is shown in no pane, not the stale cursor's word"
    );
}
