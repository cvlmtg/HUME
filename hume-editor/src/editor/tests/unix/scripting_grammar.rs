// Editor-level tests for the tree-sitter grammar wiring that load Steel
// plugins or run `init_scripting` end-to-end. The platform-neutral half
// (and the shared runtime-data/fixture helpers) lives in
// `../scripting_grammar.rs`.
//
// Tests that use the grammar fixture (grammar_fixture()) require the shared
// library built by `scripts/fetch-test-grammars.sh`. Each calls
// require_grammars first, which panics naming the fix if a fixture is
// missing.

use super::*;
use hume_grid::Rect;
use hume_platform::dirs::Dirs;

use std::path::PathBuf;
use std::sync::Arc;

use super::super::render_snapshot::render_to_styled_string;
use super::super::scripting_grammar::{
    grammar_fixture, grammar_source, grammar_warnings, helix_pin, prelude_source,
    runtime_scheme_dir,
};
use hume_scripting::ScriptingHost;
use hume_treesitter::registry::GrammarBundle;
use test_fixtures::require_grammars;

/// Blobless-clone `url` into `dest` and force-check `rev` out, test-fixture-only.
fn git_clone_rev_for_test(
    url: &str,
    dest: &std::path::Path,
    rev: &str,
) -> std::io::Result<std::process::ExitStatus> {
    let status = std::process::Command::new("git")
        .args(["clone", "--filter=blob:none", "--", url])
        .arg(dest)
        .status()?;
    if !status.success() {
        return Ok(status);
    }
    std::process::Command::new("git")
        .arg("-C")
        .arg(dest)
        .args(["checkout", "--force", "--end-of-options", rev, "--"])
        .status()
}

/// Fetch `url` to `dest` via curl, test-fixture-only. Mirrors
/// `plum/fetch-raw-query`'s `run-inline-output!` call (the removed
/// `hume_platform::process::curl_fetch` builtin's shape).
fn curl_fetch_for_test(
    url: &str,
    dest: &std::path::Path,
) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(dest)
        .args(["--", url])
        .status()
}

// ---------------------------------------------------------------------------
// Steel command-mode test
// ---------------------------------------------------------------------------

/// End-to-end: register-grammar! in command mode attaches the grammar immediately
/// (init mode only queues it) and the execute path auto-sweeps open buffers of
/// that language.
///
/// Exercises: register_grammar command branch → attach_grammar → theme.bake →
/// Effect::GrammarSweep → apply_script_effects → sweep_buffers_for_grammars
/// → setup_buffer_syntax → Buffer.syntax (Syntax::attach).
#[test]
fn register_grammar_command_mode_attaches_and_sweeps() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let tmp = tempfile::tempdir().unwrap();
    let init_path = tmp.path().join("init.scm");
    // `register-grammar!` is defined in prelude.scm: prepend the real prelude
    // source so it's in scope, since this test evals `init_path` directly
    // rather than through the full `init_scripting` path.
    let prelude_src = prelude_source();
    let body = format!(
        r#"(define-typed-command! "attach-json" "Attach JSON grammar" (lambda () (register-grammar! "json" "{}" "tree_sitter_json" "{}")))"#,
        parser.display(),
        hl.display(),
    );
    std::fs::write(&init_path, prelude_src + "\n" + &body).unwrap();

    let mut host = ScriptingHost::new(&Dirs::none());
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init");
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    ed.scripting = Some(host);
    let lang = ed.state.config.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    assert!(
        ed.state.buffers.get(bid).syntax.is_none(),
        "no grammar attached yet"
    );

    type_cmd(&mut ed, ":attach-json");

    assert!(
        ed.state.config.languages.has_grammar("json"),
        "has_grammar must be true after attach"
    );
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "buffer syntax must be set after command-mode register-grammar! + auto-sweep",
    );
}

/// Write `register_grammar_call` (a full `(register-grammar! ...)` form) into
/// a fresh init.scm, prefixed with the real prelude so the macro is in
/// scope, eval it, apply its queued effect (init-mode `register-grammar!`
/// only queues an `Effect::LanguageReg`; this is what `init_scripting`
/// would do next), and return the resulting `json` grammar bundle. The
/// effect registers the grammar without loading it, so this loads it.
fn attach_json_via_init(register_grammar_call: &str) -> Arc<GrammarBundle> {
    let tmp = tempfile::tempdir().unwrap();
    let init_path = tmp.path().join("init.scm");
    let prelude_src = prelude_source();
    std::fs::write(&init_path, prelude_src + "\n" + register_grammar_call).unwrap();

    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let mut host = ScriptingHost::new(&Dirs::none());
    let effects = {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init");
    ed.apply_script_effects(effects);

    let id = ed
        .state
        .config
        .languages
        .id_of("json")
        .expect("json must be interned");
    assert!(
        ed.state.config.languages.loaded_grammar(id).is_none(),
        "init registers a grammar without loading it"
    );
    ed.state
        .config
        .languages
        .load_grammar(id, &mut ed.view.registry)
        .expect("json loads")
}

/// A `register-grammar!` call omitting `#:injections` and passing a real
/// path for `#:textobjects` (the motivating shape: a language with
/// structural objects but nothing embedded) populates the bundle's
/// `textobjects` and leaves `injections` `None`.
#[test]
fn register_grammar_textobjects_only_populates_textobjects_not_injections() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let tmp = tempfile::tempdir().unwrap();
    let to_path = tmp.path().join("textobjects.scm");
    std::fs::write(&to_path, "(pair) @entry.inside\n").unwrap();
    let body = format!(
        r#"(register-grammar! "json" "{}" "tree_sitter_json" "{}" #:textobjects "{}")"#,
        parser.display(),
        hl.display(),
        to_path.display(),
    );

    let bundle = attach_json_via_init(&body);
    assert!(
        bundle.textobjects.is_some(),
        "the textobjects path must populate the bundle's textobjects query"
    );
    assert!(
        bundle.injections.is_none(),
        "an explicit #f for injections must leave the bundle's injections None"
    );
}

