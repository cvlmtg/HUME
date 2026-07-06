use super::*;
use crate::editor::host_impl::EditorHostImpl;
use hume_scripting::host::EditorHost;
use pretty_assertions::assert_eq;

/// Build a live `EditorHostImpl` borrowing `$ed`'s state/view, for direct
/// `run_command_sync` dispatch — bypasses the keymap entirely.
macro_rules! host {
    ($ed:ident) => {
        EditorHostImpl {
            state: &mut $ed.state,
            view: &mut $ed.view,
        }
    };
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Write `content` to a temp file and return its path (kept alive by the returned TempPath).
fn temp_file(content: &str) -> (std::path::PathBuf, tempfile::TempPath) {
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), content).unwrap();
    let path = f.path().to_path_buf();
    (path, f.into_temp_path())
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

// ── goto-alternate-file  ───────────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn goto_alternate_file_switches_to_alternate_and_is_involutive() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    let id_a = ed.focused_buffer_id();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    let id_b = ed.focused_buffer_id();

    host!(ed)
        .run_command_sync("goto-alternate-file", 1, false, None)
        .expect("goto-alternate-file must not error");
    assert_eq!(
        ed.focused_buffer_id(),
        id_a,
        "goto-alternate-file must switch to alternate"
    );

    host!(ed)
        .run_command_sync("goto-alternate-file", 1, false, None)
        .expect("goto-alternate-file must not error");
    assert_eq!(
        ed.focused_buffer_id(),
        id_b,
        "goto-alternate-file again returns to starting buffer"
    );
}

#[test]
#[cfg(not(windows))]
fn goto_alternate_file_pushes_jump_entry() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    let id_before = ed.focused_buffer_id();

    host!(ed)
        .run_command_sync("goto-alternate-file", 1, false, None)
        .expect("goto-alternate-file must not error");
    assert_ne!(
        ed.focused_buffer_id(),
        id_before,
        "goto-alternate-file changes focus"
    );
    host!(ed)
        .run_command_sync("jump-backward", 1, false, None)
        .expect("jump-backward must not error");
    assert_eq!(
        ed.focused_buffer_id(),
        id_before,
        "jump-backward retraces goto-alternate-file"
    );
}

#[test]
fn goto_alternate_file_warns_when_no_alternate() {
    let mut ed = editor_from("-[h]>ello\n");
    let id_before = ed.focused_buffer_id();
    host!(ed)
        .run_command_sync("goto-alternate-file", 1, false, None)
        .expect("goto-alternate-file must not error");
    assert_eq!(
        ed.focused_buffer_id(),
        id_before,
        "no buffer change with no alternate"
    );
    let msg = ed
        .state
        .status_msg
        .as_deref()
        .expect("warning should be reported");
    assert!(
        msg.contains("No alternate buffer"),
        "unexpected status: {msg:?}"
    );
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
    let buf_count = ed.state.buffers.len();

    type_cmd(&mut ed, ":e #");
    assert_eq!(
        ed.focused_buffer_id(),
        id_a,
        ":e # must switch to alternate"
    );
    assert_eq!(
        ed.state.buffers.len(),
        buf_count,
        ":e # must not open a duplicate"
    );
}

#[test]
fn colon_e_hash_errors_with_no_alternate() {
    let mut ed = editor_from("-[h]>ello\n");
    type_cmd(&mut ed, ":e #");
    let msg = ed
        .state
        .status_msg
        .as_deref()
        .expect("error should be reported");
    assert!(
        msg.contains("No alternate buffer"),
        "unexpected status: {msg:?}"
    );
}

#[test]
#[cfg(not(windows))]
fn colon_e_percent_is_noop_reload() {
    let (p1, _t1) = temp_file("file1\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    let id_before = ed.focused_buffer_id();
    let count_before = ed.state.buffers.len();

    type_cmd(&mut ed, ":e %");
    assert_eq!(
        ed.focused_buffer_id(),
        id_before,
        ":e % stays on same buffer"
    );
    assert_eq!(
        ed.state.buffers.len(),
        count_before,
        ":e % does not duplicate"
    );
}

#[test]
fn colon_e_percent_errors_with_no_path() {
    let mut ed = editor_from("-[h]>ello\n");
    type_cmd(&mut ed, ":e %");
    let msg = ed
        .state
        .status_msg
        .as_deref()
        .expect("error should be reported");
    assert!(msg.contains("No file name"), "unexpected status: {msg:?}");
}

// ── goto-alternate-file in registry ──────────────────────────────────────────

#[test]
fn goto_alternate_file_is_registered_as_jump() {
    let reg = super::super::registry::CommandRegistry::with_defaults();
    let cmd = reg
        .get_mappable("goto-alternate-file")
        .expect("goto-alternate-file must be registered");
    assert!(
        cmd.meta().is_jump,
        "goto-alternate-file must have jump:true"
    );
}
