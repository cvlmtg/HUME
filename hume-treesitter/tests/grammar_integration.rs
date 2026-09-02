use std::sync::Arc;

use hume_engine::theme::ScopeRegistry;
use hume_test_fixtures::{grammar_parser_path, grammar_query_path, require_grammars};
use hume_treesitter::grammar::LoadedGrammar;
use hume_treesitter::highlight::{TreeSitterHighlighter, layer_highlights_for_line};
use hume_treesitter::layers::{SyntaxLayer, SyntaxLayers};
use hume_treesitter::registry::GrammarBundle;

/// Load `name`'s compiled grammar fixture and parse `source` with it —
/// shared by every test below that needs a working tree.
fn open_and_parse(name: &str, symbol: &str, source: &str) -> (tree_sitter::Tree, ropey::Rope) {
    let gpath = grammar_parser_path(name);
    let grammar = LoadedGrammar::open(&gpath, symbol).expect("open grammar");
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(grammar.language())
        .expect("set language");
    let tree = parser.parse(source, None).expect("parse should succeed");
    let rope = ropey::Rope::from_str(source);
    (tree, rope)
}

/// Open `name`'s grammar fresh (leaked/mmap'd once per process, so a second
/// open of a fixture already opened by `open_and_parse` is free) and compile
/// `query_src` against it as a minimal `GrammarBundle` — no injections, no
/// textobjects — interning captures into a fresh [`ScopeRegistry`]. A layer
/// now carries its whole bundle, not just a highlighter, so integration
/// tests exercising `layer_highlights_for_line` need one too.
fn bundle_for(name: &str, symbol: &str, query_src: &str) -> (Arc<GrammarBundle>, ScopeRegistry) {
    let gpath = grammar_parser_path(name);
    let grammar = LoadedGrammar::open(&gpath, symbol).expect("open grammar");
    let query =
        Arc::new(tree_sitter::Query::new(grammar.language(), query_src).expect("compile query"));
    let mut scope_reg = ScopeRegistry::new();
    let highlighter = Arc::new(TreeSitterHighlighter::from_shared_query(
        query,
        &mut scope_reg,
    ));
    let bundle = Arc::new(GrammarBundle {
        grammar,
        highlighter,
        injections: None,
        textobjects: None,
        config_gen: 0,
    });
    (bundle, scope_reg)
}

/// Wrap a single parsed tree + bundle into a one-layer `SyntaxLayers` (the
/// root layer, whole-buffer `ranges`) and run the real per-line highlight
/// collection path used by the renderer.
fn highlights_for_line(
    tree: tree_sitter::Tree,
    bundle: Arc<GrammarBundle>,
    rope: &ropey::Rope,
    line_idx: usize,
) -> Vec<(usize, usize, hume_engine::types::ScopeId)> {
    let layers = SyntaxLayers {
        layers: vec![SyntaxLayer {
            tree,
            bundle,
            ranges: vec![],
            depth: 0,
        }],
    };
    let mut raw = Vec::new();
    let mut stack = Vec::new();
    let mut events = Vec::new();
    let mut out = Vec::new();
    layer_highlights_for_line(
        &layers,
        line_idx,
        rope,
        &mut raw,
        &mut stack,
        &mut events,
        &mut out,
    );
    out
}

// ---------------------------------------------------------------------------
// Grammar load tests
// ---------------------------------------------------------------------------

#[test]
fn loads_rust_grammar() {
    require_grammars(&["rust"]);
    let gpath = grammar_parser_path("rust");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_rust").expect("open rust grammar");
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(grammar.language())
        .expect("set rust language");
}

#[test]
fn loads_json_grammar() {
    require_grammars(&["json"]);
    let gpath = grammar_parser_path("json");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_json").expect("open json grammar");
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(grammar.language())
        .expect("set json language");
}

// ---------------------------------------------------------------------------
// Parse tests
// ---------------------------------------------------------------------------

#[test]
fn parses_rust_function_signature() {
    require_grammars(&["rust"]);
    let (tree, _rope) = open_and_parse(
        "rust",
        "tree_sitter_rust",
        "fn foo(x: u32) -> u32 { x + 1 }",
    );
    let root = tree.root_node();

    assert_eq!(root.kind(), "source_file");
    assert!(!root.has_error(), "parse produced errors");
    let first = root.named_child(0).expect("source_file must have a child");
    assert_eq!(first.kind(), "function_item");
}

#[test]
fn parses_json_object() {
    require_grammars(&["json"]);
    let (tree, _rope) = open_and_parse("json", "tree_sitter_json", "{\"a\":1}");
    let root = tree.root_node();

    assert!(!root.has_error(), "parse produced errors");
    let value = root.named_child(0).expect("document must have a child");
    assert_eq!(value.kind(), "object");
}

// ---------------------------------------------------------------------------
// Highlight tests
// ---------------------------------------------------------------------------

