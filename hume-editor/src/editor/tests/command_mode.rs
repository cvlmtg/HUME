use super::*;
use pretty_assertions::assert_eq;

/// Open a second real (file-backed) buffer with content `"world\n"` and
/// return its id — used by the `:q`/`:qa`/`:wq` multi-buffer tests below,
/// which need a second file buffer distinct from `editor_with_file`'s.
fn open_second_file_buffer(ed: &mut Editor) -> BufferId {
    let (buf, _tmp_path) = file_buffer("world\n");
    ed.open_buffer(buf)
}

// ── Command mode ──────────────────────────────────────────────────────────────

#[test]
fn colon_enters_command_mode() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    assert_eq!(ed.state.mode, Mode::Command);
    assert!(ed.state.minibuf.is_some());
    assert_eq!(ed.state.minibuf.as_ref().unwrap().prompt, ":");
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "");
}

#[test]
fn esc_cancels_command_mode() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('q'));
    ed.handle_key(key_esc());
    assert_eq!(ed.state.mode, Mode::Normal);
    assert!(ed.state.minibuf.is_none());
    assert!(!ed.state.should_quit);
}

#[test]
fn backspace_on_empty_input_cancels() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key_backspace());
    assert_eq!(ed.state.mode, Mode::Normal);
    assert!(ed.state.minibuf.is_none());
}

#[test]
fn backspace_clearing_last_char_keeps_minibuf_open() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('l'));
    ed.handle_key(key_backspace());
    // First Backspace clears the single char but leaves the minibuffer open.
    assert_eq!(ed.state.mode, Mode::Command);
    assert_eq!(
        ed.state.minibuf.as_ref().expect("minibuf still open").input,
        ""
    );
    // Second Backspace (cursor already at 0) dismisses.
    ed.handle_key(key_backspace());
    assert_eq!(ed.state.mode, Mode::Normal);
    assert!(ed.state.minibuf.is_none());
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
    assert_eq!(ed.state.minibuf.as_ref().unwrap().cursor, 0);
    // Backspace at start of non-empty input must be a no-op.
    ed.handle_key(key_backspace());
    assert_eq!(ed.state.mode, Mode::Command, "minibuf must stay open");
    let mb = ed
        .state
        .minibuf
        .as_ref()
        .expect("minibuf must still be present");
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
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "w");
}

#[test]
fn colon_q_enter_quits() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key('q'));
    ed.handle_key(key_enter());
    assert!(ed.state.should_quit);
    assert_eq!(ed.state.mode, Mode::Normal);
    assert!(ed.state.minibuf.is_none());
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
    // No file_path set — write should fail with an error message.
    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_enter());
    assert!(!ed.state.should_quit);
    assert_eq!(ed.state.mode, Mode::Normal);
    assert_eq!(ed.state.status_msg.as_deref(), Some("no file name"));
}

#[test]
fn colon_w_writes_file() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");

    ed.handle_key(key(':'));
    ed.handle_key(key('w'));
    ed.handle_key(key_enter());

    assert_eq!(ed.state.mode, Mode::Normal);
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
        Some("Unknown command: nonsense")
    );
    assert!(!ed.state.should_quit);
}

/// `:` resolves only typed commands — a real editor command's name (not a
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
            "'select-next-word' is an editor command — bind it to a key, or run it with call!, not `:`"
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
    assert!(!ed.state.should_quit);
    assert_eq!(
        ed.state.status_msg.as_deref(),
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
    assert!(ed.state.should_quit);
}

#[test]
fn colon_q_on_clean_buffer_quits() {
    let mut ed = editor_from("-[h]>ello\n");
    // Text is fresh (not dirty) — :q should quit.
    for ch in ":q".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(ed.state.should_quit);
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

    assert!(
        !ed.state.should_quit,
        ":q must not exit when a real buffer remains"
    );
    assert_eq!(
        ed.focused_buffer_id(),
        file_buf,
        ":q must switch focus back to the file buffer"
    );
    assert_eq!(ed.state.buffers.len(), 1, "view buffer must be removed");
}

