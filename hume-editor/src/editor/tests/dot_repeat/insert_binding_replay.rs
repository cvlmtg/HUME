//! Arrow keys, prompt stops, register prefixes and picks inside a recorded Insert session.

use super::*;
use pretty_assertions::assert_eq;

/// A bound motion inside the session (here the Left arrow) is replayed in
/// order, so text typed after it lands where it did live.
///
/// `i ab <Left> c` writes "acb" (the `c` lands between
/// `a` and `b`), so `.` at the next line's start must write "acb" too.
#[test]
fn dot_repeats_an_arrow_key_inside_insert() {
    let mut ed = editor_from("-[h]>ello\nworld\n");

    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key('b'));
    ed.feed_key(key_left());
    ed.feed_key(key('c'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "acbhello\nworld\n");

    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "acbhello\nacbworld\n");
}

/// An arrow key inside the session ends the typed run, so Esc leaves a
/// cursor instead of selecting what was typed. `.` leaves the same cursor.
#[test]
fn dot_after_an_arrow_key_inside_insert_leaves_the_cursor_it_left_live() {
    let mut ed = editor_from("-[h]>ello\nworld\n");

    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key('b'));
    ed.feed_key(key_left());
    ed.feed_key(key('c'));
    ed.feed_key(key_esc());
    assert_eq!(state(&ed), "ac-[b]>hello\nworld\n");

    select_marked(&mut ed, "acbhello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(state(&ed), "acbhello\nac-[b]>world\n");
}

/// A pattern command bound to an Insert key opens a prompt `.` cannot
/// answer, so the change ends before that key: `.` repeats the text typed
/// before it and opens no prompt.
#[test]
fn dot_stops_before_an_insert_key_that_opened_a_pattern_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'insert "ctrl-s" "search-forward")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key('x'));
    ed.feed_key(key_ctrl('s'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "xhello\nworld\n");

    select_marked(&mut ed, "xhello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "xhello\nxworld\n");
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(
        ed.state
            .input
            .find::<crate::editor::input_stack::SearchLayer>()
            .is_none()
    );
    assert_eq!(status(&ed), "");
}

/// The `:` line is an input `.` cannot answer, so an Insert key that opens
/// it ends the change there: `.` repeats the text typed before it and opens
/// no `:` line.
#[test]
fn dot_stops_before_an_insert_key_that_opened_the_command_line() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'insert "ctrl-s" "command-mode")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key('x'));
    ed.feed_key(key_ctrl('s'));
    ed.feed_key(key_esc());
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "xhello\nworld\n");
    assert_eq!(ed.state.mode(), Mode::Normal);

    select_marked(&mut ed, "xhello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "xhello\nxworld\n");
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(
        ed.state
            .input
            .find::<crate::editor::input_stack::command::CommandLayer>()
            .is_none()
    );
    assert_eq!(status(&ed), "");
}

/// A native command bound directly to an Insert key must not overwrite
/// the recorded change if it's itself repeatable. That slot belongs to
/// whichever command opened the Insert session; the key is recorded into
/// that change's transcript instead, the same mechanism
/// `dot_repeats_ctrl_w_inside_insert` covers.
///
/// The `repeat_slot_owned` gate on beginning a fresh change keeps `Ctrl-x`'s
/// own dispatch through `commands::run` from stamping `"delete"` over the
/// command that opened the session.
#[test]
fn native_command_bound_mid_insert_does_not_hijack_the_recorded_change() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[foo]> bar\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(bind-key! 'insert "ctrl-x" "delete")"#,
    );

    ed.feed_key(key('a')); // insert-at-selection-end: repeatable, opens the session
    ed.feed_key(key_ctrl('x')); // "delete" is also repeatable, dispatched mid-insert
    ed.feed_key(key_esc());

    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("insert-at-selection-end"),
        "the command that opened the Insert session must keep the slot"
    );
}

