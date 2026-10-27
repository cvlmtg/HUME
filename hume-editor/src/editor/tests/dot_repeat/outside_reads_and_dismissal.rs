//! Inputs opened outside the change, dismissed menus and refused pickers.

use super::*;
use pretty_assertions::assert_eq;

// ── Recording source accounting ──────────────────────────────────────────────

/// A character read opened mid-Insert by code outside every recorded
/// change (here a hook) ends the change: `.` replays the keys typed before
/// it, not its answer nor the keys after it.
#[test]
fn a_read_opened_outside_the_change_mid_insert_ends_the_change() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define armed #t)
           (register-hook! 'on-text-changed
             (lambda (bid)
               (when armed
                 (set! armed #f)
                 (on-next-char! (lambda (c) (+ 1 0))))))"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.settle();
    assert!(reading_char(&ed), "setup: the hook's read must be pending");
    ed.feed_key(key('b'));
    ed.feed_key(key('c'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "achello\nworld\n");

    let action = ed
        .state
        .last_action()
        .expect("`i` must have recorded a change");
    assert!(
        action.answers.is_empty(),
        "a hook's read answer must not join the change: {:?}",
        action.answers
    );
    assert!(
        matches!(action.transcript.as_slice(), [i] if is_key(i, 'a')),
        "got {:?}",
        action.transcript
    );

    select_marked(&mut ed, "achello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "achello\naworld\n");
}

/// A character read opened by code outside every recorded change (here a
/// hook) after `c`, before the first typed key, ends the change there: `.`
/// repeats `c` with no text.
#[test]
fn a_read_opened_outside_the_change_before_its_first_key_ends_the_change() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define armed #t)
           (register-hook! 'on-mode-change
             (lambda (old new)
               (when armed
                 (set! armed #f)
                 (on-next-char! (lambda (c) (+ 1 0))))))"#,
    );
    ed.feed_key(key('c'));
    ed.settle();
    assert!(reading_char(&ed), "setup: the hook's read must be pending");
    ed.feed_key(key('q'));
    ed.settle();
    ed.feed_key(key('a'));
    ed.feed_key(key('b'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "abello\nworld\n");

    let answers = &ed
        .state
        .last_action()
        .expect("`c` must have recorded a change")
        .answers;
    assert!(
        answers.is_empty(),
        "a hook's read answer must not join the change: {answers:?}"
    );

    assert!(
        ed.state
            .last_action()
            .is_some_and(|a| a.transcript.is_empty()),
        "got {:?}",
        ed.state.last_action()
    );

    select_marked(&mut ed, "abello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "abello\norld\n");
}

/// A callback of an Insert-key binding's read that opens a second read
/// records that answer under the binding too.
#[test]
fn dot_repeats_both_reads_a_binding_and_its_callback_make() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "read-twice" ""
             (lambda (pane)
               (on-next-char!
                 (lambda (a)
                   (on-next-char! (lambda (b) (when b (call! "replace" pane b)))))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-x" "read-twice")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    ed.settle();
    ed.feed_key(key('a'));
    ed.settle();
    ed.feed_key(key('z'));
    ed.settle();
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "zello\nworld\n");

    select_marked(&mut ed, "zello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "zello\nzorld\n");
    assert!(
        !status(&ed).contains("reached during"),
        "got: {}",
        status(&ed)
    );
}

/// A repeatable command that opened a menu, then `key`, which dismisses
/// the menu and runs as a key of its own.
fn dismiss_a_recorded_menu_with(key: KeyEvent) -> Editor {
    let mut ed = editor_with_steel(
        "-[h]>ello\nworld\n",
        r#"(define-command! "menu" ""
             (lambda (pane)
               (show-menu! pane (list "one")
                 (lambda (i) (when i (call! "replace" pane #\o)))))
             #:repeatable #t)"#,
    );
    ed.execute_keymap_command("menu".into(), Count::typed(1), false);
    ed.settle();
    assert!(
        ed.state.input.menu().is_some(),
        "setup: the menu must be open"
    );
    ed.feed_key(key);
    ed.settle();
    assert!(
        ed.state.input.menu().is_none(),
        "setup: the key closes the menu"
    );
    ed
}

/// The key that dismissed a recorded command's menu is a change of its
/// own: `.` repeats it, not the menu's command.
#[test]
fn dot_repeats_the_edit_key_that_dismissed_a_recorded_menu() {
    let mut ed = dismiss_a_recorded_menu_with(key('d'));
    assert_eq!(ed.doc().text().to_string(), "ello\nworld\n");

    select_marked(&mut ed, "ello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "ello\norld\n");
}

/// An Insert session entered by the key that dismissed a recorded
/// command's menu records its own change.
#[test]
fn dot_repeats_the_insert_entered_by_the_key_that_dismissed_a_recorded_menu() {
    let mut ed = dismiss_a_recorded_menu_with(key('i'));
    ed.feed_key(key('x'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "xhello\nworld\n");

    select_marked(&mut ed, "xhello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "xhello\nxworld\n");
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_ne!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("menu")
    );
}

/// A native command that dismissed a recorded command's menu and then read
/// its operand records that operand as its own.
#[test]
fn dot_repeats_the_operand_key_that_dismissed_a_recorded_menu() {
    let mut ed = dismiss_a_recorded_menu_with(key('r'));
    assert!(reading_char(&ed), "setup: `r` must be reading its operand");
    ed.feed_key(key('q'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "qello\nworld\n");

    select_marked(&mut ed, "qello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "qello\nqorld\n");
}

/// An Insert-key binding whose `picker!` is refused (here: aimed at a pane
/// that isn't focused) leaves nothing open on the recording, so the next
/// change records normally.
#[test]
fn a_refused_picker_in_an_insert_binding_leaves_the_next_change_recordable() {
    use crate::editor::commands::open_pane_in_layout;
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc def\n");
    let bid = ed.focused_buffer_id();
    open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    );
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "pick-other" ""
             (lambda (pane)
               (define other
                 (car (filter (lambda (p) (not (equal? p (focused-pane)))) (panes))))
               (picker! other (list (cons "alpha" "ALPHA")) (lambda (payload) (+ 1 0)))))
           (bind-key! 'insert "ctrl-y" "pick-other")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "setup: the picker must have been refused"
    );
    ed.feed_key(key_esc());

    ed.feed_key(key('d'));
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("delete"),
        "the delete after the refused picker must be the recorded change"
    );
}

/// A repeatable Steel command that opens an Insert session and then fails
/// still owns that session: its keys record into its own change, never into
/// the change recorded before it.
#[test]
fn a_failed_steel_body_that_opened_insert_keeps_its_session_keys() {
    let mut ed = editor_with_steel(
        "-[f]>oo bar\n",
        r#"(define-command! "ins-boom" ""
             (lambda (pane) (call! "insert-before" pane) (error "boom"))
             #:repeatable #t)"#,
    );
    ed.feed_key(key('d'));
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("delete")
    );

    ed.execute_keymap_command("ins-boom".into(), Count::typed(1), false);
    assert_eq!(
        ed.state.mode(),
        Mode::Insert,
        "setup: the body entered Insert"
    );
    ed.feed_key(key('x'));
    ed.feed_key(key_esc());

    let action = ed.state.last_action().expect("a change must be recorded");
    assert_eq!(action.command.as_ref(), "ins-boom");
    assert!(
        action.transcript.first().is_some_and(|i| is_key(i, 'x')),
        "got {:?}",
        action.transcript
    );
}
