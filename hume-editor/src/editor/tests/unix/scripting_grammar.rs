// Editor-level tests for the tree-sitter grammar wiring that load Steel
// plugins or run `init_scripting` end-to-end. The platform-neutral half
// (and the shared catalog/fixture helpers) lives in
// `../scripting_grammar.rs`.
//
// Tests that use the grammar fixture (grammar_fixture()) require the shared
// library built by `scripts/fetch-test-grammars.sh`. Each calls
// require_grammars first, which panics naming the fix if a fixture is
// missing.

use super::*;

use std::path::PathBuf;

use super::super::render_snapshot::render_to_styled_string;
use super::super::scripting_grammar::{
    grammar_fixture, grammar_source, helix_pin, runtime_scheme_dir,
};
use crate::editor::scripting_setup::make_init_host;
use hume_scripting::ScriptingHost;
use hume_test_fixtures::require_grammars;

/// Blobless-clone `url` at `rev` into `dest`, test-fixture-only — mirrors the
/// two-step shape `plum/install-grammar` now runs via `run-inline-output!`
/// (the removed `hume_platform::process::git_clone_rev`'s Rust
/// implementation collapsed clone+checkout into one call; full-trust plugin
/// model, see `user-manual/docs/plugins.md`'s "Filesystem and processes",
/// moved that shape to Scheme).
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

/// Fetch `url` to `dest` via curl, test-fixture-only — mirrors
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
/// and the execute path auto-sweeps open buffers of that language.
///
/// Exercises: register_grammar command branch → attach_grammar → theme.bake →
/// Effect::GrammarSweep → apply_script_effects → sweep_buffers_for_grammars
/// → setup_buffer_syntax → Buffer.syntax (Syntax::attach).
///
/// Flip: if the command body ran in init mode (queuing instead of attaching),
/// no sweep would fire and syntax would stay None.
#[test]
fn register_grammar_command_mode_attaches_and_sweeps() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let tmp = safe_tempdir();
    let init_path = tmp.path().join("init.scm");
    // `register-grammar!` is a prelude.scm macro (like `define-language!`) —
    // prepend the real prelude source so it's in scope, since this test evals
    // `init_path` directly rather than through the full `init_scripting` path.
    let prelude_src = std::fs::read_to_string(runtime_scheme_dir().join("prelude.scm")).unwrap();
    let body = format!(
        r#"(define-command! "attach-json" "Attach JSON grammar" (lambda () (register-grammar! "json" "{}" "tree_sitter_json" "{}")))"#,
        parser.display(),
        hl.display(),
    );
    std::fs::write(&init_path, prelude_src + "\n" + &body).unwrap();

    let mut host = ScriptingHost::new();
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init");
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[], None)
        .unwrap();
    let lang = ed.state.config.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    assert!(
        ed.state.buffers.get(bid).syntax.is_none(),
        "no grammar attached yet"
    );

    ed.scripting = Some(host);
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

// ---------------------------------------------------------------------------
// Startup (call! …) during init
// ---------------------------------------------------------------------------

