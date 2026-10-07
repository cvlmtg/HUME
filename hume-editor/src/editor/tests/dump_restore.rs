//! The restore prompt for a crash dump found next to a file on open.

use super::*;
use crate::editor::input_stack::ConfirmAction;
use pretty_assertions::assert_eq;

fn dump_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
    crate::editor::buffer::dump_path_for(&dir.path().join("foo.txt"))
}

/// An editor on a scratch buffer that has opened `path` with `:e`, so the
/// file's buffer is focused and `OnBufferEnter` has been settled.
fn editor_that_opened(path: &std::path::Path) -> Editor {
    let mut ed = editor_from("-[s]>cratch\n");
    type_cmd_event(&mut ed, &format!(":e {}", path.display()));
    ed
}

/// Leave the focused buffer and come back, the way `:b #` twice does.
fn leave_and_return(ed: &mut Editor) {
    type_cmd_event(ed, ":b #");
    type_cmd_event(ed, ":b #");
}

#[test]
fn opening_a_file_with_a_dump_asks_what_to_do_with_it() {
    let (_dir, path) = file_with_dump(Some("disk\n"), Some("crashed\n"));

    let ed = editor_that_opened(&path);

    let bid = ed.focused_buffer_id();
    let confirm = ed.state.input.confirm().expect("restore prompt is open");
    assert!(matches!(confirm.action, ConfirmAction::RestoreDump(id) if id == bid));
    let keys: Vec<char> = confirm.action.choices().iter().map(|c| c.key).collect();
    assert_eq!(keys, ['r', 'd', 'k']);
    insta::assert_snapshot!(
        confirm.render_line(),
        @"foo.txt: recovered unsaved changes found (foo.txt.dump).  [r]restore  [d]discard  [k]keep"
    );
}

#[test]
fn offering_a_dump_for_a_closed_buffer_does_nothing() {
    let (_dir, path) = file_with_dump(Some("disk\n"), Some("crashed\n"));
    let mut ed = editor_that_opened(&path);
    let bid = ed.focused_buffer_id();
    ed.close_buffer(bid);

    ed.offer_dump_restore(bid);

    assert!(ed.state.input.confirm().is_none());
}

#[test]
fn opening_a_file_without_a_dump_opens_no_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("foo.txt");
    std::fs::write(&path, "disk\n").unwrap();

    let ed = editor_that_opened(&path);

    assert!(ed.state.input.confirm().is_none());
}

#[test]
fn restore_loads_the_dump_as_a_dirty_undoable_edit_and_removes_it() {
    let (dir, path) = file_with_dump(Some("disk\n"), Some("crashed\n"));
    let mut ed = editor_that_opened(&path);
    let path_before = ed.doc().path().map(std::path::Path::to_path_buf);

    ed.handle_key(key('r'));

    assert!(ed.state.input.confirm().is_none());
    assert_eq!(ed.doc().text().to_string(), "crashed\n");
    assert!(focused_unsaved(&ed));
    assert_eq!(
        ed.doc().path().map(std::path::Path::to_path_buf),
        path_before
    );
    assert!(!ed.doc().is_new_file());
    assert!(!dump_file(&dir).exists());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "disk\n");

    ed.handle_key(key('u'));

    assert_eq!(ed.doc().text().to_string(), "disk\n");
}

#[test]
fn a_save_as_does_not_redirect_the_pending_dump_to_the_new_path() {
    let (dir, path) = file_with_dump(Some("disk\n"), Some("crashed\n"));
    std::fs::write(dir.path().join("other.txt.dump"), "unrelated\n").unwrap();
    let mut ed = editor_that_opened(&path);
    ed.handle_key(key('j'));
    type_cmd_event(
        &mut ed,
        &format!(":w {}", dir.path().join("other.txt").display()),
    );

    leave_and_return(&mut ed);

    let confirm = ed.state.input.confirm().expect("restore prompt is open");
    assert!(
        confirm.prompt.contains("(foo.txt.dump)"),
        "{}",
        confirm.prompt
    );
    ed.handle_key(key('r'));
    assert_eq!(ed.doc().text().to_string(), "crashed\n");
    assert!(!dump_file(&dir).exists());
    assert!(dir.path().join("other.txt.dump").exists());
}