/// `#:indents` alone populates the bundle's `indents` and leaves the other
/// optional queries `None`.
#[test]
fn register_grammar_indents_only_populates_indents() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let tmp = tempfile::tempdir().unwrap();
    let indents_path = tmp.path().join("indents.scm");
    std::fs::write(&indents_path, "(object) @indent\n\"}\" @outdent\n").unwrap();
    let body = format!(
        r#"(register-grammar! "json" "{}" "tree_sitter_json" "{}" #:indents "{}")"#,
        parser.display(),
        hl.display(),
        indents_path.display(),
    );

    let bundle = attach_json_via_init(&body);
    assert!(bundle.indents.is_some());
    assert!(bundle.injections.is_none());
    assert!(bundle.textobjects.is_none());
}

/// `#:injections` alone still populates `injections` when `textobjects`
/// isn't supplied at all: the sibling half of
/// `register_grammar_textobjects_only_populates_textobjects_not_injections`.
#[test]
fn register_grammar_injections_only_populates_injections() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let tmp = tempfile::tempdir().unwrap();
    let inj_path = tmp.path().join("injections.scm");
    std::fs::write(
        &inj_path,
        r#"((_) @injection.content (#set! injection.language "markdown"))"#,
    )
    .unwrap();
    let body = format!(
        r#"(register-grammar! "json" "{}" "tree_sitter_json" "{}" #:injections "{}")"#,
        parser.display(),
        hl.display(),
        inj_path.display(),
    );

    let bundle = attach_json_via_init(&body);
    assert!(
        bundle.injections.is_some(),
        "the injections path must populate the bundle's injections query"
    );
    assert!(
        bundle.textobjects.is_none(),
        "an omitted textobjects argument must leave the bundle's textobjects None"
    );
}

/// Two `register-grammar!` calls with *differently-shaped* keyword usage
/// (one omitting both `#:injections`/`#:textobjects` entirely, the other
/// passing only `#:injections`) compiled as one program (one `init.scm`).
/// Steel 0.8.2 miscompiled differently-shaped keyword calls to the same
/// `#:kw`-sugared function within one compiled unit, and this is the
/// real-code shape closest to that trigger; `init_scripting` compiles each file
/// separately, so the risk (if any remains) is scoped to calls within a
/// single file, exactly what this test constructs.
///
/// That miscompile surfaces as a `FreeIdentifier` error from `eval_init`.
#[test]
fn register_grammar_two_differently_shaped_keyword_calls_in_one_file_compiles() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let tmp = tempfile::tempdir().unwrap();
    let inj_path = tmp.path().join("injections.scm");
    std::fs::write(
        &inj_path,
        r#"((_) @injection.content (#set! injection.language "markdown"))"#,
    )
    .unwrap();
    let body = format!(
        r#"(register-grammar! "json" "{parser}" "tree_sitter_json" "{hl}")
           (register-grammar! "json" "{parser}" "tree_sitter_json" "{hl}" #:injections "{inj}")"#,
        parser = parser.display(),
        hl = hl.display(),
        inj = inj_path.display(),
    );

    let bundle = attach_json_via_init(&body);
    assert!(
        bundle.injections.is_some(),
        "the second call (last registration wins) must be the one that took effect"
    );
}

// ---------------------------------------------------------------------------
// Startup (call! …) during init
// ---------------------------------------------------------------------------

/// Passive grammar registration: an init that registers installed grammars
/// directly (like core's `register-installed-grammars!` in
/// `runtime/scheme/grammars.scm`) succeeds without error and populates the
/// pending language regs.  A `(call! "unknown-cmd")` in
/// the same init logs a warning but does not abort: unknown commands are
/// soft failures during init (buffer access unavailable; command not native).
#[test]
fn passive_load_registers_grammar_and_unknown_call_logs_warning() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let ext = test_fixtures::grammar_platform_ext();

    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("hume");
    std::fs::create_dir_all(data_dir.join("grammars/sources")).unwrap();
    std::fs::create_dir_all(data_dir.join("plugins")).unwrap();

    let grammar_out = data_dir.join("grammars").join(format!("json.{ext}"));
    std::fs::copy(&parser, &grammar_out).unwrap();
    let hl_dest = data_dir.join("grammars/sources/json-hl.scm");
    std::fs::copy(&hl, &hl_dest).unwrap();

    let init_path = tmp.path().join("init.scm");
    // `register-grammar!` is defined in prelude.scm: prepend the real prelude
    // source so it's in scope (see `register_grammar_command_mode_attaches_and_sweeps`).
    let prelude_src = prelude_source();
    let body = format!(
        r#"
(define hl-path "{hl}")
(define grammar-out-path "{grammar_out}")

(define (grammar-installed? name)
  (path-exists? grammar-out-path))

(define (do-register! names)
  (for-each
    (lambda (name)
      (when (and (grammar-installed? name) (path-exists? hl-path))
        (register-grammar! name grammar-out-path "tree_sitter_json" hl-path)))
    names))

(do-register! (list "json" "phantom"))
(call! "plum/ensure-grammars")
        "#,
        hl = hl_dest.display(),
        grammar_out = grammar_out.display(),
    );
    std::fs::write(&init_path, prelude_src + &body).unwrap();

    let mut host = ScriptingHost::new(&Dirs {
        data: Some(data_dir.clone()),
        ..Dirs::none()
    });
    let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::at_start(
        hume_editing::text::BufferText::empty(),
    ));
    let effects = {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init must succeed");

    // Passive registration populated the effect log with LanguageReg entries.
    let regs: Vec<_> = effects
        .iter()
        .filter(|e| matches!(e, hume_scripting::Effect::LanguageReg(_)))
        .collect();
    assert!(
        !regs.is_empty(),
        "passive grammar registration must populate the effect log"
    );
    // Unknown (call!) produced a warning, did not abort.
    let msgs = host.take_pending_messages();
    assert!(
        msgs.iter()
            .any(|(_, msg)| msg.contains("plum/ensure-grammars")),
        "unknown command in init must log a warning; got: {:?}",
        msgs,
    );
}

