//! Dirty-buffer tracking, `:q`/`:qa` semantics, and writing.

use super::*;
use pretty_assertions::assert_eq;

// ── Dirty-buffer tracking and :q guard ───────────────────────────────────────

#[test]
fn fresh_editor_is_not_dirty() {
    let ed = editor_from("-[h]>ello\n");
    assert!(!focused_unsaved(&ed));
}

#[test]
fn typing_in_insert_mode_makes_dirty() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(focused_unsaved(&ed));
}

#[test]
fn colon_w_marks_buffer_clean() {
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    // Make the buffer dirty.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(focused_unsaved(&ed));
    // Write: should clear dirty flag.
    for ch in ":w".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(!focused_unsaved(&ed));
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
    let name = ed.state.buffers.get(ed.focused_buffer_id()).display_name();
    assert_eq!(status(&ed), format!("{name} has unsaved changes. Use :q!"));
}

#[test]
fn colon_e_on_a_dirty_file_buffer_refuses_and_names_it() {
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());

    type_cmd(&mut ed, ":e");

    let name = ed.state.buffers.get(ed.focused_buffer_id()).display_name();
    assert_eq!(status(&ed), format!("{name} has unsaved changes. Use :e!"));
    assert!(focused_unsaved(&ed), "a refused reload keeps the edits");
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
    // Text is fresh (not dirty), so :q should quit.
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
    // close the view buffer and switch to the file buffer, not exit hume.
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let file_buf = ed.focused_buffer_id();
    // Open a read-only view buffer (simulates :messages).
    ed.open_read_only_view(
        FocusedPane::current(&ed.view),
        &view_label("test-view"),
        "log line\n",
        Some(hume_rope::line::ContentLine::new(0)),
    );
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
    // An empty scratch buffer is disposable: :q on the last file buffer should
    // exit rather than parking on the scratch.
    // Validity: revert the predicate to `|| !is_read_only()` and this test fails
    // (should_quit stays false).
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let file_buf = ed.focused_buffer_id();
    ed.open_buffer(crate::editor::buffer::Buffer::scratch());
    ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.view), file_buf);

    type_cmd(&mut ed, ":q");

    assert!(
        ed.state.should_quit,
        ":q must exit when the only remaining buffer is an empty scratch"
    );
}

#[test]
fn colon_q_with_dirty_scratch_remaining_stays() {
    // A scratch buffer with unsaved edits is worth preserving: :q on the file
    // buffer must switch to the dirty scratch rather than discard it.
    // Validity: drop the `|| ed.state.has_unsaved_changes(id)` clause and this test fails
    // (should_quit becomes true, silently discarding the scratch content).
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let file_buf = ed.focused_buffer_id();
    let scratch_id = ed.open_buffer(crate::editor::buffer::Buffer::scratch());

    // Dirty the scratch by switching to it and typing into it.
    ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.view), scratch_id);
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(focused_unsaved(&ed), "scratch must be dirty");

    ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.view), file_buf);

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
    ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.view), second_buf);
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
    // View buffers (labeled, no path) count as scratch: :q on the last
    // file buffer should exit hume even when a view buffer is still open.
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let file_buf = ed.focused_buffer_id();
    ed.open_read_only_view(
        FocusedPane::current(&ed.view),
        &view_label("test-view"),
        "log line\n",
        Some(hume_rope::line::ContentLine::new(0)),
    );
    // Switch focus back to the file buffer.
    ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.view), file_buf);

    type_cmd(&mut ed, ":q");

    assert!(
        ed.state.should_quit,
        ":q must exit when the only remaining buffer is a view buffer"
    );
}

#[test]
fn colon_q_bang_on_dirty_buffer_with_other_real_buffer_closes_not_quits() {
    // :q! on a dirty file buffer must discard changes and close the buffer,
    // not quit, when another real (file-backed) buffer is open.
    // Validity: remove the `any_other_real` branch from typed_quit and this
    // test fails (should_quit becomes true).
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let dirty_buf = ed.focused_buffer_id();

    // Dirty the buffer.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(focused_unsaved(&ed), "buffer must be dirty before :q!");

    // Open a second real file-backed buffer.
    let other_buf = open_second_file_buffer(&mut ed);
    ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.view), dirty_buf);

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
    ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.view), ed.focused_buffer_id()); // stay on file buf

    type_cmd(&mut ed, ":qa");

    assert!(
        ed.state.should_quit,
        ":qa must quit with multiple clean buffers"
    );
}

#[test]
fn colon_qa_refused_when_a_background_buffer_is_dirty() {
    // :qa must check ALL buffers, not just the focused one, and it must switch
    // focus to the first unsaved buffer so the user knows where to look.
    // Validity: swap `ed.state.buffers.iter().any(...)` for `focused_unsaved(&ed)` and
    // this test fails: the dirty background buffer would be silently ignored.
    let (mut ed, _tmp1) = editor_with_file("-[h]>ello\n", "hello\n");
    let file_buf = ed.focused_buffer_id();

    // Open a second file buffer and dirty it.
    let bg_buf = open_second_file_buffer(&mut ed);
    ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.view), bg_buf);
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(focused_unsaved(&ed), "background buffer must be dirty");

    // Switch focus back to the clean file buffer.
    ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.view), file_buf);
    assert!(!focused_unsaved(&ed), "focused buffer must be clean");

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
    let name = ed.state.buffers.get(bg_buf).display_name();
    assert_eq!(status(&ed), format!("{name} has unsaved changes. Use :qa!"));
}