/// Passive grammar registration: an init that registers installed grammars
/// directly (like core's `register-installed-grammars!` in
/// `runtime/scheme/grammars.scm`) succeeds without error and populates the
/// pending language regs.  A `(call! "unknown-cmd")` in
/// the same init logs a warning but does not abort — unknown commands are
/// soft failures during init (buffer access unavailable; command not native).
///
/// Flip: if passive load crashed, `eval_init` would return `Err`.  If the
/// unknown `(call!)` aborted the eval, the grammar registration that preceded
/// it would not show up in `pending_language_regs`.
#[test]
fn passive_load_registers_grammar_and_unknown_call_logs_warning() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let ext = hume_test_fixtures::grammar_platform_ext();

    let tmp = safe_tempdir();
    let data_dir = tmp.path().join("hume");
    std::fs::create_dir_all(data_dir.join("grammars/sources")).unwrap();
    std::fs::create_dir_all(data_dir.join("plugins")).unwrap();

    let grammar_out = data_dir.join("grammars").join(format!("json.{ext}"));
    std::fs::copy(&parser, &grammar_out).unwrap();
    let hl_dest = data_dir.join("grammars/sources/json-hl.scm");
    std::fs::copy(&hl, &hl_dest).unwrap();

    let init_path = tmp.path().join("init.scm");
    // `register-grammar!` is a prelude.scm macro — prepend the real prelude
    // source so it's in scope (see `register_grammar_command_mode_attaches_and_sweeps`).
    let prelude_src = std::fs::read_to_string(runtime_scheme_dir().join("prelude.scm")).unwrap();
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
(call! "plum-ensure-grammars")
        "#,
        hl = hl_dest.display(),
        grammar_out = grammar_out.display(),
    );
    std::fs::write(&init_path, prelude_src + &body).unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(data_dir.clone());
    let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
        hume_editing::text::BufferText::empty(),
        hume_editing::selection::SelectionSet::default(),
    ));
    let effects = {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
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
            .any(|(_, msg)| msg.contains("plum-ensure-grammars")),
        "unknown command in init must log a warning; got: {:?}",
        msgs,
    );
}

// ---------------------------------------------------------------------------
// e2e grammar install (network + tree-sitter CLI required)
// ---------------------------------------------------------------------------

/// End-to-end: clone → curl → tree-sitter build → register-grammar! for JSON.
/// Requires `git`, `curl`, and `tree-sitter` on `PATH`, and network access.
#[test]
fn install_real_json_grammar_e2e() {
    // git/curl/tree-sitter are all spawned by unqualified name below, so this
    // test is a `PATH` reader for its whole duration —
    // `scripting_lsp_install.rs` narrows process `PATH` to an empty or
    // shim-only dir in several tests, and a spawn landing inside that window
    // resolves to nothing (see `Global::Env`'s doc).
    let _lock = TEST_GLOBALS.claim(Global::Env);

    // Read the JSON grammar's url + pinned rev straight from the runtime catalog
    // (single source of truth — no hardcoded pins to drift out of sync).
    let (url, rev) = grammar_source("json");
    let (url, rev) = (url.as_str(), rev.as_str());

    let tmp = safe_tempdir();
    let data_dir = tmp.path().join("hume");
    std::fs::create_dir_all(data_dir.join("grammars/sources")).unwrap();
    std::fs::create_dir_all(data_dir.join("plugins")).unwrap();

    let src_dir = data_dir.join("grammars/sources/json");
    let ext = hume_test_fixtures::grammar_platform_ext();
    let out_path = data_dir.join("grammars").join(format!("json.{ext}"));

    let status = git_clone_rev_for_test(url, &src_dir, rev);
    match &status {
        Err(e) => panic!("git_clone_rev failed: {e}"),
        Ok(s) if !s.success() => panic!("git_clone_rev non-zero exit"),
        Ok(_) => {}
    }
    assert!(src_dir.exists(), "clone must create src dir");

    let status = hume_platform::process::tree_sitter_build(&src_dir, &out_path)
        .expect("tree_sitter_build must not fail to spawn");
    assert!(status.success(), "tree-sitter build failed");
    assert!(out_path.exists(), "compiled grammar must exist after build");

    let hl_path = src_dir.join("highlights.scm");
    // Fetch highlights query via curl, using the helix commit pinned in the catalog.
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
    // `register-grammar!` is a prelude.scm macro — prepend the real prelude
    // source so it's in scope (see `register_grammar_command_mode_attaches_and_sweeps`).
    // Command name must not contain digits: parse_typed_command stops the name
    // scan at the first non-[A-Za-z_-] char (Vim convention — digits are args).
    let prelude_src = std::fs::read_to_string(runtime_scheme_dir().join("prelude.scm")).unwrap();
    let body = format!(
        r#"(define-command! "attach-json" "attach json grammar" (lambda () (register-grammar! "json" "{}" "tree_sitter_json" "{}")))"#,
        out_path.display(),
        hl_path.display(),
    );
    std::fs::write(&init_path, prelude_src + &body).unwrap();

    let mut host = ScriptingHost::new();
    host.set_data_dir(data_dir);
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init");
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[], None)
        .unwrap();
    let lang = ed.state.config.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    ed.scripting = Some(host);

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
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    ed.view.theme.bake(&ed.view.registry);

    // Styled-frame snapshot: locks down token colours after the full e2e pipeline.
    let rect = ratatui::layout::Rect::new(0, 0, 40, 5);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

