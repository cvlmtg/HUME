use super::*;
use pretty_assertions::assert_eq;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Write `content` to a temp file and return its path (kept alive by the returned TempPath).
fn temp_file(content: &str) -> (std::path::PathBuf, tempfile::TempPath) {
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), content).unwrap();
    let path = f.path().to_path_buf();
    (path, f.into_temp_path())
}

/// Type a colon command into the editor via handle_key (goes through %/# expansion).
fn type_cmd(ed: &mut Editor, cmd: &str) {
    for ch in cmd.chars() { ed.handle_key(key(ch)); }
    ed.handle_key(key_enter());
}

// ── alternate_buffer() ────────────────────────────────────────────────────────

#[test]
fn alternate_buffer_none_with_single_buffer() {
    let ed = editor_from("-[h]>ello\n");
    assert_eq!(ed.alternate_buffer(), None);
}

#[test]
#[cfg(not(windows))]
fn alternate_buffer_is_previous_focused() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    let id_a = ed.focused_buffer_id();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    let id_b = ed.focused_buffer_id();

    assert_ne!(id_a, id_b, "A and B must be distinct");
    assert_eq!(ed.alternate_buffer(), Some(id_a));
}

// ── Ctrl+6 / goto-alternate-file ─────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn ctrl_6_switches_to_alternate_and_is_involutive() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    let id_a = ed.focused_buffer_id();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    let id_b = ed.focused_buffer_id();

    ed.handle_key(key_ctrl('6'));
    assert_eq!(ed.focused_buffer_id(), id_a, "Ctrl+6 must switch to alternate");

    ed.handle_key(key_ctrl('6'));
    assert_eq!(ed.focused_buffer_id(), id_b, "Ctrl+6 again returns to starting buffer");
}

#[test]
#[cfg(not(windows))]
fn ctrl_6_pushes_jump_entry() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    let id_before = ed.focused_buffer_id();

    ed.handle_key(key_ctrl('6'));
    assert_ne!(ed.focused_buffer_id(), id_before, "Ctrl+6 changes focus");
    ed.handle_key(key_ctrl('o'));
    assert_eq!(ed.focused_buffer_id(), id_before, "Ctrl+O retraces Ctrl+6");
}

#[test]
fn ctrl_6_warns_when_no_alternate() {
    let mut ed = editor_from("-[h]>ello\n");
    let id_before = ed.focused_buffer_id();
    ed.handle_key(key_ctrl('6'));
    assert_eq!(ed.focused_buffer_id(), id_before, "no buffer change with no alternate");
    let msg = ed.status_msg.as_deref().expect("warning should be reported");
    assert!(msg.contains("No alternate buffer"), "unexpected status: {msg:?}");
}

// ── %/# expansion in typed commands ──────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn colon_e_hash_opens_alternate() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    let id_a = ed.focused_buffer_id();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    let buf_count = ed.buffers.len();

    type_cmd(&mut ed, &format!(":e #"));
    assert_eq!(ed.focused_buffer_id(), id_a, ":e # must switch to alternate");
    assert_eq!(ed.buffers.len(), buf_count, ":e # must not open a duplicate");
}

#[test]
fn colon_e_hash_errors_with_no_alternate() {
    let mut ed = editor_from("-[h]>ello\n");
    type_cmd(&mut ed, ":e #");
    let msg = ed.status_msg.as_deref().expect("error should be reported");
    assert!(msg.contains("No alternate buffer"), "unexpected status: {msg:?}");
}

#[test]
#[cfg(not(windows))]
fn colon_e_percent_is_noop_reload() {
    let (p1, _t1) = temp_file("file1\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    let id_before = ed.focused_buffer_id();
    let count_before = ed.buffers.len();

    type_cmd(&mut ed, ":e %");
    assert_eq!(ed.focused_buffer_id(), id_before, ":e % stays on same buffer");
    assert_eq!(ed.buffers.len(), count_before, ":e % does not duplicate");
}

#[test]
fn colon_e_percent_errors_with_no_path() {
    let mut ed = editor_from("-[h]>ello\n");
    type_cmd(&mut ed, ":e %");
    let msg = ed.status_msg.as_deref().expect("error should be reported");
    assert!(msg.contains("No file name"), "unexpected status: {msg:?}");
}

// ── goto-alternate-file in registry ──────────────────────────────────────────

#[test]
fn goto_alternate_file_is_registered_as_jump() {
    let reg = super::super::registry::CommandRegistry::with_defaults();
    let cmd = reg.get_mappable("goto-alternate-file")
        .expect("goto-alternate-file must be registered");
    assert!(cmd.is_jump(), "goto-alternate-file must have jump:true");
}
