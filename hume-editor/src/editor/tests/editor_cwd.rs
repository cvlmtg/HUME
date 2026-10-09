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
