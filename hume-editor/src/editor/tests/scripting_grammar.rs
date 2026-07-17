// Editor-level tests for the tree-sitter grammar wiring: setup_buffer_syntax,
// reparse_stale_buffers, sweep_buffers_for_grammars, the register-grammar!
// Steel builtin, and the M9.3 install pipeline.
//
// Tests that use the grammar fixture (grammar_fixture()) require the shared
// library built by `scripts/fetch-test-grammars.sh`.  On a fixture-less
// checkout the helper panics with a clear install message; CI installs
// fixtures before running tests so panic never fires there.

use super::*;

use std::path::PathBuf;
use std::sync::Arc;

use super::render_snapshot::render_to_styled_string;
use crate::editor::scripting_setup::make_init_host;
use hume_scripting::ScriptingHost;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// `<repo>/runtime/scheme/` — the runtime catalog directory.
fn runtime_scheme_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("runtime/scheme")
}

/// Read `(url, rev)` for grammar `name` from `grammar-sources.scm`, the same
/// catalog PLUM reads at runtime. Avoids duplicating pins into the test (they
/// drift otherwise). Entries are 5-tuples of quoted strings:
///   ("name" "url" "rev" "symbol" "subpath")
/// so splitting a matched line on `"` puts the values at odd indices.
fn grammar_source(name: &str) -> (String, String) {
    let catalog = std::fs::read_to_string(runtime_scheme_dir().join("grammar-sources.scm"))
        .expect("read grammar-sources.scm");
    let needle = format!("(\"{name}\" ");
    let line = catalog
        .lines()
        .find(|l| l.trim_start().starts_with(&needle))
        .unwrap_or_else(|| panic!("no grammar-sources.scm entry for '{name}'"));
    let fields: Vec<&str> = line.split('"').collect();
    (fields[3].to_string(), fields[5].to_string())
}

/// Read the pinned helix commit SHA from `helix-pin.scm` (one quoted literal,
/// the rest of the file is `;;;` comments).
fn helix_pin() -> String {
    let s = std::fs::read_to_string(runtime_scheme_dir().join("helix-pin.scm"))
        .expect("read helix-pin.scm");
    s.lines()
        .find(|l| !l.trim_start().starts_with(';') && l.contains('"'))
        .and_then(|l| {
            let a = l.find('"')? + 1;
            let b = l[a..].find('"')? + a;
            Some(l[a..b].to_string())
        })
        .expect("helix-pin.scm must contain a quoted SHA")
}

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

fn grammar_fixture(name: &str) -> (PathBuf, PathBuf) {
    let parser = grammar_parser_path(name);
    if !parser.exists() {
        panic!(
            "grammar fixture missing: {}\ninstall the tree-sitter CLI (npm i -g tree-sitter-cli) and run scripts/fetch-test-grammars.sh from the repo root",
            parser.display()
        );
    }
    (parser, grammar_query_path(name))
}

// ---------------------------------------------------------------------------
// Direct-attach tests (Rust API only; no Steel dispatch)
// ---------------------------------------------------------------------------

/// Flip: without attach_grammar the grammar field is None so setup_buffer_syntax
/// returns early — all three handles stay None.
#[test]
fn attach_then_set_language_attaches_syntax() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    ed.state
        .languages
        .attach_grammar(
            "json",
            &parser,
            "tree_sitter_json",
            &hl,
            None,
            &mut ed.view.registry,
        )
        .unwrap();
    let lang = ed.state.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers();
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "syntax must be set after attach"
    );
    assert!(
        ed.state
            .buffers
            .get(bid)
            .syntax
            .as_ref()
            .unwrap()
            .layers()
            .is_some(),
        "engine tree must be set"
    );
}

/// Flip: if clear didn't propagate, parser/syntax/tree would still be Some after set(None).
#[test]
fn clear_language_detaches_syntax_keeps_identity() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    ed.state
        .languages
        .attach_grammar(
            "json",
            &parser,
            "tree_sitter_json",
            &hl,
            None,
            &mut ed.view.registry,
        )
        .unwrap();
    let lang = ed.state.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    assert!(ed.state.buffers.get(bid).syntax.is_some());

    ed.set_buffer_language(bid, None);
    assert!(
        ed.state.buffers.get(bid).syntax.is_none(),
        "syntax attachment (and its committed tree) must be cleared on language=None"
    );
    // Identity survives detach — grammar is gone, language definition is not.
    assert!(
        ed.state.languages.by_name("json").is_some(),
        "identity must survive grammar detach"
    );
}