#[test]
fn colon_q_real_buffer_with_clean_scratch_quits() {
    // An empty scratch buffer is disposable — :q on the last file buffer should
    // exit rather than parking on the scratch.
    // Validity: revert the predicate to `|| !is_read_only()` and this test fails
    // (should_quit stays false).
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let file_buf = ed.focused_buffer_id();
    ed.open_buffer(crate::editor::buffer::Buffer::scratch());
    ed.switch_to_buffer_without_jump(file_buf);

    type_cmd(&mut ed, ":q");

    assert!(
        ed.state.should_quit,
        ":q must exit when the only remaining buffer is an empty scratch"
    );
}

#[test]
fn colon_q_with_dirty_scratch_remaining_stays() {
    // A scratch buffer with unsaved edits is worth preserving — :q on the file
    // buffer must switch to the dirty scratch rather than discard it.
    // Validity: drop the `|| buf.is_dirty()` clause and this test fails
    // (should_quit becomes true, silently discarding the scratch content).
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let file_buf = ed.focused_buffer_id();
    let scratch_id = ed.open_buffer(crate::editor::buffer::Buffer::scratch());

    // Dirty the scratch by switching to it and typing into it.
    ed.switch_to_buffer_without_jump(scratch_id);
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty(), "scratch must be dirty");

    ed.switch_to_buffer_without_jump(file_buf);

    type_cmd(&mut ed, ":q");

    assert!(
        !ed.state.should_quit,
        ":q must not exit when a dirty scratch remains"
    );
    assert_eq!(
        ed.focused_buffer_id(),
        scratch_id,
        ":q must switch to the dirty scratch buffer"
    );
}

#[test]
fn colon_q_one_of_two_file_buffers_switches_not_quits() {
    // :q with two file buffers open closes the current one and switches.
    let (mut ed, _tmp1) = editor_with_file("-[h]>ello\n", "hello\n");
    let first_buf = ed.focused_buffer_id();

    let second_buf = open_second_file_buffer(&mut ed);
    ed.switch_to_buffer_without_jump(second_buf);
    assert_eq!(ed.focused_buffer_id(), second_buf);

    type_cmd(&mut ed, ":q");

    assert!(
        !ed.state.should_quit,
        ":q must not exit when another file buffer remains"
    );
    assert_eq!(
        ed.focused_buffer_id(),
        first_buf,
        ":q must switch to the MRU other buffer"
    );
    assert_eq!(ed.state.buffers.len(), 1, "closed buffer must be removed");
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
        ed.state.should_quit,
        ":q must exit when the only remaining buffer is a view buffer"
    );
}

#[test]
fn colon_q_bang_on_dirty_buffer_with_other_real_buffer_closes_not_quits() {
    // :q! on a dirty file buffer must discard changes and close the buffer —
    // not quit — when another real (file-backed) buffer is open.
    // Validity: remove the `any_other_real` branch from typed_quit and this
    // test fails (should_quit becomes true).
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let dirty_buf = ed.focused_buffer_id();

    // Dirty the buffer.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty(), "buffer must be dirty before :q!");

    // Open a second real file-backed buffer.
    let other_buf = open_second_file_buffer(&mut ed);
    ed.switch_to_buffer_without_jump(dirty_buf);

    type_cmd(&mut ed, ":q!");

    assert!(
        !ed.state.should_quit,
        ":q! must not quit when another real buffer remains"
    );
    assert_eq!(
        ed.focused_buffer_id(),
        other_buf,
        ":q! must switch focus to the remaining real buffer"
    );
    assert_eq!(ed.state.buffers.len(), 1, "dirty buffer must be removed");
}

// ── :qa quit-all behavior ─────────────────────────────────────────────────────

#[test]
fn colon_qa_quits_single_clean_buffer() {
    let mut ed = editor_from("-[h]>ello\n");
    type_cmd(&mut ed, ":qa");
    assert!(ed.state.should_quit);
}

