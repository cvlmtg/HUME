//! core:stdlib: real shipped plugin.

use super::*;
use hume_scripting::PluginStatus;

// ── core:stdlib — real shipped plugin ─────────────────────────────────────────

const STDLIB_MANIFEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../runtime/plugins/core/stdlib/manifest.scm"
));

const STDLIB_FILES: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../runtime/plugins/core/stdlib/files.scm"
));

/// Stage the real shipped `core:stdlib` plugin, manifest and both entries, into
/// an isolated runtime directory and eagerly load both entries via a real
/// `init.scm`, returning everything the caller needs kept alive plus the host
/// for further inspection or `eval_source`.
fn setup_stdlib_editor() -> (Editor, ScriptingHost, RuntimeDirs, tempfile::TempDir) {
    let guard = RuntimeDirs::new();
    write_core_plugin(&guard, "stdlib", STDLIB_PLUGIN);
    let plugin_dir = guard.runtime.path().join("plugins/core/stdlib");
    std::fs::write(plugin_dir.join("manifest.scm"), STDLIB_MANIFEST).unwrap();
    std::fs::write(plugin_dir.join("files.scm"), STDLIB_FILES).unwrap();

    let init_dir = tempfile::tempdir().unwrap();
    let init_path = init_dir.path().join("init.scm");
    std::fs::write(
        &init_path,
        format!(
            "{}\n(%activate-plugin-inline! \"core:stdlib\" \"files.scm\")",
            hume_scripting::eager_load_scm("core:stdlib", None)
        ),
    )
    .unwrap();

    let mut ed = editor_from("-[a]>b\n");
    ed.state.dirs = guard.dirs();
    let mut host = ScriptingHost::for_testing(&ed.state.dirs);
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih)
    }
    .expect("eval_init must succeed loading core:stdlib");

    (ed, host, guard, init_dir)
}

/// Both entries of the shipped `core:stdlib` plugin must load eagerly and
/// reach `Loaded`.
#[test]
fn core_stdlib_plugin_loads_eagerly() {
    use hume_scripting::attribution::{EntryFile, EntryId, PluginId};

    let (_ed, host, _guard, _init_dir) = setup_stdlib_editor();

    let id = PluginId::parse("core:stdlib").expect("valid plugin name");
    assert_eq!(
        host.plugin_status(&EntryId::main(id.clone())),
        Some(PluginStatus::Loaded),
        "core:stdlib must be Loaded after eager load-plugin!"
    );
    assert_eq!(
        host.plugin_status(&EntryId::new(id, EntryFile::parse("files.scm").unwrap())),
        Some(PluginStatus::Loaded),
        "core:stdlib's files.scm entry must be Loaded after its inline activation"
    );
}

/// The `core:stdlib` selection-query commands must compute the expected
/// results on literal selection-list arguments via `call!`, and pass `#f`
/// straight through untouched: the same cross-plugin surface
/// `core:vim-keybind` uses for its conditional `C` binding.
///
/// Each assertion compares against a hand-written literal, independent
/// of the implementation. If any command computes the wrong result, its `unless`
/// fires `(error ...)`, which propagates as an `Err`, caught by the assert
/// below, failing the test with the offending assertion name.
///
/// `stdlib`'s selection-list helpers (`primary-selection`, `single-selection?`,
/// `all-single-char?`, `cursor-char-offset`) are `call!`-reachable public
/// commands that pass `#f` through.
#[test]
fn core_stdlib_selection_commands() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();

    let assertions = r#"
