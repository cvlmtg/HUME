// Programs are looked up on `state.dirs.search_path`, never on the test
// process's own `PATH`. Each test puts a program in a directory that only
// the editor's search path names.

use super::*;
use crate::editor::tests::editor_cwd::editor_in;

const PROBE: &str = "hume-search-path-probe";

/// A directory holding [`PROBE`], a link to `/bin/sh`.
/// Not a script written here: macOS scans a newly written executable on its
/// first run, which takes seconds while the rest of the suite runs. Not a
/// coreutils program: a multicall coreutils (Ubuntu's uutils) picks the tool
/// by the name it runs under, and runs none as [`PROBE`].
fn probe_dir() -> tempfile::TempDir {
    let bin = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink("/bin/sh", bin.path().join(PROBE)).unwrap();
    bin
}

/// An editor in `tmp` whose search path is `bin` alone.
fn editor_searching(tmp: &Path, bin: &Path) -> Editor {
    let mut ed = editor_in(tmp);
    ed.state.dirs.search_path = Some(bin.as_os_str().to_owned());
    ed
}

#[test]
fn find_program_finds_a_program_on_the_editor_search_path() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = probe_dir();
    let mut ed = editor_searching(tmp.path(), bin.path());

    let found = log_probe(&mut ed, tmp.path(), &format!(r#"(find-program "{PROBE}")"#));

    assert_eq!(found, bin.path().join(PROBE).to_str().unwrap());
}

#[test]
fn find_program_finds_nothing_without_a_search_path() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_in(tmp.path());
    ed.state.dirs.search_path = None;

    let found = log_probe(
        &mut ed,
        tmp.path(),
        r#"(if (find-program "sh") "found" "none")"#,
    );

    assert_eq!(found, "none");
}

#[test]
fn path_programs_lists_the_programs_on_the_editor_search_path() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = probe_dir();
    let mut ed = editor_searching(tmp.path(), bin.path());
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (path-programs "hume-search" '()
               (lambda (names done?)
                 (when done? (log! 'info (apply string-append names)))))))"#,
    );

    call(&mut ed, "go");
    drain_until(&mut ed, |ed| ed.state.status_msg.is_some());

    assert_eq!(ed.state.status_msg.clone().unwrap(), PROBE);
}

#[test]
fn spawn_async_runs_a_program_on_the_editor_search_path() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = probe_dir();
    let mut ed = editor_searching(tmp.path(), bin.path());
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda ()
                 (spawn-async! "{PROBE}" '("-c" "echo found")
                   (lambda (result) (log! 'info (trim (hash-ref result 'stdout)))))))"#
        ),
    );
    call(&mut ed, "go");

    drain_until(&mut ed, |ed| ed.state.status_msg.is_some());

    assert_eq!(ed.state.status_msg.clone().unwrap(), "found");
}

#[test]
fn run_capture_runs_a_program_on_the_editor_search_path() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = probe_dir();
    let mut ed = editor_searching(tmp.path(), bin.path());

    let out = log_probe(
        &mut ed,
        tmp.path(),
        &format!(r#"(trim (hash-ref (run-capture! "{PROBE}" '("-c" "echo found")) 'stdout))"#),
    );

    assert_eq!(out, "found");
}

/// Defines `go`, which runs the probe program through `run-inline-output!`
/// to create `record`, declared `#:inline-output` when `inline_output`.
fn define_inline_go(
    ed: &mut Editor,
    tmp: &std::path::Path,
    record: &std::path::Path,
    inline_output: bool,
) {
    run(
        ed,
        tmp,
        &format!(
            r#"(define-command! "go" "" (lambda ()
                 (run-inline-output! "{PROBE}" (list "-c" ": > \"$1\"" "{PROBE}" {})))
                 #:inline-output {})"#,
            steel_path(record),
            if inline_output { "#t" } else { "#f" }
        ),
    );
}

#[test]
fn run_inline_output_runs_a_program_on_the_editor_search_path() {
    let tmp = tempfile::tempdir().unwrap();
    let record = tmp.path().join("ran.txt");
    let bin = probe_dir();
    let mut ed = editor_searching(tmp.path(), bin.path());
    define_inline_go(&mut ed, tmp.path(), &record, true);

    call(&mut ed, "go");

    assert!(record.exists());
}

#[test]
fn run_inline_output_raises_in_a_command_not_declared_inline_output() {
    let tmp = tempfile::tempdir().unwrap();
    let record = tmp.path().join("ran.txt");
    let bin = probe_dir();
    let mut ed = editor_searching(tmp.path(), bin.path());
    define_inline_go(&mut ed, tmp.path(), &record, false);

    call(&mut ed, "go");

    assert!(!record.exists());
    assert!(has_error(
        &ed,
        "run-inline-output!: runs only inside an #:inline-output command"
    ));
}

#[test]
fn picker_source_spawn_runs_a_program_on_the_editor_search_path() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = probe_dir();
    let mut ed = editor_searching(tmp.path(), bin.path());
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"
            (define tok #f)
            (define-typed-command! "go" "" (lambda (pane)
              (set! tok (picker! pane '() (test-picker-keys (lambda (x) (void)))))))
            (define-command! "spawn-it" "" (lambda ()
              (picker-source-spawn! tok "{PROBE}" '("-c" "echo found"))))
            "#
        ),
    );
    type_cmd(&mut ed, ":go");
    call(&mut ed, "spawn-it");

    drain_until_picker_total(&mut ed, 1);

    let picker = ed.state.input.picker().unwrap();
    assert_eq!(picker.window(10).collect::<Vec<_>>(), vec!["found"]);
}
