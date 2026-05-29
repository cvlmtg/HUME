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

// ── :q multi-buffer behavior ──────────────────────────────────────────────────

#[test]
fn colon_q_view_buffer_with_real_buffer_switches_not_quits() {
    // :q on a view buffer when a real (file) buffer is also open should
    // close the view buffer and switch to the file buffer — not exit hume.
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let file_buf = ed.focused_buffer_id();
    // Open a read-only view buffer (simulates :messages).
    ed.open_read_only_view("[test-view]", "log line\n", 0);
    let view_buf = ed.focused_buffer_id();
    assert_ne!(file_buf, view_buf, "must have switched to view buffer");

    type_cmd(&mut ed, ":q");

    assert!(!ed.should_quit, ":q must not exit when a real buffer remains");
    assert_eq!(
        ed.focused_buffer_id(),
        file_buf,
        ":q must switch focus back to the file buffer"
    );
    assert_eq!(ed.buffers.len(), 1, "view buffer must be removed");
}

#[test]
fn colon_q_real_buffer_with_editable_scratch_switches_not_quits() {
    // An editable scratch buffer (no path, not read-only) is a "real" buffer —
    // :q on a file buffer should switch to it rather than exit.
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let file_buf = ed.focused_buffer_id();
    let scratch_id = ed.open_buffer(crate::editor::buffer::Buffer::scratch());
    ed.switch_to_buffer_without_jump(file_buf);

    type_cmd(&mut ed, ":q");

    assert!(!ed.should_quit, ":q must not exit when an editable scratch buffer remains");
    assert_eq!(ed.focused_buffer_id(), scratch_id, "must switch to the scratch buffer");
}

#[test]
fn colon_q_one_of_two_file_buffers_switches_not_quits() {
    // :q with two file buffers open closes the current one and switches.
    let (mut ed, _tmp1) = editor_with_file("-[h]>ello\n", "hello\n");
    let first_buf = ed.focused_buffer_id();

    let tmp2 = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp2.path(), "world\n").unwrap();
    let (_, meta2) = platform::io::read_file(tmp2.path()).unwrap();
    let mut buf2 = crate::editor::buffer::Buffer::new(
        editing::text::Text::from("world\n"),
        SelectionSet::default(),
    );
    buf2.set_path(Some(tmp2.path().to_path_buf()));
    buf2.file_meta = Some(meta2);
    let second_buf = ed.open_buffer(buf2);
    ed.switch_to_buffer_without_jump(second_buf);
    assert_eq!(ed.focused_buffer_id(), second_buf);

    type_cmd(&mut ed, ":q");

    assert!(!ed.should_quit, ":q must not exit when another file buffer remains");
    assert_eq!(
        ed.focused_buffer_id(),
        first_buf,
        ":q must switch to the MRU other buffer"
    );
    assert_eq!(ed.buffers.len(), 1, "closed buffer must be removed");
}

#[test]
fn colon_q_real_buffer_with_only_view_buffer_remaining_quits() {
    // View buffers (labeled, no path) count as scratch — :q on the last
    // file buffer should exit hume even when a view buffer is still open.
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let file_buf = ed.focused_buffer_id();
    ed.open_read_only_view("[test-view]", "log line\n", 0);
    // Switch focus back to the file buffer.
    ed.switch_to_buffer_without_jump(file_buf);

    type_cmd(&mut ed, ":q");

    assert!(
        ed.should_quit,
        ":q must exit when the only remaining buffer is a view buffer"
    );
}

#[test]
fn colon_q_bang_on_dirty_buffer_with_other_real_buffer_closes_not_quits() {
    // :q! on a dirty file buffer must discard changes and close the buffer —
    // not quit — when another real buffer is open.
    // Validity: remove the `any_other_real` branch from typed_quit and this
    // test fails (should_quit becomes true, discarding the scratch buffer).
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let dirty_buf = ed.focused_buffer_id();

    // Dirty the buffer.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty(), "buffer must be dirty before :q!");

    // Open a second real buffer (editable scratch).
    let scratch_id = ed.open_buffer(Buffer::scratch());
    ed.switch_to_buffer_without_jump(dirty_buf);

    type_cmd(&mut ed, ":q!");

    assert!(!ed.should_quit, ":q! must not quit when another real buffer remains");
    assert_eq!(
        ed.focused_buffer_id(),
        scratch_id,
        ":q! must switch focus to the remaining real buffer"
    );
    assert_eq!(ed.buffers.len(), 1, "dirty buffer must be removed");
}

// ── :qa quit-all behavior ─────────────────────────────────────────────────────

#[test]
fn colon_qa_quits_single_clean_buffer() {
    let mut ed = editor_from("-[h]>ello\n");
    type_cmd(&mut ed, ":qa");
    assert!(ed.should_quit);
}

#[test]
fn colon_qa_quits_with_multiple_clean_buffers() {
    // :qa must exit even when :q would only close the focused buffer.
    // Validity: replace `should_quit = true` with `close_buffer` and this fails.
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let _scratch = ed.open_buffer(Buffer::scratch());
    ed.switch_to_buffer_without_jump(ed.focused_buffer_id()); // stay on file buf

    type_cmd(&mut ed, ":qa");

    assert!(ed.should_quit, ":qa must quit with multiple clean buffers");
}

#[test]
fn colon_qa_refused_when_a_background_buffer_is_dirty() {
    // :qa must check ALL buffers, not just the focused one.
    // Validity: swap `ed.buffers.iter().any(...)` for `ed.doc().is_dirty()` and
    // this test fails — the dirty background buffer would be silently ignored.
    let (mut ed, _tmp1) = editor_with_file("-[h]>ello\n", "hello\n");
    let file_buf = ed.focused_buffer_id();

    // Open a second file buffer and dirty it.
    let tmp2 = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp2.path(), "world\n").unwrap();
    let (_, meta2) = platform::io::read_file(tmp2.path()).unwrap();
    let mut buf2 = crate::editor::buffer::Buffer::new(
        editing::text::Text::from("world\n"),
        SelectionSet::default(),
    );
    buf2.set_path(Some(tmp2.path().to_path_buf()));
    buf2.file_meta = Some(meta2);
    let bg_buf = ed.open_buffer(buf2);
    ed.switch_to_buffer_without_jump(bg_buf);
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty(), "background buffer must be dirty");

    // Switch focus back to the clean file buffer.
    ed.switch_to_buffer_without_jump(file_buf);
    assert!(!ed.doc().is_dirty(), "focused buffer must be clean");

    type_cmd(&mut ed, ":qa");

    assert!(!ed.should_quit, ":qa must be refused when any buffer is dirty");
    assert_eq!(
        ed.status_msg.as_deref(),
        Some("Unsaved changes in open buffers (add ! to override)"),
    );
}

#[test]
fn colon_qa_bang_quits_despite_dirty_buffers() {
    // :qa! must discard unsaved changes and quit unconditionally.
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty());

    type_cmd(&mut ed, ":qa!");

    assert!(ed.should_quit, ":qa! must quit despite dirty buffer");
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
    assert!(
        ed.doc().is_read_only() && ed.doc().display_name() == "[buffers]",
        ":ls must open the read-only [buffers] view buffer"
    );

    submit(&mut ed, "list-buffers");
    assert!(
        ed.doc().is_read_only() && ed.doc().display_name() == "[buffers]",
        ":list-buffers must open the read-only [buffers] view buffer"
    );
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
