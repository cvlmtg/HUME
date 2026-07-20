// Editor-level tests for the tree-sitter grammar wiring that load Steel
// plugins or run `init_scripting` end-to-end. The platform-neutral half
// (and the shared catalog/fixture helpers) lives in
// `../scripting_grammar.rs`.
//
// Tests that use the grammar fixture (grammar_fixture()) require the shared
// library built by `scripts/fetch-test-grammars.sh`.  Each gates on
// skip_unless_grammars first: a fixture-less checkout skips with a note;
// HUME_REQUIRE_GRAMMAR_FIXTURES=1 (CI, scripts/test-all.sh) turns the same
// gate into a hard failure instead.

use super::*;

use std::path::PathBuf;

use super::super::render_snapshot::render_to_styled_string;
use super::super::scripting_grammar::{
    grammar_fixture, grammar_source, helix_pin, runtime_scheme_dir,
};
use crate::editor::scripting_setup::make_init_host;
use hume_scripting::ScriptingHost;
use hume_test_fixtures::skip_unless_grammars;

/// Blobless-clone `url` at `rev` into `dest`, test-fixture-only — mirrors the
/// two-step shape `plum/install-grammar` now runs via `run-inline-output!`
/// (the removed `hume_platform::process::git_clone_rev`'s Rust
/// implementation collapsed clone+checkout into one call; full-trust plugin
/// model, see `docs/ROADMAP.md`, moved that shape to Scheme).
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
/// pending_grammar_sweeps → SteelCmdResult.grammar_sweeps → sweep_buffers_for_grammars
/// → setup_buffer_syntax → Buffer.syntax (Syntax::attach).
///
/// Flip: if the command body ran in init mode (queuing instead of attaching),
/// no sweep would fire and syntax would stay None.
#[test]
fn register_grammar_command_mode_attaches_and_sweeps() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let (parser, hl) = grammar_fixture("json");
    let tmp = tempfile::tempdir().unwrap();
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
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    let lang = ed.state.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    assert!(
        ed.state.buffers.get(bid).syntax.is_none(),
        "no grammar attached yet"
    );

    ed.scripting = Some(host);
    type_cmd(&mut ed, ":attach-json");

    assert!(
        ed.state.languages.has_grammar("json"),
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
/// directly (like plum/register-installed-grammars!) succeeds without error
/// and populates the pending language regs.  A `(call! "unknown-cmd")` in
/// the same init logs a warning but does not abort — unknown commands are
/// soft failures during init (buffer access unavailable; command not native).
///
/// Flip: if passive load crashed, `eval_init` would return `Err`.  If the
/// unknown `(call!)` aborted the eval, the grammar registration that preceded
/// it would not show up in `pending_language_regs`.
#[test]
fn passive_load_registers_grammar_and_unknown_call_logs_warning() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let (parser, hl) = grammar_fixture("json");
    let ext = if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    };

    let tmp = tempfile::tempdir().unwrap();
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
        hume_editing::text::Text::empty(),
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
///
/// Gated by `HUME_REQUIRE_LIVE_GRAMMAR_E2E=1`; otherwise skipped when git,
/// curl, or tree-sitter is absent or GitHub is unreachable.
#[test]
fn install_real_json_grammar_e2e() {
    if hume_test_fixtures::skip_unless_live_grammar_e2e("install_real_json_grammar_e2e") {
        return;
    }

    // Read the JSON grammar's url + pinned rev straight from the runtime catalog
    // (single source of truth — no hardcoded pins to drift out of sync).
    let (url, rev) = grammar_source("json");
    let (url, rev) = (url.as_str(), rev.as_str());

    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("hume");
    std::fs::create_dir_all(data_dir.join("grammars/sources")).unwrap();
    std::fs::create_dir_all(data_dir.join("plugins")).unwrap();

    let src_dir = data_dir.join("grammars/sources/json");
    let ext = if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    };
    let out_path = data_dir.join("grammars").join(format!("json.{ext}"));

    // Step 1: git clone --filter=blob:none
    let status = git_clone_rev_for_test(url, &src_dir, rev);
    match &status {
        Err(e) => panic!("git_clone_rev failed: {e}"),
        Ok(s) if !s.success() => panic!("git_clone_rev non-zero exit"),
        Ok(_) => {}
    }
    assert!(src_dir.exists(), "clone must create src dir");

    // Step 2: tree-sitter build
    let status = hume_platform::process::tree_sitter_build(&src_dir, &out_path)
        .expect("tree_sitter_build must not fail to spawn");
    assert!(status.success(), "tree-sitter build failed");
    assert!(out_path.exists(), "compiled grammar must exist after build");

    // Step 3: register-grammar! via editor scripting
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
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    let lang = ed.state.languages.intern("json");
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
        ed.state.languages.has_grammar("json"),
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
    let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    let config_tmp = tempfile::tempdir().unwrap();
    let runtime_tmp = tempfile::tempdir().unwrap();
    let data_tmp = tempfile::tempdir().unwrap();

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

    ed.init_scripting();

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HUME_RUNTIME");
        std::env::remove_var("XDG_DATA_HOME");
    }

    (ed, vec![config_tmp, runtime_tmp, data_tmp])
}

/// Locks the startup invariant the `run()` reorder (`hume-editor/src/lib.rs`)
/// leans on: by the time `init_scripting` returns, the initial (already-open)
/// buffer's language has been detected from its path and its tree-sitter
/// parse has been posted to the background worker — so the run loop's first
/// frame is highlighted at most one poll later, never long after a b/w flash.
///
/// The detection + parse-post happens via the existing end-of-init
/// `detect_and_set_language` loop in `scripting_setup.rs` — this test does
/// not depend on, and does not require, any early-kickoff variant (an
/// early-detection-pass approach was considered and deliberately rejected;
/// see the ROADMAP decisions table).
///
/// Flip: comment out that end-of-init loop — `syntax` stays `None` after
/// `init_scripting` and the first assertion fails.
#[test]
fn initial_buffer_parse_is_in_flight_by_end_of_init_scripting() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
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
