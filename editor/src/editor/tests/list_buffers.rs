use super::*;
use pretty_assertions::assert_eq;

fn temp_file(content: &str) -> (std::path::PathBuf, tempfile::TempPath) {
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), content).unwrap();
    let path = f.path().to_path_buf();
    (path, f.into_temp_path())
}

fn ls_output(ed: &mut Editor) -> String {
    ed.execute_typed("ls", None).unwrap();
    let sv = ed.scratch_view.as_ref().expect(":ls must open a scratch view");
    sv.buf.rope().to_string()
}

// ── Single buffer ─────────────────────────────────────────────────────────────

#[test]
fn ls_single_buffer_marks_current() {
    let mut ed = editor_from("-[h]>ello\n");
    let out = ls_output(&mut ed);
    assert!(out.contains('%'), ":ls must mark the focused buffer with '%'");
    assert!(!out.contains('#'), ":ls must not show '#' when there is no alternate buffer");
    // Row count: 1 header + 1 buffer
    assert_eq!(out.lines().count(), 2, "must have header + 1 buffer row");
}

#[test]
fn ls_long_alias_works() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("list-buffers", None).unwrap();
    assert!(ed.scratch_view.is_some(), ":list-buffers must open a scratch view");
}

// ── Multiple buffers ──────────────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn ls_two_buffers_marks_current_and_alternate() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    // Now: p2 is current (%), p1 is alternate (#).
    let out = ls_output(&mut ed);
    let lines: Vec<&str> = out.lines().collect();
    // Header + 3 rows (initial scratch, p1, p2)
    assert_eq!(lines.len(), 4, "header + 3 buffers: scratch, p1, p2");
    let p2_name = p2.file_name().unwrap().to_str().unwrap();
    let p1_name = p1.file_name().unwrap().to_str().unwrap();
    let current_row = lines.iter().find(|l| l.contains(p2_name)).expect("p2 must have a row");
    let alternate_row = lines.iter().find(|l| l.contains(p1_name)).expect("p1 must have a row");
    assert!(current_row.contains('%'), "p2 row must be marked current with '%'");
    assert!(alternate_row.contains('#'), "p1 row must be marked alternate with '#'");
}

// ── Dirty indicator ───────────────────────────────────────────────────────────

#[test]
fn ls_dirty_buffer_shows_plus() {
    let mut ed = editor_from("-[h]>ello\n");
    // Make the buffer dirty: enter insert, type a char, escape.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty(), "buffer must be dirty after edit");
    let out = ls_output(&mut ed);
    assert!(out.contains('+'), ":ls must show '+' for dirty buffers");
}

#[test]
fn ls_clean_buffer_no_plus() {
    let mut ed = editor_from("-[h]>ello\n");
    let out = ls_output(&mut ed);
    // Header has no '+'. Buffer row should not have '+' (clean).
    // Only check the buffer rows (skip header).
    for line in out.lines().skip(1) {
        assert!(!line.contains('+'), "clean buffer row must not contain '+'");
    }
}

// ── Scratch buffer ────────────────────────────────────────────────────────────

#[test]
fn ls_scratch_buffer_shows_scratch_name() {
    let mut ed = editor_from("-[h]>ello\n");
    let out = ls_output(&mut ed);
    // The initial unnamed buffer has path=None → name is "[scratch]".
    assert!(out.contains("[scratch]"), ":ls must show '[scratch]' for nameless buffers");
}

// ── Cursor placement ──────────────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn ls_cursor_on_current_row() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    ed.execute_typed("ls", None).unwrap();

    let sv = ed.scratch_view.as_ref().unwrap();
    let cursor_char = sv.sels.primary().head;
    let cursor_line = sv.buf.rope().char_to_line(cursor_char);
    let content = sv.buf.rope().to_string();
    let p2_name = p2.file_name().unwrap().to_str().unwrap();
    // Line 0 is the header; we need the 0-indexed line that contains p2's name.
    let expected_line = content
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains(p2_name))
        .map(|(i, _)| i)
        .expect("p2 row must be in output");
    assert_eq!(cursor_line, expected_line, "cursor must be on the current buffer's row");
}
