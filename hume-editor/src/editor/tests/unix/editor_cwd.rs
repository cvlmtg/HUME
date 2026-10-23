// The editor's working directory is `state.cwd`. Every consumer reads it.
// Unit builds skip the process-cwd move in `set_cwd`, so tests set
// `state.cwd` directly and the two directories differ.

use super::*;
use crate::editor::tests::editor_cwd::{call, canonical, editor_in, relative_display_path};

#[test]
fn spawn_async_defaults_to_the_editor_cwd() {
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
fn current_directory_in_a_required_module_reports_the_editor_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    let module = tmp.path().join("where.scm");
    std::fs::write(
        &module,
        r#"(provide where) (define (where) (current-directory))"#,
    )
    .unwrap();
    let mut ed = editor_in(tmp.path());
    run(
        &mut ed,
        tmp.path(),
        &format!("(require {})", steel_path(&module)),
    );

    let out = log_probe(&mut ed, tmp.path(), "(where)");

    assert_eq!(out, canonical(tmp.path()).to_str().unwrap());
}

#[test]
fn path_entries_resolves_dot_dot_lexically_like_e() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("a")).unwrap();
    std::fs::create_dir(tmp.path().join("b")).unwrap();
    symlink(tmp.path().join("b"), tmp.path().join("a").join("link")).unwrap();
    let mut ed = editor_in(tmp.path());

    let out = log_probe(
        &mut ed,
        tmp.path(),
        r#"(string-join (path-entries "a/link/.." "") ",")"#,
    );

    assert_eq!(out, "link/");
}

#[test]
fn relative_display_path_keeps_a_symlinked_directory_as_typed() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    let proj = tmp.path().join("proj");
    let lib = tmp.path().join("lib");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("x.rs"), "x\n").unwrap();
    symlink(&lib, proj.join("vendor")).unwrap();
    let mut ed = editor_in(&proj);
    type_cmd(&mut ed, ":e vendor/x.rs");

    assert_eq!(relative_display_path(&mut ed, tmp.path()), "vendor/x.rs");
}

#[test]
fn relative_display_path_names_a_symlinked_file_as_typed() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("real")).unwrap();
    std::fs::write(tmp.path().join("real").join("b.rs"), "x\n").unwrap();
    symlink(
        tmp.path().join("real").join("b.rs"),
        tmp.path().join("a.rs"),
    )
    .unwrap();
    let mut ed = editor_in(tmp.path());
    type_cmd(&mut ed, ":e a.rs");

    assert_eq!(relative_display_path(&mut ed, tmp.path()), "a.rs");
}

#[test]
fn relative_display_path_of_a_path_typed_through_a_link_to_the_cwd() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::write(real.join("f.rs"), "x\n").unwrap();
    symlink(&real, tmp.path().join("link")).unwrap();
    let mut ed = editor_in(&real);
    type_cmd(
        &mut ed,
        &format!(":e {}", tmp.path().join("link").join("f.rs").display()),
    );

    assert_eq!(relative_display_path(&mut ed, tmp.path()), "f.rs");
}
