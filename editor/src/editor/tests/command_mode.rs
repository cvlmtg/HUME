use super::*;
use pretty_assertions::assert_eq;

// ── Command mode ──────────────────────────────────────────────────────────────

#[test]
fn colon_enters_command_mode() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    assert_eq!(ed.mode, Mode::Command);
    assert!(ed.minibuf.is_some());
    assert_eq!(ed.minibuf.as_ref().unwrap().prompt, ':');
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "");
}

#[test]
fn esc_cancels_command_mode() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('q'));
    ed.handle_key(key_esc());
    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.minibuf.is_none());
    assert!(!ed.should_quit);
}

#[test]
fn backspace_on_empty_input_cancels() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key_backspace());
    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.minibuf.is_none());
}

#[test]
fn backspace_clearing_last_char_keeps_minibuf_open() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('l'));
    ed.handle_key(key_backspace());
    // First Backspace clears the single char but leaves the minibuffer open.
    assert_eq!(ed.mode, Mode::Command);
    assert_eq!(ed.minibuf.as_ref().expect("minibuf still open").input, "");
    // Second Backspace (cursor already at 0) dismisses.
    ed.handle_key(key_backspace());
    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.minibuf.is_none());
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
    assert_eq!(ed.minibuf.as_ref().unwrap().cursor, 0);
    // Backspace at start of non-empty input must be a no-op.
    ed.handle_key(key_backspace());
    assert_eq!(ed.mode, Mode::Command, "minibuf must stay open");
    let mb = ed.minibuf.as_ref().expect("minibuf must still be present");
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
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "w");
}

#[test]
fn colon_q_enter_quits() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('q'));
    ed.handle_key(key_enter());
    assert!(ed.should_quit);
    assert_eq!(ed.mode, Mode::Normal);
    assert!(ed.minibuf.is_none());
}

#[test]
fn colon_quit_enter_quits() {
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":quit".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(ed.should_quit);
}

#[test]
fn colon_w_no_path_sets_error() {
    let mut ed = editor_from("-[h]>ello\n");
    // No file_path set — write should fail with an error message.
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_enter());
    assert!(!ed.should_quit);
    assert_eq!(ed.mode, Mode::Normal);
    assert_eq!(ed.status_msg.as_deref(), Some("no file name"));
}

#[test]
fn colon_w_writes_file() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");

    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_enter());

    assert_eq!(ed.mode, Mode::Normal);
    assert!(
        ed.status_msg
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

    assert!(ed.should_quit);
    assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "hello\n");
}

#[test]
fn colon_unknown_sets_error() {
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":nonsense".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert_eq!(ed.status_msg.as_deref(), Some("Unknown command: nonsense"));
    assert!(!ed.should_quit);
}

#[test]
fn status_msg_cleared_on_next_keypress() {
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":nonsense".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(ed.status_msg.is_some());
    // Any keypress clears it.
    ed.handle_key(key('l'));
    assert!(ed.status_msg.is_none());
}

// ── Dirty-buffer tracking and :q guard ───────────────────────────────────────

#[test]
fn fresh_editor_is_not_dirty() {
    let ed = editor_from("-[h]>ello\n");
    assert!(!ed.doc().is_dirty());
}

#[test]
fn typing_in_insert_mode_makes_dirty() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty());
}

#[test]
fn colon_w_marks_buffer_clean() {
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    // Make the buffer dirty.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty());
    // Write — should clear dirty flag.
    for ch in ":w".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(!ed.doc().is_dirty());
}

#[test]
fn colon_q_on_dirty_buffer_refuses() {
    let mut ed = editor_from("-[h]>ello\n");
    // Make dirty.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    // :q should refuse.
    for ch in ":q".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(!ed.should_quit);
    assert_eq!(
        ed.status_msg.as_deref(),
        Some("Unsaved changes (add ! to override)")
    );
}

#[test]
fn colon_q_bang_on_dirty_buffer_quits() {
    let mut ed = editor_from("-[h]>ello\n");
    // Make dirty.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    // :q! should quit regardless.
    for ch in ":q!".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(ed.should_quit);
}

#[test]
fn colon_q_on_clean_buffer_quits() {
    let mut ed = editor_from("-[h]>ello\n");
    // Text is fresh (not dirty) — :q should quit.
    for ch in ":q".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(ed.should_quit);
}