/// Flip: if sweep ignored the name filter it would attach after the rust-sweep midpoint.
#[test]
fn sweep_attaches_syntax_on_matching_language() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    // Set language BEFORE grammar is attached — no syntax yet.
    let lang = ed.state.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    assert!(
        ed.state.buffers.get(bid).syntax.is_none(),
        "no grammar → parser must be absent"
    );

    ed.state
        .languages
        .attach_grammar(
            "json",
            &parser,
            "tree_sitter_json",
            &hl,
            None,
            &mut ed.view.registry,
        )
        .unwrap();
    let json_id = ed.state.languages.intern("json");
    ed.sweep_buffers_for_grammars(vec![json_id]);
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "sweep must attach parser when language matches"
    );
}

/// Flip: if sweep applies to all buffers regardless of name, the first assert would fail.
#[test]
fn sweep_no_op_for_nonmatching_language() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    // Set language but don't attach grammar yet — parser stays absent.
    let lang = ed.state.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    assert!(ed.state.buffers.get(bid).syntax.is_none());

    // Sweep for a different language — must leave the json buffer untouched.
    let rust_id = ed.state.languages.intern("rust");
    ed.sweep_buffers_for_grammars(vec![rust_id]);
    assert!(
        ed.state.buffers.get(bid).syntax.is_none(),
        "wrong-language sweep must not attach parser for json buffer",
    );

    // Sanity flip: sweeping "json" does attach.
    ed.state
        .languages
        .attach_grammar(
            "json",
            &parser,
            "tree_sitter_json",
            &hl,
            None,
            &mut ed.view.registry,
        )
        .unwrap();
    let json_id = ed.state.languages.intern("json");
    ed.sweep_buffers_for_grammars(vec![json_id]);
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "correct-language sweep must attach parser"
    );
}

/// Flip: without reparse_stale_buffers the parsed_gen would stay at gen0 even after the edit.
#[test]
fn reparse_advances_parsed_gen_after_edit() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    ed.state
        .languages
        .attach_grammar(
            "json",
            &parser,
            "tree_sitter_json",
            &hl,
            None,
            &mut ed.view.registry,
        )
        .unwrap();
    let lang = ed.state.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers(); // drain the initial parse result

    // setup_buffer_syntax sets parsed_gen = text_gen (after drain).
    let gen0 = ed.state.buffers.get(bid).text_gen;
    assert_eq!(
        ed.state
            .buffers
            .get(bid)
            .syntax
            .as_ref()
            .unwrap()
            .parsed_gen(),
        gen0,
        "parsed_gen must equal text_gen after initial setup",
    );

    // Insert a character — bumps text_gen.
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key_esc());
    let gen1 = ed.state.buffers.get(bid).text_gen;
    assert!(gen1 > gen0, "edit must bump text_gen");
    assert_eq!(
        ed.state
            .buffers
            .get(bid)
            .syntax
            .as_ref()
            .unwrap()
            .parsed_gen(),
        gen0,
        "parsed_gen must lag behind text_gen before reparse",
    );

    // First call posts the request (inline: parse + stash); result not yet installed.
    ed.reparse_stale_buffers();
    // Second call drains and installs.
    ed.reparse_stale_buffers();
    assert_eq!(
        ed.state
            .buffers
            .get(bid)
            .syntax
            .as_ref()
            .unwrap()
            .parsed_gen(),
        gen1,
        "reparse must advance parsed_gen to current text_gen",
    );

    // Third call is a no-op — parsed_gen stays at gen1.
    ed.reparse_stale_buffers();
    assert_eq!(
        ed.state
            .buffers
            .get(bid)
            .syntax
            .as_ref()
            .unwrap()
            .parsed_gen(),
        gen1,
        "third reparse must be a no-op when gen already matches",
    );
}