#[test]
fn colon_qa_quits_with_multiple_clean_buffers() {
    // :qa must exit even when :q would only close the focused buffer.
    // Validity: replace `should_quit = true` with `close_buffer` and this fails.
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let _scratch = ed.open_buffer(Buffer::scratch());
    ed.switch_to_buffer_without_jump(ed.focused_buffer_id()); // stay on file buf

    type_cmd(&mut ed, ":qa");

    assert!(
        ed.state.should_quit,
        ":qa must quit with multiple clean buffers"
    );
}

#[test]
fn colon_qa_refused_when_a_background_buffer_is_dirty() {
    // :qa must check ALL buffers, not just the focused one — and it must switch
    // focus to the first unsaved buffer so the user knows where to look.
    // Validity: swap `ed.state.buffers.iter().any(...)` for `ed.doc().is_dirty()` and
    // this test fails — the dirty background buffer would be silently ignored.
    let (mut ed, _tmp1) = editor_with_file("-[h]>ello\n", "hello\n");
    let file_buf = ed.focused_buffer_id();

    // Open a second file buffer and dirty it.
    let bg_buf = open_second_file_buffer(&mut ed);
    ed.switch_to_buffer_without_jump(bg_buf);
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty(), "background buffer must be dirty");

    // Switch focus back to the clean file buffer.
    ed.switch_to_buffer_without_jump(file_buf);
    assert!(!ed.doc().is_dirty(), "focused buffer must be clean");

    type_cmd(&mut ed, ":qa");

    assert!(
        !ed.state.should_quit,
        ":qa must be refused when any buffer is dirty"
    );
    // Focus must have jumped to the dirty buffer.
    assert_eq!(
        ed.focused_buffer_id(),
        bg_buf,
        ":qa must switch focus to the first unsaved buffer"
    );
    let msg = ed.state.status_msg.as_deref().unwrap_or("");
    assert!(
        msg.starts_with("Unsaved changes in ") && msg.ends_with(" (add ! to override)"),
        "status message must name the unsaved buffer, got: {msg:?}"
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

    assert!(ed.state.should_quit, ":qa! must quit despite dirty buffer");
}

#[test]
fn colon_qa_stays_on_focused_dirty_buffer() {
    // When the focused buffer is already dirty, :qa must stay on it — not jump
    // to another buffer — and still refuse to quit.
    // Validity: remove the `!ed.doc().is_dirty()` guard and the editor would
    // jump away from the already-unsaved focused buffer.
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let dirty_buf = ed.focused_buffer_id();

    // Dirty the focused buffer.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty());

    type_cmd(&mut ed, ":qa");

    assert!(
        !ed.state.should_quit,
        ":qa must refuse when focused buffer is dirty"
    );
    assert_eq!(
        ed.focused_buffer_id(),
        dirty_buf,
        ":qa must not move focus when the focused buffer is already dirty"
    );
}

#[test]
fn colon_qa_lands_on_first_dirty_buffer_in_open_order() {
    // With multiple dirty buffers and a clean focused buffer, :qa must land on
    // the first dirty buffer in open-order, not just any dirty buffer.
    // Validity: change find → find last and this test fails.
    let (mut ed, _tmp1) = editor_with_file("-[h]>ello\n", "hello\n");
    let clean_buf = ed.focused_buffer_id();

    // Open two more buffers and dirty them both; open-order = clean_buf, buf2, buf3.
    let mk_dirty_buf = |ed: &mut Editor| {
        let (buf, _tmp) = file_buffer("content\n");
        let id = ed.open_buffer(buf);
        ed.switch_to_buffer_without_jump(id);
        ed.handle_key(key('i'));
        ed.handle_key(key('x'));
        ed.handle_key(key_esc());
        assert!(ed.doc().is_dirty());
        id
    };

    let first_dirty = mk_dirty_buf(&mut ed);
    let _second_dirty = mk_dirty_buf(&mut ed);

    ed.switch_to_buffer_without_jump(clean_buf);
    assert!(!ed.doc().is_dirty(), "focused buffer must be clean");

    type_cmd(&mut ed, ":qa");

    assert!(!ed.state.should_quit);
    assert_eq!(
        ed.focused_buffer_id(),
        first_dirty,
        ":qa must land on the first dirty buffer in open-order"
    );
}

