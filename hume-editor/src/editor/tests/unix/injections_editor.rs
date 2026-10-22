use super::*;

/// Core's `register-installed-grammars!` (`runtime/scheme/grammars.scm`)
/// already ran before PLUM ever loaded (`init_scripting` evaluates it
/// unconditionally), walked the empty `<data>/grammars/`, found nothing,
/// and read no language data at all. This test then loads `core:plum`
/// itself, checking its `grammars.scm` (the install pipeline) compiles
/// cleanly and its bindings resolve against those same core bindings: a
/// pure Scheme-syntax/logic smoke test, not an installation test.
#[test]
fn plum_plugin_loads_with_real_language_data() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    let errors = error_log(&ed);
    assert!(
        errors.is_empty(),
        "loading core:plum against the real language data must not error: {errors:?}"
    );
}

/// `:plum-list-plugins` exercises `plugins.scm`'s `plum/installed-plugins` (built on
/// `core:stdlib`'s `stdlib/list-subdirs`, a Steel `read-dir`-backed helper)
/// against a real (empty) data dir, no network. Pins that plugin discovery
/// via Steel's stdlib process/fs helpers (see `user-manual/docs/plugins.md`'s
/// "Filesystem and processes") works for loading and basic discovery.
#[test]
fn plum_list_runs_with_no_errors_against_empty_data_dir() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-list-plugins");

    let errors = error_log(&ed);
    assert!(
        errors.is_empty(),
        ":plum-list-plugins against an empty data dir must not error: {errors:?}"
    );
}

/// `:plum-list-grammars` shows the compiled grammars in `[plum-grammars]`,
/// splitting off the ones compiled under a name that is not a language.
/// The files are written after load so core's startup grammar walk never
/// sees them.
#[test]
fn plum_list_grammars_shows_installed_and_orphan_grammars() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());
    let grammars = data_tmp.path().join("hume/grammars");
    std::fs::create_dir_all(&grammars).unwrap();
    let ext = test_fixtures::grammar_platform_ext();
    for name in ["rust", "proto"] {
        std::fs::write(grammars.join(format!("{name}.{ext}")), b"").unwrap();
    }

    type_cmd(&mut ed, ":plum-list-grammars");

    assert_eq!(ed.doc().display_name(), "[plum-grammars]");
    insta::assert_snapshot!(ed.doc().text().to_string(), @"
    Installed grammars (1)
      rust

    Orphan grammars (1), remove with :plum-cleanup-grammars
      proto
    ");
}