/// `.` must re-run the Insert-key binding itself, not a frozen native
/// command name it happened to `call!`: a Steel binding armed with
/// `set-register-prefix!` before its `call!` must arm that same register
/// again on replay, at the new site, rather than always writing wherever
/// the very first run wrote.
///
/// Deleting "a" then "d" with register 3 armed each
/// time must leave register 3 holding "d" (the most recent capture), the
/// same as pressing the binding twice by hand, once per site.
#[test]
fn insert_key_binding_register_prefix_reruns_on_replay() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc def\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "del-to-reg3" ""
             (lambda (pane) (set-register-prefix! "3") (call! "delete" pane)))
           (bind-key! 'insert "ctrl-x" "del-to-reg3")"#,
    );

    ed.feed_key(key('i')); // insert-at-selection-start: cursor lands on "a"
    ed.feed_key(key_ctrl('x')); // del-to-reg3: deletes "a" into register 3
    ed.feed_key(key_esc());

    assert_eq!(ed.doc().text().to_string(), "bc def\n");
    assert_eq!(reg(&ed, '3'), vec!["a".to_string()]);

    select_marked(&mut ed, "bc -[d]>ef\n");
    ed.feed_key(key('.'));

    assert_eq!(ed.doc().text().to_string(), "bc ef\n");
    assert_eq!(
        reg(&ed, '3'),
        vec!["d".to_string()],
        "replay must re-run set-register-prefix!, not drop it and fall back to the kill ring"
    );
}

/// A binding that itself leaves Insert mid-body (`exit-insert`) and then
/// runs a repeatable native command must still leave the recorded change
/// naming the command that *opened* the session, and `.` must reproduce
/// both the typed text and the binding's own edit at the new site.
///
/// "Delete a char, type one char, then immediately delete that same char"
/// nets to exactly the original deletion, so a correct replay leaves the
/// second site exactly as if only the plain delete had run there, with no
/// typed character surviving.
#[test]
fn insert_key_binding_exiting_insert_mid_body_stays_off_the_repeat_slot() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>yz abc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "flush-char" ""
             (lambda (pane) (call! "exit-insert" pane) (call! "delete" pane)))
           (bind-key! 'insert "ctrl-k" "flush-char")"#,
    );

    ed.feed_key(key('c')); // change: delete "x", enter Insert
    ed.feed_key(key('q')); // "qyz abc\n"
    ed.feed_key(key_ctrl('k')); // flush-char: exit-insert, then delete "q" back out

    assert_eq!(ed.doc().text().to_string(), "yz abc\n");
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("change"),
        "the command that opened the Insert session must keep the slot, \
         not the inner `delete` dispatched after exit-insert"
    );

    select_marked(&mut ed, "yz -[a]>bc\n");
    ed.feed_key(key('.'));

    assert_eq!(
        ed.doc().text().to_string(),
        "yz bc\n",
        "replay must delete \"a\", type \"q\", then delete \"q\" back out, no leftover \"q\""
    );
}

/// A binding that runs a pure motion via `call!` before an `insert-key!`
/// fallback must have that motion replayed too. Otherwise the fallback's
/// effect lands at the pre-motion cursor instead of where the motion left it.
///
/// In Insert mode `goto-line-end` lands on the line break, so the semicolon
/// is appended to each line.
#[test]
fn insert_key_binding_motion_via_call_is_replayed_before_the_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\ndef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "append-semi" ""
             (lambda (pane) (call! "goto-line-end" pane) (insert-key! pane ";")))
           (bind-key! 'insert "ctrl-e" "append-semi")"#,
    );

    ed.feed_key(key('i')); // insert-at-selection-start: cursor before "a"
    ed.feed_key(key_ctrl('e')); // append-semi: jump to end of "abc", insert ";"
    ed.feed_key(key_esc());

    assert_eq!(ed.doc().text().to_string(), "abc;\ndef\n");

    select_marked(&mut ed, "abc;\nd-[e]>f\n");
    ed.feed_key(key('.'));

    assert_eq!(
        ed.doc().text().to_string(),
        "abc;\ndef;\n",
        "replay must redo the goto-line-end motion before the ';' fallback, \
         appending the semicolon to the second line"
    );
}

