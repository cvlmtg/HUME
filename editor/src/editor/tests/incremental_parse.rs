// Integration tests for incremental tree-sitter parsing (M9.5).
//
// Verifies end-to-end: edits record pending InputEdits, reparse_stale_buffers
// uses them to build old_tree for incremental re-parsing, and pending_edits are
// drained after each successful install.
//
// Requires grammar fixture: run scripts/fetch-test-grammars.sh.

use super::*;

use std::path::PathBuf;

use engine::grammar::LoadedGrammar;

use crate::core::selection::SelectionSet;
use crate::core::text::Text;
use crate::editor::buffer::Buffer;

// ── Setup ─────────────────────────────────────────────────────────────────────

fn grammar_path(name: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/grammars");
    let suffix = if cfg!(target_os = "macos") { "dylib" }
                 else if cfg!(windows) { "dll" }
                 else { "so" };
    base.join(name).join(format!("parser.{suffix}"))
}

fn grammar_highlights(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/grammars")
        .join(name)
        .join("queries/highlights.scm")
}

/// Create an editor with `source` as buffer text and a JSON grammar attached.
/// Runs `reparse_stale_buffers()` once to complete the initial (full) parse.
/// Works because `setup_buffer_syntax` posts the request *before* the first
/// `reparse_stale_buffers` call — InlineParseBackend resolves it immediately,
/// so the first drain installs it.
fn json_editor(source: &str) -> (Editor, engine::pipeline::BufferId) {
    let parser_path = grammar_path("json");
    if !parser_path.exists() {
        panic!(
            "grammar fixture missing: {}\nrun scripts/fetch-test-grammars.sh",
            parser_path.display()
        );
    }
    let hl_path = grammar_highlights("json");
    let buf = Buffer::new(Text::from(source), SelectionSet::default());
    let mut ed = Editor::for_testing(buf);
    let bid = ed.focused_buffer_id();
    ed.languages
        .attach_grammar("json", &parser_path, "tree_sitter_json", &hl_path, &mut ed.engine_view.registry)
        .expect("attach json grammar");
    ed.set_buffer_language(bid, Some("json".to_owned()));
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
    let (ed, bid) = json_editor("{\"x\": 1}\n");
    let buf = ed.buffers.get(bid);
    let syn = buf.syntax.as_ref().unwrap();
    assert_eq!(syn.parsed_gen, buf.text_gen, "initial parse must be up-to-date");
    assert!(syn.pending_edits.is_empty(), "no pending edits after first parse");
    assert!(ed.engine_view.buffers[bid].tree.is_some(), "tree installed");
}

#[test]
fn edit_records_pending_edits() {
    let (mut ed, bid) = json_editor("{}\n");
    let gen_before = ed.buffers.get(bid).text_gen;

    // Insert a char in insert mode.
    ed.feed_key(key('i'));
    ed.feed_key(key(' '));
    ed.feed_key(key_esc());

    let buf = ed.buffers.get(bid);
    assert!(buf.text_gen > gen_before, "edit must bump text_gen");
    let syn = buf.syntax.as_ref().unwrap();
    assert!(!syn.pending_edits.is_empty(), "edit must record pending edits");
}

#[test]
fn reparse_after_edit_drains_pending() {
    let (mut ed, bid) = json_editor("{}\n");

    ed.feed_key(key('i'));
    ed.feed_key(key(' '));
    ed.feed_key(key_esc());

    let gen_after_edit = ed.buffers.get(bid).text_gen;

    // InlineParseBackend: post request (first call) then drain+install (second call).
    reparse_edit(&mut ed);

    let buf = ed.buffers.get(bid);
    let syn = buf.syntax.as_ref().unwrap();
    assert!(syn.pending_edits.is_empty(), "pending edits must be drained after install");
    assert_eq!(syn.parsed_gen, gen_after_edit, "parsed_gen matches text_gen after edit");
    assert!(ed.engine_view.buffers[bid].tree.is_some());
}

#[test]
fn two_edits_batched_chain_resolves() {
    let (mut ed, bid) = json_editor("{}\n");

    // Two edits without reparsing between them.
    ed.feed_key(key('i'));
    ed.feed_key(key('1'));
    ed.feed_key(key_esc());
    let gen_1 = ed.buffers.get(bid).text_gen;

    ed.feed_key(key('a'));
    ed.feed_key(key('2'));
    ed.feed_key(key_esc());
    let gen_2 = ed.buffers.get(bid).text_gen;

    assert!(gen_2 > gen_1, "two edits must produce two text_gen bumps");

    // Both edits must be in pending_edits.
    let pending_count = ed.buffers.get(bid).syntax.as_ref().unwrap().pending_edits.len();
    assert!(pending_count >= 2, "both edits must be represented in pending_edits");

    // Post + install the incremental reparse covering both edits.
    reparse_edit(&mut ed);

    let buf = ed.buffers.get(bid);
    let syn = buf.syntax.as_ref().unwrap();
    assert!(syn.pending_edits.is_empty(), "all pending edits drained after install");
    assert_eq!(syn.parsed_gen, gen_2);
}

#[test]
fn incremental_tree_matches_full_reparse() {
    // The tree produced by incremental re-parsing must have the same S-expression
    // as a from-scratch parse of the exact same source bytes.
    let (mut ed, bid) = json_editor("{\"k\":1}\n");

    // Apply an edit: insert a char in insert mode.
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());

    reparse_edit(&mut ed);

    let incremental_sexp = ed.engine_view.buffers[bid]
        .tree.as_ref().unwrap()
        .root_node().to_sexp();

    // Full reparse of the same source bytes that the incremental parse used.
    let source = ed.buffers.get(bid).text().to_bytes();
    let parser_path = grammar_path("json");
    let grammar = LoadedGrammar::open(&parser_path, "tree_sitter_json")
        .expect("load json grammar");
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(grammar.language()).expect("set language");
    let full_tree = parser.parse(&source, None).expect("full parse succeeded");

    assert_eq!(
        incremental_sexp,
        full_tree.root_node().to_sexp(),
        "incremental tree must match from-scratch parse of the same text",
    );
}

#[test]
fn grammar_swap_clears_pending_and_full_reparses() {
    let (mut ed, bid) = json_editor("{\"x\":1}\n");

    // Record a pending edit.
    ed.feed_key(key('i'));
    ed.feed_key(key(' '));
    ed.feed_key(key_esc());
    assert!(
        !ed.buffers.get(bid).syntax.as_ref().unwrap().pending_edits.is_empty(),
        "pending edits must exist before grammar swap",
    );

    // Simulate a grammar re-attach: detach → re-attach → re-enable.
    // set_buffer_language(None) clears syntax (and pending_edits via BufferSyntax drop).
    // Then re-attaching re-creates BufferSyntax::new() which starts with empty pending_edits.
    let parser_path = grammar_path("json");
    let hl_path = grammar_highlights("json");
    ed.set_buffer_language(bid, None);
    ed.languages
        .attach_grammar("json", &parser_path, "tree_sitter_json", &hl_path, &mut ed.engine_view.registry)
        .expect("re-attach");
    ed.set_buffer_language(bid, Some("json".to_owned()));

    // After re-attach, pending_edits must be empty (fresh BufferSyntax).
    let syn = ed.buffers.get(bid).syntax.as_ref().unwrap();
    assert!(syn.pending_edits.is_empty(), "grammar swap must clear pending edits");

    // Full reparse succeeds (setup_buffer_syntax already posted it).
    ed.reparse_stale_buffers();
    assert!(ed.engine_view.buffers[bid].tree.is_some(), "tree must be present after re-attach");
}