/// Tab on `:plum-install-grammar`'s argument completes against the real
/// shipped language data (`runtime/scheme/languages/`), not a fixture:
/// no network, no install, so it needs neither `require_grammars` nor the
/// e2e tests' own PATH and grammar-compiler setup below. "ag" is unique to "agda"
/// among every declared name (checked at authoring time; re-verified by
/// this test every run, same discipline as this file's fixture-server
/// module doc elsewhere in this test tree).
#[test]
fn plum_install_grammar_tab_completes_a_declared_name() {
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>\n");
    load_plum(&mut ed, data_tmp.path());

    ed.handle_key(key(':'));
    type_chars(&mut ed, "plum-install-grammar ag");
    ed.handle_key(key_tab());
    ed.settle();
    assert_eq!(minibuf_input(&ed), "plum-install-grammar agda");
}

// ---------------------------------------------------------------------------
// e2e grammar install (network + an installed grammar compiler required)
// ---------------------------------------------------------------------------

/// End-to-end: clone → curl → grammar compiler → register-grammar! for JSON.
/// Requires `git` and `curl` on `PATH`, a workspace-built grammar compiler
/// (see `workspace_grammar_build`), and network access.
#[test]
fn install_real_json_grammar_e2e() {
    // Read the JSON grammar's url + pinned rev straight from the shipped data
    // (single source of truth, no hardcoded pins to drift out of sync).
    let (url, rev) = grammar_source("json");
    let (url, rev) = (url.as_str(), rev.as_str());

    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("hume");
    std::fs::create_dir_all(data_dir.join("grammars/sources")).unwrap();
    std::fs::create_dir_all(data_dir.join("plugins")).unwrap();

    let src_dir = data_dir.join("grammars/sources/json");
    let ext = test_fixtures::grammar_platform_ext();
    let out_path = data_dir.join("grammars").join(format!("json.{ext}"));

    let status = git_clone_rev_for_test(url, &src_dir, rev);
    match &status {
        Err(e) => panic!("git_clone_rev failed: {e}"),
        Ok(s) if !s.success() => panic!("git_clone_rev non-zero exit"),
        Ok(_) => {}
    }
    assert!(src_dir.exists(), "clone must create src dir");

    let tool = match workspace_grammar_build() {
        GrammarBuild::Found(tool) => tool,
        missing => panic!("build hume-grammar-build first (cargo build --workspace): {missing:?}"),
    };
    let output = hume_platform::process::grammar_build(
        &tool,
        &src_dir,
        &out_path,
        hume_platform::process::Launch::with_process_path(&src_dir),
    )
    .expect("grammar_build must not fail to spawn");
    assert!(output.status.success(), "grammar compiler failed");
    assert!(out_path.exists(), "compiled grammar must exist after build");

    let hl_path = src_dir.join("highlights.scm");
    // Fetch highlights query via curl, using the pinned helix commit.
    let pin = helix_pin();
    let hl_url = format!(
        "https://raw.githubusercontent.com/helix-editor/helix/{pin}/runtime/queries/json/highlights.scm"
    );
    let curl_status = curl_fetch_for_test(&hl_url, &hl_path);
    match &curl_status {
        Err(e) => panic!("curl_fetch failed: {e}"),
        Ok(s) if !s.success() => panic!("curl_fetch non-zero exit"),
        Ok(_) => {}
    }
    assert!(hl_path.exists(), "highlights query must exist after curl");

    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    let init_path = tmp.path().join("init.scm");
    // `register-grammar!` is defined in prelude.scm: prepend the real prelude
    // source so it's in scope (see `register_grammar_command_mode_attaches_and_sweeps`).
    let prelude_src = prelude_source();
    let body = format!(
        r#"(define-typed-command! "attach-json" "attach json grammar" (lambda () (register-grammar! "json" "{}" "tree_sitter_json" "{}")))"#,
        out_path.display(),
        hl_path.display(),
    );
    std::fs::write(&init_path, prelude_src + &body).unwrap();

    let mut host = ScriptingHost::new(&Dirs {
        data: Some(data_dir),
        ..process_dirs()
    });
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init");
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    ed.scripting = Some(host);
    let lang = ed.state.config.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));

    type_cmd(&mut ed, ":attach-json");

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .map(|e| format!("{:?}: {}", e.severity, e.text))
        .collect();
    assert!(
        ed.state.config.languages.has_grammar("json"),
        "grammar must be registered after e2e install; log={errors:#?}",
    );
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "syntax must be set after e2e install + sweep; log={errors:#?}",
    );

    // Pin a stable snapshot theme so default-theme changes don't churn this frame.
    // Bake after :attach-json has interned the grammar's scopes into the registry.
    ed.view.theme = crate::testing::build_snapshot_theme();
    ed.view.theme.bake(&ed.view.registry);

    // Styled-frame snapshot: locks down token colours after the full e2e pipeline.
    let rect = Rect::new(0, 0, 40, 5);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

