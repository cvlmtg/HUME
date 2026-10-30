// Editor-level tests for the tree-sitter grammar wiring that load Steel
// plugins or run `init_scripting` end-to-end. The platform-neutral half
// (and the shared runtime-data/fixture helpers) lives in
// `../scripting_grammar.rs`.
//
// Tests that use the grammar fixture (grammar_fixture()) require the shared
// library fetched by `scripts/fetch-test-grammars.sh`. Each calls
// require_grammars first, which panics naming the fix if a fixture is
// missing.

use super::*;
use hume_platform::dirs::Dirs;

use std::path::PathBuf;
use std::sync::Arc;

use super::super::scripting_grammar::{
    grammar_fixture, grammar_warnings, prelude_source, runtime_scheme_dir,
};
use crate::editor::tests::before_save::warning_texts;
use hume_scripting::ScriptingHost;
use hume_treesitter::registry::GrammarBundle;
use test_fixtures::require_grammars;

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

    let mut host = ScriptingHost::for_testing(&Dirs::none());
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih)
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
    let mut host = ScriptingHost::for_testing(&Dirs::none());
    let effects = {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih)
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

/// Passive grammar registration: an init that registers grammars directly
/// succeeds without error and populates the pending language regs. A `(call! "unknown-cmd")` in the same init raises,
/// naming the command; caught here, it leaves the registrations in place.
#[test]
fn passive_load_registers_grammar_and_an_unknown_call_raises() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let ext = test_fixtures::grammar_platform_ext();

    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("hume");
    std::fs::create_dir_all(data_dir.join("plugins")).unwrap();

    let grammar_out = tmp.path().join(format!("json.{ext}"));
    std::fs::copy(&parser, &grammar_out).unwrap();
    let hl_dest = tmp.path().join("json-hl.scm");
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
(with-handler (lambda (err) (log! 'warning (to-string err)))
  (call! "plum/ensure-grammars"))
        "#,
        hl = hl_dest.display(),
        grammar_out = grammar_out.display(),
    );
    std::fs::write(&init_path, prelude_src + &body).unwrap();

    let mut host = ScriptingHost::for_testing(&Dirs {
        data: Some(data_dir.clone()),
        ..Dirs::none()
    });
    let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::at_start(
        hume_editing::text::BufferText::empty(),
    ));
    let effects = {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih)
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
    let msgs = host.take_pending_messages();
    assert!(
        msgs.iter()
            .any(|(_, msg)| msg.contains("unknown command: plum/ensure-grammars")),
        "the unknown call! must raise naming the command; got: {:?}",
        msgs,
    );
}

