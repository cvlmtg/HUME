use super::*;

// All tutor tests set HUME_RUNTIME and TMPDIR to temp dirs, so they are
// gated #[cfg(not(windows))] — HUME_RUNTIME is not honoured on Windows
// because runtime_dir() uses a different branch there, and env::set_var in
// parallel tests is unsafe and requires the mutex guard.

const MARKER: &str = "=== HUME Tutor Test ===";
const STUB: &str = "=== HUME Tutor Test ===\nLesson 1\n";

/// Write STUB into `dir/tutor.txt` and return the canonical path of that file.
#[cfg(not(windows))]
fn write_stub_tutor(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("tutor.txt");
    std::fs::write(&path, STUB).unwrap();
    std::fs::canonicalize(&path).unwrap()
}

// ── :tutor opens the lesson file as a tmp copy ────────────────────────────────

#[test]
#[cfg(not(windows))]
fn tutor_opens_buffer_with_lesson_content() {
    let runtime = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    write_stub_tutor(runtime.path());
    let _guard = HumeRuntimeGuard::new(runtime.path(), tmp.path());

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("tutor", None).unwrap();

    let text = ed.doc().text().rope().to_string();
    assert!(
        text.contains(MARKER),
        "tutor buffer must contain the lesson marker, got: {text:?}"
    );

    // Buffer path must be inside the test TMPDIR, not the runtime source dir.
    let buf_path = ed.doc().path().expect("tutor buffer must have a path set");
    assert!(
        buf_path.starts_with(std::fs::canonicalize(tmp.path()).unwrap()),
        "tutor buffer path must be inside TMPDIR, got: {buf_path:?}"
    );
}

// ── :tutor is idempotent ──────────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn tutor_is_idempotent() {
    let runtime = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    write_stub_tutor(runtime.path());
    let _guard = HumeRuntimeGuard::new(runtime.path(), tmp.path());

    let mut ed = editor_from("-[h]>ello\n");
    let count_before = ed.buffers.iter().count();

    ed.execute_typed("tutor", None).unwrap();
    let bid_first = ed.focused_buffer_id();
    let count_after_first = ed.buffers.iter().count();

    ed.execute_typed("tutor", None).unwrap();
    let bid_second = ed.focused_buffer_id();
    let count_after_second = ed.buffers.iter().count();

    assert_eq!(
        count_after_first,
        count_before + 1,
        "first :tutor must open exactly one new buffer"
    );
    assert_eq!(
        count_after_second, count_after_first,
        "second :tutor must not open another buffer"
    );
    assert_eq!(
        bid_first, bid_second,
        "second :tutor must switch to the same buffer, not open a duplicate"
    );
}

// ── after :bd!, :tutor opens a fresh buffer ───────────────────────────────────

#[test]
#[cfg(not(windows))]
fn tutor_after_bd_opens_fresh() {
    let runtime = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    write_stub_tutor(runtime.path());
    let _guard = HumeRuntimeGuard::new(runtime.path(), tmp.path());

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("tutor", None).unwrap();
    let bid_first = ed.focused_buffer_id();

    // Force-close the tutor buffer.
    ed.execute_typed("bd!", None).unwrap();

    ed.execute_typed("tutor", None).unwrap();
    let bid_second = ed.focused_buffer_id();

    assert_ne!(
        bid_first, bid_second,
        ":tutor after :bd! must open a new buffer (different BufferId)"
    );
    assert!(
        ed.doc().text().rope().to_string().contains(MARKER),
        "fresh tutor buffer must still contain the lesson content"
    );
}

// ── after save-as, :tutor opens a fresh buffer at the tmp path ────────────────