// ---------------------------------------------------------------------------
// Startup ordering invariant
// ---------------------------------------------------------------------------

/// Helper: write a temp-runtime `scheme/prelude.scm` (copied verbatim from the
/// real runtime; it's self-contained and defines the `define-language!`
/// macro) plus a caller-supplied `scheme/languages.scm`, point the editor's
/// runtime, config and data directories at temp dirs, give the
/// editor's buffer a path so extension-based detection fires, and run
/// `init_scripting`. Caller must keep the returned `TempDir`s alive.
fn setup_editor_with_languages_scm(
    languages_scm: &str,
    file_name: &str,
) -> (Editor, Vec<tempfile::TempDir>) {
    let config_tmp = tempfile::tempdir().unwrap();
    let runtime_tmp = tempfile::tempdir().unwrap();
    let data_tmp = tempfile::tempdir().unwrap();

    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(hume_config.join("init.scm"), "").unwrap();

    let scheme_dir = runtime_tmp.path().join("scheme");
    std::fs::create_dir_all(&scheme_dir).unwrap();
    let prelude_src = prelude_source();
    std::fs::write(scheme_dir.join("prelude.scm"), prelude_src).unwrap();
    std::fs::write(scheme_dir.join("languages.scm"), languages_scm).unwrap();

    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .buffers
        .get_mut(bid)
        .set_file(crate::editor::buffer::BufferFile::at(
            PathBuf::from("/tmp").join(file_name),
        ));

    ed.state.dirs = Dirs {
        config: Some(hume_config),
        data: Some(data_tmp.path().join("hume")),
        runtime: Some(runtime_tmp.path().to_path_buf()),
        ..Dirs::none()
    };
    ed.init_scripting(&mut Default::default());

    (ed, vec![config_tmp, runtime_tmp, data_tmp])
}