/// `:plum-list-plugins` shows its four lists in `[plum-plugins]`.
#[test]
fn plum_list_plugins_shows_each_list() {
    let data_tmp = tempfile::tempdir().unwrap();
    let orphan_dir = data_tmp.path().join("hume/plugins/testuser/orphanrepo");
    std::fs::create_dir_all(&orphan_dir).unwrap();
    std::fs::write(orphan_dir.join("plugin.scm"), "; orphan\n").unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-list-plugins");

    assert_eq!(ed.doc().display_name(), "[plum-plugins]");
    insta::assert_snapshot!(ed.doc().text().to_string(), @"
    Declared plugins (2)
      core:stdlib
      core:plum

    Installed plugins (1)
      testuser/orphanrepo

    Orphan plugins (1), remove with :plum-cleanup-plugins
      testuser/orphanrepo

    Missing plugins (0)
      none
    ");
}

/// A stray file directly inside `<data>/plugins/` (e.g. a macOS `.DS_Store`)
/// must not make `plum/installed-plugins` raise. Walking into it as if it
/// were a "user" directory would call `Path::read_dir` on a non-directory,
/// and that error would propagate uncaught out of `:plum-list-plugins`.
///
/// `stdlib/list-subdirs` must filter by `is-dir?`, or a real `.DS_Store` next
/// to an installed plugin raises the same error.
#[test]
fn plum_installed_plugins_skips_a_stray_file_in_the_plugins_dir() {
    // `load_plum` makes `<data_tmp>/hume/` the editor's data dir, so the
    // plugin walk plum/`plugins-dir` reads is `<data_tmp>/hume/plugins/`.
    let data_tmp = tempfile::tempdir().unwrap();
    let plugins_dir = data_tmp.path().join("hume").join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    std::fs::write(plugins_dir.join(".DS_Store"), "").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-list-plugins");

    let errors = error_log(&ed);
    assert!(
        errors.is_empty(),
        ":plum-list-plugins must skip a stray file in <data>/plugins/, not raise: {errors:?}"
    );
}

/// `:plum-cleanup-plugins` exercises `plum/delete-dir` against a real
/// on-disk orphan plugin, no network. Nothing in `init.scm` declares it, so
/// it's an orphan by definition; `:plum-cleanup-plugins` must remove its directory.
#[test]
fn plum_cleanup_removes_orphan_plugin_directory() {
    let data_tmp = tempfile::tempdir().unwrap();
    let orphan_dir = data_tmp.path().join("hume/plugins/testuser/orphanrepo");
    std::fs::create_dir_all(&orphan_dir).unwrap();
    std::fs::write(orphan_dir.join("plugin.scm"), "; orphan\n").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-cleanup-plugins");

    let errors = error_log(&ed);
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
/// is dead here (`name` is `#f`, not `""`), so it must not be relied on to
/// catch this; letting a `#f` name fall through produces an opaque
/// install-failure message instead.
#[test]
fn plum_install_grammar_no_arg_no_language_warns() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-install-grammar");

    // Boundary condition, not a failure: Severity::Info, statusline only,
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

/// Runs `:plum-install-grammar rust` with `git` and `curl` replaced by shims,
/// `curl_body` being the shell script after the `-o` path is parsed into
/// `$out`. Returns the editor, the data directory and the shim directory.
fn install_rust_grammar_with_curl(
    curl_body: &str,
) -> (Editor, tempfile::TempDir, tempfile::TempDir) {
    use std::os::unix::fs::PermissionsExt;

    let shims = tempfile::tempdir().unwrap();
    for (tool, body) in [
        (
            "git",
            "#!/bin/sh\nPATH=/usr/bin:/bin\n\
             if [ \"$1\" = clone ]; then for a in \"$@\"; do last=\"$a\"; done; mkdir -p \"$last\"; fi\n"
                .to_owned(),
        ),
        (
            "curl",
            format!(
                "#!/bin/sh\nPATH=/usr/bin:/bin\n\
                 out=\"\"; prev=\"\"\n\
                 for a in \"$@\"; do [ \"$prev\" = \"-o\" ] && out=\"$a\"; prev=\"$a\"; done\n\
                 {curl_body}"
            ),
        ),
    ] {
        let path = shims.path().join(tool);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());
    ed.state.dirs.search_path = Some(format!("{}:/usr/bin:/bin", shims.path().display()).into());
    type_cmd(&mut ed, ":plum-install-grammar rust");
    (ed, data_tmp, shims)
}

/// The query-fetch temp files left in PLUM's staging directory.
fn leftover_fetch_files(data_dir: &Path) -> Vec<String> {
    let staging = canonical_data_dir(data_dir).join("plum-staging");
    std::fs::read_dir(&staging)
        .unwrap_or_else(|e| panic!("read {staging:?}: {e}"))
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("_fetch_"))
        .collect()
}

/// A fetched query that cannot be read back fails that grammar's install,
/// removes the temp file, and leaves the VM usable for the next command.
#[test]
fn plum_install_grammar_unreadable_query_fails_cleanly() {
    let (mut ed, data_tmp, _shims) =
        install_rust_grammar_with_curl("printf '\\377\\376' > \"$out\"\n");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("rust"),
        "the unreadable query must fail the install and name the grammar: {log}"
    );
    assert_eq!(leftover_fetch_files(data_tmp.path()), Vec::<String>::new());

    type_cmd(&mut ed, ":plum-install-grammar nosuchlang");
    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains(r#"no grammar for language "nosuchlang""#)),
        "a later command must still run: {:?}",
        ed.state.status_msg
    );
}

/// A download that fails after writing part of its output fails the install
/// and removes the partial temp file.
#[test]
fn plum_install_grammar_failed_download_removes_its_temp_file() {
    let (ed, data_tmp, _shims) =
        install_rust_grammar_with_curl("printf 'partial' > \"$out\"\nexit 22\n");

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("curl"),
        "the failed download must fail the install: {log}"
    );
    assert_eq!(leftover_fetch_files(data_tmp.path()), Vec::<String>::new());
}

/// `:plum-install-grammar nosuchlang`: a language with no bundled grammar
/// reports the no-grammar message instead of failing deep inside the
/// install pipeline. This validation runs before the stale-source
/// `delete-dir` purge in `plum/install-grammar`, so an unknown name deletes
/// nothing.
#[test]
fn plum_install_grammar_unknown_name_warns() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-install-grammar nosuchlang");

    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains(r#"no grammar for language "nosuchlang""#)),
        "expected no-grammar message naming 'nosuchlang', got: {:?}",
        ed.state.status_msg
    );
}

