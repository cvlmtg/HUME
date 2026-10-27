//! Entering command mode, typed commands, and editing the `:` line.

use super::*;
use pretty_assertions::assert_eq;

// ── Command mode ──────────────────────────────────────────────────────────────

#[test]
fn colon_enters_command_mode() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    assert_eq!(ed.state.mode(), Mode::Command);
    assert!(ed.state.minibuf().is_some());
    assert_eq!(ed.state.minibuf().unwrap().prompt, ":");
    assert_eq!(ed.state.minibuf().unwrap().input, "");
}

/// Truncate-before-execute: a `:cmd` body runs with the `Command` layer
/// already gone (truncated back to `Base` before `execute_command_line` is
/// called), so a body that itself pushes `Insert` lands on a clean stack
/// and stays there. Closing the `Command` layer after execution would stomp
/// it back to Normal.
#[test]
fn typed_command_body_entering_insert_stays_in_insert() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda () (call! "insert-before" (focused-pane))))"#,
    );

    type_cmd(&mut ed, ":go");

    assert_eq!(
        ed.state.mode(),
        Mode::Insert,
        "the command body's own insert-before must stick, not get stomped \
         back to Normal by the command line's own close"
    );
    assert!(
        ed.state.minibuf().is_none(),
        "the Command layer must be gone"
    );

    ed.feed_key(key('X'));
    ed.feed_key(key_esc());
    assert_eq!(
        ed.state.mode(),
        Mode::Normal,
        "Esc must still end Insert cleanly"
    );
    assert_eq!(ed.doc().text().to_string(), "Xabc\n");
}

/// A Steel typed command that logs and then raises: its log line is
/// reported before its error, during the same dispatch.
#[test]
fn a_failing_typed_commands_log_lands_before_its_error() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "fail" "" (lambda () (log! 'warning "step 1") (error "boom")))"#,
    );

    ed.execute_typed("fail", None).expect_err("`:fail` raises");

    let texts: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .map(|e| e.text.as_str())
        .collect();
    let step = texts.iter().position(|t| t.contains("step 1"));
    let boom = texts.iter().position(|t| t.contains("boom"));
    assert!(
        matches!((step, boom), (Some(s), Some(b)) if s < b),
        "the log line must be reported before the error; log: {texts:?}"
    );
}

/// `:` resolves a Steel typed command case-insensitively, and the body it
/// runs is the one the resolved entry names.
#[test]
fn typed_steel_command_runs_under_a_differently_cased_name() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda () (call! "insert-before" (focused-pane))))"#,
    );

    type_cmd(&mut ed, ":GO");

    assert_eq!(ed.state.mode(), Mode::Insert, "`:GO` must run `go`'s body");
}

#[test]
fn typed_steel_command_name_with_digits_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "p1" "" (lambda () (call! "insert-before" (focused-pane))))"#,
    );

    type_cmd(&mut ed, ":p1");

    assert_eq!(ed.state.mode(), Mode::Insert, "`:p1` must run `p1`'s body");
}

/// `:b` followed by digits is `:b` with a buffer number, never a command
/// named `b2`.
#[test]
fn colon_b_digits_switches_to_that_buffer() {
    let mut ed = editor_from("-[a]>bc\n");
    let first = ed.focused_buffer_id();
    let second = open_second_file_buffer(&mut ed);
    type_cmd(&mut ed, ":b1");
    assert_eq!(ed.focused_buffer_id(), first);

    type_cmd(&mut ed, ":b2");

    assert_eq!(ed.focused_buffer_id(), second);
}

