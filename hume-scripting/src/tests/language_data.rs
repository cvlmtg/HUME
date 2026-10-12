//! The language-data reader in `bootstrap.scm`, through
//! `ScriptingHost::bundled_language` and the two grammar functions plugins
//! call.

use crate::null_host::NullHost;
use crate::{BundledLanguage, ScriptingHost};
use hume_platform::dirs::Dirs;
use std::path::Path;

const HEADER: &str = ";;; generated header\n\n";

/// A runtime dir holding `scheme/languages/<name>.scm` for each `(name, body)`.
fn runtime_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let runtime = tempfile::tempdir().expect("create temp runtime dir");
    let dir = runtime.path().join("scheme").join("languages");
    std::fs::create_dir_all(&dir).expect("create languages dir");
    for (name, body) in files {
        std::fs::write(dir.join(format!("{name}.scm")), format!("{HEADER}{body}\n"))
            .expect("write language file");
    }
    runtime
}

fn host_for(runtime: Option<&Path>) -> ScriptingHost {
    ScriptingHost::new(&Dirs {
        runtime: runtime.map(Path::to_path_buf),
        ..Dirs::none()
    })
}

fn read(files: &[(&str, &str)], name: &str) -> Result<Option<BundledLanguage>, String> {
    let runtime = runtime_with(files);
    host_for(Some(runtime.path())).bundled_language(name, &mut NullHost)
}

fn read_ok(body: &str) -> BundledLanguage {
    read(&[("lang", body)], "lang")
        .expect("a well-formed file reads")
        .expect("the file exists")
}

/// Reads a malformed `lang.scm`, returning its error, which must name the file.
fn read_err(body: &str) -> String {
    let err = read(&[("lang", body)], "lang").expect_err("a malformed file is an error");
    assert!(err.contains("lang.scm"), "{err}");
    err
}

/// Evaluates `src` against a runtime holding `files`, returning its error.
fn eval(files: &[(&str, &str)], src: &str) -> Result<(), String> {
    let runtime = runtime_with(files);
    host_for(Some(runtime.path()))
        .eval_source(src, &mut NullHost)
        .map(|_| ())
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn read_bang_parses_dotted_pairs_raises_on_bad_syntax_and_keeps_no_state() {
    let mut host = ScriptingHost::new(&Dirs::none());
    let src = r#"
        (define parsed (hm.read! "((language-id . \"x\") (b (\"/*\" . \"*/\")))"))
        (unless (equal? parsed (list (list (cons 'language-id "x") (list 'b (cons "/*" "*/")))))
          (error (string-append "unexpected parse: " (to-string parsed))))
        (unless (equal? (with-handler (lambda (e) "raised") (hm.read! "((a")) "raised")
          (error "a parse error must raise"))
        (unless (equal? (hm.read! "(1)") (list (list 1)))
          (error "a read after a failed one must not see its text"))
    "#;
    host.eval_source(src, &mut NullHost)
        .expect("read! characterization failed");
}

#[test]
fn the_config_carries_roots_and_language_id() {
    let lang = read_ok(
        r#"((comment-tokens "//" "///")
 (block-comment-tokens ("/*" . "*/") ("/**" . "*/"))
 (language-id . "typescriptreact")
 (roots "package.json" "tsconfig.json")
 (grammar "https://ts" "abc" "tree_sitter_tsx" "tsx"))"#,
    );

    assert_eq!(
        lang,
        BundledLanguage {
            roots: Some(strings(&["package.json", "tsconfig.json"])),
            lsp_language_id: Some("typescriptreact".to_string()),
        }
    );
}

