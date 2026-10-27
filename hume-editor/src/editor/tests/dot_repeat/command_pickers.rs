//! Command pickers and inputs from bindings that are not repeatable.

use super::*;
use pretty_assertions::assert_eq;

// ── Top-level command: picker ───────────────────────────────────────────────

/// Defines the repeatable `pk` from `body` (with `pane` in scope), runs it
/// on line 1 of `hello\nworld`, feeds `keys` one at a time, then moves to
/// line 2.
fn record_pk(tmp: &std::path::Path, body: &str, keys: &[KeyEvent]) -> Editor {
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp,
        &format!(r#"(define-command! "pk" "" (lambda (pane) {body}) #:repeatable #t)"#),
    );
    ed.execute_keymap_command("pk".into(), Count::typed(1), false);
    ed.settle();
    assert!(ed.state.input.picker().is_some(), "sanity: picker open");
    for &k in keys {
        ed.feed_key(k);
        ed.settle();
    }
    assert!(ed.state.input.picker().is_none(), "sanity: picker closed");
    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));
    ed
}

/// `on_select` for the picker tests: replaces with the payload's first
/// character and wraps it in parentheses, so a repeat of only its last
/// `call!` is visible.
const PK_SELECT: &str = r#"(lambda (p)
                 (when p (call! "replace" pane (string-ref p 0)) (call! "surround-add" pane #\()))"#;

/// `.` repeats the item the user picked, without opening the picker: the
/// pick and every edit its `on_select` made belong to the command's change.
#[test]
fn dot_repeats_a_commands_picker_selection() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        &format!(r#"(picker! pane (list (cons "a" "A") (cons "b" "B")) {PK_SELECT})"#),
        &[key_down(), key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "(B)ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "`.` must not open a picker"
    );
    assert_eq!(ed.doc().text().to_string(), "(B)ello\n(B)orld\n");
    assert!(
        !status(&ed).contains("reached during"),
        "got: {}",
        status(&ed)
    );
}

/// A pick made through an `#:actions` key is repeated through that action.
#[test]
fn dot_repeats_a_commands_picker_action() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        &format!(
            r#"(picker! pane (list (cons "a" "A")) {PK_SELECT}
                 #:actions (list (cons "ctrl-t"
                   (lambda (p) (call! "replace" pane (string-ref p 0)) (call! "surround-add" pane #\[)))))"#
        ),
        &[key_ctrl('t')],
    );
    assert_eq!(ed.doc().text().to_string(), "[A]ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "[A]ello\n[A]orld\n");
}

/// A dismissed picker is repeated as a dismissal: `on_select` gets `#f`.
#[test]
fn dot_repeats_a_commands_dismissed_picker() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        r#"(picker! pane (list (cons "a" "A"))
             (lambda (p)
               (unless p (call! "replace" pane #\z) (call! "surround-add" pane #\())))"#,
        &[key_esc()],
    );
    assert_eq!(ed.doc().text().to_string(), "(z)ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "(z)ello\n(z)orld\n");
}

/// A live picker's pick is repeated without feeding it any item.
#[test]
fn dot_repeats_a_commands_live_picker_selection() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        &format!(
            r#"(let ((tok (live-picker! pane {PK_SELECT} #:source (picker-command-source (lambda (q) #f)))))
                 (picker-push! tok (list (cons "b" "B"))))"#
        ),
        &[key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "(B)ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "`.` must not open a picker"
    );
    assert_eq!(ed.doc().text().to_string(), "(B)ello\n(B)orld\n");
}

/// A re-run command that opens a picker where the change recorded a
/// character read took a different branch: `.` refuses rather than open it.
#[test]
fn dot_refuses_a_picker_where_a_read_was_recorded() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define runs 0)
           (define-command! "br" ""
             (lambda (pane)
               (set! runs (+ runs 1))
               (if (= runs 1)
                   (on-next-char! (lambda (c) (when c (call! "replace" pane c))))
                   (picker! pane (list (cons "a" "A")) (lambda (p) (+ 1 0)))))
             #:repeatable #t)"#,
    );
    ed.execute_keymap_command("br".into(), Count::typed(1), false);
    ed.settle();
    ed.feed_key(key('z'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "zello\nworld\n");
    ed.feed_key(key('j'));
    ed.feed_key(key('.'));
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "`.` must not open a picker"
    );
    assert!(
        status(&ed).contains("reached during"),
        "got: {}",
        status(&ed)
    );
}

/// A reload keeps a change holding a `picker!` pick: `.` finds the picked
/// label among the items the reloaded command builds.
#[test]
fn reload_keeps_a_label_pick() {
    let tmp = tempfile::tempdir().unwrap();
    let body = format!(r#"(picker! pane (list (cons "a" "A")) {PK_SELECT})"#);
    let mut ed = record_pk(tmp.path(), &body, &[key_enter()]);
    assert_eq!(ed.doc().text().to_string(), "(A)ello\nworld\n");
    ed.reset_config_state();
    run(
        &mut ed,
        tmp.path(),
        &format!(r#"(define-command! "pk" "" (lambda (pane) {body}) #:repeatable #t)"#),
    );
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "(A)ello\n(A)orld\n");
}

/// A `live-picker!` pick is replayed from its payload, which a reload
/// forgets: `.` then refuses rather than guess.
#[test]
fn reload_forgets_a_live_picks_payload() {
    let tmp = tempfile::tempdir().unwrap();
    let body = format!(
        r#"(let ((tok (live-picker! pane {PK_SELECT} #:source (picker-command-source (lambda (q) #f)))))
             (picker-push! tok (list (cons "b" "B"))))"#
    );
    let mut ed = record_pk(tmp.path(), &body, &[key_enter()]);
    assert_eq!(ed.doc().text().to_string(), "(B)ello\nworld\n");
    ed.reset_config_state();
    run(
        &mut ed,
        tmp.path(),
        &format!(r#"(define-command! "pk" "" (lambda (pane) {body}) #:repeatable #t)"#),
    );
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "(B)ello\nworld\n");
    assert!(
        status(&ed).contains("a config reload forgot the picked value"),
        "got: {}",
        status(&ed)
    );
}

/// A `picker!` that gets its items after it opens is replayed from the
/// payload its pick got: the re-run picker holds none of them yet when `.`
/// answers it.
#[test]
fn dot_repeats_a_pick_from_items_pushed_after_the_picker_opened() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        &format!(
            r#"(let ((tok (picker! pane '() {PK_SELECT})))
                 (picker-push! tok (list (cons "b" "B"))))"#
        ),
        &[key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "(B)ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "`.` must not open a picker"
    );
    assert_eq!(ed.doc().text().to_string(), "(B)ello\n(B)orld\n");
}