#[test]
fn colon_qa_bang_quits_despite_dirty_buffers() {
    // :qa! must discard unsaved changes and quit unconditionally.
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(focused_unsaved(&ed));

    type_cmd(&mut ed, ":qa!");

    assert!(ed.state.should_quit, ":qa! must quit despite dirty buffer");
}

#[test]
fn colon_qa_stays_on_focused_dirty_buffer() {
    // When the focused buffer is already dirty, :qa must stay on it (not jump
    // to another buffer) and still refuse to quit.
    // Validity: remove the `!focused_unsaved(&ed)` guard and the editor would
    // jump away from the already-unsaved focused buffer.
    let (mut ed, _tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let dirty_buf = ed.focused_buffer_id();

    // Dirty the focused buffer.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(focused_unsaved(&ed));

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
        ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.view), id);
        ed.handle_key(key('i'));
        ed.handle_key(key('x'));
        ed.handle_key(key_esc());
        assert!(focused_unsaved(ed));
        id
    };

    let first_dirty = mk_dirty_buf(&mut ed);
    let _second_dirty = mk_dirty_buf(&mut ed);

    ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.view), clean_buf);
    assert!(!focused_unsaved(&ed), "focused buffer must be clean");

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
    // Save the first unsaved buffer and run :qa again. It should move to the
    // next dirty buffer, verifying the "first in open-order" iteration works
    // across multiple :qa invocations.
    let (mut ed, tmp1) = editor_with_file("-[h]>ello\n", "hello\n");
    let clean_buf = ed.focused_buffer_id();

    // Open two dirty file buffers.
    let mk_dirty_buf = |ed: &mut Editor| {
        let (buf, tmp_path) = file_buffer("content\n");
        let id = ed.open_buffer(buf);
        ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.view), id);
        ed.handle_key(key('i'));
        ed.handle_key(key('x'));
        ed.handle_key(key_esc());
        assert!(focused_unsaved(ed));
        (id, tmp_path)
    };

    let (first_dirty, tmp2) = mk_dirty_buf(&mut ed);
    let (second_dirty, _tmp3) = mk_dirty_buf(&mut ed);

    // Start from the clean buffer.
    ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.view), clean_buf);

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
    assert!(!focused_unsaved(&ed), "first buffer must be clean after :w");

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
    assert!(!focused_unsaved(&ed));
}

#[test]
fn colon_w_path_updates_file_path_for_subsequent_writes() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let new_path = tmp_dir.path().join("subsequent.txt");

    let mut ed = editor_from("-[h]>ello\n");
    // First :w with path: the written file becomes the buffer's file.
    let cmd = format!(":w {}", new_path.display());
    for ch in cmd.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert!(
        ed.doc()
            .file()
            .and_then(crate::editor::buffer::BufferFile::file_meta)
            .is_some()
    );

    // Make dirty again and write without a path; should use the new path.
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
    assert!(!focused_unsaved(&ed));
}

#[test]
fn colon_w_path_on_read_only_buffer_exports_without_mutating_source() {
    // :w <path> on a read-only buffer is an export: the content lands on
    // disk at the new path, but the source buffer must not be touched:
    // no mark_saved, no file repoint, dirty state unchanged.
    // Without the `is_save_as` guard in write_file's save-as branch, every
    // :w <path> would go through mark_written_and_synced. is_dirty() below
    // would then be false and original_path would be replaced by new_path.
    let (mut ed, _original_path) = editor_with_file("-[h]>ello\n", "hello\n");
    let path_before = ed.doc().path().cloned();
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(focused_unsaved(&ed), "pre-condition: buffer must be dirty");
    ed.doc_mut().read_only = true;

    let tmp_dir = tempfile::tempdir().unwrap();
    let new_path = tmp_dir.path().join("exported.txt");
    let cmd = format!(":w {}", new_path.display());
    for ch in cmd.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());

    assert!(new_path.exists(), "export target must be written");
    assert_eq!(std::fs::read_to_string(&new_path).unwrap(), "xhello\n");
    assert!(
        focused_unsaved(&ed),
        "export must not mark the read-only source buffer saved"
    );
    assert_eq!(
        ed.doc().path().cloned(),
        path_before,
        "export must not repoint the source buffer's path"
    );
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
    // Scratch buffer (no file_path): write will fail, but :wq! should still quit.
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
    // switch focus, not quit the editor.
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let dirty_buf = ed.focused_buffer_id();

    // Dirty the buffer.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(focused_unsaved(&ed), "buffer must be dirty before :wq");
    let expected_content = ed.doc().text().to_string();

    // Open a second real file-backed buffer.
    let other_buf = open_second_file_buffer(&mut ed);
    ed.switch_to_buffer_without_jump(FocusedPane::current(&ed.view), dirty_buf);

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
