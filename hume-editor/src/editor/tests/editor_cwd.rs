// Portable half of the editor-cwd tests; the tests that spawn programs by
// name live in `unix/editor_cwd.rs`.

use std::path::Path;

use super::*;

pub(in crate::editor::tests) fn call(ed: &mut Editor, name: &str) {
    ed.execute_keymap_command(name.to_string().into(), None, false);
}

pub(in crate::editor::tests) fn canonical(dir: &Path) -> PathBuf {
    std::fs::canonicalize(dir).unwrap()
}

/// An editor whose `state.cwd` is `dir` while the process cwd stays where the
/// test binary started.
pub(in crate::editor::tests) fn editor_in(dir: &Path) -> Editor {
    let mut ed = editor_from("-[a]>bc\n");
    ed.state.cwd = canonical(dir);
    ed
}

pub(in crate::editor::tests) fn relative_display_path(ed: &mut Editor, tmp: &Path) -> String {
    log_probe(ed, tmp, "(buffer-display-path bid #:relative #t)")
}

#[test]
fn typed_cd_resolves_a_relative_argument_against_the_editor_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    let mut ed = editor_in(tmp.path());

    type_cmd(&mut ed, ":cd sub");

    assert_eq!(ed.state.cwd, canonical(&tmp.path().join("sub")));
}

#[test]
fn set_cwd_builtin_moves_the_editor_cwd_like_cd() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    let mut ed = editor_in(tmp.path());
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda () (set-cwd! (path-join (cwd) "sub"))))"#,
    );

    call(&mut ed, "go");

    assert_eq!(ed.state.cwd, canonical(&tmp.path().join("sub")));
}

#[test]
fn relative_display_path_is_relative_to_the_editor_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    std::fs::write(tmp.path().join("sub").join("f.rs"), "x\n").unwrap();
    let mut ed = editor_in(tmp.path());
    type_cmd(&mut ed, ":e sub/f.rs");

    let shown = relative_display_path(&mut ed, tmp.path());

    assert_eq!(shown, Path::new("sub").join("f.rs").display().to_string());
}

#[test]
fn relative_display_path_outside_the_cwd_is_the_display_path() {
    let cwd = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let file = elsewhere.path().join("f.rs");
    std::fs::write(&file, "x\n").unwrap();
    let mut ed = editor_in(cwd.path());
    type_cmd(&mut ed, &format!(":e {}", file.display()));
    let display = ed.doc().display_path().expect("a file buffer").to_string();

    let shown = relative_display_path(&mut ed, cwd.path());

    assert_eq!(shown, display);
}

#[test]
fn display_path_without_the_keyword_stays_absolute() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("f.rs"), "x\n").unwrap();
    let mut ed = editor_in(tmp.path());
    type_cmd(&mut ed, ":e f.rs");
    let display = ed.doc().display_path().expect("a file buffer").to_string();

    let shown = log_probe(&mut ed, tmp.path(), "(buffer-display-path bid)");

    assert_eq!(shown, display);
    assert_ne!(shown, "f.rs");
}

#[test]
fn relative_display_path_of_a_scratch_buffer_is_false() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_in(tmp.path());

    let shown = log_probe(
        &mut ed,
        tmp.path(),
        r#"(if (buffer-display-path bid #:relative #t) "a path" "none")"#,
    );

    assert_eq!(shown, "none");
}
