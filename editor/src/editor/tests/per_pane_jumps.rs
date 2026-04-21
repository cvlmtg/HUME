use super::*;
use pretty_assertions::assert_eq;

// ── Phase 7: per-pane pane_jumps ─────────────────────────────────────────────

/// Ctrl+O navigates backward in the per-pane jump list (not a global list).
#[test]
fn p7_pane_jumps_ctrl_o_backward() {
    let mut ed = jump_editor(10);
    let before = state(&ed);

    ed.handle_key(key('g'));
    ed.handle_key(key('g'));
    assert_eq!(ed.doc().text().char_to_line(ed.current_selections().primary().head), 0);

    ed.handle_key(key_ctrl('o'));
    assert_eq!(state(&ed), before, "Ctrl+O returns to pre-jump position");
}

/// Ctrl+I navigates forward in the per-pane jump list.
#[test]
fn p7_pane_jumps_ctrl_i_forward() {
    let mut ed = jump_editor(10);

    ed.handle_key(key('g'));
    ed.handle_key(key('g'));
    let at_top = state(&ed);

    ed.handle_key(key_ctrl('o'));
    assert_ne!(state(&ed), at_top);

    ed.handle_key(key_ctrl('i'));
    assert_eq!(state(&ed), at_top, "Ctrl+I returns to top position");
}

/// Ctrl+O across buffers: `:e file2`, large motion in file2, Ctrl+O lands back in file1.
#[test]
fn p7_cross_buffer_ctrl_o() {
    let dir = tempfile::tempdir().unwrap();
    let file1 = dir.path().join("file1.txt");
    let file2 = dir.path().join("file2.txt");
    // 20 lines in each file so large motions are valid.
    let content: String = (0..20).map(|i| format!("line {i}\n")).collect();
    std::fs::write(&file1, &content).unwrap();
    std::fs::write(&file2, &content).unwrap();

    let mut ed = Editor::for_testing(Buffer::new(Text::from("scratch\n"), SelectionSet::default()));
    ed.execute_typed("e", Some(file1.to_str().unwrap())).unwrap();
    let buf1 = ed.focused_buffer_id();
    let line0_state_f1 = state(&ed);

    // Open file2 — switch_to_buffer_with_jump records {file1, line 0} before switching.
    ed.execute_typed("e", Some(file2.to_str().unwrap())).unwrap();
    let buf2 = ed.focused_buffer_id();
    assert_ne!(buf1, buf2, "different buffers");
    // Now in file2, cursor at line 0. Jump list: [{scratch}, {file1}], cursor = 2.

    // Ctrl+O: saves current (file2, line 0) then goes to entries[1] = {file1, line 0}.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(ed.focused_buffer_id(), buf1, "Ctrl+O crossed back to file1");
    assert_eq!(state(&ed), line0_state_f1, "cursor restored in file1");
}

/// Closing a buffer prunes its entries from pane_jumps.
#[test]
fn p7_close_buffer_prunes_pane_jumps() {
    let dir = tempfile::tempdir().unwrap();
    let file1 = dir.path().join("prune1.txt");
    let file2 = dir.path().join("prune2.txt");
    let content: String = (0..20).map(|i| format!("row {i}\n")).collect();
    std::fs::write(&file1, &content).unwrap();
    std::fs::write(&file2, &content).unwrap();

    let mut ed = Editor::for_testing(Buffer::new(Text::from("scratch\n"), SelectionSet::default()));
    ed.execute_typed("e", Some(file1.to_str().unwrap())).unwrap();
    let buf1 = ed.focused_buffer_id();

    // Open file2, recording a jump from file1→file2.
    ed.execute_typed("e", Some(file2.to_str().unwrap())).unwrap();
    let buf2 = ed.focused_buffer_id();
    assert_ne!(buf1, buf2);

    // Close file1 — its jump entries should be pruned from pane_jumps.
    let pid = ed.focused_pane_id;
    ed.close_buffer(buf1);
    // The jump list for this pane must not contain any file1 entries.
    let has_buf1_entry = ed.pane_jumps[pid].entries_for_buffer(buf1);
    assert!(!has_buf1_entry, "pane_jumps should not contain closed buffer entries");
}