// ---------------------------------------------------------------------------
// Startup ordering invariant
// ---------------------------------------------------------------------------

/// Helper: write a temp-runtime `scheme/prelude.scm` (copied verbatim from the
/// real runtime — it's self-contained and defines the `define-language!`
/// macro) plus a caller-supplied `scheme/languages.scm`, point
/// `HUME_RUNTIME`/`XDG_CONFIG_HOME`/`XDG_DATA_HOME` at temp dirs, give the
/// editor's buffer a path so extension-based detection fires, and run
/// `init_scripting`. Caller must keep the returned `TempDir`s alive.
fn setup_editor_with_languages_scm(
    languages_scm: &str,
    file_name: &str,
) -> (Editor, Vec<tempfile::TempDir>) {
    let _lock = TEST_GLOBALS.claim(Global::Env);

    let config_tmp = safe_tempdir();
    let runtime_tmp = safe_tempdir();
    let data_tmp = safe_tempdir();

    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(hume_config.join("init.scm"), "").unwrap();

    let scheme_dir = runtime_tmp.path().join("scheme");
    std::fs::create_dir_all(&scheme_dir).unwrap();
    let prelude_src = std::fs::read_to_string(runtime_scheme_dir().join("prelude.scm")).unwrap();
    std::fs::write(scheme_dir.join("prelude.scm"), prelude_src).unwrap();
    std::fs::write(scheme_dir.join("languages.scm"), languages_scm).unwrap();

    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .buffers
        .get_mut(bid)
        .set_path(Some(PathBuf::from(file_name)));

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", config_tmp.path());
        std::env::set_var("HUME_RUNTIME", runtime_tmp.path());
        std::env::set_var("XDG_DATA_HOME", data_tmp.path());
    }

    ed.init_scripting(&mut Default::default());

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HUME_RUNTIME");
        std::env::remove_var("XDG_DATA_HOME");
    }

    (ed, vec![config_tmp, runtime_tmp, data_tmp])
}

