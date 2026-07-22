use rustc_hash::FxHashMap;
use std::sync::atomic::AtomicBool;

use crate::grammar::LoadedGrammar;
use crate::highlight::TreeSitterHighlighter;
use hume_engine::theme::ScopeRegistry;

use super::*;
use crate::registry::GrammarBundle;
use hume_test_fixtures::{
    grammar_parser_path, grammar_query_path, skip_unless_file, skip_unless_grammars,
};

/// Load a real grammar fixture with an optional custom injections source
/// (overriding whatever `injections.scm` the fixture ships, if any) —
/// keeps each test's injection query minimal and self-contained rather
/// than depending on upstream Helix query wording.
fn make_bundle(name: &str, symbol: &str, injections_src: Option<&str>) -> Arc<GrammarBundle> {
    let parser_path = grammar_parser_path(name);
    let grammar = LoadedGrammar::open(&parser_path, symbol).expect("load grammar");
    let highlights_src =
        std::fs::read_to_string(grammar_query_path(name)).expect("read highlights.scm");
    let query = Arc::new(
        tree_sitter::Query::new(grammar.language(), &highlights_src).expect("compile query"),
    );
    let mut registry = ScopeRegistry::new();
    let highlighter = Arc::new(TreeSitterHighlighter::from_shared_query(
        query,
        &mut registry,
    ));
    let injections = injections_src.map(|src| {
        let q = Arc::new(
            tree_sitter::Query::new(grammar.language(), src).expect("compile injections"),
        );
        InjectionsQuery::new(q)
    });
    Arc::new(GrammarBundle {
        grammar,
        highlighter,
        injections,
        config_gen: next_test_config_gen(),
    })
}

/// Distinct per call, mirroring `LanguageRegistry`'s `config_gen`
/// invariant so tests that compare configs by gen see real identity.
fn next_test_config_gen() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT_GEN: AtomicU32 = AtomicU32::new(0);
    NEXT_GEN.fetch_add(1, Ordering::Relaxed)
}

fn parse(bundle: &GrammarBundle, source: &str) -> (tree_sitter::Parser, tree_sitter::Tree) {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(bundle.grammar.language()).unwrap();
    let tree = parser.parse(source, None).expect("parse");
    (parser, tree)
}

// ── content_ranges (pure) ─────────────────────────────────────────────────

#[test]
fn content_ranges_excludes_only_unnamed_children_by_default() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let json = make_bundle("json", "tree_sitter_json", None);
    // `[1,2]` parses as an `array` node whose direct children are, in
    // order: `[` (unnamed), `number` "1" (named), `,` (unnamed), `number`
    // "2" (named), `]` (unnamed). Excluding only unnamed children must
    // leave the two number spans (the named children) as content,
    // cutting out just the bracket/comma punctuation.
    let (_parser, tree) = parse(&json, "[1,2]\n");
    let array = tree.root_node().named_child(0).expect("array node");
    assert_eq!(array.kind(), "array");

    let content = content_ranges(array, false);
    let byte_ranges: Vec<(usize, usize)> =
        content.iter().map(|r| (r.start_byte, r.end_byte)).collect();
    assert_eq!(
        byte_ranges,
        vec![(1, 2), (3, 4)],
        "expected the two named `number` spans, excluding `[`, `,`, `]`; got {content:?}"
    );
}

#[test]
fn content_ranges_include_unnamed_children_returns_whole_node() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let json = make_bundle("json", "tree_sitter_json", None);
    let (_parser, tree) = parse(&json, "[1,2]\n");
    let array = tree.root_node().named_child(0).expect("array node");

    let included = content_ranges(array, true);
    assert_eq!(included, vec![array.range()]);
}

// ── normalize_ranges (pure) ───────────────────────────────────────────────

