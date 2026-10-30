use crate::ScriptingHost;
use crate::null_host::NullHost;
use hume_platform::dirs::Dirs;
use steel::rvals::SteelVal;

#[test]
fn process_and_fs_globals_are_available_unrequired() {
    let mut host = ScriptingHost::for_testing(&Dirs::none());
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
                 ; (`steel/result`) is NOT reachable here: `(require-builtin
                 ; steel/result)` fails with "module not found" in HUME's
                 ; embedding, unlike steel-core's own bundled module-name
                 ; resolution. Use `Ok->value`/`Err->value` in plugin code.
                 (function? Ok?)
                 (function? Err?)
                 (function? Ok->value)
                 (function? Err->value)
                 ; needed by stdlib/list-subdirs: sort takes
                 ; an explicit comparator ((sort lst less?)), not 1-arg.
                 (function? sort)
                 (function? string<?)
                 (function? file-name))
            (begin)
            (error "one or more steel/process, steel/filesystem, steel/ports, or result globals are missing"))
    "#;
    host.eval_source(src, &mut null_host)
        .expect("steel stdlib availability pin failed");

    // string-downcase, needed by lsp-install/verify-sha256!.
    let mut host3 = ScriptingHost::for_testing(&Dirs::none());
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
    let mut host2 = ScriptingHost::for_testing(&Dirs::none());
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
    let base = crate::steel_path_literal(dir.path()).expect("tempdir path must not contain '\"'");

    let mut host = ScriptingHost::for_testing(&Dirs::none());
    let mut null_host = NullHost;
    let src = format!(
        r#"
        (define target (string-append {base} "/probe-dir"))
        (create-directory! target)
        (if (is-dir? target) (begin) (error "create-directory!/is-dir? failed"))
        (delete-directory! target)
        (if (is-dir? target) (error "delete-directory! did not remove dir") (begin))

        (define builder (with-current-dir (with-stdout-piped (command "echo" (list "hello-from-probe"))) {base}))
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

/// Pins the file read/write port round trip `stdlib/write-file!` and the
/// `call-with-input-file` readers depend on.
#[test]
fn file_write_read_port_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = crate::steel_path_literal(&dir.path().join("probe.txt"))
        .expect("tempdir path must not contain '\"'");
    let mut host = ScriptingHost::for_testing(&Dirs::none());
    let mut null_host = NullHost;
    let src = format!(
        r#"
        (define out (open-output-file {path}))
        (write-string "hello-file-probe" out)
        (close-output-port out)
        (define in (open-input-file {path}))
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

/// A Steel string literal, once parsed back by the VM,
/// must have the same character count as the original path, proof the
/// escaping round-trips rather than just "doesn't panic".
#[test]
fn steel_path_literal_escapes_windows_backslashes_for_round_trip_through_steel() {
    let path = std::path::Path::new(r"C:\Users\x");
    let literal = crate::steel_path_literal(path).expect("plain path must escape");

    let mut host = ScriptingHost::for_testing(&Dirs::none());
    let mut null_host = NullHost;
    let src = format!(
        r#"
        (define s {literal})
        (if (= (string-length s) 10)
            (begin)
            (error (string-append "unexpected length: " (to-string (string-length s)))))
        "#
    );
    host.eval_source(&src, &mut null_host)
        .expect("steel_path_literal's output must parse back as a 10-char Steel string");
}

#[test]
fn steel_path_literal_rejects_a_path_containing_a_quote() {
    let path = std::path::Path::new("weird\"quote.txt");
    assert!(
        crate::steel_path_literal(path).is_none(),
        "a quote in the path can't be embedded in a Steel string literal"
    );
}

/// A handler that raises again hands the same error to the enclosing
/// `with-handler`: its message matches the one a single handler sees.
#[test]
fn error_raised_from_a_handler_reaches_the_enclosing_handler() {
    let mut host = ScriptingHost::for_testing(&Dirs::none());
    let value = host.eval_source_for_value(
        r#"
        (list
          (with-handler (lambda (err) (error-object-message err)) (error "inner"))
          (with-handler
            (lambda (err) (error-object-message err))
            (with-handler
              (lambda (err) (raise-error err))
              (error "inner"))))
        "#,
        &mut NullHost,
    );
    let Ok(SteelVal::ListV(messages)) = &value else {
        panic!("got {value:?}");
    };
    let messages: Vec<_> = messages.iter().collect();
    assert!(
        matches!(messages.as_slice(), [SteelVal::StringV(once), SteelVal::StringV(reraised)]
            if once.as_str().contains("inner") && once == reraised),
        "got {messages:?}"
    );
}

mod language_data;

#[cfg(unix)]
mod unix;

/// A struct's own names are listed by its visibility: a public struct's
/// constructor, predicate and accessors, a private one's none. Its type
/// descriptor and the options hash steel-core's `struct` macro binds are
/// never listed.
#[test]
fn host_global_names_list_a_struct_by_its_visibility() {
    let mut host = ScriptingHost::for_testing(&Dirs::none());
    host.eval_source(
        "(struct probe-public (a)) (struct %probe-private (a))",
        &mut NullHost,
    )
    .expect("the structs evaluate");
    let names = host.host_global_names();
    for listed in ["probe-public", "probe-public?", "probe-public-a"] {
        assert!(names.iter().any(|n| n == listed), "{listed} missing");
    }
    for hidden in [
        "struct:probe-public",
        "___probe-public-options___",
        "struct:%probe-private",
        "___%probe-private-options___",
    ] {
        assert!(!names.iter().any(|n| n == hidden), "{hidden} listed");
    }
}