/// `configure-language!`'s `#:language-id` keyword round-trips through the
/// real `prelude.scm`. Exercises both shapes side by side: `plain-lang` (not
/// configured) and `tsx` (with the override).
#[test]
fn configure_language_language_id_keyword_round_trips_through_real_prelude() {
    let languages_scm = r#"
        (define-language! "plain-lang" '("plainext"))
        (define-language! "tsx" '("tsx"))
        (configure-language! "tsx" #:language-id "typescriptreact")
    "#;

    let (ed, _dirs) = setup_editor_with_languages_scm(languages_scm, "test.tsx");

    let errors = error_log(&ed);
    assert!(
        errors.is_empty(),
        "configure-language! #:language-id must parse and eval cleanly: {errors:?}"
    );

    let plain_id = ed
        .state
        .config
        .languages
        .id_of("plain-lang")
        .expect("plain-lang must have registered");
    assert_eq!(
        ed.state.config.languages.lsp_language_id_of(plain_id),
        "plain-lang",
        "a language with no configured language-id must fall back to the name"
    );

    let tsx_id = ed
        .state
        .config
        .languages
        .id_of("tsx")
        .expect("tsx must have registered");
    assert_eq!(
        ed.state.config.languages.lsp_language_id_of(tsx_id),
        "typescriptreact",
        "#:language-id must override the wire languageId"
    );
}

/// `configure-language!`'s `#:roots` keyword round-trips through the real
/// prelude into the language registry; a language not configured has no roots.
#[test]
fn configure_language_roots_keyword_round_trips_through_real_prelude() {
    let languages_scm = r#"
        (define-language! "plain-lang" '("pln"))
        (define-language! "rooted-lang" '("rtd"))
        (configure-language! "rooted-lang" #:roots '("proj.toml" "proj.lock"))
    "#;

    let (ed, _dirs) = setup_editor_with_languages_scm(languages_scm, "test.rtd");

    let registry = &ed.state.config.languages;
    let plain = registry.id_of("plain-lang").expect("plain-lang registered");
    let rooted = registry
        .id_of("rooted-lang")
        .expect("rooted-lang registered");
    assert!(registry.roots_of(plain).is_empty());
    assert_eq!(registry.roots_of(rooted), ["proj.toml", "proj.lock"]);
}

/// The shipped language data gives `tsx` the wire `languageId`
/// `"typescriptreact"`: sent HUME's own language name, `typescript-language-server`
/// logs "Invalid languageId \"tsx\" ... Correcting to \"typescriptreact\"".
/// Boots against the real, shipped `runtime/` dir (not a hand-written
/// fixture), so it reads the `language-id` overrides `scripts/sync-grammars.py`
/// writes into `languages/*.scm`.
#[test]
fn tsx_bundled_language_id_is_typescriptreact() {
    let config_tmp = tempfile::tempdir().unwrap();
    let data_tmp = tempfile::tempdir().unwrap();
    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(hume_config.join("init.scm"), "").unwrap();

    let mut ed = editor_from("-[a]>b\n");
    ed.state.dirs = Dirs {
        config: Some(hume_config),
        data: Some(data_tmp.path().join("hume")),
        runtime: Some(repo_runtime_dir()),
        ..process_dirs()
    };
    ed.init_scripting(&mut Default::default());
    let bid = ed.focused_buffer_id();
    ed.set_buffer_language_by_name(bid, Some("tsx"), hume_scripting::ConfigOrigin::User);

    let id = ed
        .state
        .config
        .languages
        .id_of("tsx")
        .expect("tsx must be a bundled language");
    assert_eq!(
        ed.state.config.languages.lsp_language_id_of(id),
        "typescriptreact"
    );
}

/// Locks the startup invariant the `run()` reorder (`hume-editor/src/lib.rs`)
/// leans on: by the time `init_scripting` returns, the initial (already-open)
/// buffer's language has been detected from its path and its tree-sitter
/// parse has been posted to the background worker, so the run loop's first
/// frame is highlighted at most one poll later, never long after a b/w flash.
///
/// The detection + parse-post happens via the end-of-init
/// `detect_and_set_language` loop in `scripting/init.rs`.
#[test]
fn initial_buffer_parse_is_in_flight_by_end_of_init_scripting() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let languages_scm = format!(
        "(define-language! \"json\" '(\"json\"))\n\
         (register-grammar! \"json\" \"{}\" \"tree_sitter_json\" \"{}\")\n",
        parser.display(),
        hl.display(),
    );

    let (mut ed, _dirs) = setup_editor_with_languages_scm(&languages_scm, "test.json");
    let bid = ed.focused_buffer_id();

    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "highlighter must be attached by the end of init_scripting (language \
         detected from the buffer's path via the end-of-init detect loop)"
    );
    assert!(
        ed.state
            .buffers
            .get(bid)
            .syntax
            .as_ref()
            .unwrap()
            .layers()
            .is_none(),
        "tree must not be installed yet, only posted; drained on the next \
         reparse_stale_buffers call (matches the run loop's first iteration)"
    );

    ed.reparse_stale_buffers();

    assert!(
        ed.state
            .buffers
            .get(bid)
            .syntax
            .as_ref()
            .unwrap()
            .layers()
            .is_some(),
        "tree must be installed after exactly one reparse_stale_buffers call \
         following init_scripting"
    );
    let parsed_gen = ed
        .state
        .buffers
        .get(bid)
        .syntax
        .as_ref()
        .unwrap()
        .parsed_gen();
    assert_eq!(
        parsed_gen,
        Some(ed.state.buffers.get(bid).text().generation()),
        "parsed_gen must catch up to generation after the drain"
    );
}

// ---------------------------------------------------------------------------
// Grammar registration survives core:plum's absence
// ---------------------------------------------------------------------------

/// `runtime/scheme/grammars.scm` registers already-compiled grammars
/// unconditionally at startup, so highlighting for an installed grammar must
/// not depend on `core:plum` being declared in `init.scm` at all. PLUM is
/// only needed to *install* a grammar in the first place.
///
/// Stages a real compiled JSON grammar at the exact paths core's
/// `grammar-output-path`/`grammar-highlights-path` expect, points the
/// runtime dir at the repo's real `runtime/` dir (so the real
/// language data and `grammars.scm` registrar run), and runs
/// `init_scripting` against an `init.scm` that never mentions PLUM.
#[test]
fn grammar_registration_survives_plum_absence() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    // No core:plum anywhere in init.scm.
    let fixture = StagedGrammarFixture::new("json", &parser, &hl, "");

    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    ed.state.dirs = fixture.dirs();
    let bid = ed.focused_buffer_id();
    ed.state
        .buffers
        .get_mut(bid)
        .set_file(crate::editor::buffer::BufferFile::at("/tmp/test.json"));
    ed.init_scripting(&mut Default::default());
    drop(fixture);

    let errors = error_log(&ed);
    assert!(
        errors.is_empty(),
        "init_scripting without core:plum must not error: {errors:?}"
    );

    assert!(
        ed.state.config.languages.has_grammar("json"),
        "grammar must be registered at startup without core:plum declared"
    );
    ed.reparse_stale_buffers();
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "buffer must be highlighted without core:plum declared"
    );

    // Sanity: PLUM's own commands really are unavailable. This run never
    // loaded the plugin, so the registration above cannot be credited to it.
    type_cmd(&mut ed, ":plum-install-grammar");
    let warns: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Warning)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        warns.contains(&"unknown command: plum-install-grammar".to_string()),
        "core:plum commands must be unavailable when it isn't declared; got {warns:?}"
    );
}

// ---------------------------------------------------------------------------
// A user override in init.scm must not undo a startup grammar registration
// ---------------------------------------------------------------------------

