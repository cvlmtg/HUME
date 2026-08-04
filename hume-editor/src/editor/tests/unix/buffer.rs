use super::*;

#[test]
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
        ed.doc().path(),
        Some(p1_canonical.as_path()),
        ":b 2 must switch to the 2nd buffer in open-order"
    );
}

#[test]
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
        ed.doc().path(),
        Some(p1_canonical.as_path()),
        ":b <full-path> must switch to the correct buffer"
    );
}

#[test]
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

#[test]
fn buffer_exact_basename_switches() {
    let (p1, _t1) = temp_file("file1\n");
    let p1_canonical = std::fs::canonicalize(&p1).unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();

    let basename = p1.file_name().unwrap().to_str().unwrap();
    ed.execute_typed("b", Some(basename)).unwrap();
    assert_eq!(
        ed.doc().path(),
        Some(p1_canonical.as_path()),
        ":b <exact-basename> must switch to that buffer"
    );
}

#[test]
fn buffer_exact_basename_ambiguous_errors() {
    // Open two files whose basenames are identical (different dirs).
    let dir1 = safe_tempdir();
    let dir2 = safe_tempdir();
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

#[test]
fn buffer_prefix_unique_switches() {
    // Use a controlled filename — `tempfile::NamedTempFile` produces random
    // basenames we can't match a prefix against.
    let dir = safe_tempdir();
    let path = dir.path().join("prefixed_file.rs");
    std::fs::write(&path, "x\n").unwrap();
    let canonical = std::fs::canonicalize(&path).unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(canonical.to_str().unwrap()))
        .unwrap();
    // "prefixed" is a unique prefix
    ed.execute_typed("b", Some("prefixed")).unwrap();
    assert_eq!(
        ed.doc().path(),
        Some(canonical.as_path()),
        ":b <prefix> must switch to the uniquely-matched buffer"
    );
}

#[test]
fn buffer_prefix_ambiguous_errors() {
    let dir = safe_tempdir();
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

/// Creates `$HOME/<name>` for a `~`-expansion test, removing it on drop.
/// `hume_platform::path::expand`'s `~` resolves against the real `$HOME` (no
/// injection point at this level), so exercising it means touching the real
/// one; each caller picks a distinct name, so parallel tests never collide,
/// and cleanup is guaranteed even if the test panics.
struct HomeScratchDir(std::path::PathBuf);

impl HomeScratchDir {
    fn new(name: &str) -> Self {
        let home = hume_platform::dirs::home_dir().expect("HOME must be set for this test");
        let dir = home.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for HomeScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn buffer_tilde_path_switches() {
    let dir = HomeScratchDir::new(".hume_test_tilde_switch");
    let path = dir.path().join("file.rs");
    std::fs::write(&path, "x\n").unwrap();
    let canonical = std::fs::canonicalize(&path).unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(path.to_str().unwrap())).unwrap();

    ed.execute_typed("b", Some("~/.hume_test_tilde_switch/file.rs"))
        .unwrap();
    assert_eq!(
        ed.doc().path(),
        Some(canonical.as_path()),
        ":b ~/<path> must switch to the buffer opened at that path"
    );
}

/// Regression test: `resolve_buffer_arg`'s ambiguity labels are built from
/// `display_path`, which `~`-collapses paths under `$HOME` — retyping the
/// exact label shown must resolve, not error again with "no buffer matching".
///
/// Fail oracle: drop the `hume_platform::path::expand` call from
/// `resolve_buffer_arg`'s absolute-path branch — `Path::is_absolute()` on a
/// literal `~/...` string is `false`, so the retype falls through to the
/// basename/prefix branches (which compare against the bare basename, not
/// the full label) and errors "no buffer matching".
#[test]
fn buffer_ambiguous_label_is_retypeable() {
    let dir1 = HomeScratchDir::new(".hume_test_tilde_p1");
    let dir2 = HomeScratchDir::new(".hume_test_tilde_p2");
    let p1 = dir1.path().join("same.txt");
    let p2 = dir2.path().join("same.txt");
    std::fs::write(&p1, "a\n").unwrap();
    std::fs::write(&p2, "b\n").unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();

    let err = ed
        .execute_typed("b", Some("same.txt"))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains('~'),
        "expected ~-collapsed labels in the ambiguity message, got: {err}"
    );
    let labels = err
        .split_once(": ")
        .map(|(_, rest)| rest)
        .expect("message must list labels");
    let first_label = labels.split(", ").next().unwrap();

    ed.execute_typed("b", Some(first_label))
        .unwrap_or_else(|e| {
            panic!("retyping the ambiguity label {first_label:?} must resolve, got: {e}")
        });
}

#[test]
fn buffer_scratch_literal_switches_back() {
    let (p1, _t1) = temp_file("file1\n");
    let mut ed = editor_from("-[h]>ello\n");
    // Open a file; scratch buffer is now the alternate.
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    assert!(ed.doc().path().is_some(), "must be on the file buffer now");

    ed.execute_typed("b", Some("*scratch*")).unwrap();
    assert!(
        ed.doc().path().is_none(),
        ":b *scratch* must switch to the unnamed scratch buffer"
    );
}

#[test]
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

    // Via `type_cmd_event`, not `execute_typed`: a *moving* `:b` only
    // switches inside `enter_buffer_with_jump` and relies on
    // `Editor::handle_event`'s tail check for the disk check itself.
    type_cmd_event(&mut ed, &format!(":b {}", canonical.display()));
    assert_eq!(
        ed.doc().path(),
        Some(canonical.as_path()),
        ":b <deleted-path> must still switch to the open buffer"
    );
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("no longer exists")),
        "must warn that the file is gone, got: {:?}",
        ed.state.status_msg.as_deref()
    );
}

#[test]
fn buffer_switch_to_deleted_file_by_basename() {
    let (p1, t1) = temp_file("file1\n");
    let canonical = std::fs::canonicalize(&p1).unwrap();
    let (p2, _t2) = temp_file("file2\n");
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    ed.execute_typed("e", Some(p2.to_str().unwrap())).unwrap();
    drop(t1);

    let basename = canonical.file_name().unwrap().to_str().unwrap();
    // Via `type_cmd_event`, not `execute_typed`: see the comment in
    // `buffer_switch_to_deleted_file_by_path`.
    type_cmd_event(&mut ed, &format!(":b {basename}"));
    assert_eq!(
        ed.doc().path(),
        Some(canonical.as_path()),
        ":b <basename> must switch even when the file is deleted"
    );
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("no longer exists")),
        "must warn that the file is gone, got: {:?}",
        ed.state.status_msg.as_deref()
    );
}

#[test]
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
        !ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("no longer exists")),
        ":b on a live file must not warn 'no longer exists', got: {:?}",
        ed.state.status_msg.as_deref()
    );
}

#[test]
fn buffer_switch_pushes_jump() {
    let (p1, _t1) = temp_file("file1\n");
    let canonical = std::fs::canonicalize(&p1).unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("e", Some(p1.to_str().unwrap())).unwrap();
    // Switch back to scratch via :b *scratch*.
    ed.execute_typed("b", Some("*scratch*")).unwrap();
    assert!(ed.doc().path().is_none(), "must be on scratch now");
    // Ctrl+O should bring us back to p1.
    ed.handle_key(key_ctrl('o'));
    assert_eq!(
        ed.doc().path(),
        Some(canonical.as_path()),
        "Ctrl+O must restore the buffer we jumped from"
    );
}
