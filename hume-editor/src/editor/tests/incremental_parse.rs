// Integration tests for incremental tree-sitter parsing.
//
// Verifies end-to-end: edits record pending InputEdits, reparse_stale_buffers
// uses them to build old_tree for incremental re-parsing, and pending_edits are
// drained after each successful install.
//
// Requires grammar fixture: run scripts/fetch-test-grammars.sh.

use super::*;

use hume_treesitter::grammar::LoadedGrammar;
use test_fixtures::{grammar_parser_path, require_grammars};

use crate::editor::buffer::Buffer;
use hume_editing::text::BufferText;

/// Create an editor with `source` as buffer text and a JSON grammar attached.
/// Runs `reparse_stale_buffers()` once to complete the initial (full) parse.
/// Works because `setup_buffer_syntax` posts the request *before* the first
/// `reparse_stale_buffers` call: InlineParseBackend resolves it immediately,
/// so the first drain installs it.
fn json_editor(source: &str) -> (Editor, hume_engine::pipeline::BufferId) {
    let buf = Buffer::at_start(BufferText::from(source));
    let mut ed = Editor::for_testing(buf);
    let bid = ed.focused_buffer_id();
    attach_fixture_grammar(&mut ed, "json", "tree_sitter_json");
    let lang = ed.state.config.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers(); // drains the initial parse (posted by setup_buffer_syntax)
    (ed, bid)
}

/// With InlineParseBackend, `post` resolves parses synchronously and queues the
/// result.  After an edit, `reparse_stale_buffers` POSTS a new request (which
/// InlineParseBackend resolves immediately into the queue), but the result sits
/// in the queue until the *next* call's drain phase.  Call this after an edit to
/// complete the full post→drain→install cycle.
fn reparse_edit(ed: &mut Editor) {
    ed.reparse_stale_buffers(); // posts request; InlineParseBackend enqueues result
    ed.reparse_stale_buffers(); // drains the queued result and installs it
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn first_parse_full_reparse_no_pending() {
    require_grammars(&["json"]);
    let (ed, bid) = json_editor("{\"x\": 1}\n");
    let buf = ed.state.buffers.get(bid);
    let syn = buf.syntax.as_ref().unwrap();
    assert_eq!(
        syn.parsed_gen(),
        Some(buf.text().generation()),
        "initial parse must be up-to-date"
    );
    assert!(
        syn.pending_edits().is_empty(),
        "no pending edits after first parse"
    );
    assert!(syn.layers().is_some(), "tree installed");
}

#[test]
fn edit_records_pending_edits() {
    require_grammars(&["json"]);
    let (mut ed, bid) = json_editor("{}\n");
    let gen_before = ed.state.buffers.get(bid).text().generation();

    // Insert a char in insert mode.
    ed.feed_key(key('i'));
    ed.feed_key(key(' '));
    ed.feed_key(key_esc());

    let buf = ed.state.buffers.get(bid);
    assert!(
        buf.text().generation() > gen_before,
        "edit must change the text version"
    );
    let syn = buf.syntax.as_ref().unwrap();
    assert!(
        !syn.pending_edits().is_empty(),
        "edit must record pending edits"
    );
}

#[test]
fn reparse_after_edit_drains_pending() {
    require_grammars(&["json"]);
    let (mut ed, bid) = json_editor("{}\n");

    ed.feed_key(key('i'));
    ed.feed_key(key(' '));
    ed.feed_key(key_esc());

    let gen_after_edit = ed.state.buffers.get(bid).text().generation();

    // InlineParseBackend: post request (first call) then drain+install (second call).
    reparse_edit(&mut ed);

    let buf = ed.state.buffers.get(bid);
    let syn = buf.syntax.as_ref().unwrap();
    assert!(
        syn.pending_edits().is_empty(),
        "pending edits must be drained after install"
    );
    assert_eq!(
        syn.parsed_gen(),
        Some(gen_after_edit),
        "parsed_gen matches generation after edit"
    );
    assert!(syn.layers().is_some());
}

#[test]
fn two_edits_batched_chain_resolves() {
    require_grammars(&["json"]);
    let (mut ed, bid) = json_editor("{}\n");

    // Two edits without reparsing between them.
    ed.feed_key(key('i'));
    ed.feed_key(key('1'));
    ed.feed_key(key_esc());
    let gen_1 = ed.state.buffers.get(bid).text().generation();

    ed.feed_key(key('a'));
    ed.feed_key(key('2'));
    ed.feed_key(key_esc());
    let gen_2 = ed.state.buffers.get(bid).text().generation();

    assert!(gen_2 > gen_1, "two edits must produce two generation bumps");

    // Both edits must be in pending_edits.
    let pending_count = ed
        .state
        .buffers
        .get(bid)
        .syntax
        .as_ref()
        .unwrap()
        .pending_edits()
        .len();
    assert!(
        pending_count >= 2,
        "both edits must be represented in pending_edits"
    );

    // Post + install the incremental reparse covering both edits.
    reparse_edit(&mut ed);

    let buf = ed.state.buffers.get(bid);
    let syn = buf.syntax.as_ref().unwrap();
    assert!(
        syn.pending_edits().is_empty(),
        "all pending edits drained after install"
    );
    assert_eq!(syn.parsed_gen(), Some(gen_2));
}

#[test]
fn incremental_tree_matches_full_reparse() {
    require_grammars(&["json"]);
    // The tree produced by incremental re-parsing must have the same S-expression
    // as a from-scratch parse of the exact same source bytes.
    let (mut ed, bid) = json_editor("{\"k\":1}\n");

    // Apply an edit: insert a char in insert mode.
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());

    reparse_edit(&mut ed);

    let incremental_sexp = ed
        .state
        .buffers
        .get(bid)
        .syntax
        .as_ref()
        .unwrap()
        .layers()
        .and_then(hume_treesitter::layers::SyntaxLayers::root_tree)
        .unwrap()
        .root_node()
        .to_sexp();

    // Full reparse of the same source bytes that the incremental parse used.
    let source = ed.state.buffers.get(bid).text().to_string().into_bytes();
    let grammar = LoadedGrammar::open(&grammar_parser_path("json"), "tree_sitter_json")
        .expect("load json grammar");
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(grammar.language())
        .expect("set language");
    let full_tree = parser.parse(&source, None).expect("full parse succeeded");

    assert_eq!(
        incremental_sexp,
        full_tree.root_node().to_sexp(),
        "incremental tree must match from-scratch parse of the same text",
    );
}