/// A re-run `picker!` with no item of the picked text: `.` refuses and
/// names the item it looked for.
#[test]
fn dot_refuses_a_pick_whose_text_is_gone() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        &format!(
            r#"(picker! pane
                 (if (= (buffer-cursor-line pane) 0) (list (cons "b" "B")) (list (cons "c" "C")))
                 {PK_SELECT})"#
        ),
        &[key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "(B)ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "(B)ello\nworld\n");
    assert!(
        status(&ed).contains(r#"no item "b""#),
        "got: {}",
        status(&ed)
    );
}

/// A pick made through an `#:actions` key the re-run picker does not bind:
/// `.` refuses and says so.
#[test]
fn dot_refuses_a_pick_whose_action_key_is_unbound() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        &format!(
            r#"(picker! pane (list (cons "a" "A")) {PK_SELECT}
                 #:actions (if (= (buffer-cursor-line pane) 0)
                               (list (cons "ctrl-t" {PK_SELECT}))
                               '()))"#
        ),
        &[key_ctrl('t')],
    );
    assert_eq!(ed.doc().text().to_string(), "(A)ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "(A)ello\nworld\n");
    assert!(
        status(&ed).contains("not one of this picker's actions"),
        "got: {}",
        status(&ed)
    );
}

/// `.` matches a `picker!` pick by its label among the items the re-run
/// builds, so a payload that depends on where the command runs is the one
/// for the new spot.
#[test]
fn dot_matches_a_pick_by_label_at_the_new_spot() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        &format!(
            r#"(picker! pane
                 (list (cons "same" (if (= (buffer-cursor-line pane) 0) "A" "B")))
                 {PK_SELECT})"#
        ),
        &[key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "(A)ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "(A)ello\n(B)orld\n");
}

// ── Insert-key binding: reads and picks ─────────────────────────────────────

/// An Insert-key binding that opens a picker is re-run by `.`, its picker
/// answered with the recorded pick: what the pick's callback does is decided
/// again at the new cursor. Here it deletes the word behind the cursor,
/// which is longer on line 2 than on line 1.
#[test]
fn dot_reruns_an_insert_binding_answering_its_picker() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("abc -[d]>ef\nlonger xyz\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "pick-word" ""
             (lambda (pane)
               (picker! pane (list (cons "alpha" "ALPHA"))
                 (lambda (payload)
                   (when payload (call! "delete-word-backward" pane)))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "pick-word")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    ed.feed_key(key_enter());
    ed.settle();
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "def\nlonger xyz\n");

    select_marked(&mut ed, "def\nlonger -[x]>yz\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "`.` must not open a picker"
    );
    assert_eq!(ed.doc().text().to_string(), "def\nxyz\n");
}

