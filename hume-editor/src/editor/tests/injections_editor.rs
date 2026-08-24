// Editor-level tests for tree-sitter injection support: real markdown +
// markdown.inline + rust grammar fixtures, exercised through the full
// setup_buffer_syntax / reparse_stale_buffers / bake_pending_edits path.
//
// Requires scripts/fetch-test-grammars.sh (markdown, markdown.inline, rust).

use super::*;

use hume_test_fixtures::{
    grammar_parser_path, grammar_query_path, helix_injections_path, skip_unless_file,
    skip_unless_grammars,
};

/// Attach the fixture grammar `name` (source name == attach identity — true
/// for every real PLUM install; there is no renaming split in production).
/// `injections` selects between the real Helix-maintained injections.scm
/// (what PLUM actually installs) and none.
fn attach(ed: &mut Editor, name: &str, symbol: &str, injections: bool) {
    let parser_path = grammar_parser_path(name);
    let hl_path = grammar_query_path(name);
    let inj_path = injections.then(|| helix_injections_path(name)).flatten();
    ed.state
        .config
        .languages
        .attach_grammar(
            name,
            &parser_path,
            symbol,
            &hl_path,
            inj_path.as_deref(),
            &mut ed.view.registry,
        )
        .unwrap_or_else(|e| panic!("attach {name}: {e}"));
}

/// Returns `true` when the caller should skip early. See
/// `hume_test_fixtures::skip_unless_grammars`/`skip_unless_file` for the
/// skip-or-require contract this composes.
fn skip_unless_fixtures() -> bool {
    if skip_unless_grammars(&["markdown", "markdown.inline", "rust"]) {
        return true;
    }
    let helix_path = grammar_parser_path("markdown").with_file_name("helix-injections.scm");
    skip_unless_file(&helix_path, "markdown helix-injections.scm")
}

/// Build an editor with markdown (+ the real Helix-maintained
/// `injections.scm` — what `:plum-install-grammar` actually fetches, not
/// the grammar's own bundled query), the `markdown.inline` grammar (the
/// name Helix's injections.scm resolves `(inline)` to — same name as the
/// fixture directory, no renaming needed), and rust, then attach markdown as
/// the buffer's language and drain the initial parse.
///
/// Builds the buffer directly (cursor at 0 via `SelectionSet::default()`)
/// rather than through the `editor_from` marker DSL — this file's tests
/// mostly care about byte-offset edits at position 0, not cursor placement.
fn markdown_editor(source: &str) -> (Editor, hume_engine::pipeline::BufferId) {
    let buf = crate::editor::buffer::Buffer::new(
        hume_editing::text::BufferText::from(source),
        hume_editing::selection::SelectionSet::default(),
    );
    let mut ed = Editor::for_testing(buf);
    let bid = ed.focused_buffer_id();

    ed.state
        .config
        .languages
        .register_identity("markdown", &["md"], &[], &[], None)
        .unwrap();
    attach(&mut ed, "markdown", "tree_sitter_markdown", true);
    ed.state
        .config
        .languages
        .register_identity("markdown.inline", &[], &[], &[], None)
        .unwrap();
    attach(
        &mut ed,
        "markdown.inline",
        "tree_sitter_markdown_inline",
        false,
    );
    ed.state
        .config
        .languages
        .register_identity("rust", &["rs"], &[], &[], None)
        .unwrap();
    attach(&mut ed, "rust", "tree_sitter_rust", false);

    let lang = ed.state.config.languages.intern("markdown");
    ed.set_buffer_language(bid, Some(lang));
    ed.reparse_stale_buffers(); // drains the initial full parse
    (ed, bid)
}

#[test]
fn markdown_buffer_installs_root_plus_injected_layers() {
    if skip_unless_fixtures() {
        return; // scripts/fetch-test-grammars.sh not run
    }
    let source = "# Title\n\nSome **bold** text.\n\n```rust\nfn f() {}\n```\n";
    let (ed, bid) = markdown_editor(source);

    let layers = &ed
        .state
        .buffers
        .get(bid)
        .syntax
        .as_ref()
        .unwrap()
        .layers()
        .expect("engine syntax must be installed")
        .layers;
    assert!(
        layers.len() > 1,
        "expected root + at least one injected layer (bold text or rust fence), got {} layer(s)",
        layers.len()
    );
    assert_eq!(layers[0].depth, 0, "layers[0] must be the root layer");
    assert!(
        layers[1..].iter().all(|l| l.depth > 0),
        "every non-root layer must have depth > 0"
    );

    // At least one injected layer must be the rust fence (depth-1, non-empty
    // ranges strictly inside the buffer, root_node parses as rust).
    let has_rust_layer = layers[1..]
        .iter()
        .any(|l| l.tree.root_node().kind() == "source_file");
    assert!(
        has_rust_layer,
        "expected a rust-parsed layer for the fenced code block"
    );

    // At least one injected layer must be the markdown_inline layer for the
    // "**bold**"/"*italic*" text (markdown_inline's own root node kind is
    // also literally "inline"). This specifically exercises the outer
    // `(inline)` node's children-exclusion path in `content_ranges` — its
    // children (`strong_emphasis`, `emphasis`) are all *named*, so a bug
    // that excludes named children too (not just unnamed/punctuation ones)
    // yields empty ranges here and silently drops this whole layer, even
    // though the simpler rust-fence case above (a childless leaf node)
    // still passes.
    let has_inline_layer = layers[1..]
        .iter()
        .any(|l| l.tree.root_node().kind() == "inline");
    assert!(
        has_inline_layer,
        "expected a markdown_inline-parsed layer for the bold/italic text; got layer kinds: {:?}",
        layers[1..]
            .iter()
            .map(|l| l.tree.root_node().kind())
            .collect::<Vec<_>>()
    );
}