/// A name the `:` line cannot read back whole is refused when a typed
/// command is defined: `b` + digits reads as `:b` with a buffer number, a
/// leading digit as a line number, and a space or `!` ends the name early.
#[test]
fn define_typed_command_name_unreachable_from_colon_errors() {
    for name in ["b1", "B12", "9x", "two words", "p1!", ""] {
        let mut ed = editor_from("-[a]>bc\n");
        let err = eval_err(
            &mut ed,
            &format!(r#"(define-typed-command! "{name}" "doc" (lambda () #t))"#),
        );
        assert!(
            err.contains(&format!("'{name}'")) && err.contains("typed command name"),
            "expected a name-grammar rejection for {name:?}, got: {err}"
        );
    }
}

/// An editor command's name is lowercase.
#[test]
fn define_command_with_an_uppercase_name_errors() {
    let mut ed = editor_from("-[a]>bc\n");
    let err = eval_err(
        &mut ed,
        r#"(define-command! "Probe" "" (lambda (pane) #t))"#,
    );
    assert!(
        err.contains("'Probe'") && err.contains("lowercase"),
        "expected a lowercase-name rejection, got: {err}"
    );
}

#[test]
fn esc_cancels_command_mode() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('q'));
    ed.handle_key(key_esc());
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(ed.state.minibuf().is_none());
    assert!(!ed.state.should_quit);
}

#[test]
fn backspace_on_empty_input_cancels() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key_backspace());
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(ed.state.minibuf().is_none());
}

#[test]
fn backspace_clearing_last_char_keeps_minibuf_open() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('l'));
    ed.handle_key(key_backspace());
    // First Backspace clears the single char but leaves the minibuffer open.
    assert_eq!(ed.state.mode(), Mode::Command);
    assert_eq!(ed.state.minibuf().expect("minibuf still open").input, "");
    // Second Backspace (cursor already at 0) dismisses.
    ed.handle_key(key_backspace());
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(ed.state.minibuf().is_none());
}

#[test]
fn backspace_at_cursor_start_with_nonempty_input_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    for ch in "hello".chars() {
        ed.handle_key(key(ch));
    }
    // Move cursor to position 0; input is still "hello".
    for _ in 0..5 {
        ed.handle_key(key_left());
    }
    assert_eq!(ed.state.minibuf().unwrap().cursor, 0);
    // Backspace at start of non-empty input must be a no-op.
    ed.handle_key(key_backspace());
    assert_eq!(ed.state.mode(), Mode::Command, "minibuf must stay open");
    let mb = ed.state.minibuf().expect("minibuf must still be present");
    assert_eq!(mb.input, "hello", "input must be unchanged");
    assert_eq!(mb.cursor, 0, "cursor must remain at start");
}

#[test]
fn backspace_removes_last_char() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key('q'));
    ed.handle_key(key_backspace());
    assert_eq!(ed.state.minibuf().unwrap().input, "w");
}

#[test]
fn colon_q_enter_quits() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('q'));
    ed.handle_key(key_enter());
    assert!(ed.state.should_quit);
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(ed.state.minibuf().is_none());
}

#[test]
fn colon_quit_enter_quits() {
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":quit".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(ed.state.should_quit);
}

#[test]
fn colon_w_no_path_sets_error() {
    let mut ed = editor_from("-[h]>ello\n");
    // No file_path set: write should fail with an error message.
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_enter());
    assert!(!ed.state.should_quit);
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(ed.state.status_msg.as_deref(), Some("no file name"));
}

#[test]
fn colon_w_writes_file() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");

    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
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
fn colon_wq_writes_and_quits() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");

    for ch in ":wq".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert!(ed.state.should_quit);
    assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "hello\n");
}

#[test]
fn colon_unknown_sets_error() {
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":nonsense".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("unknown command: nonsense")
    );
    assert!(!ed.state.should_quit);
}

/// `execute_typed` runs a line the way `:` does, so an unknown command
/// comes back as the warning `:` reports.
#[test]
fn execute_typed_unknown_command_is_the_colon_warning() {
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed
        .execute_typed("nosuch", None)
        .expect_err("an unknown command fails");
    assert_eq!(err.message(), "unknown command: nosuch");
    assert_eq!(err.severity(), Severity::Warning);
}