#[test]
fn colon_qa_walk_through_dirty_buffers() {
    // Save the first unsaved buffer and run :qa again — it should move to the
    // next dirty buffer, verifying the "first in open-order" iteration works
    // across multiple :qa invocations.
    let (mut ed, tmp1) = editor_with_file("-[h]>ello\n", "hello\n");
    let clean_buf = ed.focused_buffer_id();

    // Open two dirty file buffers.
    let mk_dirty_buf = |ed: &mut Editor| {
        let (buf, tmp_path) = file_buffer("content\n");
        let id = ed.open_buffer(buf);
        ed.switch_to_buffer_without_jump(id);
        ed.handle_key(key('i'));
        ed.handle_key(key('x'));
        ed.handle_key(key_esc());
        assert!(ed.doc().is_dirty());
        (id, tmp_path)
    };

    let (first_dirty, tmp2) = mk_dirty_buf(&mut ed);
    let (second_dirty, _tmp3) = mk_dirty_buf(&mut ed);

    // Start from the clean buffer.
    ed.switch_to_buffer_without_jump(clean_buf);

    // First :qa → lands on first_dirty.
    type_cmd(&mut ed, ":qa");
    assert!(!ed.state.should_quit);
    assert_eq!(
        ed.focused_buffer_id(),
        first_dirty,
        "first :qa must land on first dirty buffer"
    );

    // Save first_dirty, then :qa → lands on second_dirty.
    let save_path = tmp2.to_path_buf();
    let cmd = format!(":w {}", save_path.display());
    for ch in cmd.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(!ed.doc().is_dirty(), "first buffer must be clean after :w");

    type_cmd(&mut ed, ":qa");
    assert!(!ed.state.should_quit);
    assert_eq!(
        ed.focused_buffer_id(),
        second_dirty,
        "second :qa must land on second dirty buffer"
    );

    // Suppress unused-variable warnings from the tempfiles.
    let _ = tmp1;
}

#[test]
fn colon_w_path_creates_new_file() {
    let tmp_dir = safe_tempdir();
    let new_path = tmp_dir.path().join("new_file.txt");
    assert!(!new_path.exists());

    let mut ed = editor_from("-[h]>ello\n");
    let cmd = format!(":w {}", new_path.display());
    for ch in cmd.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert!(
        ed.state
            .status_msg
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
    let tmp_dir = safe_tempdir();
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
        ed.state
            .status_msg
            .as_deref()
            .unwrap_or("")
            .starts_with("Written")
    );
    assert!(!ed.doc().is_dirty());
}

#[test]
fn colon_w_path_on_read_only_buffer_exports_without_mutating_source() {
    // :w <path> on a read-only buffer is an export: the content lands on
    // disk at the new path, but the source buffer must not be touched —
    // no mark_saved, no path/file_meta repoint, dirty state unchanged.
    // Fail oracle: drop the `is_save_as` guard in write_file's save-as
    // branch (route every :w <path> through mark_written_and_synced
    // unconditionally) — is_dirty() below becomes false and original_path
    // gets overwritten with new_path.
    let (mut ed, original_path) = editor_with_file("-[h]>ello\n", "hello\n");
    let original_path_buf = original_path.to_path_buf();
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty(), "pre-condition: buffer must be dirty");
    ed.doc_mut().read_only = true;

    let tmp_dir = safe_tempdir();
    let new_path = tmp_dir.path().join("exported.txt");
    let cmd = format!(":w {}", new_path.display());
    for ch in cmd.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert!(new_path.exists(), "export target must be written");
    assert_eq!(std::fs::read_to_string(&new_path).unwrap(), "xhello\n");
    assert!(
        ed.doc().is_dirty(),
        "export must not mark the read-only source buffer saved"
    );
    assert_eq!(
        ed.doc().path(),
        Some(original_path_buf.as_path()),
        "export must not repoint the source buffer's path"
    );
}

