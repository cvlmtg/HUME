use super::*;

/// Load the real `core:plum` plugin (the actual `runtime/plugins/core/plum/`
/// sources, not a synthetic copy) plus its `core:stdlib` dependency
/// (`plum/fetch-query!` etc. call `stdlib/find`/`stdlib/write-file`/
/// `stdlib/delete-dir`/`stdlib/delete-file` via `call!`) into `ed`, pointing
/// `HUME_RUNTIME` at the repo's real `runtime/` dir (so the real
/// `grammar-sources.scm` catalog is used) and `XDG_DATA_HOME` at `data_dir`.
/// Env vars are process-global — callers must hold a
/// `TEST_GLOBALS.claim(Global::Env)` for the test's duration.
fn load_plum(ed: &mut Editor, data_dir: &std::path::Path) {
    let repo_runtime_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("runtime");
    let config_tmp = safe_tempdir();
    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(
        hume_config.join("init.scm"),
        "(load-plugin \"core:stdlib\")\n(load-plugin \"core:plum\")",
    )
    .unwrap();

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", config_tmp.path());
        std::env::set_var("HUME_RUNTIME", &repo_runtime_dir);
        std::env::set_var("XDG_DATA_HOME", data_dir);
    }
    ed.init_scripting(&mut Default::default());
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HUME_RUNTIME");
        std::env::remove_var("XDG_DATA_HOME");
    }
}

/// Load the real `core:lsp` plugin (plus its `core:stdlib` dependency) —
/// `:lsp-servers` (the `#:inline-output` command exercised below) lives
/// there, not in core:plum. Twin of `load_plum` above, different init.scm.
fn load_lsp(ed: &mut Editor, data_dir: &std::path::Path) {
    let repo_runtime_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("runtime");
    let config_tmp = safe_tempdir();
    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(
        hume_config.join("init.scm"),
        "(load-plugin \"core:stdlib\")\n(load-plugin \"core:lsp\")",
    )
    .unwrap();

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", config_tmp.path());
        std::env::set_var("HUME_RUNTIME", &repo_runtime_dir);
        std::env::set_var("XDG_DATA_HOME", data_dir);
    }
    ed.init_scripting(&mut Default::default());
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HUME_RUNTIME");
        std::env::remove_var("XDG_DATA_HOME");
    }
}

/// Core's `register-installed-grammars!` (`runtime/scheme/grammars.scm`)
/// already ran before PLUM ever loaded (`init_scripting` evaluates it
/// unconditionally) — walked the empty `<data>/grammars/`, found nothing,
/// and never touched the catalog at all. This test then loads `core:plum`
/// itself, checking its `grammars.scm` (the install pipeline) compiles
/// cleanly and its bindings resolve against those same core bindings — a
/// pure Scheme-syntax/logic smoke test, not an installation test.
#[test]
fn plum_plugin_loads_with_real_grammar_catalog() {
    let _lock = TEST_GLOBALS.claim(Global::Env);

    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    let errors: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        errors.is_empty(),
        "loading core:plum against the real grammar-sources.scm must not error: {errors:?}"
    );
}

/// `:plum-list-plugins` exercises `plugins.scm`'s `plum/installed-plugins` (built on
/// `core:stdlib`'s `stdlib/list-subdirs`, a Steel `read-dir`-backed helper)
/// against a real (empty) data dir — no network. Pins that plugin discovery
/// via Steel's stdlib process/fs helpers (see `user-manual/docs/plugins.md`'s
/// "Filesystem and processes") works for loading and basic discovery.
#[test]
fn plum_list_runs_with_no_errors_against_empty_data_dir() {
    let _lock = TEST_GLOBALS.claim(Global::Env);

    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-list-plugins");

    let errors: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        errors.is_empty(),
        ":plum-list-plugins against an empty data dir must not error: {errors:?}"
    );
}

