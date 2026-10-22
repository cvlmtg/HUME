//! Character reads and menu and prompt answers a change records.

use super::*;
use pretty_assertions::assert_eq;

/// An Insert binding to a command that reads its character from the next
/// key repeats the edit it made, not the binding: `.` reads no key.
#[test]
fn dot_repeats_an_insert_binding_that_read_a_character() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'insert "ctrl-x" "replace")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key('z'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "azello\nworld\n");
    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));

    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "azello\nazorld\n");
    assert!(!reading_char(&ed));
    assert_eq!(ed.state.mode(), Mode::Normal);
}

/// Defines the repeatable `rd` from `body`, runs it on line 1 of
/// `hello\nworld`, feeds `keys` one at a time, then moves to line 2.
fn record_rd(body: &str, keys: &[KeyEvent]) -> Editor {
    let mut ed = editor_with_steel(
        "-[h]>ello\nworld\n",
        &format!(r#"(define-command! "rd" "" (lambda (pane) {body}) #:repeatable #t)"#),
    );
    ed.execute_keymap_command("rd".into(), Some(1), false);
    ed.settle();
    for &k in keys {
        ed.feed_key(k);
        ed.settle();
    }
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("rd")
    );
    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));
    ed
}

/// A cancelled read is answered with `#f` on replay too, without waiting
/// for a key.
#[test]
fn dot_answers_a_cancelled_read_with_false() {
    let mut ed = record_rd(
        r#"(on-next-char! (lambda (c) (if c (+ 1 0) (call! "replace" (focused-pane) #\z))))"#,
        &[key_esc()],
    );
    assert_eq!(ed.doc().text().to_string(), "zello\nworld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "zello\nzorld\n");
    assert!(!reading_char(&ed));
    assert!(
        !status(&ed).contains("reached during"),
        "got: {}",
        status(&ed)
    );
}

/// A reload drops a read still pending from a repeatable command. The change
/// that read belonged to stops accepting input with it, so the next
/// repeatable command starts a change of its own.
#[test]
fn reload_drops_a_pending_reads_hold_on_the_change() {
    let mut ed = editor_with_steel(
        "-[h]>ello\nworld\n",
        r#"(define-command! "rd" "" (lambda (pane) (on-next-char! (lambda (c) (+ 1 0)))) #:repeatable #t)"#,
    );
    ed.execute_keymap_command("rd".into(), Some(1), false);
    ed.settle();
    assert!(reading_char(&ed));
    ed.reset_config_state();
    ed.feed_key(key('d'));
    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "ello\norld\n");
}