#[test]
fn highlights_emit_keyword_event() {
    require_grammars(&["rust"]);
    let (tree, rope) = open_and_parse("rust", "tree_sitter_rust", "fn foo() {}\n");

    let highlights_source =
        std::fs::read_to_string(grammar_query_path("rust")).expect("highlights.scm should exist");
    let (bundle, scope_reg) = bundle_for("rust", "tree_sitter_rust", &highlights_source);

    let out = highlights_for_line(tree, bundle, &rope, 0);

    assert!(
        !out.is_empty(),
        "should emit highlight events for `fn foo() {{}}`"
    );
    // `fn` is 2 bytes at the start of the line; it must be captured as a keyword.
    let (start, end, scope_id) = out[0];
    assert_eq!(start, 0, "first highlight should start at byte 0 (`fn`)");
    assert_eq!(end, 2, "first highlight should end at byte 2 (`fn`)");
    assert!(
        scope_reg.name_of(scope_id).contains("keyword"),
        "scope for `fn` should contain 'keyword', got: {}",
        scope_reg.name_of(scope_id)
    );
}

// Byte offsets must be line-relative, not file-relative.
#[test]
fn highlights_for_line_correct_on_nonzero_line() {
    require_grammars(&["rust"]);
    let (tree, rope) = open_and_parse("rust", "tree_sitter_rust", "fn foo() {}\nlet x = 1;\n");

    let highlights_source =
        std::fs::read_to_string(grammar_query_path("rust")).expect("highlights.scm");
    let (bundle, scope_reg) = bundle_for("rust", "tree_sitter_rust", &highlights_source);

    let out = highlights_for_line(tree, bundle, &rope, 1);

    assert!(!out.is_empty(), "line 1 should emit highlight events");
    // `let` starts at line-relative offset 0, ends at 3.
    let has_let = out.iter().any(|&(start, end, id)| {
        start == 0 && end == 3 && scope_reg.name_of(id).contains("keyword")
    });
    assert!(
        has_let,
        "expected `let` keyword at line-relative [0, 3); got: {:?}",
        out.iter()
            .map(|&(s, e, id)| (s, e, scope_reg.name_of(id)))
            .collect::<Vec<_>>()
    );
}

// Shared-start priority in flatten_overlaps's sweep-line: `@keyword` ("fn")
// and `@function` (the whole item) both start at byte 0; the later-collected
// interval at the same depth wins its region, trimming the earlier one to
// start where the winner ends.
// Mutation gate: breaking the ascending (depth, seq) stack-insertion order
// collapses this to first-collected-wins, re-emitting `function` from byte 0
// instead of trimmed to 2.
#[test]
fn highlight_overlap_shorter_wins_at_shared_start() {
    require_grammars(&["rust"]);
    let (tree, rope) = open_and_parse("rust", "tree_sitter_rust", "fn foo() {}\n");

    let query_src = "(function_item) @function\n\"fn\" @keyword";
    let (bundle, scope_reg) = bundle_for("rust", "tree_sitter_rust", query_src);

    let out = highlights_for_line(tree, bundle, &rope, 0);

    assert!(out.len() >= 2, "expected at least 2 spans; got: {out:?}");
    let keyword_span = out
        .iter()
        .find(|&&(_, _, id)| scope_reg.name_of(id).contains("keyword"));
    let function_span = out
        .iter()
        .find(|&&(_, _, id)| scope_reg.name_of(id).contains("function"));
    assert!(keyword_span.is_some(), "expected a 'keyword' scope");
    assert!(function_span.is_some(), "expected a 'function' scope");
    let (kw_start, kw_end, _) = *keyword_span.unwrap();
    let (fn_start, _, _) = *function_span.unwrap();
    assert_eq!(kw_start, 0);
    assert_eq!(kw_end, 2);
    assert_eq!(
        fn_start, kw_end,
        "function span must be trimmed to start at {kw_end}"
    );
}

// Two captures over the identical byte range collapse to one span via
// flatten_overlaps's sweep-line stack: the later-collected interval fully
// supersedes the earlier one occupying the same range, rather than emitting
// both.
// Mutation gate: breaking that stack-based supersession emits two
// overlapping spans instead of one, failing the count.
#[test]
fn highlight_overlap_fully_contained_is_dropped() {
    require_grammars(&["json"]);
    let (tree, rope) = open_and_parse("json", "tree_sitter_json", "\"hello\"\n");

    let query_src = "(string) @string\n(string) @string.duplicate";
    let (bundle, scope_reg) = bundle_for("json", "tree_sitter_json", query_src);

    let out = highlights_for_line(tree, bundle, &rope, 0);

    let string_spans: Vec<_> = out.iter().filter(|&&(s, e, _)| s == 0 && e == 7).collect();
    assert_eq!(
        string_spans.len(),
        1,
        "expected exactly one span for string node [0,7); got {}: {:?}",
        string_spans.len(),
        out.iter()
            .map(|&(s, e, id)| (s, e, scope_reg.name_of(id)))
            .collect::<Vec<_>>()
    );
}

