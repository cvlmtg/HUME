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

use super::before_save::warning_texts;
use super::render_snapshot::render_to_styled_string;
use test_fixtures::{grammar_parser_path, grammar_query_path, require_grammars};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// `<repo>/runtime/scheme/`: the runtime Scheme directory.
pub(super) fn runtime_scheme_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("runtime/scheme")
}

/// The shipped `prelude.scm`'s source.
pub(super) fn prelude_source() -> String {
    std::fs::read_to_string(runtime_scheme_dir().join("prelude.scm")).unwrap()
}

/// `(url rev symbol subpath)` of the grammar language `name` uses, from its
/// shipped data file, read by `bundled-language-grammar` the way the editor
/// and `core:plum` read it. Avoids duplicating pins into the test (they drift
/// otherwise).
pub(super) fn shipped_grammar(name: &str) -> Vec<String> {
    let mut scripting = hume_scripting::ScriptingHost::new(&repo_runtime_dirs());
    let source = scripting
        .eval_source_for_value(
            &format!("(bundled-language-grammar {name:?})"),
            &mut crate::testing::MockHost::new(),
        )
        .unwrap_or_else(|e| panic!("shipped data for '{name}' must be well-formed: {e}"));
    match source {
        steel::rvals::SteelVal::ListV(fields) => fields
            .iter()
            .map(|field| match field {
                steel::rvals::SteelVal::StringV(s) => s.to_string(),
                other => panic!("grammar fields must be strings, got {other:?}"),
            })
            .collect(),
        other => panic!("no shipped grammar for '{name}': {other:?}"),
    }
}