/// A stray file directly inside `<data>/plugins/` (e.g. a macOS `.DS_Store`)
/// used to make `plum/installed-plugins` raise: `stdlib/list-subdirs`'s
/// predecessor (the plum-local `plum/valid-dir-entry?`) only filtered `"."`/
/// `".."`, which `read-dir` never returns, so the stray name passed straight
/// through and `read-dir` was then called on it as if it were a "user"
/// directory — `Path::read_dir` on a non-directory errors, and that error
/// propagated uncaught out of `:plum-list-plugins`.
///
/// Fail oracle: revert `stdlib/list-subdirs` to list every entry instead of
/// filtering by `is-dir?` → this test's `errors.is_empty()` fails, catching
/// the same raise a real `.DS_Store` next to an installed plugin used to hit.
#[test]
fn plum_installed_plugins_skips_a_stray_file_in_the_plugins_dir() {
    let _lock = TEST_GLOBALS.claim(Global::Env);

    // `load_plum` points `XDG_DATA_HOME` at `data_tmp`; HUME's data dir is
    // `$XDG_DATA_HOME/hume/` (`hume_platform::dirs::data_dir`), so the plugin
    // walk plum/`plugins-dir` reads is `<data_tmp>/hume/plugins/`.
    let data_tmp = safe_tempdir();
    let plugins_dir = data_tmp.path().join("hume").join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    std::fs::write(plugins_dir.join(".DS_Store"), "").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-list-plugins");

    let errors: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        errors.is_empty(),
        ":plum-list-plugins must skip a stray file in <data>/plugins/, not raise: {errors:?}"
    );
}

/// Run `git` with `args` in `dir`, asserting success — test-setup helper
/// only (builds local origin/clone fixtures), not itself under test.
fn git_ok(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} in {dir:?} failed");
}

/// `:plum-update-plugins` exercises `plum/run!` (Phase 1 helper, now backing
/// `git-pull`'s replacement) against a REAL local git repo — no network. A
/// local "origin" gets a second commit after the "installed" clone is made,
/// then `:plum-update-plugins` must actually run `git pull` (via Steel's
/// `spawn-process` + `with-current-dir`, not the removed `git-pull`
/// builtin) and fast-forward the clone to match.
#[test]
fn plum_update_runs_real_git_pull_against_local_origin() {
    let _lock = TEST_GLOBALS.claim(Global::Env);

    let origin_tmp = safe_tempdir();
    let origin_dir = origin_tmp.path();
    git_ok(origin_dir, &["init", "-q"]);
    git_ok(origin_dir, &["config", "user.email", "test@example.com"]);
    git_ok(origin_dir, &["config", "user.name", "Test"]);
    std::fs::write(origin_dir.join("plugin.scm"), "; v1\n").unwrap();
    git_ok(origin_dir, &["add", "plugin.scm"]);
    git_ok(origin_dir, &["commit", "-q", "-m", "v1"]);

    let data_tmp = safe_tempdir();
    let clone_dir = data_tmp.path().join("hume/plugins/testuser/testrepo");
    std::fs::create_dir_all(clone_dir.parent().unwrap()).unwrap();
    git_ok(
        data_tmp.path(),
        &[
            "clone",
            "-q",
            origin_dir.to_str().unwrap(),
            clone_dir.to_str().unwrap(),
        ],
    );

    // Advance the origin past what the clone has.
    std::fs::write(origin_dir.join("plugin.scm"), "; v2\n").unwrap();
    git_ok(origin_dir, &["add", "plugin.scm"]);
    git_ok(origin_dir, &["commit", "-q", "-m", "v2"]);

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-update-plugins");

    let errors: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        errors.is_empty(),
        ":plum-update-plugins against a local origin must not error: {errors:?}"
    );
    let content = std::fs::read_to_string(clone_dir.join("plugin.scm")).unwrap();
    assert_eq!(
        content, "; v2\n",
        "plum/run!-backed git pull must fast-forward the clone to origin's latest commit"
    );
}

/// `:plum-cleanup-plugins` exercises `plum/delete-dir` (Phase 1 helper, now backing
/// `delete-dir`'s replacement) against a real on-disk orphan plugin — no
/// network. Nothing in `init.scm` declares it, so it's an orphan by
/// definition; `:plum-cleanup-plugins` must remove its directory.
#[test]
fn plum_cleanup_removes_orphan_plugin_directory() {
    let _lock = TEST_GLOBALS.claim(Global::Env);

    let data_tmp = safe_tempdir();
    let orphan_dir = data_tmp.path().join("hume/plugins/testuser/orphanrepo");
    std::fs::create_dir_all(&orphan_dir).unwrap();
    std::fs::write(orphan_dir.join("plugin.scm"), "; orphan\n").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-cleanup-plugins");

    let errors: Vec<&str> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        errors.is_empty(),
        ":plum-cleanup-plugins must not error: {errors:?}"
    );
    assert!(
        !orphan_dir.exists(),
        "plum/delete-dir-backed plum-cleanup-plugins must remove the orphan plugin directory"
    );
}