/// After an edit, a single `reparse_stale_buffers` call must bake the pending
/// edits into the committed tree (coordinate-aligning it with the live text)
/// before the background precise parse is installed.  This is the key invariant
/// that prevents highlight flicker: without it, the committed tree's
/// coordinates lag the live text until the precise parse lands.
///
/// With InlineParseBackend, `post` resolves immediately into the queue but does
/// NOT drain in the same call.  So after exactly one `reparse_stale_buffers`:
/// - the bake has run  (tree_gen == generation, pending cleared, tree coords shifted)
/// - the precise parse is queued but NOT yet installed (parsed_gen < generation)
#[test]
fn bake_aligns_committed_tree_before_precise_install() {
    require_grammars(&["json"]);
    let (mut ed, bid) = json_editor("{}\n");
    let old_byte_len = ed.state.buffers.get(bid).text().len_bytes();

    // Insert one space at the buffer start.
    ed.feed_key(key('i'));
    ed.feed_key(key(' '));
    ed.feed_key(key_esc());

    let generation_after = ed.state.buffers.get(bid).text().generation();
    let new_byte_len = ed.state.buffers.get(bid).text().len_bytes();
    assert_eq!(new_byte_len, old_byte_len + 1, "insert added one byte");

    // One call: bakes pending edits + posts the incremental reparse request.
    // InlineParseBackend resolves the request synchronously into its internal
    // queue, but the queue is NOT drained until the NEXT call's drain phase.
    ed.reparse_stale_buffers();

    let syn = ed.state.buffers.get(bid).syntax.as_ref().unwrap();
    assert_eq!(
        syn.tree_gen(),
        generation_after,
        "tree_gen must equal generation after bake"
    );
    assert!(
        syn.parsed_gen() < Some(generation_after),
        "parsed_gen must not yet equal generation: precise parse queued, not installed",
    );
    assert!(
        syn.pending_edits().is_empty(),
        "pending_edits must be cleared by the bake"
    );

    // Committed tree must be coordinate-aligned: root end_byte == new text length.
    // A root still ending at old_byte_len has stale coords and shifts highlight columns.
    let root_end = syn
        .layers()
        .and_then(hume_treesitter::layers::SyntaxLayers::root_tree)
        .unwrap()
        .root_node()
        .end_byte();
    assert_eq!(
        root_end, new_byte_len,
        "baked tree root end_byte must equal new text byte count; \
         a stale tree would end at {}",
        old_byte_len,
    );
}

