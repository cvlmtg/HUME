use crate::ScriptingHost;
use crate::null_host::NullHost;

#[test]
fn process_and_fs_globals_are_available_unrequired() {
    let mut host = ScriptingHost::new();
    let mut null_host = NullHost;
    let src = r#"
        (if (and (function? command)
                 (function? spawn-process)
                 (function? wait)
                 (function? wait->stdout)
                 (function? which)
                 (function? with-current-dir)
                 (function? with-stdout-piped)
                 (function? read-dir)
                 (function? create-directory!)
                 (function? delete-directory!)
                 (function? is-dir?)
                 (function? is-file?)
                 (function? open-output-file)
                 ; `Ok?`/`Err?`/`Ok->value`/`Err->value` are the raw
                 ; `steel/core/result` struct ops and ARE globally bound;
                 ; the higher-level `unwrap-ok`/`unwrap-err` wrapper
                 ; (`steel/result`) is NOT reachable here — `(require-builtin
                 ; steel/result)` fails with "module not found" in HUME's
                 ; embedding, unlike steel-core's own bundled module-name
                 ; resolution. Use `Ok->value`/`Err->value` in plugin code.
                 (function? Ok?)
                 (function? Err?)
                 (function? Ok->value)
                 (function? Err->value)
                 ; needed by stdlib/list-subdirs (Phase 1 helper): sort takes
                 ; an explicit comparator ((sort lst less?)), not 1-arg.
                 (function? sort)
                 (function? string<?)
                 (function? file-name))
            (begin)
            (error "one or more steel/process, steel/filesystem, steel/ports, or result globals are missing"))
    "#;
    host.eval_source(src, &mut null_host)
        .expect("steel stdlib availability pin failed");

    // string-downcase, needed by lsp/verify-sha256! (Phase 4 helper).
    let mut host3 = ScriptingHost::new();
    let mut null_host3 = NullHost;
    let downcase_src = r#"
        (if (equal? (string-downcase "ABC123def") "abc123def")
            (begin)
            (error "string-downcase did not lowercase"))
    "#;
    host3
        .eval_source(downcase_src, &mut null_host3)
        .expect("string-downcase probe failed");

    // Round-trip proof, not just presence: `stdlib/list-subdirs` depends on
    // `sort` taking `(lst less?)` and `file-name` extracting a basename.
    let mut host2 = ScriptingHost::new();
    let mut null_host2 = NullHost;
    let sort_src = r#"
        (define sorted (sort (list "b" "a" "c") string<?))
        (if (equal? sorted (list "a" "b" "c"))
            (begin)
            (error (string-append "sort did not sort: " (to-string sorted))))
        (if (equal? (file-name "/tmp/foo/bar.txt") "bar.txt")
            (begin)
            (error "file-name did not extract basename"))
    "#;
    host2
        .eval_source(sort_src, &mut null_host2)
        .expect("sort/file-name round trip failed");
}

/// End-to-end proof, not just presence checks: a real spawn writes to a
/// real temp directory and its piped stdout is captured back correctly.
#[test]
fn spawn_process_round_trip_with_fs_ops_and_piped_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_string_lossy().replace('\\', "\\\\");

    let mut host = ScriptingHost::new();
    let mut null_host = NullHost;
    let src = format!(
        r#"
        (define target (string-append "{base}" "/probe-dir"))
        (create-directory! target)
        (if (is-dir? target) (begin) (error "create-directory!/is-dir? failed"))
        (delete-directory! target)
        (if (is-dir? target) (error "delete-directory! did not remove dir") (begin))

        (define builder (with-current-dir (with-stdout-piped (command "echo" (list "hello-from-probe"))) "{base}"))
        (define spawned (spawn-process builder))
        (if (Ok? spawned)
            (let ([child (Ok->value spawned)])
              (let ([out (wait->stdout child)])
                (if (and (Ok? out) (string-contains? (Ok->value out) "hello-from-probe"))
                    (begin)
                    (error (string-append "unexpected wait->stdout result: " (to-string out))))))
            (error (to-string (Err->value spawned))))
        "#
    );
    host.eval_source(&src, &mut null_host)
        .expect("spawn-process round trip with fs ops and piped stdout failed");
}

/// Pins the file read/write port round trip `plum/read-file`/`plum/write-file`
/// (Phase 1 helpers) depend on.
#[test]
fn file_write_read_port_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("probe.txt")
        .to_string_lossy()
        .replace('\\', "\\\\");
    let mut host = ScriptingHost::new();
    let mut null_host = NullHost;
    let src = format!(
        r#"
        (define out (open-output-file "{path}"))
        (write-string "hello-file-probe" out)
        (close-output-port out)
        (define in (open-input-file "{path}"))
        (define content (read-port-to-string in))
        (close-input-port in)
        (if (equal? content "hello-file-probe")
            (begin)
            (error (string-append "unexpected file content: " content)))
        "#
    );
    host.eval_source(&src, &mut null_host)
        .expect("file write/read port probe failed");
}

#[cfg(unix)]
mod unix;