/// Flip: without the max_bytes gate, parser would still be Some after reparse.
#[test]
fn reparse_detaches_when_buffer_exceeds_max_bytes() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    ed.state
        .languages
        .attach_grammar(
            "json",
            &parser,
            "tree_sitter_json",
            &hl,
            None,
            &mut ed.view.registry,
        )
        .unwrap();
    let lang = ed.state.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "syntax must be set initially"
    );

    // Any non-empty buffer exceeds 1 byte.
    ed.state.settings.syntax_highlight_max_bytes = 1;
    ed.reparse_stale_buffers();
    assert!(
        ed.state.buffers.get(bid).syntax.is_none(),
        "syntax must detach when exceeding max_bytes"
    );
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
#[cfg(not(windows))]
fn register_grammar_command_mode_attaches_and_sweeps() {
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
// has_grammar reflection
// ---------------------------------------------------------------------------

/// Flip: if has_grammar ignored grammar presence it would return true for identity-only.
#[test]
fn language_has_grammar_false_for_identity_only_true_after_attach() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[a]>b\n");
    ed.state
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    assert!(
        !ed.state.languages.has_grammar("json"),
        "identity without grammar → has_grammar false"
    );
    assert!(
        !ed.state.languages.has_grammar("unknown"),
        "unknown language → has_grammar false"
    );

    ed.state
        .languages
        .attach_grammar(
            "json",
            &parser,
            "tree_sitter_json",
            &hl,
            None,
            &mut ed.view.registry,
        )
        .unwrap();
    assert!(
        ed.state.languages.has_grammar("json"),
        "has_grammar must be true after attach"
    );
}

// ---------------------------------------------------------------------------
// Fix 1 — replace_buffer_in_place must clear stale engine syntax state
// ---------------------------------------------------------------------------

/// Regression: replace_buffer_in_place must clear the buffer's syntax
/// attachment (and with it, the committed tree it owns).
///
/// Flip: if `*buffers.get_mut(id) = new_doc` were skipped, the assert fails.
#[test]
fn replace_buffer_in_place_clears_engine_syntax_state() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    ed.state
        .languages
        .attach_grammar(
            "json",
            &parser,
            "tree_sitter_json",
            &hl,
            None,
            &mut ed.view.registry,
        )
        .unwrap();
    let lang = ed.state.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers();
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "syntax (and its committed tree) must be set before replace"
    );

    // Replace with a scratch buffer (no path, language=None). detect_and_set_language
    // returns None → set_buffer_language no-ops; the whole-Buffer swap in
    // buffer::lifecycle::replace_buffer_in_place is the load-bearing cleanup here.
    ed.replace_buffer_in_place(bid, Buffer::scratch());

    assert!(
        ed.state.buffers.get(bid).syntax.is_none(),
        "stale syntax (and its committed tree) must be cleared on replace"
    );
}

// ---------------------------------------------------------------------------
// Fix 3 — reparse_stale_buffers must re-attach on shrink below cap
// ---------------------------------------------------------------------------

/// Regression: once a buffer's syntax is detached (via the max_bytes growth branch),
/// reparse_stale_buffers must re-attach it on shrink below cap. Without the re-attach
/// branch, the second `reparse_stale_buffers` call leaves parser=None.
///
/// Flip: if the re-attach branch is removed, the final `parser.is_some()` assert fails.
#[test]
fn reparse_reattaches_after_shrink_under_cap() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    ed.state
        .languages
        .attach_grammar(
            "json",
            &parser,
            "tree_sitter_json",
            &hl,
            None,
            &mut ed.view.registry,
        )
        .unwrap();
    let lang = ed.state.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "syntax must be set initially"
    );

    // Force detach by setting a 1-byte cap — any non-empty buffer exceeds it.
    ed.state.settings.syntax_highlight_max_bytes = 1;
    ed.reparse_stale_buffers();
    assert!(
        ed.state.buffers.get(bid).syntax.is_none(),
        "syntax must detach when exceeding cap"
    );

    // Restore a generous cap — next reparse must re-attach.
    ed.state.settings.syntax_highlight_max_bytes = usize::MAX;
    ed.reparse_stale_buffers();
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "syntax must re-attach when buffer shrinks back under cap",
    );
}

// ---------------------------------------------------------------------------
// Highlighting self-heals after :e! reload
// ---------------------------------------------------------------------------