/// `(url, rev)` of the grammar language `name` uses; see [`shipped_grammar`].
pub(super) fn grammar_source(name: &str) -> (String, String) {
    let mut fields = shipped_grammar(name).into_iter();
    let url = fields.next().expect("a grammar has a url");
    let rev = fields.next().expect("a grammar has a rev");
    (url, rev)
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

/// `setup_buffer_syntax` returns early and leaves `syntax` unset when the
/// language has no grammar to load, so the grammar is attached first.
#[test]
fn attach_then_set_language_attaches_syntax() {
    require_grammars(&["json"]);
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    attach_fixture_grammar(&mut ed, "json", "tree_sitter_json");
    let lang = ed.state.config.languages.intern("json");
    super::language::attach_host(&mut ed, "");
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

#[test]
fn opening_a_buffer_loads_its_registered_grammar() {
    require_grammars(&["json"]);
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    let lang = register_fixture_grammar(&mut ed, "json", "tree_sitter_json");
    assert!(
        ed.state.config.languages.loaded_grammar(lang).is_none(),
        "premise: registered, not loaded"
    );
    super::language::attach_host(&mut ed, "");

    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers();

    assert!(ed.state.config.languages.loaded_grammar(lang).is_some());
    assert!(has_layers(&ed, bid));
}

#[test]
fn a_buffer_over_the_size_cap_loads_no_grammar() {
    require_grammars(&["json"]);
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    let lang = register_fixture_grammar(&mut ed, "json", "tree_sitter_json");
    super::language::attach_host(&mut ed, "");
    ed.state.settings.syntax_highlight_max_bytes = 1;

    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers();

    assert!(ed.state.config.languages.loaded_grammar(lang).is_none());
    assert!(ed.state.buffers.get(bid).syntax.is_none());
}

/// The warnings reporting that `name`'s grammar failed to load.
pub(super) fn grammar_warnings(ed: &Editor, name: &str) -> Vec<String> {
    let prefix = format!("grammar '{name}': ");
    warning_texts(ed)
        .into_iter()
        .filter(|text| text.starts_with(&prefix))
        .collect()
}

#[test]
fn a_registered_grammar_that_fails_to_load_warns_once() {
    let mut ed = editor_from("-[{]>}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    let lang = ed
        .state
        .config
        .languages
        .register_grammar("json", GrammarSource::missing("json"));
    super::language::attach_host(&mut ed, "");
    assert!(
        grammar_warnings(&ed, "json").is_empty(),
        "registering opens nothing"
    );

    ed.set_buffer_language(bid, Some(lang));
    for _ in 0..3 {
        ed.reparse_stale_buffers();
    }
    ed.set_buffer_language(bid, None);
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers();

    assert_eq!(grammar_warnings(&ed, "json").len(), 1);
    assert!(!ed.state.config.languages.has_grammar("json"));
    assert!(ed.state.buffers.get(bid).syntax.is_none());
}

/// `reset_config_state` (via `BufferStore::clear_languages_all`) bypasses
/// `set_buffer_language` entirely: it writes `buf.language = None` directly,
/// a path `clear_language_detaches_syntax_keeps_identity` above doesn't
/// cover: that test clears through `set_buffer_language`, the normal path
/// that also tears down `buf.syntax` via `setup_buffer_syntax`.
/// `clear_languages_all` drops `syntax` too, since it holds an
/// `Arc<GrammarBundle>` from the registry the reset is about to replace.
#[test]
fn reset_config_state_clears_buffer_syntax_not_just_language() {
    require_grammars(&["json"]);
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    attach_fixture_grammar(&mut ed, "json", "tree_sitter_json");
    let lang = ed.state.config.languages.intern("json");
    super::language::attach_host(&mut ed, "");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers();
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "sanity: syntax must be attached before reset"
    );

    ed.reset_config_state();

    assert!(
        ed.state.buffers.get(bid).syntax.is_none(),
        "buffer syntax must not survive the reset: it holds an \
         Arc<GrammarBundle> from the outgoing LanguageRegistry that \
         clear_languages_all replaces"
    );
}

/// Setting the language to `None` drops the syntax attachment and its tree.
#[test]
fn clear_language_detaches_syntax_keeps_identity() {
    require_grammars(&["json"]);
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    attach_fixture_grammar(&mut ed, "json", "tree_sitter_json");
    let lang = ed.state.config.languages.intern("json");
    super::language::attach_host(&mut ed, "");
    ed.set_buffer_language(bid, Some(lang));
    assert!(ed.state.buffers.get(bid).syntax.is_some());

    ed.set_buffer_language(bid, None);
    assert!(
        ed.state.buffers.get(bid).syntax.is_none(),
        "syntax attachment (and its committed tree) must be cleared on language=None"
    );
    // Identity survives detach: grammar is gone, language definition is not.
    assert!(
        ed.state.config.languages.defined_id("json").is_some(),
        "identity must survive grammar detach"
    );
}

/// A sweep for the buffer's own language attaches syntax once the grammar exists.
#[test]
fn sweep_attaches_syntax_on_matching_language() {
    require_grammars(&["json"]);
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    // Set language BEFORE grammar is attached: no syntax yet.
    let lang = ed.state.config.languages.intern("json");
    super::language::attach_host(&mut ed, "");
    ed.set_buffer_language(bid, Some(lang));
    assert!(
        ed.state.buffers.get(bid).syntax.is_none(),
        "no grammar → parser must be absent"
    );

    attach_fixture_grammar(&mut ed, "json", "tree_sitter_json");
    let json_id = ed.state.config.languages.intern("json");
    ed.sweep_buffers_for_grammars(vec![json_id]);
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "sweep must attach parser when language matches"
    );
}

/// A sweep only touches buffers whose language is in the swept list.
#[test]
fn sweep_no_op_for_nonmatching_language() {
    require_grammars(&["json"]);
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    // Set language but don't attach grammar yet: parser stays absent.
    let lang = ed.state.config.languages.intern("json");
    super::language::attach_host(&mut ed, "");
    ed.set_buffer_language(bid, Some(lang));
    assert!(ed.state.buffers.get(bid).syntax.is_none());

    // Sweep for a different language. Must leave the json buffer untouched.
    let rust_id = ed.state.config.languages.intern("rust");
    ed.sweep_buffers_for_grammars(vec![rust_id]);
    assert!(
        ed.state.buffers.get(bid).syntax.is_none(),
        "wrong-language sweep must not attach parser for json buffer",
    );

    // Sweeping "json" does attach.
    attach_fixture_grammar(&mut ed, "json", "tree_sitter_json");
    let json_id = ed.state.config.languages.intern("json");
    ed.sweep_buffers_for_grammars(vec![json_id]);
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "correct-language sweep must attach parser"
    );
}

