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
    ed.execute_keymap_command("pk", Count::typed(1), false);
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
        &format!(
            r#"(picker! pane (list (cons "a" "A") (cons "b" "B")) (test-picker-keys {PK_SELECT}))"#
        ),
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

/// A pick is repeated through the binding of the key that closed the
/// picker.
#[test]
fn dot_repeats_a_commands_picker_through_the_closing_binding() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        r#"(picker! pane (list (cons "a" "A"))
             (hash "enter" (lambda (token p) (picker-close! token) (call! "replace" pane #\e))
                   "ctrl-t" (lambda (token p)
                     (picker-close! token)
                     (call! "replace" pane (string-ref p 0))
                     (call! "surround-add" pane #\[))))"#,
        &[key_ctrl('t')],
    );
    assert_eq!(ed.doc().text().to_string(), "[A]ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "[A]ello\n[A]orld\n");
}

/// A picker closed by a dismissing binding is repeated as a dismissal: the
/// dismissing key's binding runs again.
#[test]
fn dot_repeats_a_commands_dismissed_picker() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        r#"(picker! pane (list (cons "a" "A"))
             (test-picker-keys (lambda (p)
               (unless p (call! "replace" pane #\z) (call! "surround-add" pane #\()))))"#,
        &[key_esc()],
    );
    assert_eq!(ed.doc().text().to_string(), "(z)ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "(z)ello\n(z)orld\n");
}

/// Under `.`, `picker!` answers `#f`, as `show-menu!` does: the picker the
/// replayed body asks for never opens.
#[test]
fn a_replayed_picker_returns_false() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        &format!(
            r#"(log! 'warning (if (picker! pane (list (cons "a" "A")) (test-picker-keys {PK_SELECT})) "token" "no-token"))"#
        ),
        &[key_enter()],
    );
    ed.feed_key(key('.'));
    ed.settle();
    assert!(
        ed.state.message_log.entries().any(|e| e.text == "no-token"),
        "the replayed picker! must answer #f: {}",
        ed.state.message_log.format_for_display()
    );
}

/// The same holds for `live-picker!`.
#[test]
fn a_replayed_live_picker_returns_false() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        &format!(
            r#"(let ((tok (live-picker! pane (test-picker-keys {PK_SELECT}) #:source (picker-command-source (lambda (q) #f)))))
                 (log! 'warning (if tok "token" "no-token"))
                 (picker-push! tok (list (cons "b" "B"))))"#
        ),
        &[key_enter()],
    );
    ed.feed_key(key('.'));
    ed.settle();
    assert!(
        ed.state.message_log.entries().any(|e| e.text == "no-token"),
        "the replayed live-picker! must answer #f: {}",
        ed.state.message_log.format_for_display()
    );
}

/// A live picker's pick is repeated without feeding it any item.
#[test]
fn dot_repeats_a_commands_live_picker_selection() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        &format!(
            r#"(let ((tok (live-picker! pane (test-picker-keys {PK_SELECT}) #:source (picker-command-source (lambda (q) #f)))))
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
                   (picker! pane (list (cons "a" "A")) (test-picker-keys (lambda (p) (+ 1 0))))))
             #:repeatable #t)"#,
    );
    ed.execute_keymap_command("br", Count::typed(1), false);
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
    let body = format!(r#"(picker! pane (list (cons "a" "A")) (test-picker-keys {PK_SELECT}))"#);
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
        r#"(let ((tok (live-picker! pane (test-picker-keys {PK_SELECT}) #:source (picker-command-source (lambda (q) #f)))))
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
            r#"(let ((tok (picker! pane '() (test-picker-keys {PK_SELECT}))))
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
                 (test-picker-keys {PK_SELECT}))"#
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

/// A pick closed by a key the re-run picker's keymap does not bind: `.`
/// refuses and says so.
#[test]
fn dot_refuses_a_pick_whose_key_is_unbound() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        &format!(
            r#"(picker! pane (list (cons "a" "A"))
                 (if (= (buffer-cursor-line pane) 0)
                     (hash "ctrl-t" (lambda (token p) (picker-close! token) ({PK_SELECT} p)))
                     (hash)))"#
        ),
        &[key_ctrl('t')],
    );
    assert_eq!(ed.doc().text().to_string(), "(A)ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "(A)ello\nworld\n");
    assert!(
        status(&ed).contains("not bound in this picker's keymap"),
        "got: {}",
        status(&ed)
    );
}

/// `.` runs the closing binding the re-run command builds, so what the
/// binding captured is what the command saw at the new spot.
#[test]
fn dot_runs_the_rerun_commands_own_binding() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        r#"(let ((line (number->string (buffer-cursor-line pane))))
             (picker! pane (list (cons "a" "A"))
               (hash "enter" (lambda (token p)
                 (picker-close! token)
                 (call! "replace" pane (string-ref line 0))))))"#,
        &[key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "0ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "0ello\n1orld\n");
}