/// `reload_buffer_in_place` (`:e!`) must keep syntax highlighting alive across
/// a reload when the buffer's language is unchanged.
///
/// Mechanism: `reload_from_text` leaves `state.syntax` (the highlighter) intact
/// and bumps `text_gen`. `reparse_stale_buffers` sees `syntax.is_some()` + a
/// gen mismatch → posts a fresh full parse (no pending edits, so no incremental
/// baking) → second tick drains and installs the new tree.
///
/// The `end_byte()` assertion catches the reparse loop failing to post or
/// install a request against the new content.  `state.syntax.is_some()` would
/// fail if `detect_and_set_language` incorrectly cleared the language on reload.
/// One thing the test cannot probe: that `Syntax::clear_layers` was called
/// immediately on reload; that prevents a one-frame stale-tree in the renderer
/// but is invisible to `InlineParseBackend`.
#[test]
fn reload_buffer_in_place_keeps_syntax_highlighting() {
    use crate::editor::buffer::Buffer;
    use hume_editing::selection::SelectionSet;
    use hume_editing::text::Text;

    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();

    // Give the buffer a path so detect_and_set_language keeps returning "json"
    // after reload (pathless buffers re-detect to None, which is a language
    // *change* — a different code path than the one we're testing here).
    ed.state
        .buffers
        .get_mut(bid)
        .set_path(Some(std::path::PathBuf::from("data.json")));

    ed.state
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    ed.state
        .languages
        .attach_grammar(
            "json",
            &parser,
            "tree_sitter_json",
            &hl,
            None,
            &mut ed.view.registry,
        )
        .unwrap();
    let lang = ed.state.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers();

    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "syntax must be attached before reload"
    );
    assert!(
        ed.state
            .buffers
            .get(bid)
            .syntax
            .as_ref()
            .unwrap()
            .layers()
            .is_some(),
        "tree must be installed before reload"
    );

    // Reload with different-length content — language stays "json" (path unchanged).
    // `{"x": 1}\n` (original, 9 bytes) → `[1, 2, 3]\n` (replacement, 10 bytes).
    // The byte-length difference makes the tree-alignment check below definitive:
    // a stale tree (not rebuilt) would report end_byte() == 9, not 10.
    let new_text = "[1, 2, 3]\n";
    let new_byte_len = new_text.len();
    let mut replacement = Buffer::new(Text::from(new_text), SelectionSet::default());
    replacement.set_path(Some(std::path::PathBuf::from("data.json")));
    ed.reload_buffer_in_place(bid, replacement);
    // Two ticks: first `reparse_stale_buffers` sees the gen mismatch and posts the
    // parse request (InlineParseBackend completes synchronously into the done queue);
    // the second drains and installs the tree. The real run loop does the same across
    // two event-loop iterations.
    ed.reparse_stale_buffers();
    ed.reparse_stale_buffers();

    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "highlighter must survive reload"
    );
    let tree = ed
        .state
        .buffers
        .get(bid)
        .syntax
        .as_ref()
        .unwrap()
        .layers()
        .and_then(hume_engine::syntax_layers::SyntaxLayers::root_tree)
        .expect("engine tree must be re-installed after reload");
    assert_eq!(
        tree.root_node().end_byte(),
        new_byte_len,
        "engine tree must be aligned to the reloaded content, not the stale pre-reload text"
    );
}

// ---------------------------------------------------------------------------
// M9.4 — Off-main-thread parse worker
// ---------------------------------------------------------------------------

/// The reparse path is two-phase: the first `reparse_stale_buffers` call after an
/// edit posts the request (the inline backend stashes the result immediately), but
/// the result is not installed until `drain_done` runs — which happens at the top of
/// the next `reparse_stale_buffers` call.
#[test]
fn parse_worker_result_is_async_then_installed() {
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    ed.state
        .languages
        .attach_grammar(
            "json",
            &parser,
            "tree_sitter_json",
            &hl,
            None,
            &mut ed.view.registry,
        )
        .unwrap();
    let lang = ed.state.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers(); // drain initial parse result

    let gen0 = ed.state.buffers.get(bid).text_gen;

    // Edit — bumps text_gen.
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key_esc());
    let gen1 = ed.state.buffers.get(bid).text_gen;
    assert!(gen1 > gen0);

    // First call: drain (nothing) then post request.  Result stashed but not yet installed.
    ed.reparse_stale_buffers();
    assert_eq!(
        ed.state
            .buffers
            .get(bid)
            .syntax
            .as_ref()
            .unwrap()
            .parsed_gen(),
        gen0,
        "parsed_gen must still lag after first reparse_stale_buffers (result not yet drained)",
    );

    // Second call: drain installs the stashed result.
    ed.reparse_stale_buffers();
    assert_eq!(
        ed.state
            .buffers
            .get(bid)
            .syntax
            .as_ref()
            .unwrap()
            .parsed_gen(),
        gen1,
        "parsed_gen must equal text_gen after second reparse_stale_buffers",
    );
}