/// `grammars.scm` registers every already-compiled grammar before `init.scm`
/// runs (`scripting/init.rs`: prelude → languages → grammars → init.scm).
/// `languages.scm`'s own header documents overriding an entry by redefining
/// it in `init.scm`. This exercises exactly that documented pattern for an
/// already-grammared language and asserts highlighting survives it.
///
/// `register_identity_no_rebuild` replaces only the identity on
/// re-registration and keeps the language's registered grammar.
#[test]
fn define_language_override_in_init_keeps_startup_grammar() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    // The documented override pattern: redefine an already-grammared
    // language in init.scm to add an extension.
    let fixture = StagedGrammarFixture::new(
        "json",
        &parser,
        &hl,
        r#"(define-language! "json" '("json" "jsonc"))"#,
    );

    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    ed.state.dirs = fixture.dirs();
    let bid = ed.focused_buffer_id();
    ed.state
        .buffers
        .get_mut(bid)
        .set_file(crate::editor::buffer::BufferFile::at("/tmp/test.jsonc"));
    ed.init_scripting(&mut Default::default());
    drop(fixture);

    let errors = error_log(&ed);
    assert!(
        errors.is_empty(),
        "init.scm's define-language! override must not error: {errors:?}"
    );

    assert!(
        ed.state.config.languages.has_grammar("json"),
        "grammar registered by grammars.scm must survive init.scm's identity override"
    );
    ed.reparse_stale_buffers();
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "buffer must still be highlighted after the init.scm override"
    );
    // The override itself must have taken effect: the buffer's new .jsonc
    // extension detects as json, proving this isn't a stale pre-override
    // identity papering over a dropped-then-never-reattached grammar.
    assert_eq!(
        ed.state.buffers.get(bid).language,
        ed.state.config.languages.id_of("json"),
        "the .jsonc extension added by init.scm must detect as json"
    );
}

// ---------------------------------------------------------------------------
// Registration is driven by <data>/grammars/, reading only installed
// languages' data files
// ---------------------------------------------------------------------------

/// A bundled `json` data file naming a grammar.
const JSON_DATA: &str = "((grammar \"url\" \"rev\" \"tree_sitter_json\" \"\"))";

/// Run `init_scripting` against a temp runtime holding the real `prelude.scm`
/// and `grammars.scm`, a tiny `languages.scm` and a caller-supplied bundled
/// `languages/json.scm`, with `populate_data` free to lay out the data dir
/// first. Returns the editor's error log. Caller must keep the returned
/// `TempDir`s alive.
fn init_errors_with_json_data(
    json_data: &str,
    populate_data: impl FnOnce(&std::path::Path),
) -> (Vec<String>, Editor, Vec<tempfile::TempDir>) {
    let config_tmp = tempfile::tempdir().unwrap();
    let runtime_tmp = tempfile::tempdir().unwrap();
    let data_tmp = tempfile::tempdir().unwrap();

    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(hume_config.join("init.scm"), "").unwrap();

    let scheme_dir = runtime_tmp.path().join("scheme");
    std::fs::create_dir_all(&scheme_dir).unwrap();
    for name in ["prelude.scm", "grammars.scm"] {
        let src = std::fs::read_to_string(runtime_scheme_dir().join(name)).unwrap();
        std::fs::write(scheme_dir.join(name), src).unwrap();
    }
    std::fs::write(
        scheme_dir.join("languages.scm"),
        "(define-language! \"json\" '(\"json\"))\n",
    )
    .unwrap();
    let languages_dir = scheme_dir.join("languages");
    std::fs::create_dir_all(&languages_dir).unwrap();
    std::fs::write(languages_dir.join("json.scm"), json_data).unwrap();

    populate_data(&data_tmp.path().join("hume"));

    let mut ed = editor_from("-[a]>b\n");
    ed.state.dirs = Dirs {
        config: Some(hume_config),
        data: Some(data_tmp.path().join("hume")),
        runtime: Some(runtime_tmp.path().to_path_buf()),
        ..Dirs::none()
    };
    ed.init_scripting(&mut Default::default());

    let errors = error_log(&ed);
    (errors, ed, vec![config_tmp, runtime_tmp, data_tmp])
}

/// Startup reads a language's data file only when its grammar is installed:
/// with no `<data>/grammars/` directory there is nothing to register, so a
/// data file that cannot even be parsed must never be touched.
///
/// Uses a syntactically broken `json.scm` as the tripwire: if anything reads
/// it, `grammars.scm` raises and `init_scripting` logs an error. The last
/// case (a compiled `json` present ⇒ the same broken file now *is* read)
/// proves the tripwire works, so the first cases cannot pass vacuously.
#[test]
fn startup_reads_only_installed_languages_data_files() {
    let broken = "((grammar \"url\" \"rev\"";

    let (errors, ..) = init_errors_with_json_data(broken, |_data| {});
    assert!(
        errors.is_empty(),
        "no <data>/grammars/ ⇒ no data file is read: {errors:?}"
    );

    // A bare sources/ subdirectory yields no grammar names either, so still no read.
    let (errors, ..) = init_errors_with_json_data(broken, |data| {
        std::fs::create_dir_all(data.join("grammars").join("sources")).unwrap();
    });
    assert!(
        errors.is_empty(),
        "only sources/ present ⇒ still no grammar names, so still no read: {errors:?}"
    );

    // Tripwire check: a compiled json grammar forces json.scm, which then fails.
    let ext = test_fixtures::grammar_platform_ext();
    let (errors, ..) = init_errors_with_json_data(broken, |data| {
        let grammars = data.join("grammars");
        std::fs::create_dir_all(&grammars).unwrap();
        std::fs::write(grammars.join(format!("json.{ext}")), b"not a real library").unwrap();
    });
    assert!(
        errors.iter().any(|e| e.contains("json.scm")),
        "a compiled grammar must read its data file, surfacing the broken file: {errors:?}"
    );
}