#[test]
fn colon_wq_path_saves_to_new_file_and_quits() {
    let tmp_dir = safe_tempdir();
    let new_path = tmp_dir.path().join("wq_test.txt");
    assert!(!new_path.exists());

    let mut ed = editor_from("-[h]>ello\n");
    let cmd = format!(":wq {}", new_path.display());
    for ch in cmd.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert!(ed.state.should_quit);
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
    assert_eq!(ed.state.status_msg.as_deref(), Some("Written 1 lines"));
    assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "hello\n");
    assert!(!ed.state.should_quit);
}

#[test]
fn colon_wq_bang_quits_even_if_write_fails() {
    // Scratch buffer (no file_path) — write will fail, but :wq! should still quit.
    let mut ed = editor_from("-[h]>ello\n");
    for ch in ":wq!".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(ed.state.should_quit);
}

#[test]
fn colon_wq_single_pane_other_buffer_closes_buffer_and_stays() {
    // :wq delegates to :q after a successful write, so with another real
    // (file-backed) buffer open it must write, close the current buffer, and
    // switch focus — not quit the editor.
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let dirty_buf = ed.focused_buffer_id();

    // Dirty the buffer.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty(), "buffer must be dirty before :wq");
    let expected_content = ed.doc().text().to_string();

    // Open a second real file-backed buffer.
    let other_buf = open_second_file_buffer(&mut ed);
    ed.switch_to_buffer_without_jump(dirty_buf);

    type_cmd(&mut ed, ":wq");

    assert!(
        !ed.state.should_quit,
        ":wq must not quit when another real buffer remains"
    );
    assert_eq!(
        ed.focused_buffer_id(),
        other_buf,
        ":wq must switch focus to the remaining real buffer"
    );
    assert_eq!(ed.state.buffers.len(), 1, "written buffer must be removed");
    assert_eq!(
        std::fs::read_to_string(&tmp).unwrap(),
        expected_content,
        "the edit must have been written to disk before the buffer closed"
    );
}

// ── Command history ───────────────────────────────────────────────────────────

/// Helper: submit a typed command through the minibuffer.
pub(super) fn submit(ed: &mut Editor, cmd: &str) {
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
    ed.state
        .minibuf
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
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "q");
    ed.handle_key(key_up());
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "messages");
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
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "q");
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
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "foo");
    assert_eq!(ed.state.minibuf.as_ref().unwrap().cursor, 3);
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
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "foo");
    ed.handle_key(key_esc());
}

#[test]
fn empty_history_up_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key_up()); // empty history — input unchanged
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "");
    ed.handle_key(key_esc());
}

#[test]
fn at_oldest_up_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "messages");
    ed.handle_key(key(':'));
    ed.handle_key(key_up()); // lands on "messages"
    ed.handle_key(key_up()); // already at oldest — no change
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "messages");
    ed.handle_key(key_esc());
}

#[test]
fn consecutive_duplicate_not_recorded() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "messages");
    submit(&mut ed, "messages"); // duplicate — should be skipped
    ed.handle_key(key(':'));
    ed.handle_key(key_up()); // should land on "messages"
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "messages");
    ed.handle_key(key_up()); // at oldest — no older entry
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "messages");
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
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "");
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
    ed.handle_key(key_up()); // empty prefix — recall newest: "messages"
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "messages");
    // Type a char — demotes history navigation back to scratch.
    ed.handle_key(key('x'));
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "messagesx");
    // Up should now re-stash "messagesx" and jump to the only entry that
    // starts with it: "messagesxtra".
    ed.handle_key(key_up());
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "messagesxtra");
    // Down should restore the stashed "messagesx".
    ed.handle_key(key_down());
    assert_eq!(ed.state.minibuf.as_ref().unwrap().input, "messagesx");
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
    assert!(ed.state.minibuf_completion.is_none());
    ed.handle_key(key_esc());
}

#[test]
fn cursor_is_at_end_after_recall() {
    let mut ed = editor_from("-[h]>ello\n");
    submit(&mut ed, "messages");
    ed.handle_key(key(':'));
    ed.handle_key(key_up());
    let mb = ed.state.minibuf.as_ref().unwrap();
    assert_eq!(mb.cursor, mb.input.len());
    ed.handle_key(key_esc());
}