/// A typed argument wins over the current buffer's language.
///
/// An arity-0 `plum-install-grammar` lambda would have the minibuffer drop
/// `nosuchlang` silently and run a real install of `rust` (the buffer's
/// language), so the message would never mention `nosuchlang`.
#[test]
fn plum_install_grammar_arg_overrides_buffer_language() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":set buffer language=rust");
    type_cmd(&mut ed, ":plum-install-grammar nosuchlang");

    assert!(
        ed.state
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains(r#"no grammar for language "nosuchlang""#)),
        "typed arg must win over buffer language 'rust', got: {:?}",
        ed.state.status_msg
    );
}

/// `plum-install-grammar` is declared `#:inline-output #t`, so dispatch must
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
/// `ensure_inline_output_screen`. See
/// `inline_output_command_with_real_output_still_skips_bracket_off_event_loop`
/// below for the case that does.
#[test]
fn inline_output_command_does_not_enter_terminal_bracket_off_event_loop() {
    let data_tmp = tempfile::tempdir().unwrap();
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

/// An `#:inline-output` command that *does* print via `displayln` must,
/// off the event loop, still reach `EditorHostImpl::ensure_inline_output_screen`'s
/// no-terminal early return rather than the real terminal: printing must
/// succeed without ever entering the alt-screen.
///
/// Without the `needs_enter`/`tui` guard, `ensure_inline_output_screen` would
/// always enter, and this test would hang on `wait_for_keypress` against a
/// real TTY or panic against a non-TTY stdin in CI.
#[test]
fn inline_output_command_with_real_output_still_skips_bracket_off_event_loop() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "printer" ""
             (lambda () (displayln "one line") (log! 'info "printed"))
             #:inline-output #t)"#,
    );

    type_cmd(&mut ed, ":printer");

    assert_eq!(
        ed.inline_output_enter_count(),
        0,
        "displayln output off the event loop must not enter the real terminal bracket"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("printed"),
        "the command must run past its print"
    );
}

/// A non-empty grammar source dir with no compiled grammar beside it must not
/// break `:plum-install-grammar`: the install builds in a staging clone and
/// then replaces the whole source dir with it.
///
/// Hits the network (real git fetch + curl fetch + compile of the `json`
/// grammar); requires `git` and `curl` on `PATH`, and a workspace-built
/// grammar compiler (see `workspace_grammar_build`).
#[test]
fn plum_install_grammar_recovers_from_stale_source_dir_on_first_try() {
    let data_tmp = tempfile::tempdir().unwrap();
    // `load_plum` makes `<data_tmp>/hume` the editor's data dir.
    let data_dir = data_tmp.path().join("hume");
    // A stale, non-empty source dir with no compiled grammar.
    let src_dir = data_dir.join("grammars/sources/json");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("stale.txt"), b"leftover from a failed install").unwrap();

    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    type_cmd(&mut ed, ":plum-install-grammar json");

    let ext = test_fixtures::grammar_platform_ext();
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
        "the install must replace the stale source dir"
    );
    assert!(
        src_dir.join("highlights.scm").exists(),
        "the highlights query must land in the new source dir"
    );
}

/// `tsx`'s `highlights.scm` declares `; inherits: ecma,_typescript,_jsx`
/// instead of writing out its own patterns, so `plum/install-grammar` must
/// resolve that chain (`plum/resolve-query` in `grammars.scm`) so the file
/// written to disk has real capture patterns, not a dangling directive.
///
/// Hits the network (real git clone + compile of the `tsx` grammar, plus
/// curl fetches of its `highlights.scm` and its `ecma`/`_typescript`/`_jsx`
/// query dependencies); requires `git` and `curl` on `PATH`, and a
/// workspace-built grammar compiler (see `workspace_grammar_build`).
///
/// Plain `curl-fetch` in place of `plum/fetch-query!` would leave
/// `highlights.scm` as the raw `; inherits: …` stub, a comment-only file with
/// no `@capture` in it.
#[test]
fn plum_install_grammar_resolves_helix_inherits_chain() {
    let data_tmp = tempfile::tempdir().unwrap();
    let data_dir = data_tmp.path().join("hume");

    let buf = crate::editor::buffer::Buffer::at_start(hume_editing::text::BufferText::from(
        "const x: number = 1;\n",
    ));
    let mut ed = Editor::for_testing(buf);
    let bid = ed.focused_buffer_id();
    load_plum(&mut ed, data_tmp.path());
    // Real bootstrap loads runtime/scheme/languages.scm (which declares tsx's
    // identity) before any plugin runs; `load_plum` only loads `core:plum`,
    // so register the identity here to match that ordering: `register-grammar!`
    // attaches onto an existing identity, it doesn't create one.
    ed.state
        .config
        .languages
        .register_identity("tsx", &["tsx"], &[], &[])
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
