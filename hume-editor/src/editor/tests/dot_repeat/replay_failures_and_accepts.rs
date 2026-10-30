//! Smart accepts, bindings that fail on replay, macros and the drain cap.

use super::*;
use pretty_assertions::assert_eq;

// ── Interactive Insert-key bindings: completion via a session query ────────

/// A binding that consults `completion-top` to decide its own branch: live,
/// with no completion open, it falls back to a literal tab, a
/// non-interactive outcome, so `.` re-runs the binding itself, which
/// re-checks `completion-top` fresh at the new cursor (also empty there)
/// and reaches the same fallback.
#[test]
fn dot_repeat_smart_accept_binding_fallback_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    let source = format!(
        "{}\n{}",
        completion_source("test", &completion_labels(&["hi"]), ""),
        r#"(define-command! "smart-accept" ""
             (lambda (pane)
               (if (null? (completion-top 1))
                   (insert-key! pane "tab")
                   (completion-accept! 0))))
           (bind-key! 'insert "ctrl-y" "smart-accept")"#
    );
    run(&mut ed, tmp.path(), &source);

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "\thello\nworld\n");

    let inputs = &ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action")
        .transcript;
    assert_eq!(inputs.len(), 2, "got {inputs:?}");
    assert!(
        is_binding(&inputs[0], "smart-accept") && is_binding(&inputs[1], "exit-insert"),
        "the fallback branch is non-interactive, recorded as a re-runnable Binding: {inputs:?}"
    );

    select_marked(&mut ed, "\thello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "\thello\n\tworld\n",
        "replay re-runs the binding, which reaches the same fallback at the new cursor"
    );
}

/// The same `smart-accept` binding's *accept* branch: live, with a
/// completion open, `completion-top` is non-empty and it accepts instead,
/// recorded as an `Input::Result`, not a `Binding`, so `.` applies
/// the accepted text directly rather than re-checking `completion-top`
/// (which would find no session during replay and wrongly fall back to a
/// literal tab).
#[test]
fn dot_repeat_smart_accept_binding_accept_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("(-[foo]>) (bar)\n");
    let source = format!(
        "{}\n{}",
        completion_source("test", &completion_labels(&["hello"]), ""),
        r#"(define-command! "smart-accept" ""
             (lambda (pane)
               (if (null? (completion-top 1))
                   (insert-key! pane "tab")
                   (completion-accept! 0))))
           (bind-key! 'insert "ctrl-y" "smart-accept")"#
    );
    run(&mut ed, tmp.path(), &source);

    ed.feed_key(key('c'));
    ed.feed_key(key('h'));
    ed.feed_key(key('e'));
    ed.feed_key(key_ctrl(' '));
    ed.settle();
    ed.feed_key(key_ctrl('y'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "(hello) (bar)\n");

    let inputs = &ed
        .state
        .last_action()
        .expect("`c` must have stamped a repeatable action")
        .transcript;
    assert_eq!(inputs.len(), 4, "got {inputs:?}");
    assert!(
        matches!(&inputs[2], Input::Result(r)
            if r.back == 2 && r.forward == 0 && r.text == "hello"),
        "got {inputs:?}"
    );

    select_marked(&mut ed, "(hello) (-[bar]>)\n");
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "(hello) (hello)\n",
        "replay must apply the recorded accept, never re-check completion-top"
    );
}

// ── Interactive Insert-key bindings: failure and mode-exit corner cases ────

/// A binding that errors (live and again on replay) is reported and
/// skipped, not fatal to the rest of the session: keys typed after it must
/// still replay, matching what the live session actually did.
#[test]
fn dot_repeat_continues_past_a_binding_that_fails_on_replay() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "sometimes-fails" "" (lambda (pane) (error "boom")))
           (bind-key! 'insert "ctrl-x" "sometimes-fails")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key_ctrl('x')); // errors live too, reported
    assert!(status(&ed).contains("boom"), "got: {}", status(&ed));
    ed.feed_key(key('b'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "abhello\nworld\n");

    select_marked(&mut ed, "abhello\n-[w]>orld\n");
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "abhello\nabworld\n",
        "replay must still type 'b' after the failing binding, not stop there"
    );
}

