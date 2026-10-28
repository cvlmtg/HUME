//! Command history, parsing, and case-insensitive names.

use super::*;
use pretty_assertions::assert_eq;

// ── Command history ───────────────────────────────────────────────────────────

/// Helper: open the command minibuffer, press Up once, return the current input.
fn open_and_up(ed: &mut Editor) -> String {
    ed.handle_key(key(':'));
    ed.handle_key(key_up());
    ed.state
        .minibuf()
        .map(|m| m.input.clone())
        .unwrap_or_default()
}

#[test]
fn up_recalls_previous_command() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "messages");
    assert_eq!(open_and_up(&mut ed), "messages");
}

#[test]
fn second_up_recalls_older() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "messages");
    submit(&mut ed, "q");
    ed.handle_key(key(':'));
    ed.handle_key(key_up());
    assert_eq!(ed.state.minibuf().unwrap().input, "q");
    ed.handle_key(key_up());
    assert_eq!(ed.state.minibuf().unwrap().input, "messages");
    // Cancel to leave normal mode.
    ed.handle_key(key_esc());
}

#[test]
fn down_walks_forward() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "messages");
    submit(&mut ed, "q");
    ed.handle_key(key(':'));
    ed.handle_key(key_up()); // "q"
    ed.handle_key(key_up()); // "messages"
    ed.handle_key(key_down()); // back to "q"
    assert_eq!(ed.state.minibuf().unwrap().input, "q");
    ed.handle_key(key_esc());
}

#[test]
fn down_past_newest_restores_scratch() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "messages");
    ed.handle_key(key(':'));
    for ch in "foo".chars() {
        ed.handle_key(key(ch));
    } // in-progress "foo"
    ed.handle_key(key_up()); // stash "foo", show "messages"
    ed.handle_key(key_down()); // past newest → restore "foo"
    assert_eq!(ed.state.minibuf().unwrap().input, "foo");
    assert_eq!(ed.state.minibuf().unwrap().cursor, 3);
    ed.handle_key(key_esc());
}

#[test]
fn down_without_prior_up_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "messages");
    ed.handle_key(key(':'));
    for ch in "foo".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_down()); // not navigating: no-op
    assert_eq!(ed.state.minibuf().unwrap().input, "foo");
    ed.handle_key(key_esc());
}

#[test]
fn empty_history_up_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key_up()); // empty history: input unchanged
    assert_eq!(ed.state.minibuf().unwrap().input, "");
    ed.handle_key(key_esc());
}

#[test]
fn at_oldest_up_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "messages");
    ed.handle_key(key(':'));
    ed.handle_key(key_up()); // lands on "messages"
    ed.handle_key(key_up()); // already at oldest: no change
    assert_eq!(ed.state.minibuf().unwrap().input, "messages");
    ed.handle_key(key_esc());
}

#[test]
fn consecutive_duplicate_not_recorded() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "messages");
    submit(&mut ed, "messages"); // duplicate: should be skipped
    ed.handle_key(key(':'));
    ed.handle_key(key_up()); // should land on "messages"
    assert_eq!(ed.state.minibuf().unwrap().input, "messages");
    ed.handle_key(key_up()); // at oldest: no older entry
    assert_eq!(ed.state.minibuf().unwrap().input, "messages");
    ed.handle_key(key_esc());
}

#[test]
fn failing_command_is_still_recorded() {
    // Unknown commands are recorded so the user can Up, fix the typo, and re-submit.
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "qit"); // typo: reports "unknown command: qit"
    assert_eq!(open_and_up(&mut ed), "qit");
}

#[test]
fn empty_confirm_not_recorded() {
    let mut ed = editor_from("-[h]>ello\n");
    // Press Enter with empty input: ConfirmEmpty, should not add an entry.
    ed.handle_key(key(':'));
    ed.handle_key(key_enter()); // ConfirmEmpty
    ed.handle_key(key(':'));
    ed.handle_key(key_up()); // no entry to recall: input stays empty
    assert_eq!(ed.state.minibuf().unwrap().input, "");
    ed.handle_key(key_esc());
}

#[test]
fn edit_after_up_demotes_scratch() {
    let mut ed = editor_from("-[h]>ello\n");
    // "messagesxtra" is the only entry that prefix-matches the text the
    // user will type after recalling "messages" below.
    submit(&mut ed, "messagesxtra");
    submit(&mut ed, "othercmd");
    submit(&mut ed, "messages");
    ed.handle_key(key(':'));
    ed.handle_key(key_up()); // empty prefix: recall newest: "messages"
    assert_eq!(ed.state.minibuf().unwrap().input, "messages");
    // Type a char: demotes history navigation back to scratch.
    ed.handle_key(key('x'));
    assert_eq!(ed.state.minibuf().unwrap().input, "messagesx");
    // Up should now re-stash "messagesx" and jump to the only entry that
    // starts with it: "messagesxtra".
    ed.handle_key(key_up());
    assert_eq!(ed.state.minibuf().unwrap().input, "messagesxtra");
    // Down should restore the stashed "messagesx".
    ed.handle_key(key_down());
    assert_eq!(ed.state.minibuf().unwrap().input, "messagesx");
    ed.handle_key(key_esc());
}

