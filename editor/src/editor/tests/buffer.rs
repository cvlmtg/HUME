use super::*;
use pretty_assertions::assert_eq;

fn temp_file(content: &str) -> (std::path::PathBuf, tempfile::TempPath) {
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), content).unwrap();
    let path = f.path().to_path_buf();
    (path, f.into_temp_path())
}

// ── :b with no argument ───────────────────────────────────────────────────────

#[test]
fn buffer_no_arg_errors() {
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed.execute_typed("b", None).unwrap_err();
    assert!(
        err.to_string().contains("usage"),
        "must show usage, got: {err}"
    );
}

// ── :b by 1-based index ───────────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn buffer_index_switches_to_nth_buffer() {
    let (p1, _t1) = temp_file("file1\n");
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    // After two :e's: order is [scratch, p1, p2], p2 is current.
    let p1_canonical = std::fs::canonicalize(&p1).unwrap();
    ed.execute_typed("b", Some("2")).unwrap();
    assert_eq!(
        ed.doc().path.as_ref().map(|p| p.as_path()),
        Some(p1_canonical.as_path()),
        ":b 2 must switch to the 2nd buffer in open-order"
    );
}

#[test]
fn buffer_index_zero_errors() {
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed.execute_typed("b", Some("0")).unwrap_err();
    assert!(
        err.to_string().contains("index"),
        "must mention 'index', got: {err}"
    );
}

#[test]
fn buffer_index_out_of_range_errors() {
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed.execute_typed("b", Some("99")).unwrap_err();
    assert!(
        err.to_string().contains("99"),
        "must mention the bad index, got: {err}"
    );
}

// ── :b by full absolute path ──────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn buffer_full_path_switches() {
    let (p1, _t1) = temp_file("file1\n");
    let p1_canonical = std::fs::canonicalize(&p1).unwrap();
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();

    ed.execute_typed("b", Some(p1_canonical.to_str().unwrap()))
        .unwrap();
    assert_eq!(
        ed.doc().path.as_ref().map(|p| p.as_path()),
        Some(p1_canonical.as_path()),
        ":b <full-path> must switch to the correct buffer"
    );
}

#[test]
#[cfg(not(windows))]
fn buffer_full_path_not_open_errors() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let canonical = std::fs::canonicalize(tmp.path()).unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    // File exists on disk but is not open.
    let err = ed
        .execute_typed("b", Some(canonical.to_str().unwrap()))
        .unwrap_err();
    assert!(
        err.to_string().contains("not an open buffer"),
        "must say 'not an open buffer', got: {err}"
    );
}

// ── :b by exact basename ──────────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn buffer_exact_basename_switches() {
    let (p1, _t1) = temp_file("file1\n");
    let p1_canonical = std::fs::canonicalize(&p1).unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();

    let basename = p1.file_name().unwrap().to_str().unwrap();
    ed.execute_typed("b", Some(basename)).unwrap();
    assert_eq!(
        ed.doc().path.as_ref().map(|p| p.as_path()),
        Some(p1_canonical.as_path()),
        ":b <exact-basename> must switch to that buffer"
    );
}

#[test]
#[cfg(not(windows))]
fn buffer_exact_basename_ambiguous_errors() {
    // Open two files whose basenames are identical (different dirs).
    let dir1 = tempfile::tempdir().unwrap();
    let dir2 = tempfile::tempdir().unwrap();
    let p1 = dir1.path().join("same.txt");
    let p2 = dir2.path().join("same.txt");
    std::fs::write(&p1, "a\n").unwrap();
    std::fs::write(&p2, "b\n").unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed(
        "e",
        Some(std::fs::canonicalize(&p1).unwrap().to_str().unwrap()),
    )
    .unwrap();
    ed.execute_typed(
        "e",
        Some(std::fs::canonicalize(&p2).unwrap().to_str().unwrap()),
    )
    .unwrap();

    let err = ed.execute_typed("b", Some("same.txt")).unwrap_err();
    assert!(
        err.to_string().contains("ambiguous"),
        "duplicate basenames must error 'ambiguous', got: {err}"
    );
}

// ── :b by basename prefix ─────────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn buffer_prefix_unique_switches() {
    // Use a controlled filename — `tempfile::NamedTempFile` produces random
    // basenames we can't match a prefix against.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prefixed_file.rs");
    std::fs::write(&path, "x\n").unwrap();
    let canonical = std::fs::canonicalize(&path).unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(canonical.to_str().unwrap()))
        .unwrap();
    // "prefixed" is a unique prefix
    ed.execute_typed("b", Some("prefixed")).unwrap();
    assert_eq!(
        ed.doc().path.as_ref().map(|p| p.as_path()),
        Some(canonical.as_path()),
        ":b <prefix> must switch to the uniquely-matched buffer"
    );
}

#[test]
#[cfg(not(windows))]
fn buffer_prefix_ambiguous_errors() {
    let dir = tempfile::tempdir().unwrap();
    let p1 = dir.path().join("alpha_a.rs");
    let p2 = dir.path().join("alpha_b.rs");
    std::fs::write(&p1, "a\n").unwrap();
    std::fs::write(&p2, "b\n").unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed(
        "e",
        Some(std::fs::canonicalize(&p1).unwrap().to_str().unwrap()),
    )
    .unwrap();
    ed.execute_typed(
        "e",
        Some(std::fs::canonicalize(&p2).unwrap().to_str().unwrap()),
    )
    .unwrap();

    let err = ed.execute_typed("b", Some("alpha")).unwrap_err();
    assert!(
        err.to_string().contains("ambiguous"),
        "ambiguous prefix must error, got: {err}"
    );
}

