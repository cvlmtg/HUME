use super::*;

use pretty_assertions::assert_eq;

// ── set_cwd ───────────────────────────────────────────────────────────────────

#[test]
fn set_cwd_updates_editor_cwd() {
    let cwd = Sandbox::new();
    let canonical = cwd.path();
    let mut ed = editor_from("-[h]>ello\n");

    ed.state.set_cwd(&canonical).unwrap();

    assert_eq!(
        ed.state.cwd, canonical,
        "editor.cwd must match the target dir"
    );
}

#[test]
fn set_cwd_rejects_non_directory() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let canonical = std::fs::canonicalize(file.path()).unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    let before_editor = ed.state.cwd.clone();

    let err = ed.state.set_cwd(&canonical).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotADirectory);
    // cwd must be unchanged on failure
    assert_eq!(
        ed.state.cwd, before_editor,
        "editor.cwd must not change on error"
    );
}

#[test]
fn set_cwd_rejects_nonexistent_path() {
    let mut ed = editor_from("-[h]>ello\n");

    let err = ed
        .state
        .set_cwd(std::path::Path::new("/definitely/not/a/real/path/xyz123"))
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}

// ── :cd typed command ─────────────────────────────────────────────────────────

#[test]
fn typed_cd_absolute_path() {
    let cwd = Sandbox::new();
    let canonical = cwd.path();
    let mut ed = editor_from("-[h]>ello\n");

    ed.execute_typed("cd", Some(canonical.to_str().unwrap()))
        .unwrap();

    assert_eq!(ed.state.cwd, canonical);
}

#[test]
fn typed_cd_relative_path() {
    let cwd = Sandbox::new();
    // Create a subdirectory inside the sandboxed tempdir.
    let child = cwd.raw().join("subdir");
    std::fs::create_dir(&child).unwrap();
    let canonical_parent = cwd.path();
    let canonical_child = std::fs::canonicalize(&child).unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    // Set the editor cwd to the parent first.
    ed.state.set_cwd(&canonical_parent).unwrap();

    // Now :cd to the relative name "subdir".
    ed.execute_typed("cd", Some("subdir")).unwrap();

    assert_eq!(
        ed.state.cwd, canonical_child,
        "relative :cd must resolve against editor.cwd"
    );
}

#[test]
fn typed_cd_no_arg_goes_home() {
    let home = hume_platform::dirs::home_dir().expect("HOME must be set for this test");
    let canonical_home = std::fs::canonicalize(&home).unwrap();
    let mut ed = editor_from("-[h]>ello\n");

    ed.execute_typed("cd", None).unwrap();

    assert_eq!(
        ed.state.cwd, canonical_home,
        ":cd with no arg must go to $HOME"
    );
}

#[test]
fn typed_cd_tilde_expands_to_home() {
    let home = hume_platform::dirs::home_dir().expect("HOME must be set for this test");
    let canonical_home = std::fs::canonicalize(&home).unwrap();
    let mut ed = editor_from("-[h]>ello\n");

    ed.execute_typed("cd", Some("~")).unwrap();

    assert_eq!(ed.state.cwd, canonical_home, ":cd ~ must expand to $HOME");
}

#[test]
fn typed_cd_error_on_nonexistent() {
    let mut ed = editor_from("-[h]>ello\n");
    let before_editor = ed.state.cwd.clone();

    let err = ed
        .execute_typed("cd", Some("/definitely/not/a/real/path/xyz123"))
        .unwrap_err();
    assert!(
        err.to_string().contains("xyz123"),
        "path must appear in error message, got: {err}"
    );
    assert_eq!(
        ed.state.cwd, before_editor,
        "editor.cwd must be unchanged on error"
    );
}

#[test]
fn typed_cd_error_on_file_path() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let canonical = std::fs::canonicalize(file.path()).unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    let before_editor = ed.state.cwd.clone();

    let err = ed
        .execute_typed("cd", Some(canonical.to_str().unwrap()))
        .unwrap_err();
    assert!(
        err.to_string().contains("not a directory"),
        "expected not-a-directory, got: {err}"
    );
    assert_eq!(
        ed.state.cwd, before_editor,
        "editor.cwd must be unchanged on file target"
    );
}

#[test]
fn typed_cd_alias_works() {
    let cwd = Sandbox::new();
    let canonical = cwd.path();
    let mut ed = editor_from("-[h]>ello\n");

    // Both the canonical name and the `cd` alias must work.
    ed.execute_typed("change-directory", Some(canonical.to_str().unwrap()))
        .unwrap();
    assert_eq!(ed.state.cwd, canonical);
}

// ── :cd then :e uses new cwd ──────────────────────────────────────────────────

#[test]
fn cd_then_edit_resolves_relative_to_new_cwd() {
    let cwd = Sandbox::new();

    // Create a file inside the sandboxed tempdir.
    let file_path = cwd.raw().join("myfile.txt");
    std::fs::write(&file_path, "hello\n").unwrap();
    let canonical_dir = cwd.path();
    let canonical_file = std::fs::canonicalize(&file_path).unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("cd", Some(canonical_dir.to_str().unwrap()))
        .unwrap();
    ed.execute_typed("e", Some("myfile.txt")).unwrap();

    let open_path = ed.doc().path().expect("opened file must have a path");
    assert_eq!(
        open_path,
        canonical_file.as_path(),
        ":e after :cd must open the file in the new cwd"
    );
}

// ── :pwd typed command ────────────────────────────────────────────────────────

#[test]
fn typed_pwd_reports_current_directory() {
    let cwd = Sandbox::new();
    let canonical = cwd.path();
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.set_cwd(&canonical).unwrap();

    ed.execute_typed("pwd", None).unwrap();

    let msg = ed
        .state
        .status_msg
        .as_deref()
        .expect(":pwd must report a message");
    let expected = hume_platform::path::display_form(&canonical);
    assert_eq!(msg, expected, ":pwd must report display_form(cwd)");
}

#[test]
fn typed_pwd_long_alias_works() {
    let cwd = Sandbox::new();
    let canonical = cwd.path();
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.set_cwd(&canonical).unwrap();

    ed.execute_typed("print-working-directory", None).unwrap();

    let msg = ed
        .state
        .status_msg
        .as_deref()
        .expect(":print-working-directory must report a message");
    let expected = hume_platform::path::display_form(&canonical);
    assert_eq!(msg, expected, "long alias must match :pwd output");
}