/// A compiled grammar for a name with no bundled grammar (installed, then
/// dropped by a HUME update) has no tree-sitter symbol to look up. Walking the
/// install directory reaches it anyway, so registration must skip it rather
/// than raise.
///
/// `register-installed-grammars!`'s `(when source …)` skips a name
/// `bundled-language-grammar` returns `#f` for, before `grammar-symbol`'s
/// `(list-ref #f 2)` can raise.
#[test]
fn orphan_compiled_grammar_is_skipped_not_registered() {
    let ext = test_fixtures::grammar_platform_ext();

    let (errors, ed, _dirs) = init_errors_with_json_data(JSON_DATA, |data| {
        let grammars = data.join("grammars");
        std::fs::create_dir_all(&grammars).unwrap();
        std::fs::write(
            grammars.join(format!("no-longer-in-catalog.{ext}")),
            b"not a real library",
        )
        .unwrap();
        // A highlights query too, so only the `bundled-language-grammar`
        // check stands between this name and the symbol lookup.
        let src = grammars.join("sources").join("no-longer-in-catalog");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("highlights.scm"), "; empty\n").unwrap();
    });

    assert!(
        errors.is_empty(),
        "an orphan compiled grammar must be skipped silently: {errors:?}"
    );
    assert!(
        !ed.state
            .config
            .languages
            .has_grammar("no-longer-in-catalog"),
        "an orphan compiled grammar must not be registered"
    );
}

/// A language that still has a bundled grammar, with a compiled library on disk
/// but no `highlights.scm` (e.g. the user cleared `<data>/grammars/sources/`
/// to reclaim disk), must warn (naming the grammar so `:plum-install-grammar
/// <name>` is the obvious next step) instead of being dropped as silently as
/// a genuine orphan. Distinguishes the two cases `register-installed-grammars!`
/// must tell apart: no bundled grammar (expected-silent) vs. known-but-broken
/// (repairable, must be surfaced).
#[test]
fn known_grammar_missing_highlights_warns_and_is_not_registered() {
    let ext = test_fixtures::grammar_platform_ext();

    let (errors, ed, _dirs) = init_errors_with_json_data(JSON_DATA, |data| {
        let grammars = data.join("grammars");
        std::fs::create_dir_all(&grammars).unwrap();
        std::fs::write(grammars.join(format!("json.{ext}")), b"not a real library").unwrap();
        // No highlights.scm under sources/json/: the test covers its absence.
    });

    assert!(
        errors.is_empty(),
        "a missing highlights query is a warning, not an error: {errors:?}"
    );
    assert!(
        ed.state
            .message_log
            .entries()
            .any(|e| e.severity == Severity::Warning
                && e.text.contains("json")
                && e.text.contains("highlights")),
        "expected a warning naming the grammar and its missing highlights query; messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
    assert!(
        !ed.state.config.languages.has_grammar("json"),
        "a grammar missing its highlights query must not be registered"
    );
}

/// A file matching another platform's shared-library extension (e.g. a `.so`
/// left behind after a macOS setup migrated from Linux) is not part of this
/// platform's installed set, so `installed-grammars` must never yield its name
/// at all: `register-grammar!` must not even be attempted for it.
///
/// Startup registration opens nothing, so `has_grammar` reports a
/// registration attempt even for a file that is not a real library
/// (`grammar-output-path` re-derives its path with the *platform's*
/// extension). A skip leaves `json` with no grammar.
#[test]
fn wrong_extension_grammar_is_skipped_not_registered() {
    let wrong_ext = if cfg!(target_os = "macos") {
        "so"
    } else {
        "dylib"
    };

    let (errors, ed, _dirs) = init_errors_with_json_data(JSON_DATA, |data| {
        let grammars = data.join("grammars");
        std::fs::create_dir_all(&grammars).unwrap();
        std::fs::write(
            grammars.join(format!("json.{wrong_ext}")),
            b"not a real library",
        )
        .unwrap();
        let src = grammars.join("sources").join("json");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("highlights.scm"), "; empty\n").unwrap();
    });

    assert!(errors.is_empty(), "unexpected init errors: {errors:?}");
    assert!(
        !ed.state.config.languages.has_grammar("json"),
        "a foreign-extension file must not be registered"
    );
    let warnings: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Warning)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        warnings.is_empty(),
        "a foreign-extension file must be skipped without a warning: {warnings:?}"
    );
}

/// Startup registers an installed grammar without opening its library, so a
/// library that cannot load costs nothing and says nothing until a buffer of
/// its language needs it, and then says so once.
#[test]
fn init_registration_of_a_bogus_library_is_silent_until_a_buffer_needs_it() {
    let ext = test_fixtures::grammar_platform_ext();
    let (errors, mut ed, _dirs) = init_errors_with_json_data(JSON_DATA, |data| {
        let grammars = data.join("grammars");
        std::fs::create_dir_all(&grammars).unwrap();
        std::fs::write(grammars.join(format!("json.{ext}")), b"not a real library").unwrap();
        let src = grammars.join("sources").join("json");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("highlights.scm"), "; empty\n").unwrap();
    });
    assert!(errors.is_empty(), "unexpected init errors: {errors:?}");
    assert!(grammar_warnings(&ed, "json").is_empty());
    assert!(ed.state.config.languages.has_grammar("json"));

    let bid = ed.focused_buffer_id();
    ed.set_buffer_language_by_name(bid, Some("json"), hume_scripting::ConfigOrigin::User);
    ed.reparse_stale_buffers();

    assert_eq!(grammar_warnings(&ed, "json").len(), 1);
    assert!(!ed.state.config.languages.has_grammar("json"));
}