/// `reparse_stale_buffers` advances `parsed_gen` to the text's generation after an edit.
#[test]
fn reparse_advances_parsed_gen_after_edit() {
    require_grammars(&["json"]);
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    attach_fixture_grammar(&mut ed, "json", "tree_sitter_json");
    let lang = ed.state.config.languages.intern("json");
    super::language::attach_host(&mut ed, "");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers(); // drain the initial parse result

    // setup_buffer_syntax sets parsed_gen = generation (after drain).
    let gen0 = ed.state.buffers.get(bid).text().generation();
    assert_eq!(
        ed.state
            .buffers
            .get(bid)
            .syntax
            .as_ref()
            .unwrap()
            .parsed_gen(),
        Some(gen0),
        "parsed_gen must equal generation after initial setup",
    );

    // Insert a character: changes the text version.
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key_esc());
    let gen1 = ed.state.buffers.get(bid).text().generation();
    assert!(gen1 > gen0, "edit must change the text version");
    assert_eq!(
        ed.state
            .buffers
            .get(bid)
            .syntax
            .as_ref()
            .unwrap()
            .parsed_gen(),
        Some(gen0),
        "parsed_gen must lag behind generation before reparse",
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
        Some(gen1),
        "reparse must advance parsed_gen to current generation",
    );

    // Third call is a no-op: parsed_gen stays at gen1.
    ed.reparse_stale_buffers();
    assert_eq!(
        ed.state
            .buffers
            .get(bid)
            .syntax
            .as_ref()
            .unwrap()
            .parsed_gen(),
        Some(gen1),
        "third reparse must be a no-op when gen already matches",
    );
}

/// A buffer that grows past `syntax_highlight_max_bytes` loses its syntax on the next reparse.
#[test]
fn reparse_detaches_when_buffer_exceeds_max_bytes() {
    require_grammars(&["json"]);
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    attach_fixture_grammar(&mut ed, "json", "tree_sitter_json");
    let lang = ed.state.config.languages.intern("json");
    super::language::attach_host(&mut ed, "");
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

/// `has_grammar` is false for an identity-only language and true once a grammar is attached.
#[test]
fn language_has_grammar_false_for_identity_only_true_after_attach() {
    require_grammars(&["json"]);
    let mut ed = editor_from("-[a]>b\n");
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    assert!(
        !ed.state.config.languages.has_grammar("json"),
        "identity without grammar → has_grammar false"
    );
    assert!(
        !ed.state.config.languages.has_grammar("unknown"),
        "unknown language → has_grammar false"
    );

    attach_fixture_grammar(&mut ed, "json", "tree_sitter_json");
    assert!(
        ed.state.config.languages.has_grammar("json"),
        "has_grammar must be true after attach"
    );
}

#[test]
fn language_has_grammar_is_true_once_registered_and_false_after_a_failed_load() {
    let mut ed = editor_from("-[a]>b\n");
    let id = ed
        .state
        .config
        .languages
        .register_grammar("json", GrammarSource::missing("json"));
    assert!(
        ed.state.config.languages.has_grammar("json"),
        "a registered grammar counts before it loads"
    );

    let loaded = ed
        .state
        .config
        .languages
        .load_grammar(id, &mut ed.view.registry);

    assert!(loaded.is_none());
    assert!(!ed.state.config.languages.has_grammar("json"));
}

// ---------------------------------------------------------------------------
// Closing the last buffer must clear stale engine syntax state
// ---------------------------------------------------------------------------

/// Closing the last open buffer (`buffer::lifecycle::
/// close_buffer`'s `None` branch, which frees `bid`'s slot outright and opens a
/// fresh scratch buffer under a new id) must not leak the closed buffer's
/// syntax attachment (and the committed tree it owns) anywhere the engine
/// can still reach it: `bid`'s own slot must be gone, and the new scratch
/// buffer must start with no syntax attached, not inherit the old tree.
#[test]
fn closing_the_last_buffer_clears_engine_syntax_state() {
    require_grammars(&["json"]);
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    attach_fixture_grammar(&mut ed, "json", "tree_sitter_json");
    let lang = ed.state.config.languages.intern("json");
    super::language::attach_host(&mut ed, "");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers();
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "syntax (and its committed tree) must be set before close"
    );

    // `bid` is the only open buffer, so this hits close_buffer's last-buffer
    // branch: `bid`'s slot is freed outright and a fresh scratch buffer
    // (no path, language=None, no syntax) opens under a new id.
    ed.close_buffer(bid)
        .expect("the buffer has no unsaved changes and runs no pipe");

    assert!(
        ed.state.buffers.try_get(bid).is_none(),
        "the closed buffer's slot, and the stale syntax tree it owned, must be gone entirely"
    );
    let new_bid = ed.focused_buffer_id();
    assert_ne!(new_bid, bid, "the fresh scratch buffer must have a new id");
    assert!(
        ed.state.buffers.get(new_bid).syntax.is_none(),
        "the fresh scratch buffer must not inherit the closed buffer's syntax attachment"
    );
}

