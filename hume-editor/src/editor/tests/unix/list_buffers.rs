use super::*;
use super::super::list_buffers::ls_output;

#[test]
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
    let current_row = lines
        .iter()
        .find(|l| l.contains(p2_name))
        .expect("p2 must have a row");
    let alternate_row = lines
        .iter()
        .find(|l| l.contains(p1_name))
        .expect("p1 must have a row");
    assert!(
        current_row.contains('%'),
        "p2 row must be marked current with '%'"
    );
    assert!(
        alternate_row.contains('#'),
        "p1 row must be marked alternate with '#'"
    );
}

#[test]
fn ls_cursor_on_current_row() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    ed.execute_typed("ls", None).unwrap();

    // After :ls the [buffers] view is focused; cursor position is in pane_state.
    let cursor_char = ed.current_selections().primary().head();
    let cursor_line = ed.doc().text().rope().char_to_line(cursor_char);
    let content = ed.doc().text().rope().to_string();
    let p2_name = p2.file_name().unwrap().to_str().unwrap();
    // Line 0 is the header; we need the 0-indexed line that contains p2's name.
    let expected_line = content
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains(p2_name))
        .map(|(i, _)| i)
        .expect("p2 row must be in output");
    assert_eq!(
        cursor_line, expected_line,
        "cursor must be on the current buffer's row"
    );
}

/// `:w /path` on a synthetic buffer writes the file but leaves the buffer
/// pathless, labeled, and read-only — the buffer itself is unaffected.
///
/// Validity: remove the `is_synthetic()` guard from `write_file` and this
/// test fails — `doc().path()` will be `Some(...)` instead of `None`.
#[test]
fn view_buffer_save_as_stays_synthetic() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let out_path = tmp.path().to_path_buf();

    let mut ed = editor_from("-[h]>ello\n");
    ed.report(Severity::Warning, "test message".to_string());
    ed.execute_typed("messages", None).unwrap();
    assert!(ed.doc().is_synthetic());
    assert!(ed.doc().is_read_only());

    let content_before = ed.doc().text().to_string();

    ed.execute_typed("w", Some(out_path.to_str().unwrap()))
        .unwrap();

    // File on disk must have the buffer content.
    let written = std::fs::read_to_string(&out_path).unwrap();
    assert_eq!(
        written, content_before,
        "file on disk must match buffer content"
    );

    // Buffer state must be unchanged — still synthetic, still pathless, still RO.
    assert!(
        ed.doc().path().is_none(),
        "synthetic buffer must stay pathless after :w /path"
    );
    assert_eq!(
        ed.doc().display_name(),
        "[messages]",
        "label must be preserved"
    );
    assert!(ed.doc().is_synthetic(), "is_synthetic() must remain true");
    assert!(ed.doc().is_read_only(), "is_read_only() must remain true");
}