/// A binding whose branch depends on mutable Steel state takes a
/// non-interactive path live (recorded as a re-runnable `Binding`) but the
/// interactive one on replay (the same global flipped by the live run):
/// `completion-accept!` refuses loudly instead of silently accepting
/// against a session that was never opened this time.
#[test]
fn dot_repeat_binding_taking_the_interactive_branch_only_on_replay_errors_loudly() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define first-run #t)
           (define-command! "toggle" ""
             (lambda (pane)
               (if first-run
                   (begin (set! first-run #f) (insert-key! pane "tab"))
                   (completion-accept! 0))))
           (bind-key! 'insert "ctrl-x" "toggle")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "\thello\nworld\n");

    select_marked(&mut ed, "\thello\n-[w]>orld\n");
    ed.feed_key(key('.'));

    assert!(
        status(&ed).contains("reached during"),
        "got: {}",
        status(&ed)
    );
    assert!(
        ed.state.input.picker().is_none(),
        "the guard must refuse before touching any session"
    );
    assert_eq!(
        ed.doc().text().to_string(),
        "\thello\nworld\n",
        "the failed toggle inserted nothing, but the replayed exit-insert still ran"
    );
}

// ── A replayed macro that leaves Insert open ───────────────────────────────

/// A replayed macro can leave Insert mode open (its own Esc rebound to a
/// no-op before replay, so the macro's queue drains while still in Insert).
/// The session the user finishes by hand belongs to the macro's own entry
/// command, and `.` repeats all of it.
#[test]
fn dot_after_macro_leaves_insert_open_repeats_the_macros_own_session() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>yz\nabc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "stay-in-insert" "" (lambda (pane) (begin)))
           (bind-key! 'insert "escape" "stay-in-insert")"#,
    );

    ed.feed_key(key('d')); // stamps "delete", must not survive the macro below
    assert_eq!(ed.doc().text().to_string(), "yz\nabc\n");

    ed.state
        .registers
        .write_macro('q', vec![key('i'), key('f'), key('o'), key('o'), key_esc()]);
    ed.feed_key(key('q'));
    ed.feed_key(key('q'));
    ed.settle();

    assert_eq!(
        ed.state.mode(),
        Mode::Insert,
        "sanity: the rebound Esc must not have left Insert"
    );
    assert_eq!(ed.doc().text().to_string(), "fooyz\nabc\n");

    ed.feed_key(key('b'));
    ed.feed_key(key('a'));
    ed.feed_key(key('r'));
    ed.feed_key(key_ctrl('c')); // the real exit
    assert_eq!(ed.doc().text().to_string(), "foobaryz\nabc\n");

    select_marked(&mut ed, "foobaryz\n-[a]>bc\n");
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "foobaryz\nfoobarabc\n",
        "`.` must repeat the macro's own `i` session (foo, stay-in-insert, bar, exit), \
         not the pre-macro `delete`"
    );
}