/// Bindings that leave the picker open (moving the selection, here) are not
/// part of the change: `.` replays only the binding that closed it, with the
/// item it was handed.
#[test]
fn dot_skips_bindings_that_left_the_picker_open() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        r#"(picker! pane (list (cons "a" #\a) (cons "b" #\b) (cons "c" #\c))
             (hash "down" (lambda (token c) (picker-move! token 1) (call! "replace" pane #\x))
                   "enter" (lambda (token c) (picker-close! token) (call! "replace" pane c))))"#,
        &[key_down(), key_down(), key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "cello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "cello\ncorld\n");
}

/// A picker closed by something other than one of its bindings replays as
/// that close: its `#:on-close` runs, and no binding does.
#[test]
fn dot_repeats_a_close_no_binding_made_through_on_close_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define tok #f)
           (define-command! "pk" ""
             (lambda (pane)
               (set! tok (picker! pane (list (cons "a" "A"))
                 (hash "enter" (lambda (token p) (picker-close! token) (call! "replace" pane #\e)))
                 #:on-close (lambda () (call! "surround-add" pane #\()))))
             #:repeatable #t)
           (define-command! "close-it" "" (lambda () (picker-close! tok)))"#,
    );
    ed.execute_keymap_command("pk", Count::typed(1), false);
    ed.settle();
    ed.execute_keymap_command("close-it", Count::typed(1), false);
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "(h)ello\nworld\n");
    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "(h)ello\n(w)orld\n");
}

/// A binding that closes its picker and opens another chains the two picks
/// into one change, each replayed through its own closing binding.
#[test]
fn dot_repeats_a_chain_of_pickers() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = record_pk(
        tmp.path(),
        r#"(picker! pane (list (cons "a" #\a) (cons "b" #\b))
             (hash "enter" (lambda (token first)
               (picker-close! token)
               (picker! pane (list (cons "paren" #\() (cons "bracket" #\[))
                 (hash "down" (lambda (token second) (picker-move! token 1))
                       "enter" (lambda (token second)
                         (picker-close! token)
                         (call! "replace" pane first)
                         (call! "surround-add" pane second)))))))"#,
        &[key_enter(), key_down(), key_enter()],
    );
    assert_eq!(ed.doc().text().to_string(), "[a]ello\nworld\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert!(
        ed.state.input.picker().is_none(),
        "`.` must not open either picker"
    );
    assert_eq!(ed.doc().text().to_string(), "[a]ello\n[a]orld\n");
}

/// A binding that closes a picker no change records runs as code outside
/// every change: a repeatable command it runs after the close is a change of
/// its own.
#[test]
fn a_repeatable_command_after_an_unrecorded_pickers_close_is_its_own_change() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "pick-op" ""
             (lambda (pane)
               (picker! pane (list (cons "delete" "delete"))
                 (hash "enter" (lambda (token name) (picker-close! token) (call! name pane))))))"#,
    );
    ed.execute_keymap_command("pick-op", Count::typed(1), false);
    ed.settle();
    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "ello\nworld\n");
    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));
    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.doc().text().to_string(), "ello\norld\n");
}

/// A binding that leaves its picker open is no change of its own: a
/// repeatable command it runs is not what `.` repeats.
#[test]
fn a_repeatable_command_from_a_binding_that_leaves_the_picker_open_is_no_change() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "pick-op" ""
             (lambda (pane)
               (picker! pane (list (cons "a" "A"))
                 (hash "ctrl-r" (lambda (token item) (call! "replace" pane #\z))
                       "esc" (lambda (token item) (picker-close! token))))))"#,
    );
    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "ello\nworld\n");

    ed.execute_keymap_command("pick-op", Count::typed(1), false);
    ed.settle();
    ed.feed_key(key_ctrl('r'));
    ed.settle();
    ed.feed_key(key_esc());
    ed.settle();
    assert!(ed.state.input.picker().is_none());
    assert_eq!(ed.doc().text().to_string(), "zllo\nworld\n");

    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(
        ed.doc().text().to_string(),
        "llo\nworld\n",
        "`.` repeats `d`, not the binding's replace"
    );
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
                 (test-picker-keys (lambda (payload)
                   (when payload (call! "delete-word-backward" pane))))))
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
             (test-picker-keys (lambda (p) (when p (call! "delete-word-backward" pane)))))"#,
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
            if &**name == "b" && matches!(answers.as_slice(), [Answer::Unanswered])),
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
             (lambda (pane) (picker! pane (list (cons "a" "A")) (test-picker-keys (lambda (p) (+ 1 0))))))
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
