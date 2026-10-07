// The editor's working directory is `state.cwd`. Every consumer reads it, and
// the process cwd is never consulted or moved. Tests here set `state.cwd`
// directly so the two directories differ.

use std::path::Path;

use super::*;

fn call(ed: &mut Editor, name: &str) {
    ed.execute_keymap_command(name.to_string().into(), None, false);
}

fn canonical(dir: &Path) -> PathBuf {
    std::fs::canonicalize(dir).unwrap()
}

/// An editor whose `state.cwd` is `dir` while the process cwd stays where the
/// test binary started.
fn editor_in(dir: &Path) -> Editor {
    let mut ed = editor_from("-[a]>bc\n");
    ed.state.cwd = canonical(dir);
    ed
}

#[test]
fn set_cwd_leaves_the_process_cwd_alone() {
    let before = std::env::current_dir().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\n");

    ed.state.set_cwd(tmp.path()).unwrap();

    assert_eq!(ed.state.cwd, canonical(tmp.path()));
    assert_eq!(std::env::current_dir().unwrap(), before);
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
fn spawn_async_defaults_to_the_editor_cwd() {
    let _path = path_reader();
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_in(tmp.path());
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define-command! "go" "" (lambda ()
          (spawn-async! "pwd" '()
            (lambda (out err code) (log! 'info (trim out))))))
        "#,
    );
    call(&mut ed, "go");

    drain_until(&mut ed, |ed| ed.state.status_msg.is_some());

    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        canonical(tmp.path()).to_str().unwrap()
    );
}

#[test]
fn spawn_async_joins_a_relative_cwd_onto_the_editor_cwd() {
    let _path = path_reader();
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    let mut ed = editor_in(tmp.path());
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define-command! "go" "" (lambda ()
          (spawn-async! "pwd" '()
            (lambda (out err code) (log! 'info (trim out)))
            #:cwd "sub")))
        "#,
    );
    call(&mut ed, "go");

    drain_until(&mut ed, |ed| ed.state.status_msg.is_some());

    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        canonical(&tmp.path().join("sub")).to_str().unwrap()
    );
}

#[test]
fn run_capture_defaults_to_the_editor_cwd() {
    let _path = path_reader();
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_in(tmp.path());

    let out = log_probe(
        &mut ed,
        tmp.path(),
        r#"(trim (hash-ref (run-capture! "pwd" '()) 'stdout))"#,
    );

    assert_eq!(out, canonical(tmp.path()).to_str().unwrap());
}

#[test]
fn run_inline_output_defaults_to_the_editor_cwd() {
    let _path = path_reader();
    let tmp = tempfile::tempdir().unwrap();
    let record = tmp.path().join("pwd.txt");
    let mut ed = editor_in(tmp.path());
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda ()
                 (run-inline-output! "sh" (list "-c" "pwd > \"$1\"" "sh" {}))))"#,
            steel_path(&record)
        ),
    );

    call(&mut ed, "go");

    assert_eq!(
        std::fs::read_to_string(&record).unwrap().trim(),
        canonical(tmp.path()).to_str().unwrap()
    );
}

#[test]
fn picker_source_spawn_defaults_to_the_editor_cwd() {
    let _path = path_reader();
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_in(tmp.path());
    run(
        &mut ed,
        tmp.path(),
        r#"
        (define tok #f)
        (define-typed-command! "go" "" (lambda (pane)
          (set! tok (picker! pane '() (lambda (x) (void))))))
        (define-command! "spawn-it" "" (lambda ()
          (picker-source-spawn! tok "pwd" '())))
        "#,
    );
    type_cmd(&mut ed, ":go");
    call(&mut ed, "spawn-it");

    drain_until_picker_total(&mut ed, 1);

    let picker = ed.state.input.picker().unwrap();
    assert_eq!(
        picker.window(10).collect::<Vec<_>>(),
        vec![canonical(tmp.path()).to_str().unwrap()]
    );
}

#[test]
fn cwd_builtin_reports_the_editor_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_in(tmp.path());

    let out = log_probe(&mut ed, tmp.path(), "(cwd)");

    assert_eq!(out, canonical(tmp.path()).to_str().unwrap());
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
