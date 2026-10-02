//! Dumping dirty buffers when the editor panics.

use super::*;
use pretty_assertions::assert_eq;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn dirty_focused(ed: &mut Editor) {
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
}

fn dirty_file_editor(dir: &tempfile::TempDir, content: &str) -> Editor {
    let mut ed = editor_with_path(content, &dir.path().join("foo.txt"));
    dirty_focused(&mut ed);
    ed
}

fn dump_path(dir: &tempfile::TempDir) -> std::path::PathBuf {
    crate::editor::buffer::dump_path_for(&dir.path().join("foo.txt"))
}

fn dump_text(dir: &tempfile::TempDir) -> Option<String> {
    std::fs::read_to_string(dump_path(dir)).ok()
}

/// Run a panicking `run_dumping_on_panic` on `ed`; `true` when the panic
/// propagated out.
fn panic_inside(ed: &mut Editor) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        ed.run_dumping_on_panic(None, |_| panic!("boom"))
    }))
    .is_err()
}

#[test]
fn panic_dumps_a_dirty_file_buffer_next_to_its_file() {
    let dir = safe_tempdir();
    let mut ed = dirty_file_editor(&dir, "hello\n");

    assert!(panic_inside(&mut ed), "the panic must keep propagating");
    assert_eq!(dump_text(&dir).unwrap(), "xhello\n");
}

#[test]
fn panic_dumps_edits_of_an_open_insert_session() {
    let dir = safe_tempdir();
    let path = dir.path().join("foo.txt");
    let mut ed = editor_with_path("hello\n", &path);
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));

    assert!(panic_inside(&mut ed), "the panic must keep propagating");
    assert_eq!(dump_text(&dir).unwrap(), "xhello\n");
}

#[test]
fn panic_dump_keeps_crlf_line_endings() {
    let dir = safe_tempdir();
    let mut ed = dirty_file_editor(&dir, "a\r\nb\r\n");

    panic_inside(&mut ed);

    assert_eq!(std::fs::read(dump_path(&dir)).unwrap(), b"xa\r\nb\r\n");
}

#[test]
fn the_crash_report_names_dumps_then_queued_worker_panics() {
    let dir = safe_tempdir();
    let mut ed = dirty_file_editor(&dir, "hello\n");
    let panics = hume_platform::worker_panic::WorkerPanics::default();
    ed.attach_worker_panics(panics.clone());
    panics.record(lsp_reader_panic());

    let mut out = Vec::new();
    ed.write_crash_report(&mut out, None);

    let out = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    assert!(
        lines[0].starts_with("hume: unsaved foo.txt saved to "),
        "{out}"
    );
    assert_eq!(
        lines[1],
        "hume: thread 'hume-lsp-reader' panicked at hume-lsp/src/transport.rs:150:9: bad frame"
    );
}

#[test]
fn panic_does_not_dump_a_clean_buffer() {
    let dir = safe_tempdir();
    let mut ed = editor_with_path("hello\n", &dir.path().join("foo.txt"));

    panic_inside(&mut ed);

    assert!(!dump_path(&dir).exists());
}

#[test]
fn a_run_that_returns_normally_writes_no_dump_and_passes_its_value_through() {
    let dir = safe_tempdir();
    let mut ed = dirty_file_editor(&dir, "hello\n");

    let value = ed.run_dumping_on_panic(None, |_| 7);

    assert_eq!(value, 7);
    assert!(!dump_path(&dir).exists());
}

#[test]
fn dump_skips_read_only_view_buffers() {
    let ed = editor_with_read_only_view("log line\n", "[messages]");

    assert!(ed.dump_dirty_buffers(None).is_empty());
}

#[test]
fn dump_writes_a_dirty_scratch_buffer_into_the_scratch_dir() {
    let dir = safe_tempdir();
    let scratch_dir = dir.path().join("dumps");
    let mut ed = editor_from("-[h]>ello\n");
    dirty_focused(&mut ed);

    let outcomes = ed.dump_dirty_buffers(Some(&scratch_dir));

    assert_eq!(outcomes.len(), 1);
    let written = outcomes[0].1.as_ref().expect("scratch dump written");
    assert_eq!(written.parent().unwrap(), scratch_dir);
    assert_eq!(std::fs::read_to_string(written).unwrap(), "xhello\n");
}

#[test]
fn dump_falls_back_to_the_scratch_dir_when_the_file_s_folder_is_unwritable() {
    let dir = safe_tempdir();
    let scratch_dir = dir.path().join("dumps");
    let path = dir.path().join("missing").join("draft.md");
    let mut ed = editor_with_path("hello\n", &path);
    dirty_focused(&mut ed);

    let outcomes = ed.dump_dirty_buffers(Some(&scratch_dir));

    assert_eq!(outcomes.len(), 1);
    let written = outcomes[0].1.as_ref().expect("fallback dump written");
    assert_eq!(written.parent().unwrap(), scratch_dir);
    let file_name = written.file_name().unwrap().to_string_lossy();
    assert!(file_name.starts_with("draft.md-"), "{file_name}");
    assert!(file_name.ends_with(".dump"), "{file_name}");
    assert_eq!(std::fs::read_to_string(written).unwrap(), "xhello\n");
}