/// Verify that sweeping with a new grammar after one is already in flight does not
/// leave a stale `InFlight` entry that silences the follow-up request.
///
/// Flip: if sweep_buffers_for_grammars did not clear in_flight[bid], the
/// next reparse_stale_buffers call would see in_flight.text_gen == text_gen and
/// skip posting, leaving parsed_gen permanently stale.
#[test]
fn grammar_swap_clears_stale_in_flight() {
    let (parser_json, hl_json) = grammar_fixture("json");
    let (parser_rust, hl_rust) = grammar_fixture("rust");
    let mut ed = editor_from("-[f]>n main() {}\n");
    let bid = ed.focused_buffer_id();

    // Register json and set buffer to json.
    ed.state
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    ed.state
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .unwrap();
    ed.state
        .languages
        .attach_grammar(
            "json",
            &parser_json,
            "tree_sitter_json",
            &hl_json,
            None,
            &mut ed.view.registry,
        )
        .unwrap();
    let lang = ed.state.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers(); // drain json parse result

    // Attach rust grammar and sweep — this should clear any json in-flight and post fresh.
    let rust_bundle = ed
        .state
        .languages
        .attach_grammar(
            "rust",
            &parser_rust,
            "tree_sitter_rust",
            &hl_rust,
            None,
            &mut ed.view.registry,
        )
        .unwrap();
    let lang = ed.state.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers(); // drain rust parse result

    assert!(
        Arc::ptr_eq(
            ed.state.buffers.get(bid).syntax.as_ref().unwrap().bundle(),
            &rust_bundle
        ),
        "buffer must be parsed with rust grammar after swap"
    );
    assert!(
        ed.state
            .buffers
            .get(bid)
            .syntax
            .as_ref()
            .unwrap()
            .layers()
            .is_some(),
        "rust parse must produce a tree"
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
#[cfg(not(windows))]
fn passive_load_registers_grammar_and_unknown_call_logs_warning() {
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
#[cfg(not(windows))]
fn install_real_json_grammar_e2e() {
    use std::process::Command;

    let require_live = std::env::var("HUME_REQUIRE_LIVE_GRAMMAR_E2E")
        .map(|v| v == "1")
        .unwrap_or(false);

    if !require_live {
        eprintln!(
            "install_real_json_grammar_e2e: skipping \
             (set HUME_REQUIRE_LIVE_GRAMMAR_E2E=1 to run live e2e)"
        );
        return;
    }

    // Check prerequisites; skip unless HUME_REQUIRE_LIVE_GRAMMAR_E2E=1.
    let has_git = Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let has_curl = Command::new("curl")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let has_ts = Command::new("tree-sitter")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !has_git || !has_curl || !has_ts {
        if require_live {
            panic!(
                "HUME_REQUIRE_LIVE_GRAMMAR_E2E=1 but git={has_git} curl={has_curl} tree-sitter={has_ts}"
            );
        }
        eprintln!(
            "install_real_json_grammar_e2e: skipping (git={has_git} curl={has_curl} ts={has_ts})"
        );
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
        Err(e) => {
            if require_live {
                panic!("git_clone_rev failed: {e}");
            }
            eprintln!("install_real_json_grammar_e2e: skipping (clone failed: {e})");
            return;
        }
        Ok(s) if !s.success() => {
            if require_live {
                panic!("git_clone_rev non-zero exit");
            }
            eprintln!("install_real_json_grammar_e2e: skipping (clone non-zero exit)");
            return;
        }
        Ok(_) => {}
    }
    assert!(src_dir.exists(), "clone must create src dir");

    // Step 2: tree-sitter build
    let status = hume_platform::process::tree_sitter_build(&src_dir, &out_path)
        .expect("tree_sitter_build must not fail to spawn");
    if !status.success() {
        if require_live {
            panic!("tree-sitter build failed");
        }
        eprintln!("install_real_json_grammar_e2e: skipping (build failed)");
        return;
    }
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
        Err(e) => {
            if require_live {
                panic!("curl_fetch failed: {e}");
            }
            eprintln!("install_real_json_grammar_e2e: skipping (curl failed: {e})");
            return;
        }
        Ok(s) if !s.success() => {
            if require_live {
                panic!("curl_fetch non-zero exit");
            }
            eprintln!("install_real_json_grammar_e2e: skipping (curl non-zero)");
            return;
        }
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

/// `grammar_source` / `helix_pin` must parse real values out of the runtime
/// catalog so the e2e installs the actually-pinned revision. Always runs (no
/// network), so a malformed catalog or a broken parser is caught in normal CI.
///
/// Flip: point `grammar_source` at a bogus field index → the SHA-length /
/// prefix assertions below fail.
#[test]
fn catalog_parsing_extracts_json_pins() {
    let (url, rev) = grammar_source("json");
    assert_eq!(url, "https://github.com/tree-sitter/tree-sitter-json");
    assert_eq!(
        rev.len(),
        40,
        "git rev must be a full 40-char SHA, got: {rev}"
    );
    assert!(
        rev.chars().all(|c| c.is_ascii_hexdigit()),
        "rev must be hex: {rev}"
    );

    let pin = helix_pin();
    assert!(!pin.is_empty(), "helix pin must be non-empty");
    assert!(
        pin.chars().all(|c| c.is_ascii_hexdigit()),
        "helix pin must be hex: {pin}"
    );
}

/// Attach the pre-built Rust grammar fixture and snapshot the styled render output.
///
/// Locks down that token colours actually reach the screen — not just that
/// highlight spans are emitted.  The cursor sits on the trailing `\n` so no
/// content cell is reverse-video.
///
/// Requires `scripts/fetch-test-grammars.sh` (handled by `grammar_fixture`).
///
/// Flip: remove `theme.bake` and this snapshot fails because all scopes resolve
/// to the default style — proving the assertion is not a zero-effect check.
#[test]
fn rust_function_highlight_snapshot() {
    use hume_engine::builtins::tree_sitter_hl::layer_highlights_for_line;

    let (parser, hl) = grammar_fixture("rust");
    // Cursor on the trailing `\n` so no token cell is reverse-video in the snapshot.
    let mut ed = editor_from("// hi\nfn main() {\n    let x: u32 = 1;\n}-[\n]>");
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    ed.doc_mut().set_path(Some(PathBuf::from("test.rs")));

    let bid = ed.focused_buffer_id();
    ed.state
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .unwrap();
    ed.state
        .languages
        .attach_grammar(
            "rust",
            &parser,
            "tree_sitter_rust",
            &hl,
            None,
            &mut ed.view.registry,
        )
        .expect("attach rust grammar");
    // Bake after scopes are interned so theme.resolve() returns correct styles.
    ed.view.theme.bake(&ed.view.registry);

    let lang = ed.state.languages.intern("rust");
    ed.set_buffer_language(bid, Some(lang));
    // Drain the parse result posted synchronously inside setup_buffer_syntax
    // (InlineParseBackend completes the parse inside post; drain installs the tree).
    ed.reparse_stale_buffers();

    // Sanity: line 1 ("fn main() {") must emit at least one `keyword` span.
    // Runs the highlight pipeline directly so the test fails even if the snapshot
    // renderer masks absent colours behind cursor/selection background.
    {
        let syn = ed.state.buffers.get(bid).syntax.as_ref().unwrap();
        let layers = syn
            .layers()
            .expect("engine syntax layers must be installed");
        let rope = ed.state.buffers.get(bid).text().rope();
        let mut raw = Vec::new();
        let mut stack = Vec::new();
        let mut events = Vec::new();
        let mut spans = Vec::new();
        layer_highlights_for_line(
            layers,
            1,
            rope,
            &mut raw,
            &mut stack,
            &mut events,
            &mut spans,
        );
        assert!(
            !spans.is_empty(),
            "line 1 must emit at least one highlight span"
        );
        assert!(
            spans
                .iter()
                .any(|&(_, _, id)| ed.view.registry.name_of(id).contains("keyword")),
            "expected a 'keyword' scope for `fn`; got: {:?}",
            spans
                .iter()
                .map(|&(_, _, id)| ed.view.registry.name_of(id))
                .collect::<Vec<_>>(),
        );
    }

    let rect = ratatui::layout::Rect::new(0, 0, 30, 8);
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
#[cfg(not(windows))]
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
#[cfg(not(windows))]
fn initial_buffer_parse_is_in_flight_by_end_of_init_scripting() {
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