/// A Steel typed command's refusal comes back from `execute_typed` as the
/// error `:` reports.
#[test]
fn execute_typed_returns_a_steel_typed_command_refusal() {
    let mut ed = editor_from("-[h]>ello\n");
    attach_steel(
        &mut ed,
        r#"(define-typed-command! "noarg" "" (lambda () #t))"#,
    );
    let err = ed
        .execute_typed("noarg", Some("x"))
        .expect_err("a lambda with no argument parameter refuses one");
    assert_eq!(err.message(), "`:noarg` takes no argument");
    assert_eq!(err.severity(), Severity::Error);
}

/// `:` resolves only typed commands: a real editor command's name (not a
/// nonsense string) must be rejected the same way, and must not dispatch.
/// The message names the command's actual kind rather than saying "unknown"
/// for a name the registry does recognize. See `registry/mod.rs`'s module doc.
#[test]
fn colon_editor_command_name_is_unknown_and_does_not_dispatch() {
    let mut ed = editor_from("-[h]>ello\n");
    let before = state(&ed);
    for ch in ":select-next-word".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some(
            "'select-next-word' is an editor command; bind it to a key, or run it with call!, not `:`"
        )
    );
    assert_eq!(state(&ed), before, "selection must be untouched");
}

#[test]
fn status_msg_cleared_on_next_keypress() {
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":nonsense".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(ed.state.status_msg.is_some());
    // Any keypress clears it.
    ed.handle_key(key('l'));
    assert!(ed.state.status_msg.is_none());
}

// ── Ctrl-w delete-word in the minibuffer ─────────────────────────────────────

#[test]
fn ctrl_w_deletes_word_in_minibuf() {
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":e foo bar".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_ctrl('w'));
    assert_eq!(minibuf_input(&ed), "e foo ");
}

#[test]
fn ctrl_w_skips_trailing_whitespace_first() {
    // Readline behaviour: runs of spaces are consumed before the word.
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":e foo   ".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_ctrl('w'));
    assert_eq!(minibuf_input(&ed), "e ");
}

#[test]
fn ctrl_w_at_start_is_noop_and_keeps_minibuf_open() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key_ctrl('w'));
    assert_eq!(minibuf_input(&ed), "");
    // Unlike Backspace on empty input (which cancels), Ctrl-w is a no-op.
    assert!(
        ed.state.minibuf().is_some(),
        "Ctrl-w on empty input must not close the minibuf"
    );
}

#[test]
fn ctrl_w_stops_at_slash_for_path_args() {
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":e /tmp/alpha/one.txt".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_ctrl('w'));
    assert_eq!(minibuf_input(&ed), "e /tmp/alpha/");
}

#[test]
fn ctrl_w_on_trailing_slash_deletes_dir_component() {
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":e /tmp/alpha/".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_ctrl('w'));
    assert_eq!(minibuf_input(&ed), "e /tmp/");
}

#[test]
fn ctrl_w_works_in_search_minibuf() {
    // Ctrl-w in a `/` search prompt deletes the last word without cancelling.
    let mut ed = editor_from("-[h]>ello world\n");
    ed.handle_key(key('/'));
    assert_eq!(ed.state.mode(), Mode::Search);
    for ch in "foo bar".chars() {
        ed.handle_key(key(ch));
    }
    assert_eq!(minibuf_input(&ed), "foo bar");
    ed.handle_key(key_ctrl('w'));
    assert_eq!(minibuf_input(&ed), "foo ");
    assert!(
        ed.state.minibuf().is_some(),
        "Ctrl-w must not close the search minibuf"
    );
    assert_eq!(ed.state.mode(), Mode::Search);
    ed.handle_key(key_esc());
}

#[test]
fn ctrl_w_at_start_of_search_minibuf_is_noop() {
    let mut ed = editor_from("-[h]>ello world\n");
    ed.handle_key(key('/'));
    ed.handle_key(key_ctrl('w'));
    assert!(
        ed.state.minibuf().is_some(),
        "Ctrl-w on empty search input must not close the minibuf"
    );
    assert_eq!(ed.state.mode(), Mode::Search);
    ed.handle_key(key_esc());
}