#[test]
fn bake_pending_edits_refreshes_injected_layer_ranges() {
    if skip_unless_fixtures() {
        return;
    }
    let source = "```rust\nfn f() {}\n```\n";
    let (mut ed, bid) = markdown_editor(source);

    let ranges_before: Vec<_> = ed
        .state
        .buffers
        .get(bid)
        .syntax
        .as_ref()
        .unwrap()
        .layers()
        .unwrap()
        .layers
        .iter()
        .filter(|l| l.depth > 0)
        .map(|l| l.ranges.clone())
        .collect();
    assert!(
        !ranges_before.is_empty(),
        "expected at least one injected layer for the fenced code block"
    );

    // Insert 5 bytes at the very start of the buffer — every injected
    // layer's ranges (all strictly after byte 0) must shift by +5.
    ed.feed_key(key('i'));
    for ch in "XXXXX".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());

    // One reparse_stale_buffers call bakes pending edits (including the
    // range refresh) but does not yet install the queued precise reparse —
    // exactly the frame this test is about (see incremental_parse.rs's
    // `bake_aligns_committed_tree_before_precise_install`).
    ed.reparse_stale_buffers();

    let ranges_after: Vec<_> = ed
        .state
        .buffers
        .get(bid)
        .syntax
        .as_ref()
        .unwrap()
        .layers()
        .unwrap()
        .layers
        .iter()
        .filter(|l| l.depth > 0)
        .map(|l| l.ranges.clone())
        .collect();

    assert_eq!(
        ranges_before.len(),
        ranges_after.len(),
        "bake must not add or drop layers"
    );
    for (before, after) in ranges_before.iter().zip(ranges_after.iter()) {
        assert_eq!(
            before.len(),
            after.len(),
            "bake must not add or drop ranges within a layer"
        );
        for (b, a) in before.iter().zip(after.iter()) {
            assert_eq!(
                a.start_byte,
                b.start_byte + 5,
                "injected layer range must shift by the inserted byte count"
            );
            assert_eq!(a.end_byte, b.end_byte + 5);
        }
    }
}

#[test]
fn stale_gen_discards_whole_layer_set() {
    if skip_unless_fixtures() {
        return;
    }
    let (mut ed, bid) = markdown_editor("```rust\nfn f() {}\n```\n");
    let gen0 = ed.state.buffers.get(bid).text_gen;

    // Construct a genuinely stale result: edit, let one reparse call post the
    // request (InlineParseBackend executes it and queues the result without
    // draining it), then edit *again* before the next drain — the queued
    // result now describes a superseded generation and must be discarded
    // whole (root + every injected layer), not partially applied.
    ed.feed_key(key('i'));
    ed.feed_key(key('z'));
    ed.feed_key(key_esc());
    let gen1 = ed.state.buffers.get(bid).text_gen;
    ed.reparse_stale_buffers(); // bakes, posts request for gen1; result queued

    ed.feed_key(key('i'));
    ed.feed_key(key('y'));
    ed.feed_key(key_esc());
    let gen2 = ed.state.buffers.get(bid).text_gen;
    assert!(
        gen2 > gen1,
        "premise: second edit must supersede the request"
    );

    ed.reparse_stale_buffers(); // drains the stale gen1 result; must discard

    let syn = ed.state.buffers.get(bid).syntax.as_ref().unwrap();
    assert_eq!(
        syn.parsed_gen(),
        gen0,
        "stale result must be discarded whole — parsed_gen must stay at the \
         initial install, not advance to the stale request's generation"
    );

    // Recovery: the gen2 request (posted by the draining call above) installs
    // the full fresh layer set on the next drain.
    ed.reparse_stale_buffers();
    let syn = ed.state.buffers.get(bid).syntax.as_ref().unwrap();
    assert_eq!(syn.parsed_gen(), gen2, "fresh result must install");
    let layers = &syn.layers().unwrap().layers;
    assert!(
        layers.len() >= 2,
        "recovered layer set must include the rust fence layer, got {} layer(s)",
        layers.len()
    );
}

// ---------------------------------------------------------------------------
// PLUM pipeline syntax smoke test
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// plum-install-grammar — optional name argument
// ---------------------------------------------------------------------------