/// `define-language!`'s `#:language-id` keyword — introduced as a plain
/// function (converted from a `syntax-rules` macro) so it can take an
/// optional trailing keyword arg — round-trips through the real `prelude.scm`
/// exactly like the old positional-only calls still used everywhere else in
/// `languages.scm`. Exercises both call shapes side by side: `plain-lang` (no
/// keyword, the pre-existing shape) and `tsx` (with the override).
///
/// Flip: reverting `%define-language!`'s arity (or `prelude.scm`'s function)
/// to the old 4-arg macro breaks eval outright — `init_scripting` would log
/// an error and neither identity would register.
#[test]
fn define_language_language_id_keyword_round_trips_through_real_prelude() {
    let languages_scm = r#"
        (define-language! "plain-lang" '("plainext"))
        (define-language! "tsx" '("tsx") #:language-id "typescriptreact")
    "#;

    let (ed, _dirs) = setup_editor_with_languages_scm(languages_scm, "test.tsx");

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        errors.is_empty(),
        "#:language-id must parse and eval cleanly: {errors:?}"
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
        "a call with no #:language-id must fall back to the name"
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

/// The actual bug report this whole feature exists for: opening a `.tsx`
/// file made `typescript-language-server` log "Invalid languageId \"tsx\"
/// ... Correcting to \"typescriptreact\"", because the bundled
/// `languages.scm` sent HUME's own language name as the wire `languageId`.
/// Boots against the real, shipped `runtime/` dir (not a hand-written
/// fixture) so this fails if a future `scripts/sync-grammars.py` run ever
/// drops the `#:language-id` overrides again.
///
/// Fail oracle: drop `tsx`'s `#:language-id` override from `languages.scm`
/// (or have `lsp_language_id_of` fall back to `name_of`) → the assertion
/// below sees `"tsx"` instead of `"typescriptreact"` and fails.
#[test]
fn tsx_bundled_language_id_is_typescriptreact() {
    let _lock = TEST_GLOBALS.claim(Global::Env);
    let config_tmp = safe_tempdir();
    let data_tmp = safe_tempdir();
    let hume_config = config_tmp.path().join("hume");
    std::fs::create_dir_all(&hume_config).unwrap();
    std::fs::write(hume_config.join("init.scm"), "").unwrap();
    let real_runtime = concat!(env!("CARGO_MANIFEST_DIR"), "/../runtime");

    let mut ed = editor_from("-[a]>b\n");
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", config_tmp.path());
        std::env::set_var("HUME_RUNTIME", real_runtime);
        std::env::set_var("XDG_DATA_HOME", data_tmp.path());
    }
    ed.init_scripting(&mut Default::default());
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HUME_RUNTIME");
        std::env::remove_var("XDG_DATA_HOME");
    }

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
/// parse has been posted to the background worker — so the run loop's first
/// frame is highlighted at most one poll later, never long after a b/w flash.
///
/// The detection + parse-post happens via the end-of-init
/// `detect_and_set_language` loop in `scripting_setup.rs`.
///
/// Flip: comment out that end-of-init loop — `syntax` stays `None` after
/// `init_scripting` and the first assertion fails.
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
        "tree must not be installed yet — only posted; drained on the next \
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
        ed.state.buffers.get(bid).text_gen,
        "parsed_gen must catch up to text_gen after the drain"
    );
}

// ---------------------------------------------------------------------------
// Grammar registration survives core:plum's absence
// ---------------------------------------------------------------------------

/// `runtime/scheme/grammars.scm` registers already-compiled grammars
/// unconditionally at startup, so highlighting for an installed grammar must
/// not depend on `core:plum` being declared in `init.scm` at all — PLUM is
/// only needed to *install* a grammar in the first place.
///
/// Stages a real compiled JSON grammar at the exact paths core's
/// `grammar-output-path`/`grammar-highlights-path` expect, points
/// `HUME_RUNTIME` at the repo's real `runtime/` dir (so the real
/// `grammar-sources.scm` catalog and `grammars.scm` registrar run), and runs
/// `init_scripting` against an `init.scm` that never mentions PLUM.
///
/// Flip: if grammar registration were still PLUM-only, `has_grammar` would
/// stay false and the buffer would render unhighlighted.
#[test]
fn grammar_registration_survives_plum_absence() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    // No core:plum anywhere in init.scm.
    let fixture = StagedGrammarFixture::new("json", &parser, &hl, "");

    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .buffers
        .get_mut(bid)
        .set_path(Some(PathBuf::from("test.json")));
    ed.init_scripting(&mut Default::default());
    drop(fixture);

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
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

    // Sanity: PLUM's own commands really are unavailable — this run never
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
        warns.contains(&"Unknown command: plum-install-grammar".to_string()),
        "core:plum commands must be unavailable when it isn't declared; got {warns:?}"
    );
}

// ---------------------------------------------------------------------------
// A user override in init.scm must not undo a startup grammar attachment
// ---------------------------------------------------------------------------