// ---------------------------------------------------------------------------
// reparse_stale_buffers must re-attach on shrink below cap
// ---------------------------------------------------------------------------

/// Once a buffer's syntax is detached (via the max_bytes growth branch),
/// reparse_stale_buffers must re-attach it on shrink below cap.
#[test]
fn reparse_reattaches_after_shrink_under_cap() {
    require_grammars(&["json"]);
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    attach_fixture_grammar(&mut ed, "json", "tree_sitter_json");
    let lang = ed.state.config.languages.intern("json");
    super::language::attach_host(&mut ed, "");
    ed.set_buffer_language(bid, Some(lang));
    assert!(
        ed.state.buffers.get(bid).syntax.is_some(),
        "syntax must be set initially"
    );

    // Force detach by setting a 1-byte cap: any non-empty buffer exceeds it.
    ed.state.settings.syntax_highlight_max_bytes = 1;
    ed.reparse_stale_buffers();
    assert!(
        ed.state.buffers.get(bid).syntax.is_none(),
        "syntax must detach when exceeding cap"
    );

    // Restore a generous cap: next reparse must re-attach.
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
/// Mechanism: the reload is an edit, so the committed tree stays and the
/// reload's line diff is recorded as a pending edit. `reparse_stale_buffers`
/// sees the gen mismatch, bakes the edit into the tree and posts an
/// incremental parse; the second tick drains and installs the new tree.
#[test]
fn reload_buffer_in_place_keeps_syntax_highlighting() {
    use hume_editing::text::BufferText;

    require_grammars(&["json"]);
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();

    // Give the buffer a path so detect_and_set_language keeps returning "json"
    // after reload (pathless buffers re-detect to None, which is a language
    // *change*, a different code path than the one we're testing here).
    ed.state
        .buffers
        .get_mut(bid)
        .set_file(crate::editor::buffer::BufferFile::at("/tmp/data.json"));

    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    attach_fixture_grammar(&mut ed, "json", "tree_sitter_json");
    let lang = ed.state.config.languages.intern("json");
    super::language::attach_host(&mut ed, "");
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

    // Reload with different-length content: language stays "json" (path unchanged).
    // `{"x": 1}\n` (original, 9 bytes) → `[1, 2, 3]\n` (replacement, 10 bytes).
    // The lengths differ, so the tree's end_byte() below tells the rebuilt tree
    // (10) from the original one (9).
    let new_text = "[1, 2, 3]\n";
    let new_byte_len = new_text.len();
    ed.reload_buffer_in_place(
        FocusedPane::current(&ed.view),
        ReplaceSource::Dump(BufferText::from(new_text)),
    )
    .expect("the buffer is not held");
    assert!(
        ed.state
            .buffers
            .get(bid)
            .syntax
            .as_ref()
            .unwrap()
            .layers()
            .is_some(),
        "the committed tree must stay until the reparse replaces it"
    );
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

    // A second reload with byte-identical content: `replace_text_recorded`'s
    // `forward.is_identity()` branch returns `false` (no mutation) without
    // touching the text version, and the tree installed above stays.
    ed.reload_buffer_in_place(
        FocusedPane::current(&ed.view),
        ReplaceSource::Dump(BufferText::from(new_text)),
    )
    .expect("the buffer is not held");
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
/// the result is not installed until `drain_done` runs, which happens at the top of
/// the next `reparse_stale_buffers` call.
#[test]
fn parse_worker_result_is_async_then_installed() {
    require_grammars(&["json"]);
    let mut ed = editor_from("-[{]>\"x\": 1}\n");
    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    attach_fixture_grammar(&mut ed, "json", "tree_sitter_json");
    let lang = ed.state.config.languages.intern("json");
    super::language::attach_host(&mut ed, "");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers(); // drain initial parse result

    let gen0 = ed.state.buffers.get(bid).text().generation();

    // Edit: changes the text version.
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key_esc());
    let gen1 = ed.state.buffers.get(bid).text().generation();
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
        Some(gen0),
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
        Some(gen1),
        "parsed_gen must equal generation after second reparse_stale_buffers",
    );
}

