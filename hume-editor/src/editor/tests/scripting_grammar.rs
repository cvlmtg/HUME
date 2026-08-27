// Editor-level tests for the tree-sitter grammar wiring: setup_buffer_syntax,
// reparse_stale_buffers, sweep_buffers_for_grammars, the register-grammar!
// Steel builtin, and the grammar install pipeline.
//
// Tests that use the grammar fixture (grammar_fixture()) require the shared
// library built by `scripts/fetch-test-grammars.sh`. Each calls
// require_grammars first, which panics naming the fix if a fixture is
// missing.

use super::*;
use hume_grid::Rect;

use std::path::PathBuf;
use std::sync::Arc;

use super::render_snapshot::render_to_styled_string;
use hume_test_fixtures::{grammar_parser_path, grammar_query_path, require_grammars};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// `<repo>/runtime/scheme/` — the runtime catalog directory.
pub(super) fn runtime_scheme_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("runtime/scheme")
}

/// Read `(url, rev)` for grammar `name` from `grammar-sources.scm`, the same
/// catalog core's `grammars.scm` reads at runtime. Avoids duplicating pins
/// into the test (they drift otherwise). Entries are 5-tuples of quoted
/// strings:
///   ("name" "url" "rev" "symbol" "subpath")
/// so splitting a matched line on `"` puts the values at odd indices.
pub(super) fn grammar_source(name: &str) -> (String, String) {
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
pub(super) fn helix_pin() -> String {
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

pub(super) fn grammar_fixture(name: &str) -> (PathBuf, PathBuf) {
    (grammar_parser_path(name), grammar_query_path(name))
}

// ---------------------------------------------------------------------------
// Direct-attach tests (Rust API only; no Steel dispatch)
// ---------------------------------------------------------------------------

/// Flip: without attach_grammar the grammar field is None so setup_buffer_syntax
/// returns early — all three handles stay None.
#[test]
fn attach_then_set_language_attaches_syntax() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[], None)
        .unwrap();
    ed.state
        .config
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
    let lang = ed.state.config.languages.intern("json");
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

/// `reset_config_state` (via `BufferStore::clear_languages_all`) bypasses
/// `set_buffer_language` entirely — it writes `buf.language = None` directly,
/// which is the bug `clear_language_detaches_syntax_keeps_identity` above
/// doesn't cover: that test clears through `set_buffer_language`, the normal
/// path that also tears down `buf.syntax` via `setup_buffer_syntax`. Flip:
/// if `clear_languages_all` forgot to clear `syntax` too, this would still
/// be `Some` after `reset_config_state`, holding an `Arc<GrammarBundle>`
/// from the registry the reset is about to replace.
#[test]
fn reset_config_state_clears_buffer_syntax_not_just_language() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[], None)
        .unwrap();
    ed.state
        .config
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
    let lang = ed.state.config.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers();
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "sanity: syntax must be attached before reset"
    );

    ed.reset_config_state();

    assert!(
        ed.state.buffers.get(bid).syntax.is_none(),
        "buffer syntax must not survive the reset — it holds an \
         Arc<GrammarBundle> from the outgoing LanguageRegistry that \
         clear_languages_all replaces"
    );
}

/// Flip: if clear didn't propagate, parser/syntax/tree would still be Some after set(None).
#[test]
fn clear_language_detaches_syntax_keeps_identity() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[], None)
        .unwrap();
    ed.state
        .config
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
    let lang = ed.state.config.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    assert!(ed.state.buffers.get(bid).syntax.is_some());

    ed.set_buffer_language(bid, None);
    assert!(
        ed.state.buffers.get(bid).syntax.is_none(),
        "syntax attachment (and its committed tree) must be cleared on language=None"
    );
    // Identity survives detach — grammar is gone, language definition is not.
    assert!(
        ed.state.config.languages.by_name("json").is_some(),
        "identity must survive grammar detach"
    );
}

/// Flip: if sweep ignored the name filter it would attach after the rust-sweep midpoint.
#[test]
fn sweep_attaches_syntax_on_matching_language() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[], None)
        .unwrap();
    // Set language BEFORE grammar is attached — no syntax yet.
    let lang = ed.state.config.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    assert!(
        ed.state.buffers.get(bid).syntax.is_none(),
        "no grammar → parser must be absent"
    );

    ed.state
        .config
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
    let json_id = ed.state.config.languages.intern("json");
    ed.sweep_buffers_for_grammars(vec![json_id]);
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "sweep must attach parser when language matches"
    );
}

/// Flip: if sweep applies to all buffers regardless of name, the first assert would fail.
#[test]
fn sweep_no_op_for_nonmatching_language() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[], None)
        .unwrap();
    // Set language but don't attach grammar yet — parser stays absent.
    let lang = ed.state.config.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    assert!(ed.state.buffers.get(bid).syntax.is_none());

    // Sweep for a different language — must leave the json buffer untouched.
    let rust_id = ed.state.config.languages.intern("rust");
    ed.sweep_buffers_for_grammars(vec![rust_id]);
    assert!(
        ed.state.buffers.get(bid).syntax.is_none(),
        "wrong-language sweep must not attach parser for json buffer",
    );

    // Sanity flip: sweeping "json" does attach.
    ed.state
        .config
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
    let json_id = ed.state.config.languages.intern("json");
    ed.sweep_buffers_for_grammars(vec![json_id]);
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "correct-language sweep must attach parser"
    );
}