#[test]
fn dump_reports_the_sibling_error_when_the_fallback_fails_too() {
    let dir = safe_tempdir();
    let path = dir.path().join("missing").join("draft.md");
    let mut ed = editor_with_path("hello\n", &path);
    dirty_focused(&mut ed);

    let outcomes = ed.dump_dirty_buffers(None);

    assert_eq!(outcomes.len(), 1);
    let err = outcomes[0].1.as_ref().unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
}

#[test]
fn dump_reports_a_dirty_scratch_buffer_it_cannot_place() {
    let mut ed = editor_from("-[h]>ello\n");
    dirty_focused(&mut ed);

    let outcomes = ed.dump_dirty_buffers(None);

    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].1.is_err());
}

#[test]
fn one_failed_dump_does_not_skip_the_other_buffers() {
    let dir = safe_tempdir();
    let good = dir.path().join("good.txt");
    let bad = dir.path().join("missing-dir").join("bad.txt");
    let mut ed = editor_with_path("one\n", &bad);
    dirty_focused(&mut ed);

    let mut second = Buffer::at_start(BufferText::from("two\n"));
    second.set_path(Some(good.clone()));
    let second = ed.open_buffer(second);
    let fp = crate::editor::commands::FocusedPane::current(&ed.state);
    ed.switch_to_buffer_without_jump(fp, second);
    dirty_focused(&mut ed);

    let outcomes = ed.dump_dirty_buffers(None);

    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes.iter().filter(|(_, r)| r.is_err()).count(), 1);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("good.txt.dump")).unwrap(),
        "xtwo\n"
    );
}

// ── abnormal exits ────────────────────────────────────────────────────────

fn signalled(code: i32) -> std::sync::Arc<std::sync::atomic::AtomicI32> {
    std::sync::Arc::new(std::sync::atomic::AtomicI32::new(code))
}

#[test]
fn a_signal_exit_dumps_the_dirty_buffers() {
    let dir = safe_tempdir();
    let mut ed = dirty_file_editor(&dir, "hello\n");
    ed.attach_terminate_flag(signalled(143));

    let outcomes = ed.dump_if_abnormal_exit(&Ok(()), None);

    assert_eq!(outcomes.len(), 1);
    assert_eq!(dump_text(&dir).as_deref(), Some("xhello\n"));
}

#[test]
fn a_terminal_hangup_error_dumps_the_dirty_buffers() {
    let dir = safe_tempdir();
    let ed = dirty_file_editor(&dir, "hello\n");

    ed.dump_if_abnormal_exit(&Err(io::ErrorKind::UnexpectedEof.into()), None);

    assert_eq!(dump_text(&dir).as_deref(), Some("xhello\n"));
}

#[test]
fn any_other_run_error_dumps_the_dirty_buffers() {
    let dir = safe_tempdir();
    let ed = dirty_file_editor(&dir, "hello\n");

    ed.dump_if_abnormal_exit(&Err(io::ErrorKind::PermissionDenied.into()), None);

    assert_eq!(dump_text(&dir).as_deref(), Some("xhello\n"));
}

#[test]
fn a_deliberate_quit_is_not_dumped_even_if_a_signal_follows() {
    let dir = safe_tempdir();
    let mut ed = dirty_file_editor(&dir, "hello\n");
    ed.attach_terminate_flag(signalled(143));
    ed.state.request_quit();

    let outcomes = ed.dump_if_abnormal_exit(&Err(io::ErrorKind::BrokenPipe.into()), None);

    assert!(outcomes.is_empty());
    assert_eq!(dump_text(&dir), None);
}

#[test]
fn a_normal_exit_is_not_dumped() {
    let dir = safe_tempdir();
    let ed = dirty_file_editor(&dir, "hello\n");

    let outcomes = ed.dump_if_abnormal_exit(&Ok(()), None);

    assert!(outcomes.is_empty());
    assert_eq!(dump_text(&dir), None);
}

#[test]
fn an_abnormal_exit_leaves_a_clean_buffer_alone() {
    let dir = safe_tempdir();
    let mut ed = editor_with_path("hello\n", &dir.path().join("foo.txt"));
    ed.attach_terminate_flag(signalled(143));

    let outcomes = ed.dump_if_abnormal_exit(&Ok(()), None);

    assert!(outcomes.is_empty());
    assert_eq!(dump_text(&dir), None);
}