// ── Bug fixes: parser and empty Enter ────────────────────────────────────────

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

/// Pressing `:` then Enter must dismiss the minibuf silently — no warning.
#[test]
fn colon_enter_empty_silently_dismisses() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key(':'));
    ed.handle_key(key_enter());
    assert_eq!(ed.state.mode, Mode::Normal, "must return to Normal mode");
    assert!(ed.state.minibuf.is_none(), "minibuf must be closed");
    assert!(
        ed.state.status_msg.is_none(),
        "must not show 'Unknown command', got: {:?}",
        ed.state.status_msg
    );
}

// ── Case-insensitive typed commands ──────────────────────────────────────────

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

    assert_eq!(ed.state.mode, Mode::Normal);
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

// ── :goto / bare :N line-jump ─────────────────────────────────────────────────

/// `:goto N` places the cursor at the start of line N (1-based).
/// Validity: assert line index 2 — if the handler is a no-op the head stays on
/// line 0, which fails the assertion.
#[test]
fn colon_goto_moves_to_line() {
    let mut ed = jump_editor(0);
    type_cmd(&mut ed, ":goto 3");
    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        2, // 0-indexed: line 3 → index 2
        ":goto 3 must land on line 3 (0-indexed: 2)"
    );
}

/// Bare `:3` is accepted as shorthand for `:goto 3`.
/// Validity: same as above — no-op leaves head on line 0, failing index-2 check.
#[test]
fn colon_bare_number_moves_to_line() {
    let mut ed = jump_editor(0);
    type_cmd(&mut ed, ":3");
    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        2,
        ":3 must land on line 3 (0-indexed: 2)"
    );
}

/// Past-EOF numbers clamp to the last content line.
/// 20-line buffer → last content index is 19.
/// Validity: remove the `.min(last)` clamp and line_to_char panics or returns
/// the ghost-line position, which fails the index-19 check.
#[test]
fn colon_goto_clamps_past_eof() {
    let mut ed = jump_editor(0);
    type_cmd(&mut ed, ":goto 9999");
    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        19,
        ":goto 9999 must clamp to last content line (index 19)"
    );
}

/// Bare large number also clamps.
#[test]
fn colon_bare_number_clamps_past_eof() {
    let mut ed = jump_editor(0);
    type_cmd(&mut ed, ":9999");
    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        19,
        ":9999 must clamp to last content line (index 19)"
    );
}

/// `:goto 0` is an error — line numbers start at 1.
/// Validity: remove `checked_sub(1)` and 0 maps to index -1 (wraps to usize::MAX),
/// which would clamp to last line — a bogus move rather than an error.
#[test]
fn colon_goto_zero_is_error() {
    let mut ed = jump_editor(5);
    let before = state(&ed);
    type_cmd(&mut ed, ":goto 0");
    assert_eq!(state(&ed), before, ":goto 0 must not move the cursor");
    assert!(
        ed.state.status_msg.is_some(),
        ":goto 0 must report an error"
    );
}

/// Non-numeric argument reports an error and leaves the cursor unmoved.
/// Validity: replace the parse error with Ok(1) and state() would change.
#[test]
fn colon_goto_invalid_arg_is_error() {
    let mut ed = jump_editor(5);
    let before = state(&ed);
    type_cmd(&mut ed, ":goto abc");
    assert_eq!(state(&ed), before, ":goto abc must not move the cursor");
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .unwrap_or("")
            .contains("invalid line number"),
        ":goto abc must report 'invalid line number', got: {:?}",
        ed.state.status_msg
    );
}

/// `:goto` with no argument reports an error.
#[test]
fn colon_goto_no_arg_is_error() {
    let mut ed = jump_editor(5);
    let before = state(&ed);
    type_cmd(&mut ed, ":goto");
    assert_eq!(
        state(&ed),
        before,
        ":goto with no arg must not move the cursor"
    );
    assert!(
        ed.state.status_msg.is_some(),
        ":goto with no arg must report an error"
    );
}

