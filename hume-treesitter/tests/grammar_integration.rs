use std::sync::Arc;

use hume_engine::theme::ScopeRegistry;
use hume_test_fixtures::{grammar_parser_path, grammar_query_path, skip_unless_grammars};
use hume_treesitter::grammar::LoadedGrammar;
use hume_treesitter::highlight::{TreeSitterHighlighter, layer_highlights_for_line};
use hume_treesitter::layers::{SyntaxLayer, SyntaxLayers};

/// Wrap a single parsed tree + highlighter into a one-layer `SyntaxLayers`
/// (the root layer, whole-buffer `ranges`) and run the real per-line
/// highlight collection path used by the renderer.
fn highlights_for_line(
    tree: tree_sitter::Tree,
    highlighter: TreeSitterHighlighter,
    rope: &ropey::Rope,
    line_idx: usize,
) -> Vec<(usize, usize, hume_engine::types::ScopeId)> {
    let layers = SyntaxLayers {
        layers: vec![SyntaxLayer {
            tree,
            highlighter: Arc::new(highlighter),
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
    if skip_unless_grammars(&["rust"]) {
        return;
    }
    let gpath = grammar_parser_path("rust");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_rust").expect("open rust grammar");
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(grammar.language())
        .expect("set rust language");
}

#[test]
fn loads_json_grammar() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
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
    if skip_unless_grammars(&["rust"]) {
        return;
    }
    let gpath = grammar_parser_path("rust");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_rust").unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(grammar.language()).unwrap();

    let source = b"fn foo(x: u32) -> u32 { x + 1 }";
    let tree = parser
        .parse(source as &[u8], None)
        .expect("parse should succeed");
    let root = tree.root_node();

    assert_eq!(root.kind(), "source_file");
    assert!(!root.has_error(), "parse produced errors");
    let first = root.named_child(0).expect("source_file must have a child");
    assert_eq!(first.kind(), "function_item");
}

#[test]
fn parses_json_object() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let gpath = grammar_parser_path("json");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_json").unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(grammar.language()).unwrap();

    let source = b"{\"a\":1}";
    let tree = parser
        .parse(source as &[u8], None)
        .expect("parse should succeed");
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
    if skip_unless_grammars(&["rust"]) {
        return;
    }
    let gpath = grammar_parser_path("rust");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_rust").unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(grammar.language()).unwrap();

    let source = b"fn foo() {}\n";
    let tree = parser
        .parse(source as &[u8], None)
        .expect("parse should succeed");

    let highlights_source =
        std::fs::read_to_string(grammar_query_path("rust")).expect("highlights.scm should exist");
    let mut scope_reg = ScopeRegistry::new();
    let rope = ropey::Rope::from_str(&String::from_utf8_lossy(source));

    let highlighter =
        TreeSitterHighlighter::new(grammar.language(), &highlights_source, &mut scope_reg)
            .expect("highlighter creation should succeed");

    let out = highlights_for_line(tree, highlighter, &rope, 0);

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
    if skip_unless_grammars(&["rust"]) {
        return;
    }
    let source = b"fn foo() {}\nlet x = 1;\n";
    let gpath = grammar_parser_path("rust");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_rust").unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(grammar.language()).unwrap();
    let tree = parser.parse(source as &[u8], None).expect("parse");

    let highlights_source =
        std::fs::read_to_string(grammar_query_path("rust")).expect("highlights.scm");
    let mut scope_reg = ScopeRegistry::new();
    let rope = ropey::Rope::from_str(&String::from_utf8_lossy(source));

    let highlighter =
        TreeSitterHighlighter::new(grammar.language(), &highlights_source, &mut scope_reg)
            .expect("highlighter");

    let out = highlights_for_line(tree, highlighter, &rope, 1);

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
    if skip_unless_grammars(&["rust"]) {
        return;
    }
    let gpath = grammar_parser_path("rust");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_rust").expect("open rust grammar");
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(grammar.language()).unwrap();

    let source = b"fn foo() {}\n";
    let tree = parser.parse(source as &[u8], None).expect("parse");

    let query_src = "(function_item) @function\n\"fn\" @keyword";
    let mut scope_reg = ScopeRegistry::new();
    let rope = ropey::Rope::from_str(&String::from_utf8_lossy(source));

    let highlighter = TreeSitterHighlighter::new(grammar.language(), query_src, &mut scope_reg)
        .expect("highlighter creation should succeed");

    let out = highlights_for_line(tree, highlighter, &rope, 0);

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
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let gpath = grammar_parser_path("json");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_json").expect("open json grammar");
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(grammar.language()).unwrap();

    let source = b"\"hello\"\n";
    let tree = parser.parse(source as &[u8], None).expect("parse");

    let query_src = "(string) @string\n(string) @string.duplicate";
    let mut scope_reg = ScopeRegistry::new();
    let rope = ropey::Rope::from_str(&String::from_utf8_lossy(source));

    let highlighter = TreeSitterHighlighter::new(grammar.language(), query_src, &mut scope_reg)
        .expect("highlighter creation should succeed");

    let out = highlights_for_line(tree, highlighter, &rope, 0);

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
    if skip_unless_grammars(&["rust"]) {
        return;
    }
    let gpath = grammar_parser_path("rust");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_rust").expect("open rust grammar");
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(grammar.language()).unwrap();

    let source = b"fn main() { foo(1); }\n";
    let tree = parser.parse(source as &[u8], None).expect("parse");

    let query_src =
        "(identifier) @variable\n(call_expression function: (identifier) @function)";
    let mut scope_reg = ScopeRegistry::new();
    let rope = ropey::Rope::from_str(&String::from_utf8_lossy(source));

    let highlighter = TreeSitterHighlighter::new(grammar.language(), query_src, &mut scope_reg)
        .expect("highlighter creation should succeed");

    let out = highlights_for_line(tree, highlighter, &rope, 0);

    // `foo` is at byte offset 13..16 in `fn main() { foo(1); }`.
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
    if skip_unless_grammars(&["rust"]) {
        return;
    }
    let gpath = grammar_parser_path("rust");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_rust").expect("open rust grammar");
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(grammar.language()).unwrap();

    let source = b"fn main() { foo(1); }\n";
    let tree = parser.parse(source as &[u8], None).expect("parse");

    let query_src =
        "(call_expression function: (identifier) @function)\n(identifier) @variable";
    let mut scope_reg = ScopeRegistry::new();
    let rope = ropey::Rope::from_str(&String::from_utf8_lossy(source));

    let highlighter = TreeSitterHighlighter::new(grammar.language(), query_src, &mut scope_reg)
        .expect("highlighter creation should succeed");

    let out = highlights_for_line(tree, highlighter, &rope, 0);

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
    if skip_unless_grammars(&["rust"]) {
        return;
    }
    let gpath = grammar_parser_path("rust");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_rust").expect("open rust grammar");
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(grammar.language()).unwrap();

    let source = b"fn main() { foo(1); }\n";
    let tree = parser.parse(source as &[u8], None).expect("parse");

    // Only-underscore query: must yield zero spans for `foo`.
    let helper_only_query = "(call_expression function: (identifier) @_helper)";
    let mut scope_reg = ScopeRegistry::new();
    let rope = ropey::Rope::from_str(&String::from_utf8_lossy(source));
    let highlighter =
        TreeSitterHighlighter::new(grammar.language(), helper_only_query, &mut scope_reg)
            .expect("highlighter creation should succeed");
    let out = highlights_for_line(tree.clone(), highlighter, &rope, 0);
    assert!(
        out.is_empty(),
        "a query with only an underscore capture must emit no spans; got: {out:?}"
    );

    // Underscore capture alongside a real one on the same node: the real
    // capture must win, never the (dropped) underscore capture.
    let mixed_query =
        "(call_expression function: (identifier) @_helper)\n(identifier) @variable";
    let mut scope_reg = ScopeRegistry::new();
    let highlighter = TreeSitterHighlighter::new(grammar.language(), mixed_query, &mut scope_reg)
        .expect("highlighter creation should succeed");
    let out = highlights_for_line(tree, highlighter, &rope, 0);
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
