//! `(paste-text! pane text)`: the Steel builtin that pastes `text` into
//! whatever has focus, through the same path a terminal paste takes.

use super::*;
use crate::editor::host_impl::EditorHostImpl;
use hume_core::PaneHandle;
use hume_scripting::host::EditHost;
use pretty_assertions::assert_eq;

const PROBE: &str = "paste-text-probe";

/// What a paste leaves behind that `paste-text!` must leave identically.
#[derive(Debug, PartialEq)]
struct Outcome {
    state: String,
    mode: Mode,
    minibuf: Option<String>,
    picker_query: Option<String>,
    popup: bool,
    reading_char: bool,
    queued_calls: usize,
}

fn outcome(ed: &Editor) -> Outcome {
    Outcome {
        state: state(ed),
        mode: ed.state.mode(),
        minibuf: ed.state.input.minibuf().map(|mb| mb.input.clone()),
        picker_query: ed.state.input.picker().map(|p| p.query().to_string()),
        popup: ed.state.input.popup().is_some(),
        reading_char: reading_char(ed),
        queued_calls: pending_calls(ed).len(),
    }
}

/// The outcomes of a terminal paste of `text` and of `paste-text!` with
/// the same `text`, each on its own editor over `initial` after `setup`.
/// `source` defines any command `setup` runs. `paste-text!` runs in a
/// command dispatched outside any key binding, as a hook's would.
fn terminal_paste_then_paste_text(
    initial: &str,
    source: &str,
    setup: impl Fn(&mut Editor),
    text: &str,
) -> (Outcome, Outcome) {
    let run = |scripted: bool| {
        let mut ed = editor_from(initial);
        attach_steel(
            &mut ed,
            &format!(
                r#"{source}
                   (define-command! "{PROBE}" "" (lambda (pane) (paste-text! pane {text:?})))"#
            ),
        );
        setup(&mut ed);
        if scripted {
            ed.execute_keymap_command(PROBE, Count::typed(1), false);
        } else {
            ed.feed_paste(text);
        }
        outcome(&ed)
    };
    (run(false), run(true))
}

fn insert_mode(ed: &mut Editor) {
    ed.feed_key(key('i'));
}

fn doc_text(o: &Outcome) -> String {
    parse_state(&o.state).0.to_string()
}

// ── The same as a paste ─────────────────────────────────────────────────────

#[test]
fn paste_text_pastes_at_every_insert_cursor() {
    let (pasted, scripted) =
        terminal_paste_then_paste_text("-{a}>b -[c]>d\n", "", insert_mode, "xy");
    assert_eq!(doc_text(&scripted), "xyab xycd\n");
    assert_eq!(scripted, pasted);
}

#[test]
fn paste_text_skips_auto_pairs_and_autoindent_like_a_paste() {
    let (pasted, scripted) =
        terminal_paste_then_paste_text("  -[h]>ello\n", "", insert_mode, "(\n");
    assert_eq!(doc_text(&scripted), "  (\nhello\n");
    assert_eq!(scripted, pasted);
}

#[test]
fn paste_text_normalizes_line_endings_like_a_paste() {
    let (pasted, scripted) =
        terminal_paste_then_paste_text("-[h]>ello\n", "", insert_mode, "a\r\nb");
    assert_eq!(doc_text(&scripted), "a\nbhello\n");
    assert_eq!(scripted, pasted);
}

#[test]
fn paste_text_replaces_normal_mode_selections_like_a_paste() {
    let (pasted, scripted) = terminal_paste_then_paste_text("-[hell]>o\n", "", |_| {}, "xyz");
    assert_eq!(doc_text(&scripted), "xyzo\n");
    assert_eq!(scripted, pasted);
}

#[test]
fn paste_text_flattens_into_the_command_line_like_a_paste() {
    let (pasted, scripted) =
        terminal_paste_then_paste_text("-[h]>ello\n", "", |ed| ed.feed_key(key(':')), "a\nb\n");
    assert_eq!(scripted.minibuf.as_deref(), Some("a b"));
    assert_eq!(scripted, pasted);
}

#[test]
fn paste_text_runs_the_live_search_like_a_paste() {
    let (pasted, scripted) = terminal_paste_then_paste_text(
        "-[h]>ello world\n",
        "",
        |ed| ed.feed_key(key('/')),
        "world",
    );
    assert_eq!(scripted.state, "hello -[world]>\n");
    assert_eq!(scripted, pasted);
}

#[test]
fn paste_text_runs_the_live_sift_like_a_paste() {
    let (pasted, scripted) =
        terminal_paste_then_paste_text("-[ab cd ab]>\n", "", |ed| ed.feed_key(key('s')), "ab");
    assert_eq!(scripted.state, "-{ab}> cd -[ab]>\n");
    assert_eq!(scripted, pasted);
}

#[test]
fn paste_text_lands_in_a_prompt_like_a_paste() {
    let (pasted, scripted) = terminal_paste_then_paste_text(
        "-[h]>ello\n",
        r#"(define-command! "ask" "" (lambda (pane) (prompt! pane "q: " (lambda (t) (+ 1 0)))))"#,
        |ed| ed.execute_keymap_command("ask", Count::typed(1), false),
        "abc",
    );
    assert_eq!(scripted.minibuf.as_deref(), Some("abc"));
    assert_eq!(scripted, pasted);
}

#[test]
fn paste_text_lands_in_the_picker_query_like_a_paste() {
    let (pasted, scripted) =
        terminal_paste_then_paste_text("-[h]>ello\n", "", open_live_test_picker, "foo\nbar\n");
    assert_eq!(scripted.picker_query.as_deref(), Some("foo bar"));
    assert_eq!(scripted.queued_calls, 1, "one query-change call");
    assert_eq!(scripted, pasted);
}