/// `grammars.scm` attaches every already-compiled grammar before `init.scm`
/// runs (`scripting_setup.rs`: prelude → languages → grammars → init.scm).
/// `languages.scm`'s own header documents overriding an entry by redefining
/// it in `init.scm` — this exercises exactly that documented pattern for an
/// already-grammared language and asserts highlighting survives it.
///
/// Flip: if `register_identity_no_rebuild` still dropped an attached grammar
/// on re-registration, `has_grammar` would go false and the buffer would
/// render unhighlighted after `init.scm`'s `define-language!` call.
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
    let bid = ed.focused_buffer_id();
    ed.state
        .buffers
        .get_mut(bid)
        .set_path(Some(PathBuf::from("test.jsonc")));
    ed.init_scripting(&mut Default::default());
    drop(fixture);

    let errors: Vec<String> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        errors.is_empty(),
        "init.scm's define-language! override must not error: {errors:?}"
    );

    assert!(
        ed.state.config.languages.has_grammar("json"),
        "grammar attached by grammars.scm must survive init.scm's identity override"
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
// Registration is driven by <data>/grammars/, and the catalog is lazy
// ---------------------------------------------------------------------------

/// Run `init_scripting` against a temp runtime holding the real `prelude.scm`
/// and `grammars.scm` but a caller-supplied `grammar-sources.scm` and a
/// deliberately tiny `languages.scm`, with `populate_data` free to lay out the
/// data dir first. Returns the editor's error log. Caller must keep the
/// returned `TempDir`s alive.
fn init_errors_with_catalog(
    catalog_src: &str,
    populate_data: impl FnOnce(&std::path::Path),
) -> (Vec<String>, Editor, Vec<tempfile::TempDir>) {
    let _lock = TEST_GLOBALS.claim(Global::Env);

    let config_tmp = safe_tempdir();
    let runtime_tmp = safe_tempdir();
    let data_tmp = safe_tempdir();

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
    std::fs::write(scheme_dir.join("grammar-sources.scm"), catalog_src).unwrap();

    populate_data(&data_tmp.path().join("hume"));

    let mut ed = editor_from("-[a]>b\n");
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", config_tmp.path());
        std::env::set_var("HUME_RUNTIME", runtime_tmp.path());
        std::env::set_var("XDG_DATA_HOME", data_tmp.path());
    }
    ed.init_scripting(&mut Default::default());
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HUME_RUNTIME");
        std::env::remove_var("XDG_DATA_HOME");
    }

    let errors = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Error)
        .map(|e| e.text.clone())
        .collect();
    (errors, ed, vec![config_tmp, runtime_tmp, data_tmp])
}

/// The grammar source catalog is parsed on first use, not at startup: with no
/// `<data>/grammars/` directory there is nothing to register, so a catalog
/// that cannot even be read must never be touched.
///
/// Uses a syntactically broken catalog as the tripwire — if anything forces it,
/// `grammars.scm` raises and `init_scripting` logs an error. The second half
/// (a compiled file present ⇒ the same broken catalog now *does* raise) is what
/// keeps this from being a zero-effect assertion: it proves the tripwire works
/// and that the first half passed for the right reason.
#[test]
fn grammar_catalog_is_read_lazily_on_first_use() {
    let broken = "( (\"json\" \"url\" \"rev\" \"sym\"";

    let (errors, ..) = init_errors_with_catalog(broken, |_data| {});
    assert!(
        errors.is_empty(),
        "no <data>/grammars/ ⇒ the catalog must never be read: {errors:?}"
    );

    // A bare sources/ subdirectory yields no grammar names either — still no read.
    let (errors, ..) = init_errors_with_catalog(broken, |data| {
        std::fs::create_dir_all(data.join("grammars").join("sources")).unwrap();
    });
    assert!(
        errors.is_empty(),
        "only sources/ present ⇒ still no grammar names, so still no read: {errors:?}"
    );

    // Tripwire check: one compiled file forces the catalog, which then fails.
    let ext = hume_test_fixtures::grammar_platform_ext();
    let (errors, ..) = init_errors_with_catalog(broken, |data| {
        let grammars = data.join("grammars");
        std::fs::create_dir_all(&grammars).unwrap();
        std::fs::write(grammars.join(format!("json.{ext}")), b"not a real library").unwrap();
    });
    assert!(
        errors.iter().any(|e| e.contains("grammars.scm")),
        "a compiled grammar must force the catalog, surfacing the broken file: {errors:?}"
    );
}