fn range(start: usize, end: usize) -> tree_sitter::Range {
    tree_sitter::Range {
        start_byte: start,
        end_byte: end,
        start_point: tree_sitter::Point {
            row: 0,
            column: start,
        },
        end_point: tree_sitter::Point {
            row: 0,
            column: end,
        },
    }
}

#[test]
fn normalize_ranges_drops_zero_width_and_sorts() {
    let got = normalize_ranges(vec![range(5, 8), range(3, 3), range(0, 2)]);
    assert_eq!(got, vec![range(0, 2), range(5, 8)]);
}

#[test]
fn normalize_ranges_merges_overlapping_and_touching() {
    let got = normalize_ranges(vec![range(0, 5), range(3, 8), range(8, 10)]);
    assert_eq!(
        got,
        vec![range(0, 10)],
        "overlapping [0,5)+[3,8) and touching [8,10) must merge into one"
    );
}

// ── resolve_and_parse_injections ──────────────────────────────────────────

#[test]
fn static_language_override_wins_regardless_of_content() {
    if skip_unless_grammars(&["json", "rust"]) {
        return;
    }
    let json = make_bundle(
        "json",
        "tree_sitter_json",
        Some(
            r#"((string_content) @injection.content (#set! injection.language "rust") (#set! injection.include-unnamed-children))"#,
        ),
    );
    let rust = make_bundle("rust", "tree_sitter_rust", None);
    let mut langs = FxHashMap::default();
    langs.insert("rust".to_owned(), Arc::clone(&rust));

    let source = "[\"hello\"]\n";
    let (mut parser, tree) = parse(&json, source);
    let rope = ropey::Rope::from_str(source);
    let cancel = AtomicBool::new(false);

    let out =
        resolve_and_parse_injections(&mut parser, &tree, &json, &rope, &langs, &cancel, 1);
    assert_eq!(out.len(), 1, "expected exactly one injected layer");
    assert!(
        Arc::ptr_eq(&out[0].bundle, &rust),
        "expected the rust bundle to be resolved"
    );
    assert_eq!(out[0].depth, 1);
}

#[test]
fn unknown_injection_language_is_skipped_silently() {
    if skip_unless_grammars(&["json", "rust"]) {
        return;
    }
    let json = make_bundle(
        "json",
        "tree_sitter_json",
        Some(
            r#"((string_content) @injection.content (#set! injection.language "no-such-language") (#set! injection.include-unnamed-children))"#,
        ),
    );
    // Non-empty but irrelevant: proves the lookup is genuinely by-key,
    // not just "map happens to be empty".
    let mut langs = FxHashMap::default();
    langs.insert(
        "rust".to_owned(),
        make_bundle("rust", "tree_sitter_rust", None),
    );

    let source = "[\"hello\"]\n";
    let (mut parser, tree) = parse(&json, source);
    let rope = ropey::Rope::from_str(source);
    let cancel = AtomicBool::new(false);

    let out =
        resolve_and_parse_injections(&mut parser, &tree, &json, &rope, &langs, &cancel, 1);
    assert!(
        out.is_empty(),
        "unresolvable injection language must produce no layer, got: {} layers",
        out.len()
    );
}

#[test]
fn combined_merges_multiple_matches_into_one_layer() {
    if skip_unless_grammars(&["json", "rust"]) {
        return;
    }
    let json = make_bundle(
        "json",
        "tree_sitter_json",
        Some(
            r#"((string_content) @injection.content (#set! injection.language "rust") (#set! injection.combined) (#set! injection.include-unnamed-children))"#,
        ),
    );
    let rust = make_bundle("rust", "tree_sitter_rust", None);
    let mut langs = FxHashMap::default();
    langs.insert("rust".to_owned(), rust);

    // Three string literals — without `injection.combined` these would be
    // three separate layers; with it, exactly one layer with 3 ranges.
    let source = "[\"a\", \"b\", \"c\"]\n";
    let (mut parser, tree) = parse(&json, source);
    let rope = ropey::Rope::from_str(source);
    let cancel = AtomicBool::new(false);

    let out =
        resolve_and_parse_injections(&mut parser, &tree, &json, &rope, &langs, &cancel, 1);
    assert_eq!(
        out.len(),
        1,
        "combined matches of the same pattern+language must merge into one layer"
    );
    assert_eq!(
        out[0].ranges.len(),
        3,
        "one range per string literal, got: {:?}",
        out[0].ranges
    );
}