/// Tab on `:plum-install-grammar`'s argument completes against the real
/// shipped language data (`runtime/scheme/languages/`), not a fixture:
/// no network, no install, so it needs no `require_grammars`. "ag" is unique to "agda"
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
    ed.set_buffer_language_by_name(bid, Some("tsx"), hume_core::ConfigOrigin::User);

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
/// Installs the JSON grammar fixture under `<data>/grammars/`, points the
/// runtime dir at the repo's real `runtime/` dir (so the real
/// language data and `grammars.scm` registrar run), and runs
/// `init_scripting` against an `init.scm` that never mentions PLUM.
#[test]
fn grammar_registration_survives_plum_absence() {
    require_grammars(&["json"]);
    // No core:plum anywhere in init.scm.
    let fixture = StagedGrammarFixture::new("json", "");

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
    // The documented override pattern: redefine an already-grammared
    // language in init.scm to add an extension.
    let fixture =
        StagedGrammarFixture::new("json", r#"(define-language! "json" '("json" "jsonc"))"#);

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
// Registration is driven by <data>/grammars/<name>/<install>/, reading each
// installed grammar's manifest
// ---------------------------------------------------------------------------

/// A bundled `json` data file, which has no part in grammar registration.
const JSON_DATA: &str = "()";

/// Lays out an installed `name` grammar under `data/grammars/<name>/1/`, as
/// PLUM installs one: a library holding `library`, a manifest naming
/// `symbol`, and each `(file, text)` of `queries` under `queries/`.
fn install_grammar_files(
    data: &std::path::Path,
    name: &str,
    symbol: &str,
    library: &[u8],
    queries: &[(&str, &str)],
) {
    let dir = data.join("grammars").join(name).join("1");
    std::fs::create_dir_all(dir.join("queries")).unwrap();
    let ext = test_fixtures::grammar_platform_ext();
    std::fs::write(dir.join(format!("{name}.{ext}")), library).unwrap();
    std::fs::write(
        dir.join("manifest.scm"),
        format!("((name . \"{name}\")\n (symbol . \"{symbol}\"))\n"),
    )
    .unwrap();
    for (file, text) in queries {
        std::fs::write(dir.join("queries").join(file), text).unwrap();
    }
}

/// An installed json grammar whose library cannot load, with an empty
/// highlights query.
fn install_bogus_json_grammar(data: &std::path::Path) {
    install_grammar_files(
        data,
        "json",
        "tree_sitter_json",
        b"not a real library",
        &[("highlights.scm", "; empty\n")],
    );
}

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

/// Startup takes an installed grammar's symbol from its manifest and reads
/// no language data file: a `json.scm` that cannot even be parsed is never
/// touched, and the grammar is registered under the manifest's symbol.
#[test]
fn startup_registers_from_the_manifest_without_reading_language_data() {
    let broken = "((roots \"x\"";

    let (errors, ed, _dirs) = init_errors_with_json_data(broken, |data| {
        install_bogus_json_grammar(data);
    });

    assert!(errors.is_empty(), "no data file is read: {errors:?}");
    assert!(warning_texts(&ed).is_empty(), "{:?}", warning_texts(&ed));
    assert!(ed.state.config.languages.has_grammar("json"));
}

/// An installed grammar under a name that is not a language (installed, then
/// dropped by a HUME update) is skipped without a message.
#[test]
fn orphan_installed_grammar_is_skipped_not_registered() {
    let (errors, ed, _dirs) = init_errors_with_json_data(JSON_DATA, |data| {
        install_grammar_files(
            data,
            "no-longer-a-language",
            "tree_sitter_x",
            b"not a real library",
            &[("highlights.scm", "; empty\n")],
        );
    });

    assert!(errors.is_empty(), "{errors:?}");
    assert!(warning_texts(&ed).is_empty(), "{:?}", warning_texts(&ed));
    assert!(
        !ed.state
            .config
            .languages
            .has_grammar("no-longer-a-language")
    );
}

/// An installed grammar of a language, missing its `highlights.scm`, is
/// repairable, so it warns naming the grammar and the reinstall command
/// instead of being skipped without a message like an orphan.
#[test]
fn installed_grammar_missing_highlights_warns_and_is_not_registered() {
    let (errors, ed, _dirs) = init_errors_with_json_data(JSON_DATA, |data| {
        install_grammar_files(data, "json", "tree_sitter_json", b"not a real library", &[]);
    });

    assert!(errors.is_empty(), "a warning, not an error: {errors:?}");
    let warnings = warning_texts(&ed);
    assert!(
        warnings.iter().any(|w| w.contains("\"json\"")
            && w.contains("highlights")
            && w.contains(":plum-install-grammar json")),
        "{warnings:?}"
    );
    assert!(!ed.state.config.languages.has_grammar("json"));
}

/// An installed grammar with no manifest has no symbol to load, so it warns
/// naming the grammar and the reinstall command.
#[test]
fn installed_grammar_without_a_manifest_warns_and_is_not_registered() {
    let (errors, ed, _dirs) = init_errors_with_json_data(JSON_DATA, |data| {
        install_bogus_json_grammar(data);
        std::fs::remove_file(data.join("grammars/json/1/manifest.scm")).unwrap();
    });

    assert!(errors.is_empty(), "{errors:?}");
    let warnings = warning_texts(&ed);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("\"json\"") && w.contains(":plum-install-grammar json")),
        "{warnings:?}"
    );
    assert!(!ed.state.config.languages.has_grammar("json"));
}

/// A grammar directory holding another platform's library (e.g. a `.so` in
/// a data directory shared with a Linux setup) is not installed here, so it
/// is neither registered nor warned about.
#[test]
fn foreign_platform_grammar_is_skipped_without_a_warning() {
    let wrong_ext = if cfg!(target_os = "macos") {
        "so"
    } else {
        "dylib"
    };

    let (errors, ed, _dirs) = init_errors_with_json_data(JSON_DATA, |data| {
        install_bogus_json_grammar(data);
        let dir = data.join("grammars/json/1");
        let ext = test_fixtures::grammar_platform_ext();
        std::fs::rename(
            dir.join(format!("json.{ext}")),
            dir.join(format!("json.{wrong_ext}")),
        )
        .unwrap();
    });

    assert!(errors.is_empty(), "{errors:?}");
    assert!(warning_texts(&ed).is_empty(), "{:?}", warning_texts(&ed));
    assert!(!ed.state.config.languages.has_grammar("json"));
}