/// `:plum-install-grammar` with no argument and no buffer language must
/// report core:stdlib's shared "no language given" message
/// (`stdlib/resolve-lang-arg`) in the statusline. A `(equal? name "")` guard
/// is dead here — `name` is `#f`, not `""` — so it must not be relied on to
/// catch this; letting a `#f` name fall through produces an opaque
/// install-failure message instead.
#[test]
fn plum_install_grammar_no_arg_no_language_warns() {
    let _lock = TEST_GLOBALS.claim(Global::Env);

    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-install-grammar");

    // Boundary condition, not a failure — Severity::Info, statusline only,
    // never `:messages` (see Severity's routing table).
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("no language given")),
        "expected 'no language given' status message, got: {:?}",
        ed.state.status_msg
    );
}

/// `:plum-install-grammar nosuchlang` — a name absent from the catalog
/// reports the unknown-grammar message instead of failing deep inside the
/// install pipeline with an opaque hash-lookup error. This validation runs
/// before the stale-source `delete-dir` purge in `plum/install-grammar`, so
/// an unknown name deletes nothing.
#[test]
fn plum_install_grammar_unknown_name_warns() {
    let _lock = TEST_GLOBALS.claim(Global::Env);

    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-install-grammar nosuchlang");

    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains(r#"unknown grammar "nosuchlang""#)),
        "expected unknown-grammar message naming 'nosuchlang', got: {:?}",
        ed.state.status_msg
    );
}

/// A typed argument wins over the current buffer's language.
///
/// Flip: before the arity-1 fix, `plum-install-grammar` was arity-0 so the
/// minibuffer silently dropped `nosuchlang` and the command installed `rust`
/// (the buffer's language) instead — verified by reverting the lambda to
/// arity-0, which made this test fail because it actually ran a real
/// `git-clone-rev`/`curl-fetch`/`compile-grammar!` install of `rust` instead
/// of ever mentioning `nosuchlang`.
#[test]
fn plum_install_grammar_arg_overrides_buffer_language() {
    let _lock = TEST_GLOBALS.claim(Global::Env);

    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":set buffer language=rust");
    type_cmd(&mut ed, ":plum-install-grammar nosuchlang");

    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains(r#"unknown grammar "nosuchlang""#)),
        "typed arg must win over buffer language 'rust', got: {:?}",
        ed.state.status_msg
    );
}

/// `plum-install-grammar` is declared `#:inline-output #t` — dispatch must
/// only bracket it with the real terminal (alt-screen exit + "press any key
/// to return" block) when `Editor::run` owns the terminal. Off the event
/// loop (this test, like every other in this file, dispatches directly and
/// never calls `run`), that bracket must be skipped entirely: otherwise
/// dispatch blocks forever on a keypress that never comes whenever stdin
/// happens to be a real TTY (e.g. `cargo test` run interactively), which is
/// exactly what stalled the suite before the TUI-ownership check was
/// introduced.
///
/// This particular command errors out via `log!` only (no `displayln`), so
/// under the lazy-entry design it never even reaches
/// `ensure_inline_output_screen` — see
/// `inline_output_command_with_real_output_still_skips_bracket_off_event_loop`
/// below for the case that does.
#[test]
fn inline_output_command_does_not_enter_terminal_bracket_off_event_loop() {
    let _lock = TEST_GLOBALS.claim(Global::Env);

    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    assert_eq!(
        ed.inline_output_enter_count(),
        0,
        "bracket must not have fired before any inline-output command ran"
    );

    type_cmd(&mut ed, ":plum-install-grammar nosuchlang");

    assert_eq!(
        ed.inline_output_enter_count(),
        0,
        "inline-output bracket must stay skipped when Editor::run never took the terminal"
    );
}

/// `lsp-servers` is `#:inline-output #t` and *does* print via `displayln`
/// (one line per seeded server) — off the event loop that must still reach
/// `EditorHostImpl::ensure_inline_output_screen`'s no-terminal early return
/// rather than the real terminal: printing must succeed without ever
/// entering the alt-screen.
///
/// Flip: drop the `needs_enter`/`tui` guard so `ensure_inline_output_screen`
/// always enters → this test hangs on `wait_for_keypress` against a real
/// TTY, or panics against a non-TTY stdin in CI.
#[test]
fn inline_output_command_with_real_output_still_skips_bracket_off_event_loop() {
    let _lock = TEST_GLOBALS.claim(Global::Env);

    let data_tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    load_lsp(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":lsp-servers");

    assert_eq!(
        ed.inline_output_enter_count(),
        0,
        "displayln output off the event loop must not enter the real terminal bracket"
    );
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("seeded servers")),
        "lsp-servers must still log its summary line, got: {:?}",
        ed.state.status_msg
    );
}

