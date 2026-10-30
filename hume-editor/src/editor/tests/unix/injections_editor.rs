use super::*;

/// Core's `%register-installed-grammars!` (`runtime/scheme/grammars.scm`)
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
/// via Steel's stdlib process/fs helpers (see `user-manual/docs/writing-plugins.md`'s
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

/// `:plum-list-grammars` shows the installed grammars in `[plum-grammars]`,
/// and lists apart everything else in the grammars directory: a grammar
/// installed under a name that is not a language, and leftovers, then what
/// a stopped install left in PLUM's staging directory.
/// The files are written after load so core's startup grammar walk never
/// sees them.
#[test]
fn plum_list_grammars_shows_installed_grammars_and_what_is_not_in_use() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());
    let grammars = data_tmp.path().join("hume/grammars");
    std::fs::create_dir_all(&grammars).unwrap();
    for (name, install) in [("rust", "1"), ("rust", "2"), ("proto", "1")] {
        let dir = grammars.join(name).join(install);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(test_fixtures::checked_marker(&dir), b"").unwrap();
    }
    std::fs::create_dir_all(grammars.join("json.replaced")).unwrap();
    let staging = data_tmp.path().join("hume/plum-staging/grammars");
    std::fs::create_dir_all(staging.join("rust")).unwrap();
    std::fs::write(staging.join("rust.tar.gz"), b"").unwrap();

    type_cmd(&mut ed, ":plum-list-grammars");

    assert_eq!(ed.doc().display_name(), "[plum-grammars]");
    insta::assert_snapshot!(ed.doc().text().to_string(), @"
    Installed grammars (1)
      rust

    Not in use (3), remove with :plum-cleanup-grammars
      json.replaced
      proto
      rust/1

    Left by stopped installs (2), clear with :plum-cleanup-grammars
      rust
      rust.tar.gz
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
/// `stdlib/list-subdirs` filters by `is-dir?`, so a real `.DS_Store` next to
/// an installed plugin is skipped the same way.
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

/// `:plum-cleanup-plugins` exercises `stdlib/delete-dir!` against a real
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
        ":plum-cleanup-plugins must remove the orphan plugin directory"
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

/// `:plum-install-grammar nosuchlang`: a name that is not a language reports
/// so instead of failing deep inside the install pipeline, before it reads
/// any index.
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
            .is_some_and(|m| m.contains(r#"no language "nosuchlang""#)),
        "expected a no-language message naming 'nosuchlang', got: {:?}",
        ed.state.status_msg
    );
}

/// A typed argument wins over the current buffer's language.
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
            .is_some_and(|m| m.contains(r#"no language "nosuchlang""#)),
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
/// This particular command errors out via `log!` only (no `print!`), so
/// under the lazy-entry design it never even reaches
/// `claim_terminal`. See
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

/// An `#:inline-output` command that *does* print via `print!` must,
/// off the event loop, still reach `EditorHostImpl::claim_terminal`'s
/// no-terminal early return rather than the real terminal: printing must
/// succeed without ever entering the alt-screen.
///
/// The `needs_enter`/`tui` guard keeps `claim_terminal` from
/// entering here, where entering waits for a keypress against a real TTY or
/// panics against a non-TTY stdin in CI.
#[test]
fn inline_output_command_with_real_output_still_skips_bracket_off_event_loop() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "printer" ""
             (lambda () (print! "one line") (log! 'info "printed"))
             #:inline-output #t)"#,
    );

    type_cmd(&mut ed, ":printer");

    assert_eq!(
        ed.inline_output_enter_count(),
        0,
        "print! output off the event loop must not enter the real terminal bracket"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("printed"),
        "the command must run past its print"
    );
}
