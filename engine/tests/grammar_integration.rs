use std::path::PathBuf;

use engine::builtins::tree_sitter_hl::TreeSitterHighlighter;
use engine::grammar::LoadedGrammar;
use engine::providers::{HighlightSource, SourceContext};
use engine::theme::ScopeRegistry;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn grammar_path(name: &str) -> PathBuf {
    let fixture_base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/grammars");
    let suffix = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(windows) {
        "dll"
    } else {
        "so"
    };
    let p = fixture_base.join(name).join(format!("parser.{suffix}"));
    if !p.exists() {
        panic!(
            "grammar fixture missing: {}\ninstall the tree-sitter CLI (npm i -g tree-sitter-cli) and run scripts/fetch-test-grammars.sh from the repo root",
            p.display()
        );
    }
    p
}

fn highlights_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/grammars")
        .join(name)
        .join("queries/highlights.scm")
}

// ---------------------------------------------------------------------------
// Grammar load tests
// ---------------------------------------------------------------------------

#[test]
fn loads_rust_grammar() {
    let gpath = grammar_path("rust");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_rust").expect("open rust grammar");
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(grammar.language()).expect("set rust language");
}

#[test]
fn loads_json_grammar() {
    let gpath = grammar_path("json");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_json").expect("open json grammar");
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(grammar.language()).expect("set json language");
}

// ---------------------------------------------------------------------------
// Parse tests
// ---------------------------------------------------------------------------

#[test]
fn parses_rust_function_signature() {
    let gpath = grammar_path("rust");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_rust").unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(grammar.language()).unwrap();

    let source = b"fn foo(x: u32) -> u32 { x + 1 }";
    let tree = parser.parse(source as &[u8], None).expect("parse should succeed");
    let root = tree.root_node();

    assert_eq!(root.kind(), "source_file");
    assert!(!root.has_error(), "parse produced errors");
    let first = root.named_child(0).expect("source_file must have a child");
    assert_eq!(first.kind(), "function_item");
}

#[test]
fn parses_json_object() {
    let gpath = grammar_path("json");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_json").unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(grammar.language()).unwrap();

    let source = b"{\"a\":1}";
    let tree = parser.parse(source as &[u8], None).expect("parse should succeed");
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
    let gpath = grammar_path("rust");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_rust").unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(grammar.language()).unwrap();

    let source = b"fn foo() {}\n";
    let tree = parser.parse(source as &[u8], None).expect("parse should succeed");

    let highlights_source =
        std::fs::read_to_string(highlights_path("rust")).expect("highlights.scm should exist");
    let mut scope_reg = ScopeRegistry::new();
    let rope = ropey::Rope::from_str(&String::from_utf8_lossy(source));

    let highlighter = TreeSitterHighlighter::new(
        grammar.language(),
        &highlights_source,
        &mut scope_reg,
    )
    .expect("highlighter creation should succeed");

    let ctx = SourceContext { rope: &rope, tree: Some(&tree), source, line_start_byte: 0 };
    let mut out = Vec::new();
    highlighter.highlights_for_line(0, &ctx, &mut out);

    assert!(!out.is_empty(), "should emit highlight events for `fn foo() {{}}`");
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
    let source = b"fn foo() {}\nlet x = 1;\n";
    let gpath = grammar_path("rust");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_rust").unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(grammar.language()).unwrap();
    let tree = parser.parse(source as &[u8], None).expect("parse");

    let highlights_source =
        std::fs::read_to_string(highlights_path("rust")).expect("highlights.scm");
    let mut scope_reg = ScopeRegistry::new();
    let rope = ropey::Rope::from_str(&String::from_utf8_lossy(source));

    let highlighter = TreeSitterHighlighter::new(
        grammar.language(),
        &highlights_source,
        &mut scope_reg,
    )
    .expect("highlighter");

    let line_start_byte = rope.line_to_byte(1);
    let ctx = SourceContext { rope: &rope, tree: Some(&tree), source, line_start_byte };
    let mut out = Vec::new();
    highlighter.highlights_for_line(1, &ctx, &mut out);

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

// Overlap resolver branch 3 (partial trim): shorter interval at shared start wins.
// Mutation gate: removing the trim branch re-emits function from 0, failing the start check.
#[test]
fn highlight_overlap_shorter_wins_at_shared_start() {
    let gpath = grammar_path("rust");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_rust").expect("open rust grammar");
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(grammar.language()).unwrap();

    let source = b"fn foo() {}\n";
    let tree = parser.parse(source as &[u8], None).expect("parse");

    let query_src = "(function_item) @function\n\"fn\" @keyword";
    let mut scope_reg = ScopeRegistry::new();
    let rope = ropey::Rope::from_str(&String::from_utf8_lossy(source));

    let highlighter =
        TreeSitterHighlighter::new(grammar.language(), query_src, &mut scope_reg)
            .expect("highlighter creation should succeed");

    let ctx = SourceContext { rope: &rope, tree: Some(&tree), source, line_start_byte: 0 };
    let mut out = Vec::new();
    highlighter.highlights_for_line(0, &ctx, &mut out);

    assert!(out.len() >= 2, "expected at least 2 spans; got: {out:?}");
    let keyword_span = out.iter().find(|&&(_, _, id)| scope_reg.name_of(id).contains("keyword"));
    let function_span = out.iter().find(|&&(_, _, id)| scope_reg.name_of(id).contains("function"));
    assert!(keyword_span.is_some(), "expected a 'keyword' scope");
    assert!(function_span.is_some(), "expected a 'function' scope");
    let (kw_start, kw_end, _) = *keyword_span.unwrap();
    let (fn_start, _, _) = *function_span.unwrap();
    assert_eq!(kw_start, 0);
    assert_eq!(kw_end, 2);
    assert_eq!(fn_start, kw_end, "function span must be trimmed to start at {kw_end}");
}

// Overlap resolver branch 2 (fully-contained drop): duplicate captures → one span.
// Mutation gate: removing the `else if` guard produces two spans, failing the count.
#[test]
fn highlight_overlap_fully_contained_is_dropped() {
    let gpath = grammar_path("json");
    let grammar = LoadedGrammar::open(&gpath, "tree_sitter_json").expect("open json grammar");
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(grammar.language()).unwrap();

    let source = b"\"hello\"\n";
    let tree = parser.parse(source as &[u8], None).expect("parse");

    let query_src = "(string) @string\n(string) @string.duplicate";
    let mut scope_reg = ScopeRegistry::new();
    let rope = ropey::Rope::from_str(&String::from_utf8_lossy(source));

    let highlighter =
        TreeSitterHighlighter::new(grammar.language(), query_src, &mut scope_reg)
            .expect("highlighter creation should succeed");

    let ctx = SourceContext { rope: &rope, tree: Some(&tree), source, line_start_byte: 0 };
    let mut out = Vec::new();
    highlighter.highlights_for_line(0, &ctx, &mut out);

    let string_spans: Vec<_> = out.iter().filter(|&&(s, e, _)| s == 0 && e == 7).collect();
    assert_eq!(
        string_spans.len(),
        1,
        "expected exactly one span for string node [0,7); got {}: {:?}",
        string_spans.len(),
        out.iter().map(|&(s, e, id)| (s, e, scope_reg.name_of(id))).collect::<Vec<_>>()
    );
}