// Regression: Helix-style queries rely on a later, more specific pattern
// overriding an earlier catch-all for the SAME node — e.g. `(identifier)
// @variable` followed by `(call_expression function: (identifier)
// @function)`. `foo`'s identifier node is nested inside the call_expression
// match's root, so a `matches()`-based collector emits the call-rooted
// `@function` capture ahead of the identifier-rooted `@variable` capture,
// and flatten_overlaps's same-range last-pushed-wins then picks `@variable`
// — silently losing every keyword/function capture in real-world queries.
// `captures()` orders by node position with pattern order as the tiebreak,
// so the later pattern (`@function`) must win here.
#[test]
fn highlight_later_pattern_wins_on_same_node() {
    require_grammars(&["rust"]);
    let (tree, rope) = open_and_parse("rust", "tree_sitter_rust", "fn main() { foo(1); }\n");

    let query_src = "(identifier) @variable\n(call_expression function: (identifier) @function)";
    let (bundle, scope_reg) = bundle_for("rust", "tree_sitter_rust", query_src);

    let out = highlights_for_line(tree, bundle, &rope, 0);

    // `foo` is at byte offset 12..15 in `fn main() { foo(1); }`.
    let foo_span = out.iter().find(|&&(s, e, _)| s == 12 && e == 15);
    let (_, _, scope_id) = *foo_span.unwrap_or_else(|| {
        panic!(
            "expected a span at [12, 15) for `foo`; got: {:?}",
            out.iter()
                .map(|&(s, e, id)| (s, e, scope_reg.name_of(id)))
                .collect::<Vec<_>>()
        )
    });
    assert_eq!(
        scope_reg.name_of(scope_id),
        "function",
        "later pattern (@function) must win over the earlier catch-all (@variable)"
    );
}

// Companion to `highlight_later_pattern_wins_on_same_node`, with pattern
// order reversed: the catch-all `@variable` now comes last, so it must win.
// Guards against a fix that hardcodes "more specific pattern wins" instead
// of genuinely respecting query order.
#[test]
fn highlight_pattern_order_controls_winner_not_specificity() {
    require_grammars(&["rust"]);
    let (tree, rope) = open_and_parse("rust", "tree_sitter_rust", "fn main() { foo(1); }\n");

    let query_src = "(call_expression function: (identifier) @function)\n(identifier) @variable";
    let (bundle, scope_reg) = bundle_for("rust", "tree_sitter_rust", query_src);

    let out = highlights_for_line(tree, bundle, &rope, 0);

    let foo_span = out.iter().find(|&&(s, e, _)| s == 12 && e == 15);
    let (_, _, scope_id) = *foo_span.unwrap_or_else(|| {
        panic!(
            "expected a span at [12, 15) for `foo`; got: {:?}",
            out.iter()
                .map(|&(s, e, id)| (s, e, scope_reg.name_of(id)))
                .collect::<Vec<_>>()
        )
    });
    assert_eq!(
        scope_reg.name_of(scope_id),
        "variable",
        "with order swapped, the now-later catch-all (@variable) must win"
    );
}

// Leading-underscore captures are Helix's convention for pattern-internal
// predicate helpers (e.g. `@_f`, `@_lib`) and must never be styled — they
// should be dropped entirely rather than interned as a real scope that could
// clobber a legitimate capture on the same node.
#[test]
fn highlight_underscore_captures_are_ignored() {
    require_grammars(&["rust"]);
    let (tree, rope) = open_and_parse("rust", "tree_sitter_rust", "fn main() { foo(1); }\n");

    // Only-underscore query: must yield zero spans for `foo`.
    let helper_only_query = "(call_expression function: (identifier) @_helper)";
    let (bundle, _scope_reg) = bundle_for("rust", "tree_sitter_rust", helper_only_query);
    let out = highlights_for_line(tree.clone(), bundle, &rope, 0);
    assert!(
        out.is_empty(),
        "a query with only an underscore capture must emit no spans; got: {out:?}"
    );

    // Underscore capture alongside a real one on the same node: the real
    // capture must win, never the (dropped) underscore capture.
    let mixed_query = "(call_expression function: (identifier) @_helper)\n(identifier) @variable";
    let (bundle, scope_reg) = bundle_for("rust", "tree_sitter_rust", mixed_query);
    let out = highlights_for_line(tree, bundle, &rope, 0);
    let foo_span = out.iter().find(|&&(s, e, _)| s == 12 && e == 15);
    let (_, _, scope_id) = *foo_span.unwrap_or_else(|| {
        panic!(
            "expected a span at [12, 15) for `foo`; got: {:?}",
            out.iter()
                .map(|&(s, e, id)| (s, e, scope_reg.name_of(id)))
                .collect::<Vec<_>>()
        )
    });
    assert_eq!(scope_reg.name_of(scope_id), "variable");
}