// ── Interactive Insert-key bindings: picker ─────────────────────────────────

/// An Insert-key binding that opens a picker records its pick: `.` re-runs
/// the binding with the picker answered by that pick, never reopening it.
///
/// A live `i <ctrl-y> <enter>` deletes the word behind
/// the cursor ("abc ") with nothing typed in between, so `.` at the front
/// of "xyz" must delete "uvw " the same way, with no picker ever opening
/// during replay.
#[test]
fn dot_repeats_a_picker_pick_from_an_insert_key_binding() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("abc -[d]>ef\nuvw xyz\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "pick-word" ""
             (lambda (pane)
               (picker! pane (list (cons "alpha" "ALPHA"))
                 (test-picker-keys (lambda (payload)
                   (when payload (call! "delete-word-backward" pane))))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "pick-word")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    assert!(ed.state.input.picker().is_some(), "sanity: picker open");
    ed.feed_key(key_enter());
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "the pick's own edit must close the picker"
    );
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "def\nuvw xyz\n");

    let inputs = &ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action")
        .transcript;
    assert_eq!(inputs.len(), 2, "got {inputs:?}");
    assert!(
        matches!(&inputs[0], Input::Binding { name, answers, .. }
                if &**name == "pick-word"
                    && matches!(answers.as_slice(), [Answer::Pick(Pick { picked: Some(Picked::Choice(c)), .. })] if c.label == "alpha" && c.nth == 0))
            && is_binding(&inputs[1], "exit-insert"),
        "got {inputs:?}"
    );

    select_marked(&mut ed, "def\nuvw -[x]>yz\n");
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "def\nxyz\n",
        "replay must delete \"uvw \" directly, never reopen the picker"
    );
    assert!(ed.state.input.picker().is_none());
}

/// An edit landing through the shared funnel from anywhere else (a timer,
/// an LSP response) while a binding's picker sits open is not part of the
/// change: the binding records only its pick. Simulates that "anywhere
/// else" edit directly through `apply_doc_edit_grouped`, the one funnel a
/// real one would also use.
#[test]
fn dot_repeat_ignores_a_foreign_edit_made_while_a_picker_is_open() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("abc -[d]>ef\nuvw xyz\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "pick-word" ""
             (lambda (pane)
               (picker! pane (list (cons "alpha" "ALPHA"))
                 (test-picker-keys (lambda (payload)
                   (when payload (call! "delete-word-backward" pane))))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "pick-word")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    assert!(ed.state.input.picker().is_some(), "sanity: picker open");

    // A foreign edit, far from the cursor, landing on the same buffer while
    // the picker sits open, standing in for a timer/LSP edit unrelated to
    // the pick.
    let fp = FocusedPane::current(&ed.view);
    let (pid, bid) = (fp.pid(), fp.bid(&ed.view));
    let (buffers, mut stores) = ed.state.edit_parts();
    crate::editor::doc_ops::apply_doc_edit_grouped(buffers, &mut stores, pid, bid, |s| {
        hume_ops::edit::insert_char(s, 'Z')
    })
    .expect("the buffer is not held");
    assert_eq!(ed.doc().text().to_string(), "abc Zdef\nuvw xyz\n");

    ed.feed_key(key_enter());
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "the pick's own edit must close the picker"
    );
    ed.feed_key(key_esc());

    let inputs = &ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action")
        .transcript;
    assert_eq!(inputs.len(), 2, "got {inputs:?}");
    assert!(
        matches!(&inputs[0], Input::Binding { name, answers, .. }
                if &**name == "pick-word"
                    && matches!(answers.as_slice(), [Answer::Pick(Pick { picked: Some(Picked::Choice(c)), .. })] if c.label == "alpha" && c.nth == 0))
            && is_binding(&inputs[1], "exit-insert"),
        "the binding records its pick, and nothing of the foreign 'Z' insert: {inputs:?}"
    );
}