/// Regression test: a grammar's source dir left non-empty by a prior failed
/// install (clone succeeded, compile didn't) must not break
/// `:plum-install-grammar` on the very next attempt — `plum/install-grammar`
/// purges any existing source dir before re-cloning, so retrying "just works"
/// on the first try instead of requiring a second attempt to clear the
/// leftover directory as a side effect of the first attempt's own failure.
///
/// Hits the network (real git clone + curl fetch + tree-sitter build of the
/// `json` grammar); requires `git`, `curl`, and `tree-sitter` on `PATH`.
///
/// Flip: without the `(delete-dir src-dir)` fix in `plum/install-grammar`,
/// `git-clone-rev` refuses to clone into this pre-seeded non-empty dir and
/// the command logs an error instead of installing — `out_path` never
/// appears, and this assertion fails.
#[test]
fn plum_install_grammar_recovers_from_stale_source_dir_on_first_try() {
    let _lock = TEST_GLOBALS.claim(Global::Env);

    let data_tmp = safe_tempdir();
    // `load_plum` points XDG_DATA_HOME at data_tmp — the real data dir is
    // XDG_DATA_HOME/hume (see dirs.rs's ScriptDirs::new).
    let data_dir = data_tmp.path().join("hume");
    // Seed a stale, non-empty source dir exactly like a prior clone-succeeded/
    // compile-failed install would leave behind — git-clone-rev refuses to
    // clone into this without the pre-clean fix.
    let src_dir = data_dir.join("grammars/sources/json");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("stale.txt"), b"leftover from a failed install").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-install-grammar json");

    let ext = hume_test_fixtures::grammar_platform_ext();
    let out_path = data_dir.join("grammars").join(format!("json.{ext}"));
    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .map(|e| format!("{:?}: {}", e.severity, e.text))
        .collect();
    assert!(
        out_path.exists(),
        "compiled json grammar must exist after install recovers from a stale \
         source dir on the first try; log={errors:#?}"
    );
    assert!(
        !src_dir.join("stale.txt").exists(),
        "stale leftover file must be purged by the pre-clone delete-dir"
    );
}

/// `tsx`'s `highlights.scm` declares `; inherits: ecma,_typescript,_jsx`
/// instead of writing out its own patterns — `plum/install-grammar` must
/// resolve that chain (`plum/resolve-query` in `grammars.scm`) so the file
/// written to disk has real capture patterns, not a dangling directive.
///
/// Hits the network (real git clone + tree-sitter build of the `tsx` grammar,
/// plus curl fetches of its `highlights.scm` and its `ecma`/`_typescript`/
/// `_jsx` query dependencies); requires `git`, `curl`, and `tree-sitter` on
/// `PATH`.
///
/// Flip: reverting the `plum/fetch-query!` call sites back to plain
/// `curl-fetch` leaves `highlights.scm` as the raw `; inherits: …` stub —
/// the `starts_with("; inherits")` and `contains('@')` assertions below both
/// fail on that stub (no `@capture` in a comment-only file).
#[test]
fn plum_install_grammar_resolves_helix_inherits_chain() {
    let _lock = TEST_GLOBALS.claim(Global::Env);

    let data_tmp = safe_tempdir();
    let data_dir = data_tmp.path().join("hume");

    let buf = crate::editor::buffer::Buffer::new(
        hume_editing::text::BufferText::from("const x: number = 1;\n"),
        hume_editing::selection::SelectionSet::default(),
    );
    let mut ed = Editor::for_testing(buf);
    let bid = ed.focused_buffer_id();
    load_plum(&mut ed, data_tmp.path());
    // Real bootstrap loads runtime/scheme/languages.scm (which declares tsx's
    // identity) before any plugin runs; `load_plum` only loads `core:plum`,
    // so register the identity here to match that ordering — `register-grammar!`
    // attaches onto an existing identity, it doesn't create one.
    ed.state
        .config
        .languages
        .register_identity("tsx", &["tsx"], &[], &[], None)
        .unwrap();

    type_cmd(&mut ed, ":plum-install-grammar tsx");

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .map(|e| format!("{:?}: {}", e.severity, e.text))
        .collect();

    let hl_path = data_dir.join("grammars/sources/tsx/highlights.scm");
    let hl_content = std::fs::read_to_string(&hl_path).unwrap_or_else(|e| {
        panic!("highlights.scm must exist after install; log={errors:#?}; err={e}")
    });
    assert!(
        !hl_content.trim_start().starts_with("; inherits"),
        "highlights.scm must be resolved, not left as a dangling inherits stub: {hl_content:?}"
    );
    assert!(
        hl_content.contains('@'),
        "resolved highlights.scm must contain real tree-sitter capture patterns, got: {hl_content:?}"
    );

    let lang = ed.state.config.languages.intern("tsx");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers();
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "syntax must attach after tsx grammar install; log={errors:#?}"
    );
}