#[test]
fn history_survives_minibuf_close_and_reopen() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "messages");
    // Open, press Esc; history entry should survive the close.
    ed.handle_key(key(':'));
    ed.handle_key(key_esc());
    // Re-open and recall.
    assert_eq!(open_and_up(&mut ed), "messages");
}

#[test]
fn history_up_clears_completion_popup() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "messages");
    // Open and trigger completion.
    ed.handle_key(key(':'));
    ed.handle_key(key('q')); // partial input
    ed.handle_key(key_tab()); // may open the popup
    // Completion may or may not be Some depending on candidates, but pressing
    // Up must clear it regardless.
    ed.handle_key(key_up());
    assert!(ed.state.input.minibuf_completion().is_none());
    ed.handle_key(key_esc());
}

#[test]
fn cursor_is_at_end_after_recall() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "messages");
    ed.handle_key(key(':'));
    ed.handle_key(key_up());
    let mb = ed.state.minibuf().unwrap();
    assert_eq!(mb.cursor, mb.input.len());
    ed.handle_key(key_esc());
}

// ── Bug fixes: parser and empty Enter ────────────────────────────────────────

/// `:ls` and `:list-buffers` (hyphen in name) still dispatch correctly after
/// the parser rewrite (regression guard).
#[test]
fn colon_list_buffers_aliases_work() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "ls");
    assert!(
        ed.state.input.drawer().is_some(),
        ":ls must open the drawer"
    );

    ed.handle_key(key_esc());
    assert!(ed.state.input.drawer().is_none());
    submit(&mut ed, "list-buffers");
    assert!(
        ed.state.input.drawer().is_some(),
        ":list-buffers must open the drawer"
    );
}

/// Pressing `:` then Enter must dismiss the minibuf silently, with no warning.
#[test]
fn colon_enter_empty_silently_dismisses() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key_enter());
    assert_eq!(ed.state.mode(), Mode::Normal, "must return to Normal mode");
    assert!(ed.state.minibuf().is_none(), "minibuf must be closed");
    assert!(
        ed.state.status_msg.is_none(),
        "must not show 'unknown command', got: {:?}",
        ed.state.status_msg
    );
}

// ── Case-insensitive command names ────────────────────────────────────────────

#[test]
fn colon_capital_q_quits() {
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":Q".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(ed.state.should_quit);
}

#[test]
fn colon_quit_mixed_case_quits() {
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":Quit".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(ed.state.should_quit);
}

#[test]
fn colon_capital_w_writes_file() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");

    for ch in ":W".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .unwrap_or("")
            .starts_with("Written")
    );
    assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "hello\n");
}

#[test]
fn colon_capital_wq_writes_and_quits() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");

    for ch in ":WQ".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert!(ed.state.should_quit);
    assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "hello\n");
}

#[test]
fn colon_capital_qa_quits_single_clean_buffer() {
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":QA".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(ed.state.should_quit);
}

/// Command names ignore case, so a typed command and an editor command
/// whose names differ only in case collide, whichever is defined first.
#[test]
fn typed_and_editor_command_names_differing_in_case_collide() {
    let cases = [
        r#"(define-command! "probe" "" (lambda (pane) #t))
           (define-typed-command! "PROBE" "" (lambda () #t))"#,
        r#"(define-typed-command! "Probe" "" (lambda () #t))
           (define-command! "probe" "" (lambda (pane) #t))"#,
    ];
    for source in cases {
        let mut ed = editor_from("-[a]>bc\n");
        let err = eval_err(&mut ed, source);
        assert!(
            err.contains("is already defined"),
            "expected a collision for {source}, got: {err}"
        );
    }
}

/// `call!` resolves an editor command's name in any case.
#[test]
fn call_runs_an_editor_command_named_in_another_case() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda () (call! "INSERT-BEFORE" (focused-pane))))"#,
    );

    type_cmd(&mut ed, ":go");

    assert_eq!(
        ed.state.mode(),
        Mode::Insert,
        "`INSERT-BEFORE` must run `insert-before`"
    );
}

/// An editor command named in another case does not dispatch from `:`
/// either, and the kind hint names it in its own spelling.
#[test]
fn colon_editor_command_name_in_another_case_does_not_dispatch() {
    let mut ed = editor_from("-[h]>ello\n");
    let before = state(&ed);
    type_cmd(&mut ed, ":SELECT-NEXT-WORD");
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some(
            "'select-next-word' is an editor command; bind it to a key, or run it with call!, not `:`"
        )
    );
    assert_eq!(state(&ed), before, "selection must be untouched");
}

/// A native typed command's alias is as taken as its name.
#[test]
fn a_native_alias_cannot_be_defined() {
    let mut ed = editor_from("-[a]>bc\n");
    let err = eval_err(&mut ed, r#"(define-typed-command! "w" "" (lambda () #t))"#);
    assert!(
        err.contains("'w' conflicts with a built-in command and cannot be redefined"),
        "got: {err}"
    );
}