#[test]
fn absent_keys_read_as_empty() {
    assert_eq!(
        read_ok(r##"((comment-tokens "#"))"##),
        BundledLanguage::default()
    );
}

#[test]
fn the_empty_list_reads_as_empty() {
    assert_eq!(read_ok("()"), BundledLanguage::default());
}

#[test]
fn a_dotted_language_name_reads_its_own_file() {
    let lang = read(
        &[("markdown.inline", r#"((roots "x"))"#)],
        "markdown.inline",
    )
    .expect("reads")
    .expect("exists");

    assert_eq!(lang.roots, Some(strings(&["x"])));
}

#[test]
fn a_missing_file_is_no_data() {
    assert_eq!(
        read(&[], "nope").expect("a missing file is not an error"),
        None
    );
}

#[test]
fn no_runtime_dir_is_no_data() {
    assert_eq!(
        host_for(None)
            .bundled_language("rust", &mut NullHost)
            .expect("no runtime dir is not an error"),
        None
    );
}

#[test]
fn a_malformed_file_is_an_error_naming_the_file_and_the_problem() {
    for (body, problem) in [
        (r#"((indent "  "))"#, "indent"),
        (r#"((roots "a") (roots "b"))"#, "roots"),
        (r#"(("roots" "a"))"#, "key symbol"),
        ("((comment-tokens 3))", "comment-tokens"),
        (
            r#"((block-comment-tokens ("/*" "*/")))"#,
            "block-comment-tokens",
        ),
        ("((roots 1))", "roots"),
        (r#"((grammar "url" "rev" "sym"))"#, "grammar"),
        (r#"((language-id "tsx"))"#, "language-id"),
        (r#"((roots "a""#, "lang.scm"),
        ("() ()", "top-level form"),
    ] {
        let err = read_err(body);
        assert!(err.contains(problem), "{body}: {err}");
    }
}

#[test]
fn a_name_that_is_not_a_listed_file_is_no_data() {
    let runtime = runtime_with(&[("rust", r#"((roots "Cargo.toml"))"#)]);
    std::fs::write(
        runtime.path().join("scheme").join("init.scm"),
        r#"((roots "outside"))"#,
    )
    .expect("write a file outside the languages dir");
    let mut host = host_for(Some(runtime.path()));

    for name in [
        "../init", "a/b", "a\\b", "..", ".", "", "C:x", "Rust", "rust.scm", "a b",
    ] {
        assert_eq!(
            host.bundled_language(name, &mut NullHost),
            Ok(None),
            "{name:?}"
        );
    }
}

#[test]
fn bundled_language_grammar_returns_the_four_source_fields() {
    eval(
        &[(
            "jsx",
            r#"((grammar "u" "r" "tree_sitter_javascript" "sub"))"#,
        )],
        r#"(let ((source (bundled-language-grammar "jsx")))
             (unless (equal? source (list "u" "r" "tree_sitter_javascript" "sub"))
               (error (to-string source))))"#,
    )
    .expect("grammar source");
}

#[test]
fn bundled_language_grammar_is_false_without_a_file_a_grammar_key_or_a_runtime_dir() {
    eval(
        &[("cabal", r#"((comment-tokens "--"))"#)],
        r#"(when (bundled-language-grammar "cabal") (error "cabal has no grammar"))
           (when (bundled-language-grammar "cobol") (error "cobol has no file"))"#,
    )
    .expect("no grammar");
    host_for(None)
        .eval_source(
            r#"(when (bundled-language-grammar "jsx") (error "no runtime dir"))"#,
            &mut NullHost,
        )
        .expect("no runtime dir");
}

#[test]
fn bundled_grammar_languages_lists_only_files_with_a_grammar_sorted() {
    eval(
        &[
            ("yaml", r#"((grammar "u" "r" "tree_sitter_yaml" ""))"#),
            ("cabal", r#"((comment-tokens "--"))"#),
            (
                "docker-compose",
                r#"((grammar "u" "r" "tree_sitter_yaml" ""))"#,
            ),
        ],
        r#"(let ((names (bundled-grammar-languages)))
             (unless (equal? names (list "docker-compose" "yaml"))
               (error (to-string names))))"#,
    )
    .expect("sorted grammar languages");
}

#[test]
fn bundled_grammar_languages_is_empty_without_a_languages_dir_or_a_runtime_dir() {
    let runtime = tempfile::tempdir().expect("create temp runtime dir");
    for dirs_runtime in [Some(runtime.path()), None] {
        host_for(dirs_runtime)
            .eval_source(
                r#"(unless (null? (bundled-grammar-languages)) (error "expected no languages"))"#,
                &mut NullHost,
            )
            .expect("no languages");
    }
}

#[test]
fn bundled_grammar_languages_reports_a_malformed_file() {
    let err = eval(&[("bad", "((roots 1))")], "(bundled-grammar-languages)")
        .expect_err("a malformed file is an error");

    assert!(err.contains("bad.scm"), "{err}");
}

#[test]
fn an_unparsable_file_read_inside_an_outer_handler_leaves_the_vm_usable() {
    eval(
        &[
            ("bad", r#"((roots "a""#),
            ("good", r#"((grammar "u" "r" "s" ""))"#),
        ],
        r#"(define caught
             (with-handler (lambda (err) (to-string err))
                           (bundled-language-grammar "bad")))
           (unless (and (string? caught) (string-contains? caught "bad.scm"))
             (error (string-append "expected an error naming bad.scm: " (to-string caught))))
           (unless (equal? (bundled-language-grammar "good") (list "u" "r" "s" ""))
             (error "a read after the caught error failed"))"#,
    )
    .expect("the VM stays usable after a caught parse error");
}