(unless (equal? (call! "stdlib/all-single-char?" #f) #f) (error "all-single-char? #f passthrough"))
(unless (equal? (call! "stdlib/single-selection?" #f) #f) (error "single-selection? #f passthrough"))
(unless (equal? (call! "stdlib/cursor-char-offset" #f) #f) (error "cursor-char-offset #f passthrough"))
(unless (equal? (call! "stdlib/primary-selection" #f) #f) (error "primary-selection #f passthrough"))

(unless (equal? (call! "stdlib/single-selection?" (list (list 0 1 0 2 #t))) #t)
  (error "single-selection? true"))
(unless (equal? (call! "stdlib/single-selection?" (list (list 0 1 0 2 #t) (list 2 3 2 4 #f))) #f)
  (error "single-selection? false"))

(unless (equal? (call! "stdlib/all-single-char?" (list (list 2 2 2 3 #t) (list 5 5 5 6 #f))) #t)
  (error "all-single-char? true"))
(unless (equal? (call! "stdlib/all-single-char?" (list (list 2 3 2 4 #t))) #f)
  (error "all-single-char? false"))
(unless (equal? (call! "stdlib/all-single-char?" (list (list 2 2 2 3 #f) (list 5 6 5 7 #t))) #f)
  (error "all-single-char? false when only a later selection is wide"))

(unless (equal? (call! "stdlib/cursor-char-offset" (list (list 0 0 0 1 #f) (list 7 4 4 8 #t))) 4)
  (error "cursor-char-offset picks the primary's head"))

(unless (equal? (call! "stdlib/primary-selection" (list (list 0 0 0 1 #f) (list 7 4 4 8 #t) (list 1 1 1 2 #f))) (list 7 4 4 8 #t))
  (error "primary-selection picks the flagged selection out of a list"))
(unless (equal? (call! "stdlib/primary-selection" (list (list 0 0 0 1 #f) (list 1 1 1 2 #f))) #f)
  (error "primary-selection #f when no selection is flagged"))
"#;

    let result = {
        let mut ih = init_host!(ed);
        host.eval_source(assertions, &mut ih)
    };
    assert!(
        result.is_ok(),
        "stdlib selection command assertions must all pass: {result:?}"
    );
}

/// The `core:stdlib` config-validation commands (`stdlib/config-boolean`,
/// `stdlib/config-string`, `stdlib/config-enum`, `stdlib/config-integer`,
/// `stdlib/config-list`) must resolve a present key, fall back to the given
/// default when the key is absent, and raise an error naming both the given
/// plugin and the offending key when the resolved value fails its
/// type/membership/range check.
///
/// Each wrong-type/wrong-value case is wrapped in `with-handler`, which
/// checks the caught error's message via `string-contains?`/`to-string` and,
/// if it doesn't name what's expected, raises a new `(error ...)` saying so.
#[test]
fn core_stdlib_config_commands() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();

    let assertions = r#"
(unless (equal? (call! "stdlib/config-boolean" "p" (hash 'k #f) 'k #t) #f)
  (error "config-boolean: present key wins over default"))
(unless (equal? (call! "stdlib/config-boolean" "p" (hash) 'k #t) #t)
  (error "config-boolean: absent key falls back to default"))
(with-handler
  (lambda (err)
    (unless (and (string-contains? (to-string err) "p") (string-contains? (to-string err) "'k"))
      (error (string-append "config-boolean: error must name plugin and key: " (to-string err)))))
  (begin
    (call! "stdlib/config-boolean" "p" (hash 'k "not-a-bool") 'k #t)
    (error "config-boolean: expected a raise on a non-boolean value")))

(unless (equal? (call! "stdlib/config-string" "p" (hash 'k "custom") 'k "default") "custom")
  (error "config-string: present key wins over default"))
(unless (equal? (call! "stdlib/config-string" "p" (hash) 'k "default") "default")
  (error "config-string: absent key falls back to default"))
(with-handler
  (lambda (err)
    (unless (and (string-contains? (to-string err) "p") (string-contains? (to-string err) "'k"))
      (error (string-append "config-string: error must name plugin and key: " (to-string err)))))
  (begin
    (call! "stdlib/config-string" "p" (hash 'k 'not-a-string) 'k "default")
    (error "config-string: expected a raise on a non-string value")))

(unless (equal? (call! "stdlib/config-enum" "p" (hash 'k 'on) 'k 'smart '(on smart off)) 'on)
  (error "config-enum: present key wins over default"))
(unless (equal? (call! "stdlib/config-enum" "p" (hash) 'k 'smart '(on smart off)) 'smart)
  (error "config-enum: absent key falls back to default"))
(with-handler
  (lambda (err)
    (unless (and (string-contains? (to-string err) "p") (string-contains? (to-string err) "'k"))
      (error (string-append "config-enum: error must name plugin and key: " (to-string err)))))
  (begin
    (call! "stdlib/config-enum" "p" (hash 'k 'bogus) 'k 'smart '(on smart off))
    (error "config-enum: expected a raise on a disallowed value")))

(unless (equal? (call! "stdlib/config-integer" "p" (hash 'k 5) 'k 1 0) 5)
  (error "config-integer: present key wins over default"))
(unless (equal? (call! "stdlib/config-integer" "p" (hash) 'k 1 0) 1)
  (error "config-integer: absent key falls back to default"))
(with-handler
  (lambda (err)
    (unless (string-contains? (to-string err) "minimum")
      (error (string-append "config-integer: error must name the minimum: " (to-string err)))))
  (begin
    (call! "stdlib/config-integer" "p" (hash 'k -5) 'k 1 #f)
    (error "config-integer: expected a raise on a #f minimum")))
(with-handler
  (lambda (err)
    (unless (and (string-contains? (to-string err) "p") (string-contains? (to-string err) "'k"))
      (error (string-append "config-integer: error must name plugin and key: " (to-string err)))))
  (begin
    (call! "stdlib/config-integer" "p" (hash 'k "not-an-int") 'k 1 0)
    (error "config-integer: expected a raise on a non-integer value")))
(with-handler
  (lambda (err)
    (unless (and (string-contains? (to-string err) "p") (string-contains? (to-string err) "'k"))
      (error (string-append "config-integer: error must name plugin and key: " (to-string err)))))
  (begin
    (call! "stdlib/config-integer" "p" (hash 'k -1) 'k 1 0)
    (error "config-integer: expected a raise on a value below the minimum")))

(unless (equal? (call! "stdlib/config-list" "p" (hash 'k (list "a" "b")) 'k '() ) (list "a" "b"))
  (error "config-list: present key wins over default"))
(unless (equal? (call! "stdlib/config-list" "p" (hash) 'k (list "d")) (list "d"))
  (error "config-list: absent key falls back to default"))
(unless (equal? (call! "stdlib/config-list" "p" (hash 'k '()) 'k (list "d")) '())
  (error "config-list: empty list accepted"))
(with-handler
  (lambda (err)
    (unless (and (string-contains? (to-string err) "p") (string-contains? (to-string err) "'k"))
      (error (string-append "config-list: error must name plugin and key: " (to-string err)))))
  (begin
    (call! "stdlib/config-list" "p" (hash 'k "not-a-list") 'k '())
    (error "config-list: expected a raise on a non-list value")))
(with-handler
  (lambda (err)
    (unless (and (string-contains? (to-string err) "p") (string-contains? (to-string err) "'k"))
      (error (string-append "config-list: error must name plugin and key: " (to-string err)))))
  (begin
    (call! "stdlib/config-list" "p" (hash 'k (list "a" 'not-a-string)) 'k '())
    (error "config-list: expected a raise on a non-string element")))
(with-handler
  (lambda (err)
    (unless (string-contains? (to-string err) "must be a symbol")
      (error (string-append "config-value: error must ask for a symbol key: " (to-string err)))))
  (begin
    (call! "stdlib/config-boolean" "p" (hash) "k" #t)
    (error "config-value: expected a raise on a string key")))
"#;

    let result = {
        let mut ih = init_host!(ed);
        host.eval_source(assertions, &mut ih)
    };
    assert!(
        result.is_ok(),
        "stdlib config command assertions must all pass: {result:?}"
    );
}

/// `stdlib/list-subdirs` must return only subdirectory basenames, sorted,
/// filtering out a stray file that sits alongside them (e.g. `.DS_Store`),
/// which `core:plum`'s plugin walk must skip (see `injections_editor.rs`'s
/// `plum_installed_plugins_skips_a_stray_file_in_the_plugins_dir`).
///
/// The directory tree is built directly via `std::fs`, with the expected
/// sorted subdir list written out by hand.
#[test]
fn core_stdlib_list_subdirs_filters_stray_files() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();

    let scan_dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(scan_dir.path().join("beta")).unwrap();
    std::fs::create_dir_all(scan_dir.path().join("alpha")).unwrap();
    std::fs::write(scan_dir.path().join("stray.txt"), "not a dir").unwrap();
    let dir = steel_path(scan_dir.path());

    let assertions = format!(
        r#"
(let ([got (call! "stdlib/list-subdirs" {dir})])
  (unless (equal? got (list "alpha" "beta"))
    (error (string-append "list-subdirs must return only sorted subdir names, got "
                           (to-string got)))))
"#
    );

    let result = {
        let mut ih = init_host!(ed);
        host.eval_source(&assertions, &mut ih)
    };
    assert!(
        result.is_ok(),
        "stdlib/list-subdirs assertions must pass: {result:?}"
    );
}

/// `stdlib/list-entries` returns every entry's basename, files and
/// directories alike, sorted. Both listings raise for a directory that does
/// not exist rather than reading it as empty.
#[test]
fn core_stdlib_list_entries_lists_everything_and_both_listings_raise_when_missing() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();

    let scan_dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(scan_dir.path().join("beta")).unwrap();
    std::fs::create_dir_all(scan_dir.path().join("alpha")).unwrap();
    std::fs::write(scan_dir.path().join("stray.txt"), "not a dir").unwrap();
    let dir = steel_path(scan_dir.path());
    let missing = steel_path(&scan_dir.path().join("missing"));

    let assertions = format!(
        r#"
(let ([got (call! "stdlib/list-entries" {dir})])
  (unless (equal? got (list "alpha" "beta" "stray.txt"))
    (error (string-append "list-entries must return every sorted entry name, got "
                           (to-string got)))))
(define (raises? thunk)
  (with-handler (lambda (err) #t) (thunk) #f))
(unless (raises? (lambda () (call! "stdlib/list-entries" {missing})))
  (error "list-entries must raise for a missing directory"))
(unless (raises? (lambda () (call! "stdlib/list-subdirs" {missing})))
  (error "list-subdirs must raise for a missing directory"))
"#
    );

    let result = {
        let mut ih = init_host!(ed);
        host.eval_source(&assertions, &mut ih)
    };
    assert!(
        result.is_ok(),
        "stdlib listing assertions must pass: {result:?}"
    );
}

#[test]
fn core_stdlib_path_absolute_holds_only_for_a_rooted_path() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();
    let assertions = r#"
(define (check path expected)
  (unless (equal? (call! "stdlib/path-absolute?" path) expected)
    (error (string-append "path-absolute? of " path " must be " (to-string expected)))))
(check "/" #t)
(check "/usr/lib" #t)
(check "lib" #f)
(check "./lib" #f)
(check "~/lib" #f)
(check "" #f)
"#;

    let result = {
        let mut ih = init_host!(ed);
        host.eval_source(assertions, &mut ih)
    };

    assert!(result.is_ok(), "{result:?}");
}

/// `stdlib/concat` joins a list of lists in order, one level deep: lists
/// that are themselves elements stay whole, and an empty first list loses
/// nothing after it.
#[test]
fn core_stdlib_concat_joins_lists_in_order() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();

    let assertions = r#"
(define (check name got want)
  (unless (equal? got want)
    (error (string-append name ": got " (to-string got)))))
(check "no lists" (call! "stdlib/concat" '()) '())
(check "in order" (call! "stdlib/concat" (list '(1 2) '() '(3) '(4 5))) '(1 2 3 4 5))
(check "empty first, then mapped"
       (map (lambda (n) (* n 10)) (call! "stdlib/concat" (list '() '(1 2 3 4 5 6))))
       '(10 20 30 40 50 60))
(check "one level deep"
       (call! "stdlib/concat" (list (list '(a b)) (list '(c) '(d e))))
       (list '(a b) '(c) '(d e)))
(check "many lists" (call! "stdlib/concat" (map list (range 0 2000))) (range 0 2000))
"#;

    let result = {
        let mut ih = init_host!(ed);
        host.eval_source(assertions, &mut ih)
    };
    assert!(
        result.is_ok(),
        "stdlib/concat assertions must pass: {result:?}"
    );
}

/// `stdlib/write-file!` must replace a file's entire content, not just
/// overwrite its leading bytes: writing a shorter string over a longer
/// existing file must not leave the old tail behind.
///
/// The file is read back with `std::fs::read_to_string`
/// on the Rust side, not through any Steel port primitive `write-file`
/// itself might share a bug with.
#[test]
fn core_stdlib_write_file_truncates_existing_content() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();

    let scan_dir = tempfile::tempdir().unwrap();
    let target = scan_dir.path().join("target.txt");
    let path = steel_path(&target);

    let assertions = format!(
        r#"
(call! "stdlib/write-file!" {path} "long-content")
(call! "stdlib/write-file!" {path} "short")
"#
    );

    let result = {
        let mut ih = init_host!(ed);
        host.eval_source(&assertions, &mut ih)
    };
    assert!(
        result.is_ok(),
        "stdlib/write-file! assertions must pass: {result:?}"
    );

    let written = std::fs::read_to_string(&target).unwrap();
    assert_eq!(
        written, "short",
        "stdlib/write-file! must truncate, not leave the previous content's tail behind"
    );
}

/// `stdlib/resolve-lang-arg` must resolve a typed string argument first,
/// fall back to the current buffer's language when the argument isn't a
/// string, and return `#f` plus a `cmd`-naming warning when neither is
/// available.
///
/// `bid` only exists as a dispatched command's own injected leading
/// parameter (an `Init` session eval has no dispatch to inject it from), so
/// unlike the other `stdlib` command tests this defines a throwaway probe
/// command and dispatches it via `:`, the same shape `run_probe`
/// (`tests/mod.rs`) uses, rather than `eval_source`'s bare init-mode
/// assertions.
///
/// The buffer's language is set via `set-buffer-option!`'s `"language"`
/// (a command already covered elsewhere), and the expected fallback value is
/// asserted literally, not derived from `stdlib/resolve-lang-arg` itself.
#[test]
fn core_stdlib_resolve_lang_arg_falls_back_then_warns() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();

    let (_, outcome) = ed.state.config.languages.register_identity_no_rebuild(
        "python",
        hume_core::LanguageIdentity::default(),
        hume_core::ConfigOrigin::User,
    );
    assert_eq!(outcome, hume_core::WriteOutcome::Applied);

    let define_probe = r#"
(define-typed-command! "probe-resolve-lang-arg" ""
  (lambda (bid)
    (unless (equal? (call! "stdlib/resolve-lang-arg" bid "probe-cmd" "rust") "rust")
      (error "resolve-lang-arg: typed string argument must win"))
    (unless (equal? (call! "stdlib/resolve-lang-arg" bid "probe-cmd" 1) #f)
      (error "resolve-lang-arg: no typed arg and no buffer language must return #f"))
    (set-buffer-option! bid "language" "python")
    (unless (equal? (call! "stdlib/resolve-lang-arg" bid "probe-cmd" 1) "python")
      (error "resolve-lang-arg: non-string arg must fall back to the buffer's language"))))
"#;
    {
        let mut ih = init_host!(ed);
        host.eval_source(define_probe, &mut ih)
            .expect("define probe-resolve-lang-arg command");
    }
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":probe-resolve-lang-arg");

    let errors = error_log(&ed);
    assert!(
        errors.is_empty(),
        "resolve-lang-arg assertions must all pass: {errors:?}"
    );
    // Boundary condition, not a failure: Severity::Info, statusline only:
    // the third `resolve-lang-arg` call (after setting `"language"`)
    // succeeds silently, so nothing overwrites the message from the middle
    // (no-fallback) call.
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("probe-cmd") && m.contains("no language given")),
        "the no-fallback case must report a status message naming the calling command, got: {:?}",
        ed.state.status_msg
    );
}

/// `stdlib/split-words` must tokenize using the *given* buffer's own
/// `word-chars`, pulled via `get-buffer-option` internally, not a value the
/// caller fetches and passes itself (the SSOT concern a bare `split-words`
/// call site would otherwise have to manage by hand). Each expected token
/// list is hand-written from the literal input.
/// `bid` only exists as a dispatched command's own injected leading
/// parameter, unavailable during init evaluation, so this runs the
/// assertions from inside a typed command (`:probe-split-words`), the same
/// shape `core_stdlib_resolve_lang_arg_falls_back_then_warns` uses above.
#[test]
fn core_stdlib_split_words_uses_the_buffers_own_word_chars() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();

    let define_probe = r#"
(define-typed-command! "probe-split-words" ""
  (lambda (bid)
    (unless (equal? (call! "stdlib/split-words" bid "foo-bar baz")
                    (list "foo" "bar" "baz"))
      (error "split-words: default word-chars must split on '-'"))
    (set-buffer-option! bid "word-chars" "-")
    (unless (equal? (call! "stdlib/split-words" bid "foo-bar baz")
                    (list "foo-bar" "baz"))
      (error "split-words: must pick up the buffer's own word-chars setting"))))
"#;
    {
        let mut ih = init_host!(ed);
        host.eval_source(define_probe, &mut ih)
            .expect("define probe-split-words command");
    }
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":probe-split-words");

    let errors = error_log(&ed);
    assert!(
        errors.is_empty(),
        "stdlib/split-words assertions must all pass: {errors:?}"
    );
}

/// `stdlib/git-repo?` and `stdlib/git-toplevel` must both report true/the real
/// root from inside a work tree, even when the editor's cwd is a
/// subdirectory of it, the case that actually exercises `--show-toplevel`
/// rather than just echoing cwd back.
///
/// The expected root is `sandbox.path()`, the tempdir's
/// own canonicalized path, asserted literally, never re-derived through
/// either command under test.
#[test]
fn core_stdlib_git_probes_inside_a_work_tree() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();
    let sandbox = Sandbox::new();
    git_init(sandbox.raw());
    std::fs::create_dir_all(sandbox.raw().join("sub")).unwrap();
    ed.state
        .set_cwd(&hume_platform::path::AbsolutePath::rooted(
            sandbox.path().join("sub"),
        ))
        .unwrap();

    let root = steel_path(&sandbox.path());
    let assertions = format!(
        r#"
(unless (equal? (call! "stdlib/git-repo?") #t)
  (error "git-repo? must be #t inside a work tree"))
(unless (equal? (call! "stdlib/git-toplevel") {root})
  (error (string-append "git-toplevel must be the repo root, got "
                         (to-string (call! "stdlib/git-toplevel")))))
"#
    );

    let result = {
        let mut ih = init_host!(ed);
        host.eval_source(&assertions, &mut ih)
    };
    assert!(
        result.is_ok(),
        "stdlib git probe assertions must pass inside a work tree: {result:?}"
    );
}

/// Outside any git work tree, both probes must return `#f` rather than
/// raising or reporting a stale/wrong root. `stdlib/git-repo?` is a
/// registered command, so its not-a-repo branch is reachable via `call!`
/// here, unlike `core:pickers`' own `pickers/files`, which has no such seam
/// (see `pickers_plugin.rs`'s module doc comment).
#[test]
fn core_stdlib_git_probes_outside_a_work_tree() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();
    let sandbox = Sandbox::new();
    ed.state
        .set_cwd(&hume_platform::path::AbsolutePath::rooted(sandbox.path()))
        .unwrap();

    let assertions = r#"
(unless (equal? (call! "stdlib/git-repo?") #f)
  (error "git-repo? must be #f outside a work tree"))
(unless (equal? (call! "stdlib/git-toplevel") #f)
  (error "git-toplevel must be #f outside a work tree"))
"#;

    let result = {
        let mut ih = init_host!(ed);
        host.eval_source(assertions, &mut ih)
    };
    assert!(
        result.is_ok(),
        "stdlib git probe assertions must pass outside a work tree: {result:?}"
    );
}

/// `stdlib/report-section` renders a counted header, a hint only when there
/// are items, then one indented line per item or `none`.
#[test]
fn core_stdlib_report_section_command() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();
    let mut section = |args: &str| {
        let mut ih = init_host!(ed);
        match host.eval_source_for_value(
            &format!(r#"(call! "stdlib/report-section" {args})"#),
            &mut ih,
        ) {
            Ok(steel::rvals::SteelVal::StringV(s)) => s.to_string(),
            other => panic!("report-section {args} must return a string: {other:?}"),
        }
    };

    insta::assert_snapshot!(section(r#""Fruit" '("apple" "pear") "eat them""#), @"
    Fruit (2), eat them
      apple
      pear
    ");
    insta::assert_snapshot!(section(r#""Fruit" '("apple") #f"#), @"
    Fruit (1)
      apple
    ");
    insta::assert_snapshot!(section(r#""Fruit" '() "eat them""#), @"
    Fruit (0)
      none
    ");
}

// ── The markdown-heading example in writing-plugins.md ───────────────────────

/// The `markdown-heading` command `writing-plugins.md` shows, with its
/// binding for Markdown buffers: rewrite the line of each selection's anchor
/// as a heading whose level is the count.
const MARKDOWN_HEADING: &str = r#"
(define (heading-prefix-length line)
  (let* ((n (string-length line))
         (skip (lambda (i c)
                 (let loop ((i i))
                   (if (and (< i n) (char=? (string-ref line i) c))
                       (loop (+ i 1))
                       i)))))
    (skip (skip 0 #\#) #\space)))

(define (anchor-lines pane)
  (let loop ((sels (buffer-selections pane)) (lines '()))
    (if (null? sels)
        (reverse lines)
        (let ((line (char-offset->line pane (selection-anchor (car sels)))))
          (loop (cdr sels)
                (if (and (pair? lines) (= line (car lines)))
                    lines
                    (cons line lines)))))))

(define-command! "markdown-heading"
  "Turn the line of each selection's anchor into a Markdown heading. The count sets the level."
  (lambda (pane count)
    (let ((prefix (string-append (make-string (min 6 (max 1 count)) #\#) " ")))
      (edit-text! pane
        (filter (lambda (edit) edit)
          (map (lambda (line)
                 (let ((text (car (buffer-lines pane #:range (hash 'start-line line 'end-line (+ line 1))))))
                   (and (> (string-length text) 0)
                        (let ((start (line->char-offset pane line)))
                          (hash 'start-char start
                                'end-char (+ start (heading-prefix-length text))
                                'text prefix)))))
               (anchor-lines pane)))))))

(bind-key! 'normal "G H" "markdown-heading" #:keymap "markdown-keys")

(register-hook! 'on-language-set
  (lambda (pane lang)
    (if (equal? lang "markdown")
        (attach-keymap! pane "markdown-keys")
        (detach-keymap! pane "markdown-keys"))))
"#;

/// An editor on `input` in a `language` buffer, with the shipped
/// `core:stdlib` and the example loaded the way a user's `init.scm` would
/// load them, after `keys` are pressed. The returned guards keep the runtime
/// and init directories alive.
fn heading_editor_after(
    language: &str,
    input: &str,
    keys: &str,
) -> (Editor, RuntimeDirs, tempfile::TempDir) {
    let guard = RuntimeDirs::new();
    write_core_plugin(&guard, "stdlib", STDLIB_PLUGIN);
    let mut ed = editor_from(input);
    ed.state.dirs = guard.dirs();
    let init_dir = tempfile::tempdir().unwrap();
    run(
        &mut ed,
        init_dir.path(),
        &format!("(load-plugin! \"core:stdlib\")\n{MARKDOWN_HEADING}"),
    );
    let bid = ed.focused_buffer_id();
    let language = ed.state.config.languages.intern(language);
    ed.set_buffer_language(bid, Some(language));
    ed.settle();
    for ch in keys.chars() {
        ed.feed_key(key(ch));
    }
    (ed, guard, init_dir)
}

fn heading_after(input: &str, keys: &str) -> String {
    heading_after_in("markdown", input, keys)
}

fn heading_after_in(language: &str, input: &str, keys: &str) -> String {
    let (ed, _guard, _init_dir) = heading_editor_after(language, input, keys);
    ed.state
        .buffers
        .get(ed.focused_buffer_id())
        .text()
        .to_string()
}

/// The marked state after `keys`, in a Markdown buffer.
fn heading_state_after(input: &str, keys: &str) -> String {
    let (ed, _guard, _init_dir) = heading_editor_after("markdown", input, keys);
    state(&ed)
}

#[test]
fn markdown_heading_leaves_the_cursor_on_its_character() {
    assert_eq!(
        heading_state_after("f-[o]>o\nbar\n", "GH"),
        "# f-[o]>o\nbar\n"
    );
}

#[test]
fn markdown_heading_on_the_last_line_leaves_the_cursor_on_it() {
    assert_eq!(
        heading_state_after("foo\n-[b]>ar\n", "GH"),
        "foo\n# -[b]>ar\n"
    );
}

#[test]
fn markdown_heading_leaves_each_cursor_on_its_own_heading() {
    assert_eq!(
        heading_state_after("-{f}>oo\nbar\n-[b]>az\n", "2GH"),
        "## -{f}>oo\nbar\n## -[b]>az\n"
    );
}

#[test]
fn markdown_heading_keeps_a_cursor_on_the_old_prefix_at_the_line_start() {
    assert_eq!(heading_state_after("-[#]> foo\n", "2GH"), "-[#]># foo\n");
}

#[test]
fn markdown_heading_without_a_count_makes_a_level_one_heading() {
    assert_eq!(heading_after("-[f]>oo\n", "GH"), "# foo\n");
}

#[test]
fn markdown_heading_replaces_the_existing_level() {
    assert_eq!(heading_after("-[#]>## foo\n", "2GH"), "## foo\n");
}

#[test]
fn markdown_heading_handles_headings_and_plain_lines_together() {
    assert_eq!(
        heading_after("-{f}>oo\n#### -[b]>ar\n", "3GH"),
        "### foo\n### bar\n"
    );
}

#[test]
fn markdown_heading_caps_the_level_at_six() {
    assert_eq!(heading_after("-[f]>oo\n", "9GH"), "###### foo\n");
}

#[test]
fn markdown_heading_changes_only_the_line_of_the_anchor() {
    assert_eq!(heading_after("a -[bc\nd]>e\nf\n", "GH"), "# a bc\nde\nf\n");
}

#[test]
fn markdown_heading_leaves_an_empty_line_alone() {
    assert_eq!(heading_after("-{\n}>-[f]>oo\n", "GH"), "\n# foo\n");
}

#[test]
fn markdown_heading_with_two_cursors_on_one_line_writes_one_heading() {
    assert_eq!(heading_after("-{f}>o-[o]>\n", "2GH"), "## foo\n");
}

#[test]
fn markdown_heading_keys_do_nothing_outside_markdown() {
    assert_eq!(heading_after_in("rust", "-[f]>oo\n", "GH"), "foo\n");
}

#[test]
fn markdown_heading_is_undone_by_one_undo() {
    assert_eq!(
        heading_after("-{f}>oo\n## -[b]>ar\n", "2GHu"),
        "foo\n## bar\n"
    );
}

/// Deleting a large tree is waiting on the OS, which the budget does not
/// count: `stdlib/delete-dir!` removes a tree that takes longer to delete
/// than its budget.
#[test]
fn delete_dir_does_not_count_the_delete_against_the_budget() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();
    let budget = std::time::Duration::from_millis(250);

    test_fixtures::slow_delete::assert_delete_outlasts_budget(budget, |root| {
        let source = format!(r#"(call! "stdlib/delete-dir!" {})"#, steel_path(root));
        let mut ih = init_host!(ed);
        host.eval_source_watchdog(&source, budget, &mut ih)
    });
}

/// `stdlib/delete-dir!` on a directory that does not exist does nothing.
#[test]
fn delete_dir_of_a_missing_directory_is_a_no_op() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("missing");

    let source = format!(r#"(call! "stdlib/delete-dir!" {})"#, steel_path(&missing));
    let result = {
        let mut ih = init_host!(ed);
        host.eval_source(&source, &mut ih)
    };

    assert!(result.is_ok(), "{result:?}");
}

/// `stdlib/verify-sha256!` accepts the file's digest in either case and
/// raises on any other. The expected digest of `abc` is the FIPS 180-2 test
/// vector, not computed here.
#[test]
fn core_stdlib_verify_sha256_compares_case_insensitively() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("abc.txt");
    std::fs::write(&file, b"abc").unwrap();
    let path = steel_path(&file);

    let assertions = format!(
        r#"
(call! "stdlib/verify-sha256!" {path}
       "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD")
(let ((raised (with-handler (lambda (err) (to-string err))
                (call! "stdlib/verify-sha256!" {path}
                       "0000000000000000000000000000000000000000000000000000000000000000")
                #f)))
  (unless (and raised (string-contains? raised "sha256 mismatch"))
    (error (string-append "a wrong digest must raise a mismatch, got " (to-string raised)))))
"#
    );

    let result = {
        let mut ih = init_host!(ed);
        host.eval_source(&assertions, &mut ih)
    };
    assert!(result.is_ok(), "{result:?}");
}

/// Off Windows, `stdlib/find-extractor` finds `tar` for a tar archive and
/// `unzip` for a zip archive on the search path, and raises for a format it
/// does not extract.
#[test]
fn core_stdlib_find_extractor_resolves_through_the_search_path() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();

    let assertions = r#"
(for-each
  (lambda (pair)
    (let ((found (call! "stdlib/find-extractor" (car pair))))
      (unless (and (string? found) (equal? found (find-program (cdr pair))))
        (error (string-append (car pair) " resolved to " (to-string found))))))
  (list (cons "tar" "tar") (cons "zip" "unzip")))
(let ((raised (with-handler (lambda (err) (to-string err))
                (call! "stdlib/find-extractor" "rar")
                #f)))
  (unless (and raised (string-contains? raised "rar"))
    (error (string-append "an unknown format must raise naming it, got " (to-string raised)))))
"#;

    let result = {
        let mut ih = init_host!(ed);
        host.eval_source(assertions, &mut ih)
    };
    assert!(result.is_ok(), "{result:?}");
}

/// `stdlib/find-extractor` is `#f` for both formats when the search path
/// holds no extractor.
#[test]
fn core_stdlib_find_extractor_is_false_with_no_extractor() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();
    let empty_path_dir = tempfile::tempdir().unwrap();
    ed.state.dirs.search_path = Some(empty_path_dir.path().into());

    let assertions = r#"
(for-each
  (lambda (format)
    (let ((found (call! "stdlib/find-extractor" format)))
      (when found
        (error (string-append format " resolved to " (to-string found))))))
  (list "tar" "zip"))
"#;

    let result = {
        let mut ih = init_host!(ed);
        host.eval_source(assertions, &mut ih)
    };
    assert!(result.is_ok(), "{result:?}");
}

/// `stdlib/download!` fetches a whole file or a byte range of one with
/// `curl`, failing on an HTTP error and never reading the URL as an option,
/// and refuses a range that is not an offset and a size.
#[test]
fn core_stdlib_download_runs_curl_for_a_file_or_a_range() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();
    let work = tempfile::tempdir().unwrap();
    let argv_log = work.path().join("argv");
    hume_platform::path::test_support::write_script(
        work.path(),
        "curl",
        &format!(
            "PATH=/usr/bin:/bin\n\
             printf '%s\\n' \"$@\" END >> '{log}'\n\
             out=''; prev=''\n\
             for a in \"$@\"; do [ \"$prev\" = -o ] && out=\"$a\"; prev=\"$a\"; done\n\
             printf x > \"$out\"\n",
            log = argv_log.display(),
        ),
        0o755,
    );
    ed.state.dirs.search_path = Some(format!("{}:/usr/bin:/bin", work.path().display()).into());
    let whole = work.path().join("whole");
    let part = work.path().join("part");

    let define_probe = format!(
        r#"
(define-typed-command! "probe-download" ""
  (lambda ()
    (call! "stdlib/download!" "https://example.invalid/a" {whole})
    (call! "stdlib/download!" "https://example.invalid/b" {part} 3 4)
    (let ((raised #f))
      (with-handler (lambda (err) (set! raised #t))
        (call! "stdlib/download!" "https://example.invalid/c" {part} 3))
      (unless raised
        (error "a range of one number must raise"))))
  #:inline-output #t)
"#,
        whole = steel_path(&whole),
        part = steel_path(&part),
    );
    {
        let mut ih = init_host!(ed);
        host.eval_source(&define_probe, &mut ih)
            .expect("define probe-download command");
    }
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":probe-download");

    assert_eq!(error_log(&ed), Vec::<String>::new());
    let whole = whole.display().to_string();
    let part = part.display().to_string();
    assert_eq!(
        std::fs::read_to_string(&argv_log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        [
            "-fsSL",
            "-o",
            &whole,
            "--",
            "https://example.invalid/a",
            "END",
            "-fsSL",
            "-r",
            "3-6",
            "-o",
            &part,
            "--",
            "https://example.invalid/b",
            "END",
        ]
    );
}

/// `stdlib/extract-archive!` creates the target directory, extracts a
/// gzip-compressed tar into it, and returns the extracted regular files.
#[test]
fn core_stdlib_extract_archive_returns_the_extracted_files() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();
    let work = tempfile::tempdir().unwrap();
    let payload = work.path().join("payload");
    std::fs::create_dir_all(payload.join("queries")).unwrap();
    std::fs::write(payload.join("queries/highlights.scm"), b"(x) @y\n").unwrap();
    std::fs::write(payload.join("manifest.scm"), b"()\n").unwrap();
    let archive = work.path().join("member.tar.gz");
    tar_gz(&payload, &["manifest.scm", "queries"], &archive);
    let dest = work.path().join("out/member");

    let define_probe = format!(
        r#"
(define-typed-command! "probe-extract" ""
  (lambda ()
    (let ((names (sort (map file-name
                            (call! "stdlib/extract-archive!" "tar" {archive} {dest}))
                       string<?)))
      (unless (equal? names (list "highlights.scm" "manifest.scm"))
        (error (string-append "extract-archive! returned " (to-string names))))))
  #:inline-output #t)
"#,
        archive = steel_path(&archive),
        dest = steel_path(&dest),
    );
    {
        let mut ih = init_host!(ed);
        host.eval_source(&define_probe, &mut ih)
            .expect("define probe-extract command");
    }
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":probe-extract");

    assert_eq!(error_log(&ed), Vec::<String>::new());
    assert_eq!(
        std::fs::read_to_string(dest.join("queries/highlights.scm")).unwrap(),
        "(x) @y\n"
    );
}