/// A binding that edits the buffer *before* opening a picker, with a foreign
/// edit landing on the same buffer while that picker sits open: the binding
/// is re-run on `.`, so what it edits is decided again there and the
/// foreign edit has nothing to break. Its pick is recorded with no warning.
#[test]
fn dot_repeat_keeps_a_pick_when_a_foreign_edit_follows_the_bindings_own_edit() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("abc -[d]>ef\nuvw xyz\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "edit-then-pick" ""
             (lambda (pane)
               (insert-key! pane "x")
               (picker! pane (list (cons "alpha" "ALPHA"))
                 (test-picker-keys (lambda (payload)
                   (when payload (call! "delete-word-backward" pane))))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "edit-then-pick")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    assert!(ed.state.input.picker().is_some(), "sanity: picker open");
    assert_eq!(
        ed.doc().text().to_string(),
        "abc xdef\nuvw xyz\n",
        "sanity: the binding's own edit landed before the picker opened"
    );

    // A foreign edit, landing on the same buffer while the picker sits open,
    // standing in for a timer/LSP edit unrelated to the pick.
    let fp = FocusedPane::current(&ed.view);
    let (pid, bid) = (fp.pid(), fp.bid(&ed.view));
    let (buffers, mut stores) = ed.state.edit_parts();
    crate::editor::doc_ops::apply_doc_edit_grouped(buffers, &mut stores, pid, bid, |s| {
        hume_ops::edit::insert_char(s, 'Z')
    })
    .expect("the buffer is not held");

    ed.feed_key(key_enter()); // picks "alpha", runs delete-word-backward
    ed.settle();
    ed.feed_key(key_esc());

    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Warning && e.text.contains("won't repeat")),
        "nothing to warn about: {:?}",
        ed.state.message_log.entries().collect::<Vec<_>>()
    );

    let inputs = &ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action")
        .transcript;
    assert!(
        matches!(inputs.first(), Some(Input::Binding { name, answers, .. })
            if &**name == "edit-then-pick"
                && matches!(answers.as_slice(), [Answer::Pick(Pick { picked: Some(Picked::Choice(c)), .. })] if c.label == "alpha" && c.nth == 0)),
        "the binding records its pick: {inputs:?}"
    );
}

/// A picker dismissed with Esc is recorded as a pick through the dismissing
/// key's binding: `.` re-runs the Insert binding with its picker answered by
/// that binding, which does nothing here, between the keys typed around it.
#[test]
fn dot_repeat_replays_a_dismissed_pick_as_a_dismissal() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "pick-word" ""
             (lambda (pane)
               (picker! pane (list (cons "alpha" "ALPHA"))
                 (test-picker-keys (lambda (payload)
                   (when payload (call! "delete-word-backward" pane))))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "pick-word")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key('x'));
    ed.feed_key(key('y'));
    ed.feed_key(key_ctrl('y'));
    ed.feed_key(key_esc()); // dismisses the picker, staying in Insert
    ed.settle();
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: still in Insert");
    ed.feed_key(key_esc()); // leaves Insert
    assert_eq!(ed.doc().text().to_string(), "xyhello\nworld\n");

    let inputs = &ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action")
        .transcript;
    assert_eq!(inputs.len(), 4, "got {inputs:?}");
    assert!(
        is_key(&inputs[0], 'x')
            && is_key(&inputs[1], 'y')
            && matches!(&inputs[2], Input::Binding { name, answers, .. }
                if &**name == "pick-word"
                    && matches!(answers.as_slice(), [Answer::Pick(Pick { action: Some(_), .. })]))
            && is_binding(&inputs[3], "exit-insert"),
        "got {inputs:?}"
    );

    select_marked(&mut ed, "xyhello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "xyhello\nxyworld\n");
}