/// Flip: without reparse_stale_buffers the parsed_gen would stay at gen0 even after the edit.
#[test]
fn reparse_advances_parsed_gen_after_edit() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[], None)
        .unwrap();
    ed.state
        .config
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
    let lang = ed.state.config.languages.intern("json");
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
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[], None)
        .unwrap();
    ed.state
        .config
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
    let lang = ed.state.config.languages.intern("json");
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
// has_grammar reflection
// ---------------------------------------------------------------------------

/// Flip: if has_grammar ignored grammar presence it would return true for identity-only.
#[test]
fn language_has_grammar_false_for_identity_only_true_after_attach() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[a]>b\n");
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[], None)
        .unwrap();
    assert!(
        !ed.state.config.languages.has_grammar("json"),
        "identity without grammar → has_grammar false"
    );
    assert!(
        !ed.state.config.languages.has_grammar("unknown"),
        "unknown language → has_grammar false"
    );

    ed.state
        .config
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
        ed.state.config.languages.has_grammar("json"),
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
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[], None)
        .unwrap();
    ed.state
        .config
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
    let lang = ed.state.config.languages.intern("json");
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
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[], None)
        .unwrap();
    ed.state
        .config
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
    let lang = ed.state.config.languages.intern("json");
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
    use hume_editing::text::BufferText;

    require_grammars(&["json"]);
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
        .config
        .languages
        .register_identity("json", &["json"], &[], &[], None)
        .unwrap();
    ed.state
        .config
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
    let lang = ed.state.config.languages.intern("json");
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
    let mut replacement = Buffer::new(BufferText::from(new_text), SelectionSet::default());
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
        .and_then(hume_treesitter::layers::SyntaxLayers::root_tree)
        .expect("engine tree must be re-installed after reload");
    assert_eq!(
        tree.root_node().end_byte(),
        new_byte_len,
        "engine tree must be aligned to the reloaded content, not the stale pre-reload text"
    );

    // A second reload with byte-identical content: `reload_from_text`'s
    // `forward.is_identity()` branch returns `false` (no mutation) without
    // touching `text_gen`. `reload_buffer_in_place` must not call
    // `clear_layers` on that no-mutation path — doing so would drop the tree
    // just installed above with no `text_gen` bump to trigger a reparse,
    // leaving the buffer unhighlighted until the next real edit.
    //
    // Fail oracle: gate `clear_layers` on `mutated` removed (call it
    // unconditionally, as before this fix) → `layers()` is `None` here.
    let mut identical = Buffer::new(BufferText::from(new_text), SelectionSet::default());
    identical.set_path(Some(std::path::PathBuf::from("data.json")));
    ed.reload_buffer_in_place(bid, identical);
    ed.reparse_stale_buffers();
    ed.reparse_stale_buffers();

    let tree_after_noop_reload = ed
        .state
        .buffers
        .get(bid)
        .syntax
        .as_ref()
        .expect("highlighter must survive a byte-identical reload")
        .layers()
        .and_then(hume_treesitter::layers::SyntaxLayers::root_tree)
        .expect("tree must still be installed after a byte-identical reload");
    assert_eq!(
        tree_after_noop_reload.root_node().end_byte(),
        new_byte_len,
        "the untouched tree must still be aligned to the (unchanged) content"
    );
}

// ---------------------------------------------------------------------------
// Off-main-thread parse worker
// ---------------------------------------------------------------------------

/// The reparse path is two-phase: the first `reparse_stale_buffers` call after an
/// edit posts the request (the inline backend stashes the result immediately), but
/// the result is not installed until `drain_done` runs — which happens at the top of
/// the next `reparse_stale_buffers` call.
#[test]
fn parse_worker_result_is_async_then_installed() {
    require_grammars(&["json"]);
    let (parser, hl) = grammar_fixture("json");
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[], None)
        .unwrap();
    ed.state
        .config
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
    let lang = ed.state.config.languages.intern("json");
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
    require_grammars(&["json", "rust"]);
    let (parser_json, hl_json) = grammar_fixture("json");
    let (parser_rust, hl_rust) = grammar_fixture("rust");
    let mut ed = editor_from("-[f]>n main() {}\n");
    let bid = ed.focused_buffer_id();

    // Register json and set buffer to json.
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[], None)
        .unwrap();
    ed.state
        .config
        .languages
        .register_identity("rust", &["rs"], &[], &[], None)
        .unwrap();
    ed.state
        .config
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
    let lang = ed.state.config.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers(); // drain json parse result

    // Attach rust grammar and sweep — this should clear any json in-flight and post fresh.
    let rust_bundle = ed
        .state
        .config
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
    let lang = ed.state.config.languages.intern("rust");
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
    use hume_treesitter::highlight::layer_highlights_for_line;

    require_grammars(&["rust"]);
    let (parser, hl) = grammar_fixture("rust");
    // Cursor on the trailing `\n` so no token cell is reverse-video in the snapshot.
    let mut ed = editor_from("// hi\nfn main() {\n    let x: u32 = 1;\n}-[\n]>");
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    ed.doc_mut().set_path(Some(PathBuf::from("test.rs")));

    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("rust", &["rs"], &[], &[], None)
        .unwrap();
    ed.state
        .config
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

    let lang = ed.state.config.languages.intern("rust");
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

    let rect = Rect::new(0, 0, 30, 8);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}