/// Switching a parsed buffer to another language's grammar leaves it parsed with
/// the new grammar, with a tree.
#[test]
fn grammar_swap_parses_with_the_new_grammar() {
    require_grammars(&["json", "rust"]);
    let mut ed = editor_from("-[f]>n main() {}\n");
    let bid = ed.focused_buffer_id();

    // Register json and set buffer to json.
    ed.state
        .config
        .languages
        .register_identity("json", &["json"], &[], &[])
        .unwrap();
    ed.state
        .config
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .unwrap();
    attach_fixture_grammar(&mut ed, "json", "tree_sitter_json");
    let lang = ed.state.config.languages.intern("json");
    super::language::attach_host(&mut ed, "");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers(); // drain json parse result

    // Attach the rust grammar and switch the buffer to rust.
    let rust_bundle = attach_fixture_grammar(&mut ed, "rust", "tree_sitter_rust");
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

/// `grammar_source` / `helix_pin` must read real values out of the shipped
/// runtime data so the e2e installs the actually-pinned revision. Always runs
/// (no network), so malformed shipped data is caught in normal CI.
#[test]
fn shipped_data_reading_extracts_json_pins() {
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
/// Token colours reach the screen, not only the emitted highlight spans.
/// The cursor sits on the trailing `\n` so no content cell is reverse-video.
///
/// Requires `scripts/fetch-test-grammars.sh` (handled by `grammar_fixture`).
#[test]
fn rust_function_highlight_snapshot() {
    use hume_treesitter::highlight::layer_highlights_for_line;

    require_grammars(&["rust"]);
    // Cursor on the trailing `\n` so no token cell is reverse-video in the snapshot.
    let mut ed = editor_from("// hi\nfn main() {\n    let x: u32 = 1;\n}-[\n]>");
    ed.view.theme = crate::testing::build_snapshot_theme();

    let bid = ed.focused_buffer_id();
    ed.state
        .config
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .unwrap();
    attach_fixture_grammar(&mut ed, "rust", "tree_sitter_rust");
    // Bake after scopes are interned so theme.resolve() returns correct styles.
    ed.view.theme.bake(&ed.view.registry);

    let lang = ed.state.config.languages.intern("rust");
    super::language::attach_host(&mut ed, "");
    ed.set_buffer_language(bid, Some(lang));
    // Drain the parse result posted synchronously inside setup_buffer_syntax
    // (InlineParseBackend completes the parse inside post; drain installs the tree).
    ed.reparse_stale_buffers();

    // Sanity: line 1 ("fn main() {") must emit at least one `keyword` span.
    // Runs the highlight pipeline directly, apart from the snapshot renderer,
    // whose cursor/selection background can hide absent colours.
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
            hume_rope::line::ContentLine::new(1),
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
