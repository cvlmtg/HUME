use super::*;

use pretty_assertions::assert_eq;

// ── set_cwd ───────────────────────────────────────────────────────────────────

#[test]
fn set_cwd_updates_editor_and_process_cwd() {
    let cwd = CwdSandbox::new();
    let canonical = cwd.path();
    let mut ed = editor_from("-[h]>ello\n");

    ed.set_cwd(&canonical).unwrap();

    assert_eq!(
        ed.state.cwd, canonical,
        "editor.cwd must match the target dir"
    );
    assert_eq!(
        std::env::current_dir().unwrap(),
        canonical,
        "process cwd must match the target dir"
    );
}

#[test]
fn set_cwd_rejects_non_directory() {
    let _guard = CwdGuard::new();
    let file = tempfile::NamedTempFile::new().unwrap();
    let canonical = std::fs::canonicalize(file.path()).unwrap();
    let before = std::env::current_dir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    let before_editor = ed.state.cwd.clone();

    let err = ed.set_cwd(&canonical).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotADirectory);
    // cwd must be unchanged on failure
    assert_eq!(
        ed.state.cwd, before_editor,
        "editor.cwd must not change on error"
    );
    assert_eq!(
        std::env::current_dir().unwrap(),
        before,
        "process cwd must not change on error"
    );
}

#[test]
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
fn typed_cd_absolute_path() {
    let cwd = CwdSandbox::new();
    let canonical = cwd.path();
    let mut ed = editor_from("-[h]>ello\n");

    ed.execute_typed("cd", Some(canonical.to_str().unwrap()))
        .unwrap();

    assert_eq!(ed.state.cwd, canonical);
    assert_eq!(std::env::current_dir().unwrap(), canonical);
}

#[test]
fn typed_cd_relative_path() {
    let cwd = CwdSandbox::new();
    // Create a subdirectory inside the sandboxed tempdir.
    let child = cwd.raw().join("subdir");
    std::fs::create_dir(&child).unwrap();
    let canonical_parent = cwd.path();
    let canonical_child = std::fs::canonicalize(&child).unwrap();

    let mut ed = editor_from("-[h]>ello\n");
    // Set the editor (and process) cwd to the parent first.
    ed.set_cwd(&canonical_parent).unwrap();

    // Now :cd to the relative name "subdir".
    ed.execute_typed("cd", Some("subdir")).unwrap();

    assert_eq!(
        ed.state.cwd, canonical_child,
        "relative :cd must resolve against editor.cwd"
    );
    assert_eq!(std::env::current_dir().unwrap(), canonical_child);
}

#[test]
fn typed_cd_no_arg_goes_home() {
    let _guard = CwdGuard::new();
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
    let _guard = CwdGuard::new();
    let home = hume_platform::dirs::home_dir().expect("HOME must be set for this test");
    let canonical_home = std::fs::canonicalize(&home).unwrap();
    let mut ed = editor_from("-[h]>ello\n");

    ed.execute_typed("cd", Some("~")).unwrap();

    assert_eq!(ed.state.cwd, canonical_home, ":cd ~ must expand to $HOME");
}

#[test]
fn typed_cd_error_on_nonexistent() {
    let _guard = CwdGuard::new();
    let before = std::env::current_dir().unwrap();
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
    assert_eq!(
        std::env::current_dir().unwrap(),
        before,
        "process cwd must be unchanged on error"
    );
}

#[test]
fn typed_cd_error_on_file_path() {
    let _guard = CwdGuard::new();
    let file = tempfile::NamedTempFile::new().unwrap();
    let canonical = std::fs::canonicalize(file.path()).unwrap();
    let before = std::env::current_dir().unwrap();
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
    assert_eq!(
        std::env::current_dir().unwrap(),
        before,
        "process cwd must be unchanged on file target"
    );
}