#[test]
fn colon_w_path_creates_new_file() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let new_path = tmp_dir.path().join("new_file.txt");
    assert!(!new_path.exists());

    let mut ed = editor_from("-[h]>ello\n");
    let cmd = format!(":w {}", new_path.display());
    for ch in cmd.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert!(
        ed.status_msg
            .as_deref()
            .unwrap_or("")
            .starts_with("Written")
    );
    assert!(new_path.exists());
    assert_eq!(std::fs::read_to_string(&new_path).unwrap(), "hello\n");
    // file_path should be updated.
    assert!(ed.doc_mut().path().is_some());
    // Text should now be clean.
    assert!(!ed.doc().is_dirty());
}

#[test]
fn colon_w_path_updates_file_path_for_subsequent_writes() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let new_path = tmp_dir.path().join("subsequent.txt");

    let mut ed = editor_from("-[h]>ello\n");
    // First :w with path — sets file_path and file_meta.
    let cmd = format!(":w {}", new_path.display());
    for ch in cmd.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(ed.doc_mut().file_meta.is_some());

    // Make dirty again and write without a path — should use the new path.
    ed.handle_key(key('i'));
    ed.handle_key(key('y'));
    ed.handle_key(key_esc());
    for ch in ":w".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(
        ed.status_msg
            .as_deref()
            .unwrap_or("")
            .starts_with("Written")
    );
    assert!(!ed.doc().is_dirty());
}

#[test]
fn colon_wq_path_saves_to_new_file_and_quits() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let new_path = tmp_dir.path().join("wq_test.txt");
    assert!(!new_path.exists());

    let mut ed = editor_from("-[h]>ello\n");
    let cmd = format!(":wq {}", new_path.display());
    for ch in cmd.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert!(ed.should_quit);
    assert!(new_path.exists());
    assert_eq!(std::fs::read_to_string(&new_path).unwrap(), "hello\n");
}

#[test]
fn colon_w_bang_writes_writable_file() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    for ch in ":w!".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert_eq!(ed.status_msg.as_deref(), Some("Written 1 lines"));
    assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "hello\n");
    assert!(!ed.should_quit);
}

#[test]
fn colon_wq_bang_quits_even_if_write_fails() {
    // Scratch buffer (no file_path) — write will fail, but :wq! should still quit.
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":wq!".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(ed.should_quit);
}

// ── Command history ───────────────────────────────────────────────────────────

/// Helper: submit a typed command through the minibuffer.
fn submit(ed: &mut Editor, cmd: &str) {
    ed.handle_key(key(':'));
    for ch in cmd.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
}

/// Helper: open the command minibuffer, press Up once, return the current input.
fn open_and_up(ed: &mut Editor) -> String {
    ed.handle_key(key(':'));
    ed.handle_key(key_up());
    ed.minibuf
        .as_ref()
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
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "q");
    ed.handle_key(key_up());
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "messages");
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
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "q");
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
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "foo");
    assert_eq!(ed.minibuf.as_ref().unwrap().cursor, 3);
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
    ed.handle_key(key_down()); // not navigating — no-op
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "foo");
    ed.handle_key(key_esc());
}

#[test]
fn empty_history_up_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key_up()); // empty history — input unchanged
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "");
    ed.handle_key(key_esc());
}

#[test]
fn at_oldest_up_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "messages");
    ed.handle_key(key(':'));
    ed.handle_key(key_up()); // lands on "messages"
    ed.handle_key(key_up()); // already at oldest — no change
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "messages");
    ed.handle_key(key_esc());
}

#[test]
fn consecutive_duplicate_not_recorded() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "messages");
    submit(&mut ed, "messages"); // duplicate — should be skipped
    ed.handle_key(key(':'));
    ed.handle_key(key_up()); // should land on "messages"
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "messages");
    ed.handle_key(key_up()); // at oldest — no older entry
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "messages");
    ed.handle_key(key_esc());
}

#[test]
fn failing_command_is_still_recorded() {
    // Unknown commands are recorded so the user can Up, fix the typo, and re-submit.
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "qit"); // typo — reports "Unknown command: qit"
    assert_eq!(open_and_up(&mut ed), "qit");
}

#[test]
fn empty_confirm_not_recorded() {
    let mut ed = editor_from("-[h]>ello\n");
    // Press Enter with empty input — ConfirmEmpty, should not add an entry.
    ed.handle_key(key(':'));
    ed.handle_key(key_enter()); // ConfirmEmpty
    ed.handle_key(key(':'));
    ed.handle_key(key_up()); // no entry to recall — input stays empty
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "");
    ed.handle_key(key_esc());
}