/// An Insert-key binding that reads a character is re-run by `.` with the
/// recorded character.
#[test]
fn dot_reruns_an_insert_binding_answering_its_read() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("abc -[d]>ef\nlonger xyz\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "read-word" ""
             (lambda (pane)
               (on-next-char!
                 (lambda (c) (when (eqv? c #\w) (call! "delete-word-backward" pane)))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "read-word")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    ed.feed_key(key('w'));
    ed.settle();
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "def\nlonger xyz\n");

    select_marked(&mut ed, "def\nlonger -[x]>yz\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert!(!reading_char(&ed), "`.` must not wait for a key");
    assert_eq!(ed.doc().text().to_string(), "def\nxyz\n");
}

/// Keys `i a <ctrl-y> <answer> b <Esc>` on "hello": `ctrl-y` runs a
/// non-repeatable command whose `open` input, once answered, deletes the word
/// behind the cursor.
fn record_insert_with_unrepeatable_input(open: &str, answer: KeyEvent) -> Editor {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "ask" "" (lambda (pane) {open}))
               (bind-key! 'insert "ctrl-y" "ask")"#
        ),
    );
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key_ctrl('y'));
    ed.feed_key(answer);
    ed.settle();
    ed.feed_key(key('b'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "bhello\nworld\n");
    select_marked(&mut ed, "bhello\n-[w]>orld\n");
    ed
}

/// A picker a non-repeatable Insert binding opens ends the change: `.`
/// repeats the keys typed before that binding, not the binding, its pick,
/// or the keys after it.
#[test]
fn a_non_repeatable_insert_bindings_picker_ends_the_change() {
    let mut ed = record_insert_with_unrepeatable_input(
        r#"(picker! pane (list (cons "w" "W"))
             (lambda (p) (when p (call! "delete-word-backward" pane))))"#,
        key_enter(),
    );
    let transcript = &ed.state.last_action().expect("`i` recorded").transcript;
    assert!(
        matches!(transcript.as_slice(), [i] if is_key(i, 'a')),
        "got {transcript:?}"
    );
    ed.feed_key(key('.'));
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "`.` must not open a picker"
    );
    assert_eq!(ed.doc().text().to_string(), "bhello\naworld\n");
}

/// A character read a non-repeatable Insert binding opens ends the change,
/// the same as a picker.
#[test]
fn a_non_repeatable_insert_bindings_read_ends_the_change() {
    let mut ed = record_insert_with_unrepeatable_input(
        r#"(on-next-char!
             (lambda (c) (when (eqv? c #\w) (call! "delete-word-backward" pane))))"#,
        key('w'),
    );
    let transcript = &ed.state.last_action().expect("`i` recorded").transcript;
    assert!(
        matches!(transcript.as_slice(), [i] if is_key(i, 'a')),
        "got {transcript:?}"
    );
    ed.feed_key(key('.'));
    ed.settle();
    assert!(!reading_char(&ed), "`.` must not wait for a key");
    assert_eq!(ed.doc().text().to_string(), "bhello\naworld\n");
}

/// An answer whose binding entry a completion accept removed is dropped: it
/// never lands in the entry of a binding recorded after it. No key sequence
/// reaches this, since a waiting input takes every key, so it drives the
/// recording directly.
#[test]
fn a_stale_answer_never_lands_in_a_later_bindings_entry() {
    use crate::editor::replay::DotCapture;
    use crate::editor::replay::capture::DotChain;
    use crate::editor::replay::{
        AnswerTicket, InputOpener, RecordingScope, finalize_dot_capture, with_recording_scope,
    };

    fn open_input(ed: &mut Editor, scope: RecordingScope) -> AnswerTicket {
        with_recording_scope(&mut ed.state, Some(scope), |state| {
            state
                .waiting_input(InputOpener::Code)
                .expect("not during `.`")
                .take_answer()
        })
        .expect("a repeatable binding's input records")
    }

    let mut ed = editor_from("-[h]>ello\n");
    ed.feed_key(key('i'));

    let (a_entry, a) = ed
        .state
        .record_binding("a".into(), true)
        .expect("`i` is recording");
    let stale = open_input(&mut ed, a);

    let text = ed.doc().text().clone();
    let state = &mut ed.state;
    let session = state.input.insert_session().expect("`i` opened a session");
    finalize_dot_capture(
        &mut state.recording,
        session,
        DotCapture {
            chain: DotChain::new(test_fixtures::testing::at(&text, 0), &text),
            interactive: true,
            binding: Some(a_entry),
        },
    )
    .expect("nothing edited the buffer under the capture");

    let (_, b) = ed
        .state
        .record_binding("b".into(), true)
        .expect("`i` is still recording");
    let _b_ticket = open_input(&mut ed, b);

    ed.state.record_answer(&stale, Answer::Char(Some('z')));

    let transcript = &ed.state.last_action().expect("`i` recorded").transcript;
    assert!(
        matches!(transcript.as_slice(), [Input::Binding { name, answers, .. }]
            if name == "b" && matches!(answers.as_slice(), [Answer::Unanswered])),
        "got {transcript:?}"
    );
}

/// A reload while an Insert-key binding's picker is open releases that
/// change: a later repeatable command starts its own.
#[test]
fn reload_with_a_bindings_picker_open_releases_the_change() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "pick" ""
             (lambda (pane) (picker! pane (list (cons "a" "A")) (lambda (p) (+ 1 0)))))
           (bind-key! 'insert "ctrl-y" "pick")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    assert!(ed.state.input.picker().is_some(), "sanity: picker open");
    ed.reset_config_state();
    ed.feed_key(key('d'));
    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "ello\norld\n");
}
