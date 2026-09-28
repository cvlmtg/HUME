//! core:stdlib: real shipped plugin.

use super::*;
use hume_scripting::PluginStatus;

// ── core:stdlib — real shipped plugin ─────────────────────────────────────────

/// Stage the real shipped `core:stdlib` plugin into an isolated `HUME_RUNTIME`
/// and eagerly load it via a real `init.scm`, returning everything the caller
/// needs kept alive plus the host for further inspection or `eval_source`.
fn setup_stdlib_editor() -> (Editor, ScriptingHost, HumeRuntimeGuard, tempfile::TempDir) {
    let guard = HumeRuntimeGuard::new();
    write_core_plugin(&guard, "stdlib", STDLIB_PLUGIN);

    let init_dir = safe_tempdir();
    let init_path = init_dir.path().join("init.scm");
    std::fs::write(&init_path, r#"(load-plugin! "core:stdlib")"#).unwrap();

    let mut ed = editor_from("-[a]>b\n");
    let mut host = ScriptingHost::new();
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed loading core:stdlib");

    (ed, host, guard, init_dir)
}

/// The shipped `core:stdlib` plugin must load eagerly and reach `Loaded`.
#[test]
fn core_stdlib_plugin_loads_eagerly() {
    use hume_scripting::attribution::PluginId;

    let (_ed, host, _guard, _init_dir) = setup_stdlib_editor();

    let id = PluginId::parse("core:stdlib").expect("valid plugin name");
    assert_eq!(
        host.plugin_status(&id),
        Some(PluginStatus::Loaded),
        "core:stdlib must be Loaded after eager load-plugin!"
    );
}

/// The `core:stdlib` selection-query commands must compute the expected
/// results on literal selection-list arguments via `call!`, and pass `#f`
/// straight through untouched: the same cross-plugin surface
/// `core:vim-keybind` uses for its conditional `C` binding.
///
/// Each assertion compares against a hand-written literal tuple, independent
/// of the implementation. If any command computes the wrong result, its `unless`
/// fires `(error ...)`, which propagates as an `Err`, caught by the assert
/// below, failing the test with the offending assertion name.
///
/// `stdlib`'s per-triple accessors (`selection-anchor`/`-head`/`-primary?`,
/// `primary-selection`) are `call!`-reachable public commands, same as the
/// three list-level predicates. A plugin holding a single selection triple
/// (not a list) needs them directly rather than picking it apart with raw
/// `car`/`cadr`/`caddr`.
#[test]
fn core_stdlib_selection_commands() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();

    let assertions = r#"
(unless (equal? (call! "stdlib/all-single-char?" #f) #f) (error "all-single-char? #f passthrough"))
(unless (equal? (call! "stdlib/single-selection?" #f) #f) (error "single-selection? #f passthrough"))
(unless (equal? (call! "stdlib/cursor-char-index" #f) #f) (error "cursor-char-index #f passthrough"))
(unless (equal? (call! "stdlib/selection-anchor" #f) #f) (error "selection-anchor #f passthrough"))
(unless (equal? (call! "stdlib/selection-head" #f) #f) (error "selection-head #f passthrough"))
(unless (equal? (call! "stdlib/selection-primary?" #f) #f) (error "selection-primary? #f passthrough"))
(unless (equal? (call! "stdlib/primary-selection" #f) #f) (error "primary-selection #f passthrough"))

(unless (equal? (call! "stdlib/single-selection?" (list (list 0 1 #t))) #t)
  (error "single-selection? true"))
(unless (equal? (call! "stdlib/single-selection?" (list (list 0 1 #t) (list 2 3 #f))) #f)
  (error "single-selection? false"))

(unless (equal? (call! "stdlib/all-single-char?" (list (list 2 2 #t) (list 5 5 #f))) #t)
  (error "all-single-char? true"))
(unless (equal? (call! "stdlib/all-single-char?" (list (list 2 3 #t))) #f)
  (error "all-single-char? false"))
(unless (equal? (call! "stdlib/all-single-char?" (list (list 2 2 #f) (list 5 6 #t))) #f)
  (error "all-single-char? false when only a later selection is wide"))

(unless (equal? (call! "stdlib/cursor-char-index" (list (list 0 0 #f) (list 7 4 #t))) 4)
  (error "cursor-char-index picks the primary's head"))

(unless (equal? (call! "stdlib/selection-anchor" (list 3 9 #t)) 3)
  (error "selection-anchor reads the triple's first field"))
(unless (equal? (call! "stdlib/selection-head" (list 3 9 #t)) 9)
  (error "selection-head reads the triple's second field"))
(unless (equal? (call! "stdlib/selection-primary?" (list 3 9 #t)) #t)
  (error "selection-primary? true"))
(unless (equal? (call! "stdlib/selection-primary?" (list 3 9 #f)) #f)
  (error "selection-primary? false"))
(unless (equal? (call! "stdlib/primary-selection" (list (list 0 0 #f) (list 7 4 #t) (list 1 1 #f))) (list 7 4 #t))
  (error "primary-selection picks the flagged triple out of a list"))
(unless (equal? (call! "stdlib/primary-selection" (list (list 0 0 #f) (list 1 1 #f))) #f)
  (error "primary-selection #f when no triple is flagged"))
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

/// `stdlib/safe-path-segment?` (the merged `core:plum`/`core:lsp`
/// path-segment predicate) must reject every unsafe input (empty, `.`,
/// `..`, a path separator, `:`, `"`, NUL) and accept ordinary names.
/// Each literal input/expected pair is hand-picked, not
/// derived from the implementation, mirroring `hume-platform/src/path/tests.rs`'s
/// `is_safe_segment` coverage for the Rust copy.
#[test]
fn core_stdlib_safe_path_segment_command() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();

    let assertions = r#"
(unless (equal? (call! "stdlib/safe-path-segment?" "") #f) (error "empty string rejected"))
(unless (equal? (call! "stdlib/safe-path-segment?" ".") #f) (error "dot rejected"))
(unless (equal? (call! "stdlib/safe-path-segment?" "..") #f) (error "dotdot rejected"))
(unless (equal? (call! "stdlib/safe-path-segment?" "a/b") #f) (error "forward slash rejected"))
(unless (equal? (call! "stdlib/safe-path-segment?" "a\\b") #f) (error "backslash rejected"))
(unless (equal? (call! "stdlib/safe-path-segment?" "c:evil") #f) (error "colon rejected"))
(unless (equal? (call! "stdlib/safe-path-segment?" "a\"b") #f) (error "quote rejected"))
(unless (equal? (call! "stdlib/safe-path-segment?" "a\0b") #f) (error "nul rejected"))

(unless (equal? (call! "stdlib/safe-path-segment?" "v1.2.3") #t) (error "version string accepted"))
(unless (equal? (call! "stdlib/safe-path-segment?" "rust-analyzer") #t) (error "plain name accepted"))

;; A non-string answers #f rather than raising from inside string-contains?.
;; It's a public command, so a plugin author can reach it with anything.
(unless (equal? (call! "stdlib/safe-path-segment?" 42) #f) (error "number rejected"))
(unless (equal? (call! "stdlib/safe-path-segment?" #f) #f) (error "boolean rejected"))
"#;

    let result = {
        let mut ih = init_host!(ed);
        host.eval_source(assertions, &mut ih)
    };
    assert!(
        result.is_ok(),
        "stdlib/safe-path-segment? assertions must all pass: {result:?}"
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
/// if it doesn't name what's expected, raises a *new* `(error ...)`, never
/// `(raise-error err)` on the caught value itself, which corrupts the VM's
/// continuation stack when a native-builtin error crosses a second
/// with-handler (see `hume-scripting/src/tests/unix.rs`).
#[test]
fn core_stdlib_config_commands() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();

    let assertions = r#"
(unless (equal? (call! "stdlib/config-boolean" "p" (hash "k" #f) "k" #t) #f)
  (error "config-boolean: present key wins over default"))
(unless (equal? (call! "stdlib/config-boolean" "p" (hash) "k" #t) #t)
  (error "config-boolean: absent key falls back to default"))
(with-handler
  (lambda (err)
    (unless (and (string-contains? (to-string err) "p") (string-contains? (to-string err) "k"))
      (error (string-append "config-boolean: error must name plugin and key: " (to-string err)))))
  (begin
    (call! "stdlib/config-boolean" "p" (hash "k" "not-a-bool") "k" #t)
    (error "config-boolean: expected a raise on a non-boolean value")))

(unless (equal? (call! "stdlib/config-string" "p" (hash "k" "custom") "k" "default") "custom")
  (error "config-string: present key wins over default"))
(unless (equal? (call! "stdlib/config-string" "p" (hash) "k" "default") "default")
  (error "config-string: absent key falls back to default"))
(with-handler
  (lambda (err)
    (unless (and (string-contains? (to-string err) "p") (string-contains? (to-string err) "k"))
      (error (string-append "config-string: error must name plugin and key: " (to-string err)))))
  (begin
    (call! "stdlib/config-string" "p" (hash "k" 'not-a-string) "k" "default")
    (error "config-string: expected a raise on a non-string value")))

(unless (equal? (call! "stdlib/config-enum" "p" (hash "k" 'on) "k" 'smart '(on smart off)) 'on)
  (error "config-enum: present key wins over default"))
(unless (equal? (call! "stdlib/config-enum" "p" (hash) "k" 'smart '(on smart off)) 'smart)
  (error "config-enum: absent key falls back to default"))
(with-handler
  (lambda (err)
    (unless (and (string-contains? (to-string err) "p") (string-contains? (to-string err) "k"))
      (error (string-append "config-enum: error must name plugin and key: " (to-string err)))))
  (begin
    (call! "stdlib/config-enum" "p" (hash "k" 'bogus) "k" 'smart '(on smart off))
    (error "config-enum: expected a raise on a disallowed value")))

(unless (equal? (call! "stdlib/config-integer" "p" (hash "k" 5) "k" 1 0) 5)
  (error "config-integer: present key wins over default"))
(unless (equal? (call! "stdlib/config-integer" "p" (hash) "k" 1 0) 1)
  (error "config-integer: absent key falls back to default"))
(unless (equal? (call! "stdlib/config-integer" "p" (hash "k" -5) "k" 1 #f) -5)
  (error "config-integer: negative value allowed when minimum is #f"))
(with-handler
  (lambda (err)
    (unless (and (string-contains? (to-string err) "p") (string-contains? (to-string err) "k"))
      (error (string-append "config-integer: error must name plugin and key: " (to-string err)))))
  (begin
    (call! "stdlib/config-integer" "p" (hash "k" "not-an-int") "k" 1 0)
    (error "config-integer: expected a raise on a non-integer value")))
(with-handler
  (lambda (err)
    (unless (and (string-contains? (to-string err) "p") (string-contains? (to-string err) "k"))
      (error (string-append "config-integer: error must name plugin and key: " (to-string err)))))
  (begin
    (call! "stdlib/config-integer" "p" (hash "k" -1) "k" 1 0)
    (error "config-integer: expected a raise on a value below the minimum")))

(unless (equal? (call! "stdlib/config-list" "p" (hash "k" (list "a" "b")) "k" '() ) (list "a" "b"))
  (error "config-list: present key wins over default"))
(unless (equal? (call! "stdlib/config-list" "p" (hash) "k" (list "d")) (list "d"))
  (error "config-list: absent key falls back to default"))
(unless (equal? (call! "stdlib/config-list" "p" (hash "k" '()) "k" (list "d")) '())
  (error "config-list: empty list accepted"))
(with-handler
  (lambda (err)
    (unless (and (string-contains? (to-string err) "p") (string-contains? (to-string err) "k"))
      (error (string-append "config-list: error must name plugin and key: " (to-string err)))))
  (begin
    (call! "stdlib/config-list" "p" (hash "k" "not-a-list") "k" '())
    (error "config-list: expected a raise on a non-list value")))
(with-handler
  (lambda (err)
    (unless (and (string-contains? (to-string err) "p") (string-contains? (to-string err) "k"))
      (error (string-append "config-list: error must name plugin and key: " (to-string err)))))
  (begin
    (call! "stdlib/config-list" "p" (hash "k" (list "a" 'not-a-string)) "k" '())
    (error "config-list: expected a raise on a non-string element")))
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

    let scan_dir = safe_tempdir();
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

/// `stdlib/write-file` must replace a file's entire content, not just
/// overwrite its leading bytes: writing a shorter string over a longer
/// existing file must not leave the old tail behind.
///
/// The file is read back with `std::fs::read_to_string`
/// on the Rust side, not through any Steel port primitive `write-file`
/// itself might share a bug with.
#[test]
fn core_stdlib_write_file_truncates_existing_content() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();

    let scan_dir = safe_tempdir();
    let target = scan_dir.path().join("target.txt");
    let path = steel_path(&target);

    let assertions = format!(
        r#"
(call! "stdlib/write-file" {path} "long-content")
(call! "stdlib/write-file" {path} "short")
"#
    );

    let result = {
        let mut ih = init_host!(ed);
        host.eval_source(&assertions, &mut ih)
    };
    assert!(
        result.is_ok(),
        "stdlib/write-file assertions must pass: {result:?}"
    );

    let written = std::fs::read_to_string(&target).unwrap();
    assert_eq!(
        written, "short",
        "stdlib/write-file must truncate, not leave the previous content's tail behind"
    );
}

/// `stdlib/run` must cover all three subprocess outcomes it promises:
/// success with captured stdout, a nonzero exit with captured stderr, and a
/// spawn failure (nonexistent binary) reporting exit-code `#f` with the
/// failure reason standing in for stderr.
///
/// Each case's expected shape is asserted directly
/// against the real `sh`/nonexistent-binary spawn, not against any of
/// `stdlib/run`'s own internals.
#[test]
fn core_stdlib_run_covers_success_failure_and_spawn_error() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();

    let assertions = r#"
(let ([r (call! "stdlib/run" "echo" (list "hello-world"))])
  (unless (and (string-contains? (car r) "hello-world") (equal? (caddr r) 0))
    (error (string-append "stdlib/run success case: " (to-string r)))))

(let ([r (call! "stdlib/run" "sh" (list "-c" "echo err-msg 1>&2; exit 3"))])
  (unless (and (string-contains? (cadr r) "err-msg") (equal? (caddr r) 3))
    (error (string-append "stdlib/run nonzero-exit case: " (to-string r)))))

(let ([r (call! "stdlib/run" "hume-definitely-not-a-real-binary-xyz" '())])
  (unless (not (caddr r))
    (error (string-append "stdlib/run spawn-failure case: " (to-string r)))))

(let ([r (call! "stdlib/run" "pwd" '() #:cwd "/")])
  (unless (equal? (trim (car r)) "/")
    (error (string-append "stdlib/run #:cwd case: " (to-string r)))))
"#;

    let result = {
        let mut ih = init_host!(ed);
        host.eval_source(assertions, &mut ih)
    };
    assert!(
        result.is_ok(),
        "stdlib/run assertions must pass: {result:?}"
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
/// The buffer's language is set via `set-buffer-language!`
/// (a command already covered elsewhere), and the expected fallback value is
/// asserted literally, not derived from `stdlib/resolve-lang-arg` itself.
#[test]
fn core_stdlib_resolve_lang_arg_falls_back_then_warns() {
    use crate::editor::Severity;

    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();

    let define_probe = r#"
(define-typed-command! "probe-resolve-lang-arg" ""
  (lambda (bid)
    (unless (equal? (call! "stdlib/resolve-lang-arg" bid "probe-cmd" "rust") "rust")
      (error "resolve-lang-arg: typed string argument must win"))
    (unless (equal? (call! "stdlib/resolve-lang-arg" bid "probe-cmd" 1) #f)
      (error "resolve-lang-arg: no typed arg and no buffer language must return #f"))
    (set-buffer-language! bid "python")
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

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        errors.is_empty(),
        "resolve-lang-arg assertions must all pass: {errors:?}"
    );
    // Boundary condition, not a failure: Severity::Info, statusline only:
    // the third `resolve-lang-arg` call (after `set-buffer-language!`)
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

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == crate::editor::Severity::Error)
        .map(|e| e.text.clone())
        .collect();
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
    let sandbox = CwdSandbox::new();
    git_init(sandbox.raw());
    std::fs::create_dir_all(sandbox.raw().join("sub")).unwrap();
    ed.set_cwd(&sandbox.path().join("sub")).unwrap();

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
/// here, unlike `core:pickers`' own `picker-files`, which has no such seam
/// (see `pickers_plugin.rs`'s module doc comment).
#[test]
fn core_stdlib_git_probes_outside_a_work_tree() {
    let (mut ed, mut host, _guard, _init_dir) = setup_stdlib_editor();
    let sandbox = CwdSandbox::new();
    ed.set_cwd(&sandbox.path()).unwrap();

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