/// A compiled grammar whose name is no longer in the catalog (installed, then
/// dropped by a HUME update) has no tree-sitter symbol to look up. Walking the
/// install directory reaches it where the old catalog-driven walk never could,
/// so registration must skip it rather than raise on the missing entry.
///
/// Flip: drop the `grammar-source-known?` guard in `register-installed-grammars!`
/// and `grammar-source-symbol`'s `hash-ref` raises, failing the error assertion.
#[test]
fn orphan_compiled_grammar_is_skipped_not_registered() {
    let catalog = "((\"json\" \"url\" \"rev\" \"tree_sitter_json\" \"\"))";
    let ext = hume_test_fixtures::grammar_platform_ext();

    let (errors, ed, _dirs) = init_errors_with_catalog(catalog, |data| {
        let grammars = data.join("grammars");
        std::fs::create_dir_all(&grammars).unwrap();
        std::fs::write(
            grammars.join(format!("no-longer-in-catalog.{ext}")),
            b"not a real library",
        )
        .unwrap();
        // A highlights query too: without it the `when` short-circuits before
        // the symbol lookup and the guard under test never runs.
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

/// A grammar the catalog still knows about, with a compiled library on disk
/// but no `highlights.scm` (e.g. the user cleared `<data>/grammars/sources/`
/// to reclaim disk), must warn — naming the grammar so `:plum-install-grammar
/// <name>` is the obvious next step — instead of being dropped as silently as
/// a genuine orphan. Distinguishes the two cases `register-installed-grammars!`
/// must tell apart: unknown-to-catalog (expected-silent) vs. known-but-broken
/// (repairable, must be surfaced).
#[test]
fn known_grammar_missing_highlights_warns_and_is_not_registered() {
    let catalog = "((\"json\" \"url\" \"rev\" \"tree_sitter_json\" \"\"))";
    let ext = hume_test_fixtures::grammar_platform_ext();

    let (errors, ed, _dirs) = init_errors_with_catalog(catalog, |data| {
        let grammars = data.join("grammars");
        std::fs::create_dir_all(&grammars).unwrap();
        std::fs::write(grammars.join(format!("json.{ext}")), b"not a real library").unwrap();
        // Deliberately no highlights.scm under sources/json/.
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
/// at all — `register-grammar!` must not even be attempted for it.
///
/// `has_grammar` alone can't tell them apart: `grammar-output-path` always
/// re-derives the *platform's* extension regardless of which file the walk
/// matched, so an attempt on this entry is doomed to fail on a missing path
/// either way — attempted-and-failed and never-attempted both leave `json`
/// unregistered. What differs is whether the attempt happens: a failed init-time
/// attach logs a `Warning` (`editor/syntax/mod.rs`), so an attempt leaves a
/// trace in the message log that a skip does not.
///
/// Flip: match on "has any extension" instead of the platform extension in
/// `installed-grammars` and this file starts matching, so `register-grammar!`
/// is attempted and logs `register-grammar! 'json': grammar library not found`.
#[test]
fn wrong_extension_grammar_is_skipped_not_registered() {
    let catalog = "((\"json\" \"url\" \"rev\" \"tree_sitter_json\" \"\"))";
    let wrong_ext = if cfg!(target_os = "macos") {
        "so"
    } else {
        "dylib"
    };

    let (errors, ed, _dirs) = init_errors_with_catalog(catalog, |data| {
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
        "a foreign-extension file must be skipped before ever attempting register-grammar!, \
         not attempted-and-failed: {warnings:?}"
    );
}