/// `:goto` records the pre-jump position so `Ctrl+O` returns to it.
/// Validity: remove the jump-push block and Ctrl+O leaves the cursor at line 14
/// instead of restoring to before.
#[test]
fn colon_goto_records_jump() {
    let mut ed = jump_editor(5);
    let before = state(&ed);

    type_cmd(&mut ed, ":goto 15");
    assert_eq!(
        ed.doc()
            .text()
            .char_to_line(ed.current_selections().primary().head()),
        14, // 0-indexed: line 15 → index 14
        ":goto 15 must land on line 15 (index 14)"
    );

    // Ctrl+O must restore the pre-jump position.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(
        state(&ed),
        before,
        "Ctrl+O must restore the pre-goto position"
    );
}

/// `:goto` to the line the cursor is already on is a no-op and must not
/// truncate forward jump-list history.
#[test]
fn colon_goto_noop_does_not_clobber_forward_history() {
    let mut ed = jump_editor(10);

    // `gg` — records a jump, puts us at line 0.
    ed.handle_key(key('g'));
    ed.handle_key(key('g'));
    let at_top = state(&ed);

    // Jump backward to the original line-10 position.
    ed.handle_key(key_ctrl('o'));
    let back_at_start = state(&ed);
    assert_ne!(back_at_start, at_top);

    // `:goto 11` — already on line 11 (index 10) — a no-op.
    type_cmd(&mut ed, ":goto 11");
    assert_eq!(
        state(&ed),
        back_at_start,
        ":goto to the current line must not move"
    );

    // Forward history (the jump to line 0) must still be there.
    ed.handle_key(key_ctrl('i'));
    assert_eq!(
        state(&ed),
        at_top,
        "a no-op :goto must not have truncated forward jump-list history"
    );
}

// ── :sort ─────────────────────────────────────────────────────────────────────

#[test]
fn sort_end_to_end_through_the_minibuffer() {
    let mut ed = editor_from("C -[B]> D\nF -[A]> G\n");
    submit(&mut ed, "sort");
    assert_eq!(state(&ed), "F -[A]> G\nC -[B]> D\n");
}

#[test]
fn sort_reverse_flag_through_the_minibuffer() {
    let mut ed = editor_from("-[a]>\n-[b]>\n");
    submit(&mut ed, "sort -r");
    assert_eq!(state(&ed), "-[b]>\n-[a]>\n");
}

#[test]
fn sort_insensitive_flag_through_the_minibuffer() {
    let mut ed = editor_from("-[Banana]>\n-[apple]>\n");
    submit(&mut ed, "sort -i");
    assert_eq!(state(&ed), "-[apple]>\n-[Banana]>\n");
}

#[test]
fn sort_bundled_flags_through_the_minibuffer() {
    // `-ri` = reverse + insensitive, bundled into one token.
    let mut ed = editor_from("-[Banana]>\n-[apple]>\n");
    submit(&mut ed, "sort -ri");
    assert_eq!(state(&ed), "-[Banana]>\n-[apple]>\n");
}

#[test]
fn sort_long_flags_through_the_minibuffer() {
    // `-r`/`-i` and their `--reverse`/`--insensitive` long forms are parsed
    // by the same match arm — exercise the long forms directly rather than
    // trusting that pairing to hold.
    let mut ed = editor_from("-[Banana]>\n-[apple]>\n");
    submit(&mut ed, "sort --reverse --insensitive");
    assert_eq!(state(&ed), "-[Banana]>\n-[apple]>\n");
}

#[test]
fn sort_no_adjacent_rows_reports_info_without_logging() {
    let mut ed = editor_from("-[h]>ello\n");
    let before = state(&ed);
    ed.execute_typed("sort", None)
        .expect("a single row is refused, not an error");
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("sort needs at least two adjacent rows")
    );
    assert_eq!(
        ed.state.message_log.totals(),
        (0, 0),
        "NoAdjacentRows is Severity::Info — a boundary condition, not a failure worth \
         keeping in message_log"
    );
    assert_eq!(state(&ed), before, "a refusal must not touch the buffer");
}