#[cfg(unix)]
#[test]
fn a_dump_that_is_a_symlink_is_ignored_with_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("foo.txt");
    std::fs::write(&path, "disk\n").unwrap();
    let secret = dir.path().join("secret");
    std::fs::write(&secret, "private\n").unwrap();
    std::os::unix::fs::symlink(&secret, dir.path().join("foo.txt.dump")).unwrap();

    let ed = editor_that_opened(&path);

    assert!(ed.state.input.confirm().is_none());
    assert_eq!(ed.doc().text().to_string(), "disk\n");
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|m| m.text.contains("foo.txt.dump")),
        "a warning names the ignored dump"
    );
}

#[test]
fn restore_keeps_the_dumps_line_endings() {
    let (_dir, path) = file_with_dump(Some("disk\n"), Some("a\r\nb\r\n"));
    let mut ed = editor_that_opened(&path);

    ed.handle_key(key('r'));

    assert_eq!(ed.doc().serialized(), "a\r\nb\r\n");
}

#[test]
fn restoring_a_dump_equal_to_the_file_leaves_the_buffer_clean() {
    let (dir, path) = file_with_dump(Some("same\n"), Some("same\n"));
    let mut ed = editor_that_opened(&path);

    ed.handle_key(key('r'));

    assert!(!focused_unsaved(&ed));
    assert!(!dump_file(&dir).exists());
}

#[test]
fn restore_works_for_a_file_that_does_not_exist_yet() {
    let (dir, path) = file_with_dump(None, Some("crashed\n"));
    let mut ed = editor_that_opened(&path);
    assert!(ed.doc().is_new_file(), "setup: the file is missing");

    ed.handle_key(key('r'));

    assert_eq!(ed.doc().text().to_string(), "crashed\n");
    assert!(focused_unsaved(&ed));
    assert!(ed.doc().is_new_file());
    assert!(!dump_file(&dir).exists());
}

#[test]
fn discard_removes_the_dump_and_leaves_the_buffer_as_on_disk() {
    let (dir, path) = file_with_dump(Some("disk\n"), Some("crashed\n"));
    let mut ed = editor_that_opened(&path);

    ed.handle_key(key('d'));

    assert!(ed.state.input.confirm().is_none());
    assert_eq!(ed.doc().text().to_string(), "disk\n");
    assert!(!focused_unsaved(&ed));
    assert!(!dump_file(&dir).exists());
}

#[test]
fn keep_leaves_the_dump_and_does_not_ask_again_this_session() {
    let (dir, path) = file_with_dump(Some("disk\n"), Some("crashed\n"));
    let mut ed = editor_that_opened(&path);

    ed.handle_key(key('k'));
    leave_and_return(&mut ed);

    assert!(ed.state.input.confirm().is_none());
    assert_eq!(ed.doc().text().to_string(), "disk\n");
    assert!(dump_file(&dir).exists());
}

#[test]
fn escape_counts_as_keep() {
    let (dir, path) = file_with_dump(Some("disk\n"), Some("crashed\n"));
    let mut ed = editor_that_opened(&path);

    ed.handle_key(key_esc());
    leave_and_return(&mut ed);

    assert!(ed.state.input.confirm().is_none());
    assert!(dump_file(&dir).exists());
}

#[test]
fn a_stray_key_leaves_the_question_open_for_the_next_buffer_enter() {
    let (dir, path) = file_with_dump(Some("disk\n"), Some("crashed\n"));
    let mut ed = editor_that_opened(&path);

    ed.handle_key(key('l'));
    assert!(
        ed.state.input.confirm().is_none(),
        "the stray key dismisses it"
    );
    leave_and_return(&mut ed);

    assert!(ed.state.input.confirm().is_some());
    assert!(dump_file(&dir).exists());
}

#[test]
fn reloading_a_file_does_not_ask_about_its_kept_dump() {
    let (_dir, path) = file_with_dump(Some("disk\n"), Some("crashed\n"));
    let mut ed = editor_that_opened(&path);
    ed.handle_key(key('k'));

    type_cmd_event(&mut ed, ":e!");

    assert!(ed.state.input.confirm().is_none());
}