#[test]
fn paste_text_closes_a_popup_and_pastes_below_like_a_paste() {
    let (pasted, scripted) = terminal_paste_then_paste_text(
        "-[h]>ello\n",
        "",
        super::bracketed_paste::open_scrollable_test_popup,
        "xyz",
    );
    assert!(!scripted.popup);
    assert_eq!(doc_text(&scripted), "xyzhello\n");
    assert_eq!(scripted, pasted);
}

#[test]
fn paste_text_ends_a_pending_character_read_like_a_paste() {
    let (pasted, scripted) = terminal_paste_then_paste_text(
        "-[h]>ello\n",
        r#"(define-command! "read" "" (lambda () (on-next-char! (lambda (ch) (+ 1 0)))))"#,
        |ed| ed.execute_keymap_command("read", Count::typed(1), false),
        "xyz",
    );
    assert!(!scripted.reading_char);
    assert_eq!(scripted.queued_calls, 1, "the read's callback, with #f");
    assert_eq!(doc_text(&scripted), "hello\n");
    assert_eq!(scripted, pasted);
}

// ── As a builtin ────────────────────────────────────────────────────────────

#[test]
fn paste_text_is_visible_to_the_rest_of_the_body() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    bind_insert_key(
        &mut ed,
        tmp.path(),
        "ctrl-x",
        r#"(paste-text! pane "ab")
           (when (equal? (buffer-text pane) "abhello\n") (paste-text! pane "!"))"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    assert_eq!(ed.doc().text().to_string(), "ab!hello\n");
}

#[test]
fn paste_text_composes_into_the_insert_undo_group() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    bind_insert_key(&mut ed, tmp.path(), "ctrl-x", r#"(paste-text! pane "yz")"#);
    ed.feed_key(key('i'));
    ed.feed_key(key('x'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key('w'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "xyzwhello\n");
    ed.feed_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "hello\n");
}

#[test]
fn paste_text_refuses_for_an_unfocused_pane() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();
    let bid = ed.focused_buffer_id();
    let pid_b = vsplit_focused(&mut ed);
    assert_ne!(pid_b, pid_a);
    ed.state.focus.set_for_test(&mut ed.view, pid_a);

    let err = EditorHostImpl::new(&mut ed.state, &mut ed.view)
        .paste_text(
            hume_scripting::host::FocusedHandle::for_test(PaneHandle::with_pane(bid, pid_b)),
            vec!["x".to_string()],
        )
        .unwrap_err();
    assert!(err.contains("focused pane"), "got: {err}");
    assert_eq!(ed.doc().text().to_string(), "hello\n");
}

// ── Dot-repeat ──────────────────────────────────────────────────────────────

#[test]
fn paste_text_in_an_insert_binding_replays_once() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    bind_insert_key(&mut ed, tmp.path(), "ctrl-x", r#"(paste-text! pane "ab")"#);
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "abhello\n");

    select_marked(&mut ed, "abhel-[l]>o\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "abhelablo\n");
}

/// The callback runs in its binding's scope, so `.` re-runs the binding
/// with the recorded character and the text lands once.
#[test]
fn paste_text_from_a_repeatable_bindings_char_callback_replays_the_read_char() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "twice" ""
             (lambda (pane) (on-next-char! (lambda (ch) (paste-text! pane (string ch ch)))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-r" "twice")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('r'));
    ed.feed_key(key('x'));
    ed.settle();
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "xxhello\n");

    select_marked(&mut ed, "xxhel-[l]>o\n");
    ed.feed_key(key('.'));
    ed.settle();
    assert!(
        !reading_char(&ed),
        "the recorded character answers the read"
    );
    assert_eq!(ed.doc().text().to_string(), "xxhelxxlo\n");
}

/// A non-repeatable binding's read ends the change at that key, so `.`
/// repeats only what was typed before it.
#[test]
fn paste_text_from_a_non_repeatable_bindings_callback_is_not_replayed() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "twice" ""
             (lambda (pane) (on-next-char! (lambda (ch) (paste-text! pane (string ch ch))))))
           (bind-key! 'insert "ctrl-r" "twice")"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key('q'));
    ed.feed_key(key_ctrl('r'));
    ed.feed_key(key('x'));
    ed.settle();
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "qxxhello\n");

    select_marked(&mut ed, "qxxhel-[l]>o\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "qxxhelqlo\n");
}

/// Outside every binding the call is input arriving at the Insert session,
/// recorded as the paste it is.
#[test]
fn paste_text_outside_any_binding_is_recorded_as_a_paste() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(r#"(define-command! "{PROBE}" "" (lambda (pane) (paste-text! pane "ab")))"#),
    );
    ed.feed_key(key('i'));
    ed.execute_keymap_command(PROBE, Count::typed(1), false);
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "abhello\n");

    select_marked(&mut ed, "abhel-[l]>o\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "abhelablo\n");
}

#[test]
fn paste_text_of_a_list_outside_any_binding_replays_placed_again() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-{a}>b -[c]>d\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(r#"(define-command! "{PROBE}" "" (lambda (pane) (paste-text! pane '("x" "y"))))"#),
    );
    ed.feed_key(key('i'));
    ed.execute_keymap_command(PROBE, Count::typed(1), false);
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "xab ycd\n");

    select_marked(&mut ed, "xa-{b}> yc-[d]>\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "xaxb ycyd\n");
}