#[test]
fn buffer_no_match_errors() {
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed
        .execute_typed("b", Some("definitely_not_a_buffer_xyz"))
        .unwrap_err();
    assert!(
        err.to_string().contains("no buffer matching"),
        "must say 'no buffer matching', got: {err}"
    );
}

// ── :b scratch buffer ─────────────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn buffer_scratch_literal_switches_back() {
    let (p1, _t1) = temp_file("file1\n");
    let mut ed = editor_from("-[h]>ello\n");
    // Open a file; scratch buffer is now the alternate.
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    assert!(ed.doc().path.is_some(), "must be on the file buffer now");

    ed.execute_typed("b", Some("*scratch*")).unwrap();
    assert!(
        ed.doc().path.is_none(),
        ":b *scratch* must switch to the unnamed scratch buffer"
    );
}

// ── :b current buffer is a no-op ─────────────────────────────────────────────

#[test]
fn buffer_current_buffer_is_noop() {
    let mut ed = editor_from("-[h]>ello\n");
    // The only buffer is the scratch buffer; :b *scratch* should be a no-op.
    let before_id = ed.focused_buffer_id();
    ed.execute_typed("b", Some("*scratch*")).unwrap();
    assert_eq!(
        ed.focused_buffer_id(),
        before_id,
        ":b to current buffer must not change focus"
    );
}

// ── :b and :buffer aliases ────────────────────────────────────────────────────

#[test]
fn buffer_long_alias_accepted() {
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed
        .execute_typed("buffer", Some("xyz_no_such_buf"))
        .unwrap_err();
    assert!(
        err.to_string().contains("no buffer matching"),
        "canonical name 'buffer' must work too, got: {err}"
    );
}

#[test]
fn buffer_bang_force_is_ignored() {
    // `:b` takes a `force` flag for syntactic compatibility with the
    // `<cmd>!` convention, but there is nothing to force on a plain
    // buffer switch — `:b!` must behave identically to `:b`.
    let mut ed = editor_from("-[h]>ello\n");
    let before_id = ed.focused_buffer_id();
    ed.execute_typed("b!", Some("*scratch*")).unwrap();
    assert_eq!(
        ed.focused_buffer_id(),
        before_id,
        ":b! to current buffer must be a no-op, same as :b"
    );
    let err = ed.execute_typed("b!", Some("xyz_no_such_buf")).unwrap_err();
    assert!(
        err.to_string().contains("no buffer matching"),
        ":b! must still report resolution errors, got: {err}"
    );
}

// ── :b on a buffer whose backing file has been deleted ───────────────────────

#[test]
#[cfg(not(windows))]
fn buffer_switch_to_deleted_file_by_path() {
    let (p1, t1) = temp_file("file1\n");
    let canonical = std::fs::canonicalize(&p1).unwrap();
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    // Delete p1 from disk while its buffer stays open.
    drop(t1);
    assert!(!canonical.exists(), "precondition: file must be gone");

    ed.execute_typed("b", Some(canonical.to_str().unwrap()))
        .unwrap();
    assert_eq!(
        ed.doc().path.as_ref().map(|p| p.as_path()),
        Some(canonical.as_path()),
        ":b <deleted-path> must still switch to the open buffer"
    );
    assert!(
        ed.status_msg
            .as_deref()
            .is_some_and(|m| m.contains("no longer exists")),
        "must warn that the file is gone, got: {:?}",
        ed.status_msg.as_deref()
    );
}

#[test]
#[cfg(not(windows))]
fn buffer_switch_to_deleted_file_by_basename() {
    let (p1, t1) = temp_file("file1\n");
    let canonical = std::fs::canonicalize(&p1).unwrap();
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    drop(t1);

    let basename = canonical.file_name().unwrap().to_str().unwrap();
    ed.execute_typed("b", Some(basename)).unwrap();
    assert_eq!(
        ed.doc().path.as_ref().map(|p| p.as_path()),
        Some(canonical.as_path()),
        ":b <basename> must switch even when the file is deleted"
    );
    assert!(
        ed.status_msg
            .as_deref()
            .is_some_and(|m| m.contains("no longer exists")),
        "must warn that the file is gone, got: {:?}",
        ed.status_msg.as_deref()
    );
}

#[test]
#[cfg(not(windows))]
fn buffer_switch_to_live_file_no_warning() {
    let (p1, _t1) = temp_file("file1\n");
    let canonical = std::fs::canonicalize(&p1).unwrap();
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();

    ed.execute_typed("b", Some(canonical.to_str().unwrap()))
        .unwrap();
    assert!(
        !ed.status_msg
            .as_deref()
            .is_some_and(|m| m.contains("no longer exists")),
        ":b on a live file must not warn 'no longer exists', got: {:?}",
        ed.status_msg.as_deref()
    );
}

// ── Ctrl+O restores position after :b ────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn buffer_switch_pushes_jump() {
    let (p1, _t1) = temp_file("file1\n");
    let canonical = std::fs::canonicalize(&p1).unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    // Switch back to scratch via :b *scratch*.
    ed.execute_typed("b", Some("*scratch*")).unwrap();
    assert!(ed.doc().path.is_none(), "must be on scratch now");
    // Ctrl+O should bring us back to p1.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(
        ed.doc().path.as_ref().map(|p| p.as_path()),
        Some(canonical.as_path()),
        "Ctrl+O must restore the buffer we jumped from"
    );
}