/// `completion-trigger` refuses loudly under `.`, the same discipline
/// `completion-accept!`/`picker!` follow: a binding that reaches it during
/// replay decided differently than it did live, and there is no completion
/// request to make sense of replaying.
#[test]
fn dot_repeat_completion_trigger_reached_only_on_replay_errors_loudly() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define first-run #t)
           (define-command! "toggle" ""
             (lambda (pane)
               (if first-run
                   (begin (set! first-run #f) (insert-key! pane "tab"))
                   (call! "completion-trigger" pane))))
           (bind-key! 'insert "ctrl-x" "toggle")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "\thello\nworld\n");

    select_marked(&mut ed, "\thello\n-[w]>orld\n");
    ed.feed_key(key('.'));

    assert!(
        status(&ed).contains("reached during"),
        "got: {}",
        status(&ed)
    );
    assert!(
        ed.state.input.buffer_completion().is_none(),
        "the guard must refuse before opening anything"
    );
    assert_eq!(ed.doc().text().to_string(), "\thello\nworld\n");
}

/// A `Binding` that leaves Insert and then fails must stop the replay loop
/// there (the same as one that leaves Insert and succeeds), not keep going
/// and type the rest of the recorded session's keys outside any Insert
/// session, in whatever mode is now current.
#[test]
fn dot_repeat_stops_when_a_binding_leaves_insert_and_then_fails_on_replay() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define first-run #t)
           (define-command! "toggle" ""
             (lambda (pane)
               (if first-run
                   (begin (set! first-run #f) (insert-key! pane "tab"))
                   (begin (call! "exit-insert" pane) (error "boom")))))
           (bind-key! 'insert "ctrl-x" "toggle")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x')); // first-run: inserts a tab, stays in Insert
    ed.feed_key(key('q')); // typed after it, into the same session
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "\tqhello\nworld\n");

    select_marked(&mut ed, "\tqhello\n-[w]>orld\n");
    ed.feed_key(key('.'));

    assert!(status(&ed).contains("boom"), "got: {}", status(&ed));
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(
        ed.doc().text().to_string(),
        "\tqhello\nworld\n",
        "toggle's own exit-insert ran, but 'q' and the replayed exit-insert \
         must not run after replay has already left Insert"
    );
}

// ── Interactive edits captured at the funnel, not diffed after the fact ────

/// An accept whose item carries `additionalTextEdits` edits two separate
/// regions of the buffer in one dispatch (the import, then the cursor's own
/// replacement): recording must capture only the cursor's own region, not
/// error out over the extra one.
#[test]
fn dot_repeat_of_an_accept_with_additional_text_edits_replays_only_the_cursor_edit() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("(-[x]>) (bar)\n");
    let source = completion_source(
        "test",
        r#"(list (hash "label" "hello" "insertText" "hello"
                       "additionalTextEdits"
                         (list (hash "range" (hash "start" (hash "line" 0 "character" 0)
                                                  "end" (hash "line" 0 "character" 0))
                                 "newText" "// "))))"#,
        "",
    );
    run(&mut ed, tmp.path(), &source);

    ed.feed_key(key('c'));
    ed.feed_key(key('h'));
    ed.feed_key(key('e'));
    ed.feed_key(key_ctrl(' '));
    ed.settle();
    ed.feed_key(key_enter());
    assert!(
        !status(&ed).contains("dot-repeat"),
        "accept must not report a multi-region error: {}",
        status(&ed)
    );
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "// (hello) (bar)\n");

    select_marked(&mut ed, "// (hello) (-[bar]>)\n");
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "// (hello) (hello)\n",
        "`.` must replay only the cursor's own replacement, not re-insert the import"
    );
}

/// A multi-cursor accept composes one `ChangeSet` with an edited region at
/// every cursor: recording must extract only the primary's own region
/// instead of erroring over the others.
#[test]
fn dot_repeat_of_a_multi_cursor_accept_replays_at_every_cursor() {
    let mut ed = editor_from("-{foo}> -[bar]>\nfoo bar\n");
    ed.feed_key(key('c'));
    type_chars(&mut ed, "st");
    open_completion_session(&mut ed, &["std"]);
    ed.feed_key(key_enter());
    assert!(
        !status(&ed).contains("dot-repeat"),
        "a multi-cursor accept must not report a multi-region error: {}",
        status(&ed)
    );
    ed.feed_key(key('!'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "std! std!\nfoo bar\n");

    select_marked(&mut ed, "std! std!\n-{foo}> -[bar]>\n");
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "std! std!\nstd! std!\n");
}