/// `register-grammar!` from a command loads the grammar right away, so a
/// library that cannot load fails the command that registered it.
#[test]
fn runtime_register_grammar_with_a_bad_library_raises() {
    let tmp = tempfile::tempdir().unwrap();
    let init_path = tmp.path().join("init.scm");
    let missing = tmp.path().join("missing");
    let body = format!(
        r#"(define-typed-command! "attach-json" "Attach JSON grammar" (lambda () (register-grammar! "json" "{}" "tree_sitter_json" "{}")))"#,
        missing.join("json.so").display(),
        missing.join("highlights.scm").display(),
    );
    std::fs::write(&init_path, prelude_source() + "\n" + &body).unwrap();
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut ed = editor_from("-[a]>b\n");
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init");
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":attach-json");

    assert!(
        has_error(&ed, "register-grammar!: 'json': "),
        "messages: {:?}",
        ed.state
            .message_log
            .entries()
            .map(|e| format!("{:?}: {}", e.severity, e.text))
            .collect::<Vec<_>>()
    );
    assert!(!ed.state.config.languages.has_grammar("json"));
}

/// `installed-grammars` must sort by stem, not by full filename: `"b-x.<ext>"
/// < "b.<ext>"` (`-` sorts before `.`), but stem order puts `b` ahead of
/// `b-x`. Neither name has a bundled grammar, so
/// `register-installed-grammars!` skips both silently. Only the list order
/// is under test.
#[test]
fn installed_grammars_sorts_by_stem_not_filename() {
    let ext = test_fixtures::grammar_platform_ext();

    let (errors, mut ed, _dirs) = init_errors_with_json_data(JSON_DATA, |data| {
        let grammars = data.join("grammars");
        std::fs::create_dir_all(&grammars).unwrap();
        std::fs::write(grammars.join(format!("b.{ext}")), b"").unwrap();
        std::fs::write(grammars.join(format!("b-x.{ext}")), b"").unwrap();
    });
    assert!(errors.is_empty(), "unexpected init errors: {errors:?}");

    let mut host = ed.scripting.take().expect("init_scripting installs a host");
    let result = {
        let mut ih = init_host!(ed);
        host.eval_source(r#"(error (string-join (installed-grammars) ","))"#, &mut ih)
    };
    ed.scripting = Some(host);

    let err = result.expect_err("the raised list must surface as an eval error");
    assert!(
        err.contains("b,b-x"),
        "installed-grammars must sort stems after stripping the extension: {err:?}"
    );
}

/// Startup registration reads every optional query file that exists beside
/// `highlights.scm`: with `textobjects.scm` and `indents.scm` present the
/// bundle has text objects and indents and no injections.
#[test]
fn register_grammar_from_sources_picks_up_only_the_optional_files_present() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let ext = test_fixtures::grammar_platform_ext();

    let (errors, mut ed, _dirs) = init_errors_with_json_data(JSON_DATA, |data| {
        let grammars = data.join("grammars");
        let src = grammars.join("sources").join("json");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::copy(&parser, grammars.join(format!("json.{ext}"))).unwrap();
        std::fs::copy(&hl, src.join("highlights.scm")).unwrap();
        std::fs::write(src.join("textobjects.scm"), "(pair) @entry.inside\n").unwrap();
        std::fs::write(src.join("indents.scm"), "(object) @indent\n").unwrap();
    });

    assert!(errors.is_empty(), "unexpected init errors: {errors:?}");
    let id = ed
        .state
        .config
        .languages
        .id_of("json")
        .expect("json must be interned");
    let bundle = ed
        .state
        .config
        .languages
        .load_grammar(id, &mut ed.view.registry)
        .expect("json loads");
    assert!(bundle.textobjects.is_some());
    assert!(bundle.indents.is_some());
    assert!(bundle.injections.is_none());
}

/// The shipped language data carries each language's root markers:
/// `go` lists two, `gomod` none.
#[test]
fn bundled_languages_carry_root_markers() {
    let config_tmp = tempfile::tempdir().unwrap();
    let data_tmp = tempfile::tempdir().unwrap();
    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(hume_config.join("init.scm"), "").unwrap();

    let mut ed = editor_from("-[a]>b\n");
    ed.state.dirs = Dirs {
        config: Some(hume_config),
        data: Some(data_tmp.path().join("hume")),
        runtime: Some(repo_runtime_dir()),
        ..process_dirs()
    };
    ed.init_scripting(&mut Default::default());
    let bid = ed.focused_buffer_id();
    ed.set_buffer_language_by_name(bid, Some("go"), hume_scripting::ConfigOrigin::User);
    ed.set_buffer_language_by_name(bid, Some("gomod"), hume_scripting::ConfigOrigin::User);

    let languages = &ed.state.config.languages;
    let go = languages
        .id_of("go")
        .expect("go must be a bundled language");
    let gomod = languages
        .id_of("gomod")
        .expect("gomod must be a bundled language");
    assert!(languages.bundled_loaded(gomod));
    assert_eq!(languages.roots_of(go), ["go.work", "go.mod"]);
    assert!(languages.roots_of(gomod).is_empty());
}