#[test]
fn depth_cap_stops_recursion_at_max_depth() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    // Self-injecting: every `array` node re-parses its own (identical)
    // text as json again, matching `array` once more in the fresh tree —
    // unbounded without a depth cap, since the content never changes.
    let json = make_bundle(
        "json",
        "tree_sitter_json",
        Some(
            r#"((array) @injection.content (#set! injection.language "json") (#set! injection.include-unnamed-children))"#,
        ),
    );
    let mut langs = FxHashMap::default();
    langs.insert("json".to_owned(), Arc::clone(&json));

    let source = "[1]\n";
    let (mut parser, tree) = parse(&json, source);
    let rope = ropey::Rope::from_str(source);
    let cancel = AtomicBool::new(false);

    let out =
        resolve_and_parse_injections(&mut parser, &tree, &json, &rope, &langs, &cancel, 1);
    assert_eq!(
        out.len(),
        3,
        "expected one layer per depth (1, 2, 3), got: {}",
        out.len()
    );
    let max_depth = out.iter().map(|i| i.depth).max().unwrap();
    assert_eq!(
        max_depth, MAX_INJECTION_DEPTH,
        "recursion must stop exactly at the cap"
    );
}

#[test]
fn dynamic_language_capture_reads_fenced_code_info_string() {
    if skip_unless_grammars(&["markdown", "rust"]) {
        return;
    }
    let inj_path = grammar_query_path("markdown").with_file_name("injections.scm");
    if skip_unless_file(&inj_path, "markdown injections.scm") {
        return;
    }
    let inj_src = std::fs::read_to_string(inj_path).unwrap();
    let markdown = make_bundle("markdown", "tree_sitter_markdown", Some(&inj_src));
    let rust = make_bundle("rust", "tree_sitter_rust", None);
    let mut langs = FxHashMap::default();
    langs.insert("rust".to_owned(), Arc::clone(&rust));

    let source = "```rust\nfn main() {}\n```\n";
    let (mut parser, tree) = parse(&markdown, source);
    let rope = ropey::Rope::from_str(source);
    let cancel = AtomicBool::new(false);

    let out =
        resolve_and_parse_injections(&mut parser, &tree, &markdown, &rope, &langs, &cancel, 1);
    assert!(
        out.iter().any(|i| Arc::ptr_eq(&i.bundle, &rust)),
        "expected a rust layer resolved from the fenced code block's info string, got {} layers",
        out.len()
    );
}

#[test]
fn dynamic_language_capture_unknown_info_string_no_layer() {
    if skip_unless_grammars(&["markdown"]) {
        return;
    }
    let inj_path = grammar_query_path("markdown").with_file_name("injections.scm");
    if skip_unless_file(&inj_path, "markdown injections.scm") {
        return;
    }
    let inj_src = std::fs::read_to_string(inj_path).unwrap();
    let markdown = make_bundle("markdown", "tree_sitter_markdown", Some(&inj_src));
    let langs: FxHashMap<String, Arc<GrammarBundle>> = FxHashMap::default();

    let source = "```no-such-lang\nwhatever\n```\n";
    let (mut parser, tree) = parse(&markdown, source);
    let rope = ropey::Rope::from_str(source);
    let cancel = AtomicBool::new(false);

    let out =
        resolve_and_parse_injections(&mut parser, &tree, &markdown, &rope, &langs, &cancel, 1);
    assert!(
        out.is_empty(),
        "unknown fenced-code language must produce no layer, got: {} layers",
        out.len()
    );
}