#[test]
fn sort_already_sorted_reports_info_without_logging() {
    let mut ed = editor_from("-[a]>\n-[b]>\n");
    ed.execute_typed("sort", None)
        .expect("already-sorted input is refused, not an error");
    assert_eq!(ed.state.status_msg.as_deref(), Some("already sorted"));
    assert_eq!(
        ed.state.message_log.totals(),
        (0, 0),
        "AlreadySorted is Severity::Info — it must never reach message_log"
    );
}

#[test]
fn sort_unknown_flag_reports_an_error() {
    let mut ed = editor_from("-[b]>\n-[a]>\n");
    let before = state(&ed);
    let err = ed
        .execute_typed("sort -z", None)
        .expect_err("-z is not a recognized flag");
    assert_eq!(err.message(), "unknown flag: -z");
    assert_eq!(
        state(&ed),
        before,
        "an argument error must not touch the buffer"
    );
}

#[test]
fn sort_unknown_long_flag_reports_a_flag_error_not_an_argument_error() {
    // Validity: an unrecognized `--`-prefixed token is still a flag mistake,
    // not a positional argument — it must not fall through to the generic
    // "unknown argument" phrasing that genuine positionals get.
    let mut ed = editor_from("-[b]>\n-[a]>\n");
    let before = state(&ed);
    let err = ed
        .execute_typed("sort --bogus", None)
        .expect_err("--bogus is not a recognized flag");
    assert_eq!(err.message(), "unknown flag: --bogus");
    assert_eq!(
        state(&ed),
        before,
        "an argument error must not touch the buffer"
    );
}

#[test]
fn sort_bang_is_rejected() {
    // Validity: `!` means "force" everywhere else in HUME; `:sort` has no
    // force behavior, so it points the user at `-r` instead of silently
    // ignoring the `!`.
    let mut ed = editor_from("-[b]>\n-[a]>\n");
    let before = state(&ed);
    let err = ed
        .execute_typed("sort!", None)
        .expect_err(":sort! must be rejected");
    assert_eq!(err.message(), "`:sort` takes no `!` — use `-r` to reverse");
    assert_eq!(state(&ed), before);
}

#[test]
fn sort_on_read_only_buffer_is_blocked() {
    // Validity: delete the `focused_buffer_read_only` check in `typed_sort`
    // and this starts silently sorting `:messages` instead of erroring.
    let mut ed = editor_from("-[h]>ello\n");
    ed.report(Severity::Warning, "one".to_string());
    ed.report(Severity::Warning, "two".to_string());
    ed.execute_typed("messages", None).unwrap();
    let before = state(&ed);

    let err = ed
        .execute_typed("sort", None)
        .expect_err("a read-only buffer must refuse :sort");
    assert_eq!(err.message(), "Buffer is read-only");
    assert_eq!(
        state(&ed),
        before,
        ":sort must not mutate a read-only buffer"
    );
}

#[test]
fn sort_undo_restores_text_and_selections_in_one_step() {
    let mut ed = editor_from("C -[B]> D\nF -[A]> G\n");
    let before = state(&ed);
    submit(&mut ed, "sort");
    assert_eq!(state(&ed), "F -[A]> G\nC -[B]> D\n");

    ed.handle_key(key('u'));
    assert_eq!(state(&ed), before, "undo must restore text and selections");
    assert!(
        !ed.doc().can_undo(),
        ":sort must record exactly one undo step"
    );
}

#[test]
fn sort_on_already_sorted_input_leaves_buffer_clean() {
    // Validity: this is the test that pins `sort_rows` returning a
    // `SortRefusal` instead of a successful identity edit — replace step 4's
    // refusal with an always-successful edit and `is_dirty()` starts
    // reporting `true` here.
    let mut ed = editor_from("-[a]>\n-[b]>\n");
    assert!(!ed.doc().is_dirty());
    submit(&mut ed, "sort");
    assert!(
        !ed.doc().is_dirty(),
        "sorting already-sorted rows must not touch the undo history"
    );
}