/// Startup registers an installed grammar without opening its library, so a
/// library that cannot load costs nothing and says nothing until a buffer of
/// its language needs it, and then says so once.
#[test]
fn init_registration_of_a_bogus_library_is_silent_until_a_buffer_needs_it() {
    let (errors, mut ed, _dirs) = init_errors_with_json_data(JSON_DATA, |data| {
        install_bogus_json_grammar(data);
    });
    assert!(errors.is_empty(), "unexpected init errors: {errors:?}");
    assert!(grammar_warnings(&ed, "json").is_empty());
    assert!(ed.state.config.languages.has_grammar("json"));

    let bid = ed.focused_buffer_id();
    ed.set_buffer_language_by_name(bid, Some("json"), hume_core::ConfigOrigin::User);
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
    let mut host = ScriptingHost::for_testing(&Dirs::none());
    let mut ed = editor_from("-[a]>b\n");
    {
        let mut ih = init_host!(ed);
        host.eval_init(&init_path, 10_000, &mut ih)
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

/// `installed-grammar-folders` pairs each grammar directory of a known
/// language that holds this platform's library with its current install,
/// sorted by name: a directory with no library, a stray file and a grammar
/// of no language are left out.
#[test]
fn installed_grammar_folders_pair_each_known_library_directory_with_its_install() {
    let (errors, mut ed, dirs) = init_errors_with_json_data(JSON_DATA, |data| {
        for name in ["b-x", "b", "orphan"] {
            install_grammar_files(data, name, "tree_sitter_b", b"", &[]);
        }
        std::fs::create_dir_all(data.join("grammars/no-library")).unwrap();
        std::fs::write(data.join("grammars/.DS_Store"), b"").unwrap();
    });
    assert!(errors.is_empty(), "unexpected init errors: {errors:?}");
    let languages_dir = dirs[1].path().join("scheme/languages");
    for name in ["b", "b-x"] {
        std::fs::write(languages_dir.join(format!("{name}.scm")), "()").unwrap();
    }

    let mut host = ed.scripting.take().expect("init_scripting installs a host");
    let listed = {
        let mut ih = init_host!(ed);
        host.eval_source_for_value(r#"(string-join (map (lambda (p) (string-append (car p) "/" (file-name (cdr p)))) (installed-grammar-folders)) ",")"#, &mut ih)
    };
    ed.scripting = Some(host);

    assert_eq!(
        listed.expect("installed-grammar-folders").to_string(),
        "\"b/1,b-x/1\""
    );
}

/// Startup registration reads every optional query file present beside
/// `highlights.scm`: with `textobjects.scm` and `indents.scm` present the
/// bundle has text objects and indents and no injections.
#[test]
fn register_installed_grammar_picks_up_only_the_optional_files_present() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");

    let (errors, mut ed, _dirs) = init_errors_with_json_data(JSON_DATA, |data| {
        install_grammar_files(
            data,
            "json",
            "tree_sitter_json",
            &std::fs::read(&parser).unwrap(),
            &[
                ("highlights.scm", &std::fs::read_to_string(&hl).unwrap()),
                ("textobjects.scm", "(pair) @entry.inside\n"),
                ("indents.scm", "(object) @indent\n"),
            ],
        );
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
    ed.set_buffer_language_by_name(bid, Some("go"), hume_core::ConfigOrigin::User);
    ed.set_buffer_language_by_name(bid, Some("gomod"), hume_core::ConfigOrigin::User);

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

// ── Grammar loads and the command budget ─────────────────────────────────────

/// Well above the scheduling noise of the Scheme each probe runs.
const GRAMMAR_LOAD_BUDGET_MS: usize = 250;

/// How long each grammar library open takes in these tests, several times
/// [`GRAMMAR_LOAD_BUDGET_MS`].
const SLOW_OPEN: Duration = Duration::from_secs(1);

/// An editor showing `text`, whose data dir holds the `installed` fixture
/// grammars, registered at startup and not yet loaded. The command budget is
/// [`GRAMMAR_LOAD_BUDGET_MS`] and each library open waits [`SLOW_OPEN`]
/// inside the scripting host's own budget pause.
fn editor_with_installed_grammars(data_tmp: &Path, installed: &[&str], text: &str) -> Editor {
    for name in installed {
        test_fixtures::install_fixture_grammar(&data_tmp.join("hume/grammars"), name);
    }
    let mut ed = editor_from(text);
    load_plum(&mut ed, data_tmp);
    ed.state.settings.steel_command_budget_ms = GRAMMAR_LOAD_BUDGET_MS;
    let budget = ed.scripting.as_ref().expect("scripting is on").os_wait();
    ed.state.library_wait.connect(move |open| {
        budget.waiting(|| {
            std::thread::sleep(SLOW_OPEN);
            open();
        })
    });
    ed
}

/// Runs `:probe` and checks it outlasted its budget, which only a library
/// open through the slow wait makes it do.
fn run_probe(ed: &mut Editor) {
    let started = Instant::now();
    type_cmd(ed, ":probe");
    let elapsed = started.elapsed();
    assert!(
        elapsed > Duration::from_millis(GRAMMAR_LOAD_BUDGET_MS as u64),
        "the probe took {elapsed:?}, within its budget"
    );
}

fn grammar_loaded(ed: &Editor, name: &str) -> bool {
    let languages = &ed.state.config.languages;
    languages
        .id_of(name)
        .is_some_and(|id| languages.loaded_grammar(id).is_some())
}

#[test]
fn registering_a_grammar_from_a_command_does_not_count_its_load() {
    require_grammars(&["yaml"]);
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_with_installed_grammars(data_tmp.path(), &[], "-[x]>\n");
    test_fixtures::install_fixture_grammar(
        &canonical_data_dir(data_tmp.path()).join("grammars"),
        "yaml",
    );
    run(
        &mut ed,
        data_tmp.path(),
        r#"(define-typed-command! "probe" "" (lambda () (register-installed-grammar! "yaml" (%grammar-current-install "yaml"))))"#,
    );

    run_probe(&mut ed);

    assert_eq!(error_log(&ed), Vec::<String>::new());
    assert!(grammar_loaded(&ed, "yaml"));
}

#[test]
fn a_popup_whose_language_loads_its_grammar_does_not_count_the_load() {
    require_grammars(&["yaml"]);
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_with_installed_grammars(data_tmp.path(), &["yaml"], "-[x]>\n");
    assert!(
        !grammar_loaded(&ed, "yaml"),
        "premise: registered, not loaded"
    );
    run(
        &mut ed,
        data_tmp.path(),
        r#"(define-typed-command! "probe" "" (lambda () (show-popup! (focused-pane) "a: 1" #:lang "yaml")))"#,
    );

    run_probe(&mut ed);

    assert_eq!(error_log(&ed), Vec::<String>::new());
    let popup = ed.state.input.popup().expect("the popup is shown");
    assert!(popup.syntax.is_some());
}

/// An edit command that `call!` runs loads the grammar of a language the
/// edit injects: the edit fences the cursor's line as yaml, and toggling its
/// comment uses yaml's `#`.
#[test]
fn an_editor_command_that_loads_an_injected_grammar_does_not_count_the_load() {
    require_grammars(&["markdown", "markdown.inline", "yaml"]);
    let data_tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_with_installed_grammars(
        data_tmp.path(),
        &["markdown", "markdown.inline", "yaml"],
        "x\n-[a]>: 1\n",
    );
    type_cmd(&mut ed, ":set buffer language=markdown");
    ed.reparse_stale_buffers();
    assert!(
        !grammar_loaded(&ed, "yaml"),
        "premise: registered, not loaded"
    );
    run(
        &mut ed,
        data_tmp.path(),
        r#"(define-typed-command! "probe" ""
             (lambda ()
               (edit-text! (focused-pane)
                           (list (hash 'start-char 0 'end-char 2 'text "```yaml\n")
                                 (hash 'start-char 7 'end-char 7 'text "```\n")))
               (call! "toggle-comment" (focused-pane))))"#,
    );

    run_probe(&mut ed);

    assert_eq!(error_log(&ed), Vec::<String>::new());
    assert!(grammar_loaded(&ed, "yaml"));
    assert_eq!(ed.doc().text().to_string(), "```yaml\n# a: 1\n```\n");
}
