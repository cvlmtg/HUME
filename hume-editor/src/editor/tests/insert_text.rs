//! `(insert-text! pane text)`: the Steel builtin that pastes `text` into
//! whatever has focus, through the same path a terminal paste takes.

use super::*;
use crate::editor::host_impl::EditorHostImpl;
use hume_scripting::PaneHandle;
use hume_scripting::host::{CommandHost, EditHost, NativeCall, PopupKind, UiHost};
use pretty_assertions::assert_eq;

const PROBE: &str = "insert-text-probe";

/// What a paste leaves behind that `insert-text!` must leave identically.
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

/// The outcomes of a terminal paste of `text` and of `insert-text!` with
/// the same `text`, each on its own editor over `initial` after `setup`.
/// `source` defines any command `setup` runs. `insert-text!` runs in a
/// command dispatched outside any key binding, as a hook's would.
fn paste_then_insert_text(
    initial: &str,
    source: &str,
    setup: impl Fn(&mut Editor),
    text: &str,
) -> (Outcome, Outcome) {
    let run = |insert_text: bool| {
        let mut ed = editor_from(initial);
        attach_steel(
            &mut ed,
            &format!(
                r#"{source}
                   (define-command! "{PROBE}" "" (lambda (pane) (insert-text! pane {text:?})))"#
            ),
        );
        setup(&mut ed);
        if insert_text {
            ed.execute_keymap_command(PROBE.into(), Some(1), false);
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
fn insert_text_pastes_at_every_insert_cursor() {
    let (pasted, inserted) = paste_then_insert_text("-{a}>b -[c]>d\n", "", insert_mode, "xy");
    assert_eq!(doc_text(&inserted), "xyab xycd\n");
    assert_eq!(inserted, pasted);
}

#[test]
fn insert_text_skips_auto_pairs_and_autoindent_like_a_paste() {
    let (pasted, inserted) = paste_then_insert_text("  -[h]>ello\n", "", insert_mode, "(\n");
    assert_eq!(doc_text(&inserted), "  (\nhello\n");
    assert_eq!(inserted, pasted);
}

#[test]
fn insert_text_normalizes_line_endings_like_a_paste() {
    let (pasted, inserted) = paste_then_insert_text("-[h]>ello\n", "", insert_mode, "a\r\nb");
    assert_eq!(doc_text(&inserted), "a\nbhello\n");
    assert_eq!(inserted, pasted);
}

#[test]
fn insert_text_replaces_normal_mode_selections_like_a_paste() {
    let (pasted, inserted) = paste_then_insert_text("-[hell]>o\n", "", |_| {}, "xyz");
    assert_eq!(doc_text(&inserted), "xyzo\n");
    assert_eq!(inserted, pasted);
}

#[test]
fn insert_text_flattens_into_the_command_line_like_a_paste() {
    let (pasted, inserted) =
        paste_then_insert_text("-[h]>ello\n", "", |ed| ed.feed_key(key(':')), "a\nb\n");
    assert_eq!(inserted.minibuf.as_deref(), Some("a b"));
    assert_eq!(inserted, pasted);
}

#[test]
fn insert_text_runs_the_live_search_like_a_paste() {
    let (pasted, inserted) =
        paste_then_insert_text("-[h]>ello world\n", "", |ed| ed.feed_key(key('/')), "world");
    assert_eq!(inserted.state, "hello -[world]>\n");
    assert_eq!(inserted, pasted);
}

#[test]
fn insert_text_runs_the_live_sift_like_a_paste() {
    let (pasted, inserted) =
        paste_then_insert_text("-[ab cd ab]>\n", "", |ed| ed.feed_key(key('s')), "ab");
    assert_eq!(inserted.state, "-{ab}> cd -[ab]>\n");
    assert_eq!(inserted, pasted);
}

#[test]
fn insert_text_lands_in_a_prompt_like_a_paste() {
    let (pasted, inserted) = paste_then_insert_text(
        "-[h]>ello\n",
        r#"(define-command! "ask" "" (lambda (pane) (prompt! pane "q: " (lambda (t) (+ 1 0)))))"#,
        |ed| ed.execute_keymap_command("ask".into(), Some(1), false),
        "abc",
    );
    assert_eq!(inserted.minibuf.as_deref(), Some("abc"));
    assert_eq!(inserted, pasted);
}

#[test]
fn insert_text_lands_in_the_picker_query_like_a_paste() {
    let (pasted, inserted) = paste_then_insert_text(
        "-[h]>ello\n",
        "",
        super::bracketed_paste::open_live_test_picker,
        "foo\nbar\n",
    );
    assert_eq!(inserted.picker_query.as_deref(), Some("foo bar"));
    assert_eq!(inserted.queued_calls, 1, "one query-change call");
    assert_eq!(inserted, pasted);
}

#[test]
fn insert_text_closes_a_popup_and_pastes_below_like_a_paste() {
    let (pasted, inserted) = paste_then_insert_text(
        "-[h]>ello\n",
        "",
        |ed| {
            ed.feed_key(key('i'));
            let pane = focused_pane(ed);
            EditorHostImpl::new(&mut ed.state, &mut ed.view)
                .show_popup(pane, "doc".to_string(), PopupKind::Scrollable, false, None)
                .expect("show-popup! succeeds");
        },
        "xyz",
    );
    assert!(!inserted.popup);
    assert_eq!(doc_text(&inserted), "xyzhello\n");
    assert_eq!(inserted, pasted);
}

#[test]
fn insert_text_ends_a_pending_character_read_like_a_paste() {
    let (pasted, inserted) = paste_then_insert_text(
        "-[h]>ello\n",
        r#"(define-command! "read" "" (lambda () (on-next-char! (lambda (ch) (+ 1 0)))))"#,
        |ed| ed.execute_keymap_command("read".into(), Some(1), false),
        "xyz",
    );
    assert!(!inserted.reading_char);
    assert_eq!(inserted.queued_calls, 1, "the read's callback, with #f");
    assert_eq!(doc_text(&inserted), "hello\n");
    assert_eq!(inserted, pasted);
}

// ── As a builtin ────────────────────────────────────────────────────────────

#[test]
fn insert_text_is_visible_to_the_rest_of_the_body() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    bind_insert_key(
        &mut ed,
        tmp.path(),
        "ctrl-x",
        r#"(insert-text! pane "ab")
           (when (equal? (buffer-text pane) "abhello\n") (insert-text! pane "!"))"#,
    );
    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    assert_eq!(ed.doc().text().to_string(), "ab!hello\n");
}

#[test]
fn insert_text_composes_into_the_insert_undo_group() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    bind_insert_key(&mut ed, tmp.path(), "ctrl-x", r#"(insert-text! pane "yz")"#);
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
fn insert_text_refuses_for_an_unfocused_pane() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.view.focused_pane();
    let bid = ed.focused_buffer_id();
    live_host!(ed)
        .run_command_sync(
            "pane-vsplit",
            PaneHandle::with_pane(bid, pid_a),
            NativeCall {
                count: Some(1),
                ..NativeCall::default()
            },
        )
        .expect("pane-vsplit succeeds");
    let pid_b = ed.view.focused_pane();
    assert_ne!(pid_b, pid_a);
    ed.state.focus.set_for_test(&mut ed.view, pid_a);

    let err = EditorHostImpl::new(&mut ed.state, &mut ed.view)
        .insert_text(PaneHandle::with_pane(bid, pid_b), "x".to_string())
        .unwrap_err();
    assert!(err.contains("focused pane"), "got: {err}");
    assert_eq!(ed.doc().text().to_string(), "hello\n");
}

// ── Dot-repeat ──────────────────────────────────────────────────────────────

#[test]
fn insert_text_in_an_insert_binding_replays_once() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    bind_insert_key(&mut ed, tmp.path(), "ctrl-x", r#"(insert-text! pane "ab")"#);
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
fn insert_text_from_a_repeatable_bindings_char_callback_replays_the_read_char() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "twice" ""
             (lambda (pane) (on-next-char! (lambda (ch) (insert-text! pane (string ch ch)))))
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
fn insert_text_from_a_non_repeatable_bindings_callback_is_not_replayed() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "twice" ""
             (lambda (pane) (on-next-char! (lambda (ch) (insert-text! pane (string ch ch))))))
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
fn insert_text_outside_any_binding_is_recorded_as_a_paste() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(r#"(define-command! "{PROBE}" "" (lambda (pane) (insert-text! pane "ab")))"#),
    );
    ed.feed_key(key('i'));
    ed.execute_keymap_command(PROBE.into(), Some(1), false);
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "abhello\n");

    select_marked(&mut ed, "abhel-[l]>o\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "abhelablo\n");
}