/// `.` repeats the command with the character it read, and the edits its
/// callback made are the command's: none of them takes `.`'s slot. No
/// settle after `.`: the callback ran during it.
#[test]
fn dot_repeats_a_repeatable_command_with_the_character_it_read() {
    let mut ed = record_rd(
        r#"(on-next-char!
             (lambda (c)
               (when c (call! "replace" pane c) (call! "surround-add" pane #\())))"#,
        &[key('z')],
    );
    assert_eq!(ed.doc().text().to_string(), "(z)ello\nworld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "(z)ello\n(z)orld\n");
    assert!(!reading_char(&ed));
}

#[test]
fn dot_repeats_characters_read_by_a_chained_callback() {
    let mut ed = record_rd(
        r#"(on-next-char!
             (lambda (a)
               (when a
                 (call! "replace" pane a)
                 (on-next-char! (lambda (b) (when b (call! "surround-add" pane b)))))))"#,
        &[key('z'), key('[')],
    );
    assert_eq!(ed.doc().text().to_string(), "[z]ello\nworld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "[z]ello\n[z]orld\n");
    assert!(!reading_char(&ed));
}

/// A read the body ends itself (here, by leaving Insert mode above it)
/// hears `#f` before the command is recorded, and `.` answers it the same.
#[test]
fn dot_answers_a_read_the_body_itself_ended() {
    let mut ed = record_rd(
        r#"(call! "insert-before" pane)
           (on-next-char! (lambda (c) (unless c (call! "replace" pane #\z))))
           (call! "exit-insert" pane)"#,
        &[],
    );
    assert_eq!(ed.doc().text().to_string(), "zello\nworld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "zello\nzorld\n");
    assert!(
        !status(&ed).contains("reached during"),
        "got: {}",
        status(&ed)
    );
}

/// Replay reading more characters than the original run did refuses the
/// extra read, keeping what the answered ones did.
#[test]
fn dot_refuses_a_read_the_recording_has_no_answer_for() {
    let mut ed = editor_with_steel(
        "-[h]>ello\nworld\n",
        r#"(define runs 0)
           (define-command! "rd" ""
             (lambda (pane)
               (set! runs (+ runs 1))
               (on-next-char!
                 (lambda (c)
                   (call! "replace" pane c)
                   (when (> runs 1) (on-next-char! (lambda (d) (+ 1 0)))))))
             #:repeatable #t)"#,
    );
    ed.execute_keymap_command("rd".into(), Some(1), false);
    ed.feed_key(key('z'));
    ed.settle();
    select_marked(&mut ed, "zello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "zello\nzorld\n");
    assert!(!reading_char(&ed));
    assert!(
        status(&ed).contains("on-next-char!: reached during `.`"),
        "got: {}",
        status(&ed)
    );
}

// ── Several inputs waiting at once from one Insert-key binding ───────────────

#[test]
fn dot_repeats_both_edits_of_two_queued_character_reads() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "two-reads" ""
             (lambda (pane)
               (on-next-char! (lambda (a) (when a (call! "replace" pane a))))
               (on-next-char! (lambda (b) (when b (call! "surround-add" pane b)))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-x" "two-reads")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key('z'));
    ed.settle();
    ed.feed_key(key('('));
    ed.settle();
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "(z)ello\nworld\n");

    select_marked(&mut ed, "(z)ello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "(z)ello\n(z)orld\n");
    assert!(!reading_char(&ed));
}

/// The picker opens above the pending read, so it resolves first; `.`
/// replays both edits in the order they happened.
#[test]
fn dot_repeats_a_pick_and_a_character_read_from_one_binding() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("abc -[d]>ef\nuvw xyz\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "read-and-pick" ""
             (lambda (pane)
               (on-next-char! (lambda (c) (when c (call! "replace" pane c))))
               (picker! pane (list (cons "alpha" "ALPHA"))
                 (lambda (payload)
                   (when payload (call! "delete-word-backward" pane)))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "read-and-pick")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    ed.feed_key(key_enter());
    ed.settle();
    ed.feed_key(key('z'));
    ed.settle();
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "zef\nuvw xyz\n");

    select_marked(&mut ed, "zef\nuvw -[x]>yz\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "zef\nzyz\n");
    assert!(ed.state.input.picker().is_none());
    assert!(!reading_char(&ed));
}

#[test]
fn dot_repeats_an_edit_a_binding_makes_after_starting_a_character_read() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "read-then-type" ""
             (lambda (pane)
               (on-next-char! (lambda (c) (when c (call! "replace" pane c))))
               (insert-key! pane "x"))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-x" "read-then-type")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key('z'));
    ed.settle();
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "xzello\nworld\n");

    select_marked(&mut ed, "xzello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "xzello\nxzorld\n");
    assert!(!reading_char(&ed));
}

#[test]
fn dot_repeats_an_edit_a_binding_makes_after_opening_a_picker() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "pick-then-type" ""
             (lambda (pane)
               (picker! pane (list (cons "alpha" "ALPHA"))
                 (lambda (payload) (when payload (call! "replace" pane #\z))))
               (insert-key! pane "x"))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "pick-then-type")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    ed.feed_key(key_enter());
    ed.settle();
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "xzello\nworld\n");

    select_marked(&mut ed, "xzello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "xzello\nxzorld\n");
    assert!(ed.state.input.picker().is_none());
}

/// A read for a command bound directly to the key, cancelled with Esc,
/// leaves nothing for `.` to replay from the binding.
#[test]
fn dot_skips_a_binding_whose_character_read_was_cancelled() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'insert "ctrl-x" "replace")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key_esc());
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "ahello\nworld\n");

    select_marked(&mut ed, "ahello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "ahello\naworld\n");
    assert!(!reading_char(&ed));
    assert_eq!(ed.state.mode(), Mode::Normal);
}

/// A prompt a non-repeatable Insert binding opens ends the change: `.`
/// repeats neither the binding nor the edit it made before the prompt, and
/// never opens the prompt.
#[test]
fn a_non_repeatable_insert_bindings_prompt_ends_the_change() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "type-then-ask" ""
             (lambda (pane)
               (insert-key! pane "x")
               (prompt! pane "x:" (lambda (t) (+ 1 0)))))
           (bind-key! 'insert "ctrl-x" "type-then-ask")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key_esc());
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "xhello\nworld\n");
    assert!(
        ed.state
            .last_action()
            .is_some_and(|a| a.transcript.is_empty()),
        "got {:?}",
        ed.state.last_action()
    );

    select_marked(&mut ed, "xhello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "xhello\nworld\n");
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(
        !status(&ed).contains("reached during"),
        "got: {}",
        status(&ed)
    );
}

/// `.` repeats the menu item a command's menu was answered with, matched by
/// its label, without opening the menu.
#[test]
fn dot_repeats_a_commands_menu_choice() {
    let mut ed = record_rd(
        r#"(show-menu! pane (list "one" "two")
             (lambda (i) (when i (call! "replace" pane (if (= i 1) #\t #\o)))))"#,
        &[key('j'), key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "tello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert!(ed.state.input.menu().is_none(), "`.` must not open a menu");
    assert_eq!(ed.doc().text().to_string(), "tello\ntorld\n");
}

/// A menu `close-menu!` closed before it was answered: `.` opens nothing
/// for it and runs no callback, and the rest of the command repeats.
#[test]
fn dot_replays_a_menu_closed_unanswered_as_nothing() {
    let mut ed = record_rd(
        r#"(let ((tok (show-menu! pane (list "one")
                        (lambda (i) (call! "replace" pane #\x)))))
             (close-menu! tok)
             (call! "delete" pane))"#,
        &[],
    );
    assert_eq!(ed.doc().text().to_string(), "ello\nworld\n");
    assert!(
        matches!(
            ed.state.last_action().map(|a| a.answers.as_slice()),
            Some([Answer::Unanswered])
        ),
        "got: {:?}",
        ed.state.last_action()
    );
    ed.feed_key(key('.'));
    ed.settle();
    assert!(ed.state.input.menu().is_none(), "`.` must not open a menu");
    assert_eq!(ed.doc().text().to_string(), "ello\norld\n");
}

/// A drawer is not an input of the command that opened it: picking a row
/// records no answer, and a repeatable command its `on-select` runs is a
/// change of its own.
#[test]
fn a_drawer_pick_is_no_part_of_the_commands_change() {
    let mut ed = editor_with_steel(
        "-[h]>ello\nworld\n",
        r#"(define-command! "dr" ""
             (lambda (pane)
               (call! "replace" pane #\x)
               (show-drawer-list! pane (list "a" "b")
                 (lambda (idx tok) (when idx (call! "delete" pane)))))
             #:repeatable #t)"#,
    );
    ed.execute_keymap_command("dr".into(), Some(1), false);
    ed.settle();
    assert!(ed.state.input.drawer().is_some(), "sanity: drawer open");
    assert_eq!(ed.doc().text().to_string(), "xello\nworld\n");
    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "ello\nworld\n");
    let action = ed.state.last_action().expect("a change is recorded");
    assert_eq!(action.command.as_ref(), "delete");
    assert!(action.answers.is_empty(), "got: {:?}", action.answers);
}

/// Of two menu items with the same text, `.` repeats the one picked.
#[test]
fn dot_repeats_the_second_of_two_menu_items_with_the_same_text() {
    let mut ed = record_rd(
        r#"(show-menu! pane (list "dup" "dup")
             (lambda (i) (when i (call! "replace" pane (if (= i 1) #\b #\a)))))"#,
        &[key('j'), key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "bello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "bello\nborld\n");
}

/// Of two `picker!` items with the same text, `.` repeats the one picked.
#[test]
fn dot_repeats_the_second_of_two_picker_items_with_the_same_text() {
    let mut ed = record_rd(
        r#"(picker! pane (list (cons "dup" #\a) (cons "dup" #\b))
             (lambda (c) (when c (call! "replace" pane c))))"#,
        &[key_down(), key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "bello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "bello\nborld\n");
}

/// `.` repeats the text a command's prompt was answered with, without
/// opening the prompt.
#[test]
fn dot_repeats_a_commands_prompt_answer() {
    let mut ed = record_rd(
        r#"(prompt! pane "x:"
             (lambda (t) (when t (call! "replace" pane (string-ref t 0)))))"#,
        &[key('z'), key('q'), key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "zello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(ed.doc().text().to_string(), "zello\nzorld\n");
}