/// Typing, moving the cursor away, typing again, then accepting: the
/// session's cumulative changeset has two separate insert regions (one per
/// typed run) by the time the accept lands, but only the accept's own
/// dispatch (the window `DotCapture` is armed for) feeds its capture.
#[test]
fn dot_repeat_of_an_accept_after_typing_moving_and_typing_again() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("( )\n(-[ ]>)\n");
    run(
        &mut ed,
        tmp.path(),
        &completion_source("test", &completion_labels(&["hello"]), ""),
    );

    ed.feed_key(key('i'));
    ed.feed_key(key('z')); // typed run #1, on line 2
    ed.feed_key(key_up()); // moves to line 1, same column, inside "( )"
    ed.feed_key(key('h'));
    ed.feed_key(key('e')); // typed run #2, on line 1
    ed.feed_key(key_ctrl(' '));
    ed.settle();
    ed.feed_key(key_enter()); // accepts "hello" over "he"
    assert!(
        !status(&ed).contains("dot-repeat"),
        "a delta spanning two disjoint typed runs must not error: {}",
        status(&ed)
    );
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "( hello)\n(z )\n");
}

/// A binding that calls `completion-accept!` and then `exit-insert` in the
/// same body tears the Insert session down from inside its own dispatch,
/// before `handle_insert`'s post-dispatch checkpoint ever runs:
/// `tear_down_insert`'s own backstop must still finalize the capture, not
/// lose the accept.
#[test]
fn dot_repeat_of_a_binding_that_accepts_then_exits_insert_in_one_body() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("(-[x]>) (bar)\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            "{}\n{}",
            completion_source("test", &completion_labels(&["hello"]), ""),
            r#"(define-command! "accept-and-leave" ""
                 (lambda (pane)
                   (completion-accept! 0)
                   (call! "exit-insert" pane)))
               (bind-key! 'insert "ctrl-y" "accept-and-leave")"#
        ),
    );

    ed.feed_key(key('c'));
    ed.feed_key(key('h'));
    ed.feed_key(key('e'));
    ed.feed_key(key_ctrl(' '));
    ed.settle();
    ed.feed_key(key_ctrl('y'));
    assert_eq!(
        ed.state.mode(),
        Mode::Normal,
        "accept-and-leave must leave Insert"
    );
    assert_eq!(ed.doc().text().to_string(), "(hello) (bar)\n");

    select_marked(&mut ed, "(hello) (-[bar]>)\n");
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        "(hello) (hello)\n",
        "`.` must reproduce the accept, not lose it to the same-dispatch exit-insert"
    );
}

/// A `MAX_EVENT_DRAIN` cascade drop that catches the batch holding a
/// picker's own `#:on-close`: the pick the user made is still the binding's
/// answer, and the session keeps recording what is typed after it.
#[test]
fn dot_repeat_keeps_a_pick_whose_on_close_is_lost_to_the_drain_cap() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "pick-word" ""
             (lambda (pane)
               (picker! pane (list (cons "alpha" "ALPHA"))
                 (test-picker-keys (lambda (payload) (void)))
                 #:on-close (lambda () (void))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "pick-word")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    assert!(ed.state.input.picker().is_some(), "sanity: picker open");
    ed.feed_key(key_enter()); // queues the binding that closes the picker
    let binding = ed
        .state
        .config
        .pending_work
        .pop_front()
        .expect("Enter queued its binding");
    ed.run_pending_batch(std::collections::VecDeque::from([binding]));
    assert!(
        ed.state.input.picker().is_none(),
        "sanity: the binding closed it"
    );
    assert!(
        !ed.state.config.pending_work.is_empty(),
        "sanity: #:on-close is queued, not yet run"
    );

    // Pad the same batch past the drain cap (1000, `event/pump.rs`'s
    // `MAX_EVENT_DRAIN`), so the whole thing, `#:on-close` included, is
    // dropped without ever running.
    for _ in 0..1100 {
        ed.state
            .config
            .pending_work
            .push_back(crate::editor::event::PendingWork::Event(
                crate::editor::event::EditorEvent::OnFocusGained,
            ));
    }
    ed.settle();
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Error
                && e.text.contains("event/callback cascade exceeded")),
        "the drain cap must have tripped"
    );

    assert!(
        matches!(
            ed.state
                .last_action()
                .and_then(|a| a.transcript.last()),
            Some(Input::Binding { name, answers, .. })
                if &**name == "pick-word"
                    && matches!(answers.as_slice(), [Answer::Pick(Pick { picked: Some(Picked::Choice(c)), .. })] if c.label == "alpha" && c.nth == 0)
        ),
        "the binding records the pick the user made"
    );

    // The session must still be usable: later typing records normally.
    ed.feed_key(key('z'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "zhello\n");
    let inputs = &ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action")
        .transcript;
    assert!(
        inputs.iter().any(|i| is_key(i, 'z')),
        "typing after the dropped callback must still be recorded: {inputs:?}"
    );
}

