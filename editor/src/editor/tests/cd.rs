use super::*;

use pretty_assertions::assert_eq;

// ── set_cwd ───────────────────────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn set_cwd_updates_editor_and_process_cwd() {
    let _guard = CwdGuard::new();
    let dir = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    let mut ed = editor_from("-[h]>ello\n");

    ed.set_cwd(&canonical).unwrap();

    assert_eq!(ed.cwd, canonical, "editor.cwd must match the target dir");
    assert_eq!(
        std::env::current_dir().unwrap(),
        canonical,
        "process cwd must match the target dir"
    );
}

#[test]
#[cfg(not(windows))]
fn set_cwd_rejects_non_directory() {
    let _guard = CwdGuard::new();
    let file = tempfile::NamedTempFile::new().unwrap();
    let canonical = std::fs::canonicalize(file.path()).unwrap();
    let before = std::env::current_dir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    let before_editor = ed.cwd.clone();

    let err = ed.set_cwd(&canonical).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotADirectory);
    // cwd must be unchanged on failure
    assert_eq!(ed.cwd, before_editor, "editor.cwd must not change on error");
    assert_eq!(
        std::env::current_dir().unwrap(),
        before,
        "process cwd must not change on error"
    );
}

#[test]
#[cfg(not(windows))]
fn set_cwd_rejects_nonexistent_path() {
    let _guard = CwdGuard::new();
    let mut ed = editor_from("-[h]>ello\n");

    let err = ed
        .set_cwd(std::path::Path::new("/definitely/not/a/real/path/xyz123"))
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}

// ── :cd typed command ─────────────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn typed_cd_absolute_path() {
    let _guard = CwdGuard::new();
    let dir = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    let mut ed = editor_from("-[h]>ello\n");

    ed.execute_typed("cd", Some(canonical.to_str().unwrap()))
        .unwrap();

    assert_eq!(ed.cwd, canonical);
    assert_eq!(std::env::current_dir().unwrap(), canonical);
}

#[test]
#[cfg(not(windows))]
fn typed_cd_relative_path() {
    let _guard = CwdGuard::new();
    // Create a tempdir containing a subdirectory.
    let parent = tempfile::tempdir().unwrap();
    let child = parent.path().join("subdir");
    std::fs::create_dir(&child).unwrap();
    let canonical_parent = std::fs::canonicalize(parent.path()).unwrap();
    let canonical_child = std::fs::canonicalize(&child).unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    // Set the editor (and process) cwd to the parent first.
    ed.set_cwd(&canonical_parent).unwrap();

    // Now :cd to the relative name "subdir".
    ed.execute_typed("cd", Some("subdir")).unwrap();

    assert_eq!(
        ed.cwd, canonical_child,
        "relative :cd must resolve against editor.cwd"
    );
    assert_eq!(std::env::current_dir().unwrap(), canonical_child);
}

#[test]
#[cfg(not(windows))]
fn typed_cd_no_arg_goes_home() {
    let _guard = CwdGuard::new();
    let home = crate::os::dirs::home_dir().expect("HOME must be set for this test");
    let canonical_home = std::fs::canonicalize(&home).unwrap();
    let mut ed = editor_from("-[h]>ello\n");

    ed.execute_typed("cd", None).unwrap();

    assert_eq!(ed.cwd, canonical_home, ":cd with no arg must go to $HOME");
}

#[test]
#[cfg(not(windows))]
fn typed_cd_tilde_expands_to_home() {
    let _guard = CwdGuard::new();
    let home = crate::os::dirs::home_dir().expect("HOME must be set for this test");
    let canonical_home = std::fs::canonicalize(&home).unwrap();
    let mut ed = editor_from("-[h]>ello\n");

    ed.execute_typed("cd", Some("~")).unwrap();

    assert_eq!(ed.cwd, canonical_home, ":cd ~ must expand to $HOME");
}

#[test]
#[cfg(not(windows))]
fn typed_cd_error_on_nonexistent() {
    let _guard = CwdGuard::new();
    let before = std::env::current_dir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    let before_editor = ed.cwd.clone();

    let err = ed
        .execute_typed("cd", Some("/definitely/not/a/real/path/xyz123"))
        .unwrap_err();
    assert!(
        err.to_string().contains("xyz123"),
        "path must appear in error message, got: {err}"
    );
    assert_eq!(
        ed.cwd, before_editor,
        "editor.cwd must be unchanged on error"
    );
    assert_eq!(
        std::env::current_dir().unwrap(),
        before,
        "process cwd must be unchanged on error"
    );
}

