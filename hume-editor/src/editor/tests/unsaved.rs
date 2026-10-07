//! `EditorState::has_unsaved_changes`: revision-dirty or an open Insert
//! session with edits.

use super::*;
use crate::statusline::{StatusElement, render_element};
use hume_scripting::host::BufferHost;

fn file_editor() -> (tempfile::TempDir, Editor) {
    let dir = tempfile::tempdir().unwrap();
    let ed = editor_with_path("hello\n", &dir.path().join("foo.txt"));
    (dir, ed)
}

#[test]
fn a_fresh_buffer_has_no_unsaved_changes() {
    let (_dir, ed) = file_editor();

    assert!(!ed.state.has_unsaved_changes(ed.focused_buffer_id()));
}

#[test]
fn an_insert_session_with_an_edit_is_unsaved() {
    let (_dir, mut ed) = file_editor();
    let bid = ed.focused_buffer_id();

    ed.handle_key(key('i'));
    ed.handle_key(key('x'));

    assert!(ed.state.has_unsaved_changes(bid));
}

#[test]
fn an_insert_session_without_an_edit_is_not_unsaved() {
    let (_dir, mut ed) = file_editor();

    ed.handle_key(key('i'));

    assert!(!ed.state.has_unsaved_changes(ed.focused_buffer_id()));
}

#[test]
fn undoing_a_committed_session_back_to_the_save_point_is_clean() {
    let (_dir, mut ed) = file_editor();
    let bid = ed.focused_buffer_id();
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.state.has_unsaved_changes(bid));

    ed.handle_key(key('u'));

    assert!(!ed.state.has_unsaved_changes(bid));
}

#[test]
fn another_buffers_session_does_not_count() {
    let dir = tempfile::tempdir().unwrap();
    let (first, second) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
    std::fs::write(&first, "a\n").unwrap();
    std::fs::write(&second, "b\n").unwrap();
    let mut ed = editor_from("-[s]>cratch\n");
    type_cmd_event(&mut ed, &format!(":e {}", first.display()));
    let first_bid = ed.focused_buffer_id();
    type_cmd_event(&mut ed, &format!(":e {}", second.display()));
    assert_ne!(ed.focused_buffer_id(), first_bid);

    ed.handle_key(key('i'));
    ed.handle_key(key('x'));

    assert!(ed.state.has_unsaved_changes(ed.focused_buffer_id()));
    assert!(!ed.state.has_unsaved_changes(first_bid));
}

#[test]
fn the_script_host_reports_an_open_session_as_dirty() {
    let (_dir, mut ed) = file_editor();
    let bid = ed.focused_buffer_id();
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));

    let host = crate::editor::host_impl::EditorHostImpl::new(&mut ed.state, &mut ed.view);

    assert_eq!(host.buffer_is_dirty(bid), Some(true));
}

#[test]
fn the_statusline_shows_the_indicator_while_an_insert_session_has_an_edit() {
    let (_dir, mut ed) = file_editor();
    let colors = crate::statusline::colors::EditorColors::default();
    let (before, _) = render_element(
        &StatusElement::DirtyIndicator,
        &ed.statusline(),
        &colors,
        "",
    );
    assert!(before.is_empty(), "a fresh buffer is clean, got {before:?}");

    ed.handle_key(key('i'));
    ed.handle_key(key('x'));

    let (during, _) = render_element(
        &StatusElement::DirtyIndicator,
        &ed.statusline(),
        &colors,
        "",
    );
    assert_eq!(during.as_ref(), "[+]");
}