#[test]
fn typed_cd_alias_works() {
    let cwd = CwdSandbox::new();
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
    let cwd = CwdSandbox::new();

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
    let cwd = CwdSandbox::new();
    let canonical = cwd.path();
    let mut ed = editor_from("-[h]>ello\n");
    ed.set_cwd(&canonical).unwrap();

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
    let cwd = CwdSandbox::new();
    let canonical = cwd.path();
    let mut ed = editor_from("-[h]>ello\n");
    ed.set_cwd(&canonical).unwrap();

    ed.execute_typed("print-working-directory", None).unwrap();

    let msg = ed
        .state
        .status_msg
        .as_deref()
        .expect(":print-working-directory must report a message");
    let expected = hume_platform::path::display_form(&canonical);
    assert_eq!(msg, expected, "long alias must match :pwd output");
}

// ── PathCompleter dirs_only ───────────────────────────────────────────────────

#[test]
fn path_completer_dirs_only_mode() {
    use crate::editor::completion::{Completer, CompletionCtx, PathCompleter};

    let dir = safe_tempdir();
    let subdir = dir.path().join("mysubdir");
    let file = dir.path().join("myfile.txt");
    std::fs::create_dir(&subdir).unwrap();
    std::fs::write(&file, "x\n").unwrap();

    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    let registry = crate::editor::registry::CommandRegistry::with_defaults();
    let buffers = crate::editor::buffer::store::BufferStore::new();
    let languages = hume_treesitter::registry::LanguageRegistry::new();
    let ctx = CompletionCtx {
        registry: &registry,
        buffers: &buffers,
        cwd: &canonical,
        languages: &languages,
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

// ── CwdSandbox teardown ordering ───────────────────────────────────────────────

/// Basic mechanics, checked entirely while `CwdSandbox` (and thus `CWD_MUTEX`)
/// is held, so nothing else can mutate cwd mid-check: cd into the sandbox's
/// tempdir, confirm cwd matches it, then drop and confirm the tempdir is
/// actually gone from disk.
///
/// Deliberately does NOT re-read `std::env::current_dir()` after `cwd` drops
/// and releases `CWD_MUTEX` — cwd is process-global, so any other
/// CWD-mutating test could legitimately acquire the lock and change it before
/// the next line ran, making such a check racy against unrelated tests, not a
/// signal about this sandbox's correctness.
#[test]
fn cwd_sandbox_restores_cwd_and_deletes_tempdir() {
    let cwd = CwdSandbox::new();
    let raw = cwd.raw().to_path_buf();

    std::env::set_current_dir(&raw).unwrap();
    assert_eq!(
        std::env::current_dir().unwrap(),
        std::fs::canonicalize(&raw).unwrap(),
        "cwd must be the sandboxed tempdir while the sandbox is held"
    );

    drop(cwd);

    assert!(
        !raw.exists(),
        "tempdir must be deleted once CwdSandbox drops"
    );
}

/// Stress regression guard for the actual historical bug: a background thread
/// polls `std::env::current_dir()` in a tight loop while the main thread
/// repeatedly opens and tears down `CwdSandbox`es. `current_dir()` only
/// errors when cwd points at a deleted directory, so this asserts the
/// property a single after-the-fact check can't: cwd is never left dangling
/// even under concurrent reads, for the whole lifetime of every sandbox.
///
/// Fail oracle: swap `CwdSandbox` here for the historical buggy pattern
/// (`CwdGuard::new()` + a separately-scoped `tempfile::tempdir()` local) and
/// the reader thread reliably observes `current_dir()` failing mid-loop.
#[test]
fn cwd_sandbox_never_dangles_under_concurrent_reads() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let stop = Arc::new(AtomicBool::new(false));
    let saw_error = Arc::new(AtomicBool::new(false));

    let reader = {
        let stop = Arc::clone(&stop);
        let saw_error = Arc::clone(&saw_error);
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if std::env::current_dir().is_err() {
                    saw_error.store(true, Ordering::Relaxed);
                }
            }
        })
    };

    for _ in 0..200 {
        let cwd = CwdSandbox::new();
        std::env::set_current_dir(cwd.raw()).unwrap();
    }

    stop.store(true, Ordering::Relaxed);
    reader.join().expect("reader thread must not panic");

    assert!(
        !saw_error.load(Ordering::Relaxed),
        "cwd must never dangle while any CwdSandbox is in use"
    );
}