/// Two edits without an intervening drain: the bake must handle a chain of
/// multiple pending InputEdits and advance tree_gen in one shot.
#[test]
fn bake_handles_multi_edit_chain_in_one_shot() {
    require_grammars(&["json"]);
    let (mut ed, bid) = json_editor("{}\n");

    // Two separate insert-mode characters → two generation bumps, two pending edits.
    ed.feed_key(key('i'));
    ed.feed_key(key('A'));
    ed.feed_key(key_esc());
    ed.feed_key(key('a'));
    ed.feed_key(key('B'));
    ed.feed_key(key_esc());

    let generation_after = ed.state.buffers.get(bid).text().generation();
    let new_byte_len = ed.state.buffers.get(bid).text().len_bytes();

    let pending_count = ed
        .state
        .buffers
        .get(bid)
        .syntax
        .as_ref()
        .unwrap()
        .pending_edits()
        .len();
    assert!(
        pending_count >= 2,
        "two edits must produce ≥2 pending entries"
    );

    // One call bakes all pending edits at once.
    ed.reparse_stale_buffers();

    let syn = ed.state.buffers.get(bid).syntax.as_ref().unwrap();
    assert_eq!(
        syn.tree_gen(),
        generation_after,
        "tree_gen must jump to generation after bake"
    );
    assert!(
        syn.pending_edits().is_empty(),
        "all pending edits cleared by bake"
    );

    let root_end = syn
        .layers()
        .and_then(hume_treesitter::layers::SyntaxLayers::root_tree)
        .unwrap()
        .root_node()
        .end_byte();
    assert_eq!(
        root_end, new_byte_len,
        "multi-edit baked tree must span new byte length"
    );
}

#[test]
fn grammar_swap_clears_pending_and_full_reparses() {
    require_grammars(&["json"]);
    let (mut ed, bid) = json_editor("{\"x\":1}\n");

    // Record a pending edit.
    ed.feed_key(key('i'));
    ed.feed_key(key(' '));
    ed.feed_key(key_esc());
    assert!(
        !ed.state
            .buffers
            .get(bid)
            .syntax
            .as_ref()
            .unwrap()
            .pending_edits()
            .is_empty(),
        "pending edits must exist before grammar swap",
    );

    // Simulate a grammar re-attach: detach → re-attach → re-enable.
    // set_buffer_language(None) drops the whole Syntax attachment (and its
    // pending_edits with it). Then re-attaching creates a fresh Syntax that
    // starts with empty pending_edits.
    ed.set_buffer_language(bid, None);
    attach_fixture_grammar(&mut ed, "json", "tree_sitter_json");
    let lang = ed.state.config.languages.intern("json");
    ed.set_buffer_language(bid, Some(lang));

    // After re-attach, pending_edits must be empty (fresh Syntax attachment).
    let syn = ed.state.buffers.get(bid).syntax.as_ref().unwrap();
    assert!(
        syn.pending_edits().is_empty(),
        "grammar swap must clear pending edits"
    );

    // Full reparse succeeds (setup_buffer_syntax already posted it).
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
        "tree must be present after re-attach"
    );
}

/// An edit built from several steps installs one text that consumes several
/// text versions. The recorded edit still leads from the tree's text to the
/// current one, so no full reparse is forced.
#[test]
fn an_edit_spanning_several_text_versions_keeps_the_pending_edit_chain_linked() {
    use crate::editor::position_stores::PositionStores;
    use hume_engine::pipeline::PaneId;
    use hume_ops::edit::insert_str;

    require_grammars(&["json"]);
    let (mut ed, bid) = json_editor("[1,\n2,\n3]\n");
    let gen_before = ed.state.buffers.get(bid).text().generation();
    let sels = ed.current_selections().clone();

    ed.state.buffers.get_mut(bid).apply_edit(
        bid,
        &mut PositionStores::new(
            &mut ed.state.panes,
            &mut ed.state.input,
            &mut ed.state.buffer_positions,
            &mut ed.state.config.decorations,
        ),
        PaneId::default(),
        sels,
        |state| insert_str(state, " ").then(|state| insert_str(state, " ")),
    );

    let gen_after = ed.state.buffers.get(bid).text().generation();
    assert!(
        gen_after > gen_before + 1,
        "setup: the edit skips text versions"
    );
    ed.reparse_stale_buffers();

    assert!(
        !ed.state
            .message_log
            .entries()
            .any(|e| e.text.contains("pending-edit chain broken")),
        "the chain must not break"
    );
}
