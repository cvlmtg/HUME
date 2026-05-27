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
    // After :ls the focused buffer is the read-only [buffers] view.
    ed.doc().text().rope().to_string()
}

// ── Single buffer ─────────────────────────────────────────────────────────────

#[test]
fn ls_single_buffer_marks_current() {
    let mut ed = editor_from("-[h]>ello\n");
    let out = ls_output(&mut ed);
    assert!(
        out.contains('%'),
        ":ls must mark the focused buffer with '%'"
    );
    assert!(
        !out.contains('#'),
        ":ls must not show '#' when there is no alternate buffer"
    );
    // Row count: 1 header + 1 buffer
    assert_eq!(out.lines().count(), 2, "must have header + 1 buffer row");
}

#[test]
fn ls_long_alias_works() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("list-buffers", None).unwrap();
    assert!(
        ed.doc().is_read_only(),
        ":list-buffers must open a read-only view buffer"
    );
    assert_eq!(
        ed.doc().display_name(),
        "[buffers]",
        ":list-buffers must focus the [buffers] view buffer"
    );
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
    // The initial unnamed buffer has path=None, label=None → display_name() = "*scratch*".
    assert!(
        out.contains("*scratch*"),
        ":ls must show '*scratch*' for nameless buffers, got:\n{out}"
    );
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

    // After :ls the [buffers] view is focused; cursor position is in pane_state.
    let cursor_char = ed.current_selections().primary().head;
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

// ── Read-only view buffer properties ─────────────────────────────────────────

/// `:messages` opens a real read-only buffer with label `[messages]`.
#[test]
fn messages_opens_read_only_view_buffer() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.report(Severity::Warning, "test message".to_string());
    ed.execute_typed("messages", None).unwrap();
    assert!(ed.doc().is_read_only(), ":messages must open a read-only buffer");
    assert_eq!(ed.doc().display_name(), "[messages]");
}

/// Bug regression: Up/Down in a read-only view must move the cursor (collapsed),
/// not select whole lines. In Normal mode, Down maps to `move-down`.
#[test]
fn view_buffer_arrow_keys_move_cursor_not_select() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.report(Severity::Warning, "line one".to_string());
    ed.report(Severity::Warning, "line two".to_string());
    ed.execute_typed("messages", None).unwrap();

    // Cursor starts at last content line. Move up one line.
    let head_before = ed.current_selections().primary().head;
    ed.handle_key(key_up());

    let sel = ed.current_selections().primary();
    // Selection must be collapsed (anchor == head) — not a whole-line span.
    assert_eq!(
        sel.anchor, sel.head,
        "Up in view buffer must produce a collapsed cursor, not a selection"
    );
    assert!(
        sel.head < head_before,
        "Up must move the cursor backward in the buffer"
    );
}

/// Bug regression: syntax highlighting from the previously focused buffer must
/// not bleed into the messages view. The view buffer must have no language.
#[test]
fn view_buffer_has_no_language() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.report(Severity::Warning, "test".to_string());
    ed.execute_typed("messages", None).unwrap();
    assert!(
        ed.doc().language.is_none(),
        "view buffer must have no language (no syntax highlighting)"
    );
}

/// Repeated `:messages` calls reuse the same buffer rather than accumulating duplicates.
#[test]
fn messages_reuses_existing_view_buffer() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.report(Severity::Warning, "msg1".to_string());
    ed.execute_typed("messages", None).unwrap();
    // Switch back to the scratch buffer so the second :messages performs a real switch.
    let scratch_id = ed
        .buffers
        .iter()
        .find(|(_, buf)| buf.label.is_none() && buf.path().is_none())
        .map(|(id, _)| id)
        .expect("scratch buffer must exist");
    ed.switch_to_buffer_without_jump(scratch_id);
    ed.report(Severity::Warning, "msg2".to_string());
    ed.execute_typed("messages", None).unwrap();

    // Count buffers with the [messages] label — must be exactly 1.
    let count = ed
        .buffers
        .iter()
        .filter(|(_, buf)| buf.label.as_deref() == Some("[messages]"))
        .count();
    assert_eq!(count, 1, ":messages must reuse the existing [messages] buffer");
}

/// Editing commands on a read-only view buffer must be silently blocked.
#[test]
fn view_buffer_blocks_edits() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.report(Severity::Warning, "test".to_string());
    ed.execute_typed("messages", None).unwrap();
    let content_before = ed.doc().text().to_string();

    // Try to delete the focused character — should be a no-op.
    ed.handle_key(key('x'));
    assert_eq!(
        ed.doc().text().to_string(),
        content_before,
        "x (delete) must not mutate a read-only buffer"
    );
}

/// Entering Insert mode on a read-only buffer must be refused.
#[test]
fn view_buffer_blocks_insert_mode() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.report(Severity::Warning, "test".to_string());
    ed.execute_typed("messages", None).unwrap();

    ed.handle_key(key('i'));
    assert_ne!(
        ed.mode,
        Mode::Insert,
        "i must not enter Insert mode on a read-only buffer"
    );
}
