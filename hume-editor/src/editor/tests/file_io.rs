#[cfg(unix)]
use hume_engine::pipeline::BufferId;

use super::*;
use pretty_assertions::assert_eq;

// ── :w! force-write ───────────────────────────────────────────────────────────

/// `:wq!` force-writes and then quits. Even for a writable (scratch-free)
/// file, `should_quit` must be `true` after the command.
#[test]
fn colon_wq_bang_force_writes_and_quits() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    for ch in ":wq!".chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
    assert_eq!(ed.state.status_msg.as_deref(), Some("Written 1 lines"));
    assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "hello\n");
    assert!(ed.state.should_quit);
}

// ── insert-at-selection-start / insert-at-selection-end ──────────────────────

/// `i` with a forward selection collapses to the start of the selection.
#[test]
fn insert_at_selection_start_forward() {
    let mut ed = editor_from("foo -[bar]> baz\n");
    ed.handle_key(key('i'));
    assert_eq!(state(&ed), "foo -[b]>ar baz\n");
    assert_eq!(ed.state.mode, Mode::Insert);
}

/// `i` with a backward selection also collapses to the start (lower index).
#[test]
fn insert_at_selection_start_backward() {
    let mut ed = editor_from("foo <[bar]- baz\n");
    ed.handle_key(key('i'));
    assert_eq!(state(&ed), "foo -[b]>ar baz\n");
    assert_eq!(ed.state.mode, Mode::Insert);
}

/// `i` with a collapsed cursor just enters insert at the same position.
#[test]
fn insert_at_selection_start_collapsed() {
    let mut ed = editor_from("foo -[b]>ar baz\n");
    ed.handle_key(key('i'));
    assert_eq!(state(&ed), "foo -[b]>ar baz\n");
    assert_eq!(ed.state.mode, Mode::Insert);
}

// ── :e on already-open buffers ────────────────────────────────────────────────

// ── open_extra_files ──────────────────────────────────────────────────────────

// ── :wa (write all) ────────────────────────────────────────────────────────────

/// Make the focused buffer dirty by inserting 'x' at cursor.
#[cfg(unix)]
pub(super) fn dirty_focused(ed: &mut Editor) {
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
}

/// Create a temp file and open it as one more buffer. The buffer starts clean
/// (same content as the file). Caller must dirty it themselves after switching.
#[cfg(unix)]
pub(super) fn open_file_buffer(ed: &mut Editor, content: &str) -> (tempfile::TempPath, BufferId) {
    let (buf, tmp_path) = file_buffer(content);
    let bid = ed.open_buffer(buf);
    (tmp_path, bid)
}