#[test]
#[cfg(not(windows))]
fn tutor_after_save_as_opens_fresh() {
    let runtime = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    write_stub_tutor(runtime.path());
    let _guard = HumeRuntimeGuard::new(runtime.path(), tmp.path());

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("tutor", None).unwrap();
    let bid_first = ed.focused_buffer_id();
    let count_after_first = ed.buffers.iter().count();

    // Simulate save-as: change the tutor buffer's path to a different location.
    let elsewhere = tmp.path().join("elsewhere.txt");
    std::fs::write(&elsewhere, STUB).unwrap();
    let elsewhere_canonical = std::fs::canonicalize(&elsewhere).unwrap();
    ed.doc_mut().set_path(Some(elsewhere_canonical));

    // :tutor should now find nothing at the canonical tmp path and open a new buffer.
    ed.execute_typed("tutor", None).unwrap();
    let bid_second = ed.focused_buffer_id();

    assert_ne!(
        bid_first, bid_second,
        ":tutor after save-as must open a new buffer at the canonical tmp path"
    );
    assert_eq!(
        ed.buffers.iter().count(),
        count_after_first + 1,
        "a second tutor buffer must have been opened"
    );
    assert!(
        ed.doc().text().rope().to_string().contains(MARKER),
        "new tutor buffer must contain lesson content"
    );
}

// ── tutor buffer is editable ──────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn tutor_buffer_is_editable() {
    let runtime = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    write_stub_tutor(runtime.path());
    let _guard = HumeRuntimeGuard::new(runtime.path(), tmp.path());

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("tutor", None).unwrap();

    let before = ed.doc().text().rope().to_string();

    // Enter insert mode, type a character, exit.
    ed.handle_key(key('i'));
    ed.handle_key(key('Z'));
    ed.handle_key(key_esc());

    let after = ed.doc().text().rope().to_string();

    assert_ne!(
        before, after,
        "tutor buffer must be editable — text must change after insert"
    );
    assert!(
        after.contains('Z'),
        "inserted character 'Z' must appear in the buffer"
    );
}

// ── missing tutor.txt produces a clear error ──────────────────────────────────

#[test]
#[cfg(not(windows))]
fn tutor_missing_file_returns_error() {
    let runtime = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    // Do NOT write tutor.txt — the runtime directory is empty.
    let _guard = HumeRuntimeGuard::new(runtime.path(), tmp.path());

    let mut ed = editor_from("-[h]>ello\n");
    let count_before = ed.buffers.iter().count();

    let result = ed.execute_typed("tutor", None);

    assert!(
        result.is_err(),
        ":tutor on a missing file must return an error"
    );
    let msg = result.unwrap_err().0;
    assert!(
        msg.contains("tutor.txt not found"),
        "error message must mention 'tutor.txt not found', got: {msg:?}"
    );
    assert_eq!(
        ed.buffers.iter().count(),
        count_before,
        "no new buffer must be created when tutor.txt is missing"
    );
}

// ── :w writes to tmp, not to the install source ───────────────────────────────

#[test]
#[cfg(not(windows))]
fn tutor_save_does_not_overwrite_source() {
    let runtime = tempfile::tempdir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let source_path = runtime.path().join("tutor.txt");
    std::fs::write(&source_path, STUB).unwrap();
    let _guard = HumeRuntimeGuard::new(runtime.path(), tmp.path());

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("tutor", None).unwrap();

    // Edit the tutor buffer.
    ed.handle_key(key('i'));
    ed.handle_key(key('Z'));
    ed.handle_key(key_esc());

    // Save via :w (no-arg write to the buffer's own path, which is the tmp copy).
    ed.execute_typed("w", None).unwrap();

    // The install source must be unchanged.
    let source_on_disk = std::fs::read_to_string(&source_path).unwrap();
    assert_eq!(
        source_on_disk, STUB,
        ":w must not modify the install source; source still has original content"
    );
    assert!(
        !source_on_disk.contains('Z'),
        "the edited character must not appear in the install source"
    );

    // The tmp copy must have the edit.
    let tmp_path = ed.doc().path().expect("tutor buffer must have a path");
    let tmp_on_disk = std::fs::read_to_string(tmp_path).unwrap();
    assert!(
        tmp_on_disk.contains('Z'),
        "the edited character must appear in the tmp copy after :w"
    );
}