#[test]
#[cfg(not(windows))]
fn typed_cd_error_on_file_path() {
    let _guard = CwdGuard::new();
    let file = tempfile::NamedTempFile::new().unwrap();
    let canonical = std::fs::canonicalize(file.path()).unwrap();
    let before = std::env::current_dir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    let before_editor = ed.cwd.clone();

    let err = ed
        .execute_typed("cd", Some(canonical.to_str().unwrap()))
        .unwrap_err();
    assert!(
        err.to_string().contains("not a directory"),
        "expected not-a-directory, got: {err}"
    );
    assert_eq!(
        ed.cwd, before_editor,
        "editor.cwd must be unchanged on file target"
    );
    assert_eq!(
        std::env::current_dir().unwrap(),
        before,
        "process cwd must be unchanged on file target"
    );
}

#[test]
#[cfg(not(windows))]
fn typed_cd_alias_works() {
    let _guard = CwdGuard::new();
    let dir = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    let mut ed = editor_from("-[h]>ello\n");

    // Both the canonical name and the `cd` alias must work.
    ed.execute_typed("change-directory", Some(canonical.to_str().unwrap()))
        .unwrap();
    assert_eq!(ed.cwd, canonical);
}

// ── :cd then :e uses new cwd ──────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn cd_then_edit_resolves_relative_to_new_cwd() {
    let _guard = CwdGuard::new();

    // Create a tempdir with a file in it.
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("myfile.txt");
    std::fs::write(&file_path, "hello\n").unwrap();
    let canonical_dir = std::fs::canonicalize(dir.path()).unwrap();
    let canonical_file = std::fs::canonicalize(&file_path).unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("cd", Some(canonical_dir.to_str().unwrap()))
        .unwrap();
    ed.execute_typed("e", Some("myfile.txt")).unwrap();

    let open_path = ed
        .doc()
        .path()
        .expect("opened file must have a path");
    assert_eq!(
        open_path,
        canonical_file.as_path(),
        ":e after :cd must open the file in the new cwd"
    );
}

// ── :pwd typed command ────────────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn typed_pwd_reports_current_directory() {
    let _guard = CwdGuard::new();
    let dir = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    ed.set_cwd(&canonical).unwrap();

    ed.execute_typed("pwd", None).unwrap();

    let msg = ed
        .status_msg
        .as_deref()
        .expect(":pwd must report a message");
    let expected = crate::os::path::shorten_home(&canonical);
    assert_eq!(msg, expected, ":pwd must report shorten_home(cwd)");
}

#[test]
#[cfg(not(windows))]
fn typed_pwd_long_alias_works() {
    let _guard = CwdGuard::new();
    let dir = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    ed.set_cwd(&canonical).unwrap();

    ed.execute_typed("print-working-directory", None).unwrap();

    let msg = ed
        .status_msg
        .as_deref()
        .expect(":print-working-directory must report a message");
    let expected = crate::os::path::shorten_home(&canonical);
    assert_eq!(msg, expected, "long alias must match :pwd output");
}

// ── PathCompleter dirs_only ───────────────────────────────────────────────────

#[test]
#[cfg(not(windows))]
fn path_completer_dirs_only_mode() {
    use crate::editor::completion::{Completer, CompletionCtx, PathCompleter};

    let dir = tempfile::tempdir().unwrap();
    let subdir = dir.path().join("mysubdir");
    let file = dir.path().join("myfile.txt");
    std::fs::create_dir(&subdir).unwrap();
    std::fs::write(&file, "x\n").unwrap();

    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    let registry = crate::editor::registry::CommandRegistry::with_defaults();
    let buffers = crate::editor::buffer_store::BufferStore::new();
    let ctx = CompletionCtx {
        registry: &registry,
        buffers: &buffers,
        cwd: &canonical,
    };

    // dirs_only: true — files must be excluded.
    let dirs = PathCompleter { dirs_only: true }.complete("cd m", 4, &ctx);
    let dir_names: Vec<&str> = dirs.candidates.iter().map(|c| c.display.as_str()).collect();
    assert!(
        dir_names.contains(&"mysubdir/"),
        "dirs_only must include subdirectory"
    );
    assert!(
        !dir_names.contains(&"myfile.txt"),
        "dirs_only must exclude files"
    );

    // dirs_only: false — both dirs and files must appear.
    let all = PathCompleter { dirs_only: false }.complete("e m", 3, &ctx);
    let all_names: Vec<&str> = all.candidates.iter().map(|c| c.display.as_str()).collect();
    assert!(
        all_names.contains(&"mysubdir/"),
        "dirs_only=false must include subdirectory"
    );
    assert!(
        all_names.contains(&"myfile.txt"),
        "dirs_only=false must include files"
    );
}