#[test]
fn edit_after_up_demotes_scratch() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "messages");
    ed.handle_key(key(':'));
    ed.handle_key(key_up()); // recall "messages"
    // Type a char — demotes history navigation back to scratch.
    ed.handle_key(key('x'));
    // Up should now re-stash "messagesx" and jump to newest entry.
    ed.handle_key(key_up());
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "messages");
    // Down should restore the stashed "messagesx".
    ed.handle_key(key_down());
    assert_eq!(ed.minibuf.as_ref().unwrap().input, "messagesx");
    ed.handle_key(key_esc());
}

#[test]
fn history_survives_minibuf_close_and_reopen() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "messages");
    // Open, press Esc — history entry should survive the close.
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
    ed.handle_key(key_tab()); // Tab → CompleteRequested → may open popup
    // Completion may or may not be Some depending on candidates, but pressing
    // Up must clear it regardless.
    ed.handle_key(key_up());
    assert!(ed.completion.is_none());
    ed.handle_key(key_esc());
}

#[test]
fn cursor_is_at_end_after_recall() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "messages");
    ed.handle_key(key(':'));
    ed.handle_key(key_up());
    let mb = ed.minibuf.as_ref().unwrap();
    assert_eq!(mb.cursor, mb.input.len());
    ed.handle_key(key_esc());
}

// ── Bug fixes: parser and empty Enter ────────────────────────────────────────

/// `:b#` (no space) must switch to the alternate buffer via the minibuf path.
#[test]
#[cfg(not(windows))]
fn colon_b_hash_switches_to_alternate() {
    let f1 = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f1.path(), "file1\n").unwrap();
    let c1 = std::fs::canonicalize(f1.path()).unwrap();

    let f2 = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f2.path(), "file2\n").unwrap();
    let c2 = std::fs::canonicalize(f2.path()).unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    // Open both files. After this: current=f2, alternate=f1.
    ed.execute_typed("e", Some(c1.to_str().unwrap())).unwrap();
    ed.execute_typed("e", Some(c2.to_str().unwrap())).unwrap();
    assert_eq!(ed.doc().path(), Some(c2.as_path()), "should be on f2");

    // `:b#` through the key handler (minibuf path) must switch to the
    // alternate (f1) without a space before `#`.
    submit(&mut ed, "b#");
    assert_eq!(
        ed.doc().path(),
        Some(c1.as_path()),
        ":b# must switch to alternate (f1), but got {:?}",
        ed.doc().path()
    );
}

/// `:ls` and `:list-buffers` (hyphen in name) still dispatch correctly after
/// the parser rewrite — regression guard.
#[test]
fn colon_list_buffers_aliases_work() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "ls");
    assert!(ed.scratch_view.is_some(), ":ls must open scratch view");
    ed.handle_key(key_esc()); // dismiss scratch view

    submit(&mut ed, "list-buffers");
    assert!(ed.scratch_view.is_some(), ":list-buffers must open scratch view");
}

/// `:e! /path` (force + path, no space between `!` and arg) must parse as
/// force=true with the path as argument — regression guard for the new parser.
#[test]
#[cfg(not(windows))]
fn colon_edit_bang_path_parses() {
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), "clean\n").unwrap();
    let canonical = std::fs::canonicalize(f.path()).unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    // Open the file first so it's in the buffer list.
    ed.execute_typed("e", Some(canonical.to_str().unwrap())).unwrap();
    // :e! with no space before path must still open/switch correctly.
    let cmd = format!("e!{}", canonical.display());
    submit(&mut ed, &cmd);
    assert_eq!(
        ed.doc().path(),
        Some(canonical.as_path()),
        ":e!<path> (no space) must parse as cmd=e force=true arg=<path>"
    );
}

/// Pressing `:` then Enter must dismiss the minibuf silently — no warning.
#[test]
fn colon_enter_empty_silently_dismisses() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key_enter());
    assert_eq!(ed.mode, Mode::Normal, "must return to Normal mode");
    assert!(ed.minibuf.is_none(), "minibuf must be closed");
    assert!(
        ed.status_msg.is_none(),
        "must not show 'Unknown command', got: {:?}",
        ed.status_msg
    );
}