/// `PickerLayer::tear_down`'s own hand-off, reached when a picker is
/// removed by something other than `close_picker`/`close_picker_with`
/// (every other picker test in this file closes through a binding's
/// `picker-close!`, which routes through those). `end_insert_session` (what
/// a reload or a focus/buffer switch runs) truncates the Insert layer
/// top-first, tearing down the Picker on top of it via `tear_down` (not
/// `close_picker`) and then Insert itself. `#:on-close` must still fire,
/// after the session is gone, and the binding stays recorded with its
/// picker answered as a close no binding made.
#[test]
fn picker_torn_down_by_a_reload_fires_on_close_and_records_a_dismissal() {
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
                   (when payload (call! "delete-word-backward" pane))))
                 #:on-close (lambda () (log! 'info "closed"))))
             #:repeatable #t)
           (bind-key! 'insert "ctrl-y" "edit-then-pick")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('y'));
    assert!(ed.state.input.picker().is_some(), "sanity: picker open");
    assert_eq!(ed.doc().text().to_string(), "abc xdef\nuvw xyz\n");

    // Simulate the teardown a reload or a focus/buffer switch runs
    // (`end_focus_sessions`'s own chokepoint) while the picker still sits
    // on top of the Insert session.
    crate::editor::commands::end_insert_session(&mut ed.state, &ed.view);
    ed.settle();

    assert!(ed.state.input.picker().is_none(), "the picker must be gone");
    assert_ne!(
        ed.state.mode(),
        Mode::Insert,
        "the Insert session must be gone too"
    );
    assert_eq!(
        ed.doc().text().to_string(),
        "abc xdef\nuvw xyz\n",
        "no binding runs for a torn-down picker"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("closed"),
        "#:on-close must have run, not been skipped"
    );

    let inputs = &ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action")
        .transcript;
    assert!(
        matches!(inputs.as_slice(), [Input::Binding { name, answers, .. }]
            if &**name == "edit-then-pick"
                && matches!(answers.as_slice(), [Answer::Pick(Pick { closing_key: None, picked: None })])),
        "the binding is recorded, its torn-down picker as a dismissal: {inputs:?}"
    );
}

/// An edit before the last insertion moves it; `mii` follows it.
#[test]
fn mii_follows_the_insertion_when_text_before_it_changes() {
    let mut ed = editor_from("abc-[d]>ef\n");
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key('Y'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "abcXYdef\n");

    set_cursor(&mut ed, 0);
    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "bcXYdef\n");

    ed.feed_key(key('m'));
    ed.feed_key(key('i'));
    ed.feed_key(key('i'));
    assert_eq!(state(&ed), "bc-[XY]>def\n");
}

/// Deleting the last insertion outright leaves nothing for `mii` to select.
#[test]
fn mii_refuses_once_the_insertion_is_deleted() {
    let mut ed = editor_from("-[a]>bc\n");
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "Xabc\n");

    set_cursor(&mut ed, 0);
    ed.feed_key(key('d'));
    assert_eq!(ed.doc().text().to_string(), "abc\n");
    let before = state(&ed);

    ed.feed_key(key('m'));
    ed.feed_key(key('i'));
    ed.feed_key(key('i'));
    assert_eq!(state(&ed), before, "no insertion is left to select");
}
