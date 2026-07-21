use std::sync::Arc;

use hume_engine::theme::ScopeRegistry;
use hume_test_fixtures::{grammar_parser_path, skip_unless_grammars};

use super::{SyntaxLayer, layer_covers_line};
use crate::grammar::LoadedGrammar;
use crate::highlight::TreeSitterHighlighter;

// `layer_covers_line` only ever reads `ranges` — the tree's actual
// content is irrelevant, so every test case shares one parsed layer and
// just varies `ranges`/`line_start`/`line_end`.
fn injected_layer(ranges: Vec<tree_sitter::Range>) -> SyntaxLayer {
    let path = grammar_parser_path("json");
    assert!(
        path.exists(),
        "grammar fixture missing: {}\nrun scripts/fetch-test-grammars.sh from the repo root",
        path.display()
    );
    let grammar = LoadedGrammar::open(&path, "tree_sitter_json").expect("load grammar");
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(grammar.language())
        .expect("set language");
    let tree = parser.parse("{}\n", None).expect("parse");
    let query = Arc::new(tree_sitter::Query::new(grammar.language(), "").expect("empty query"));
    let mut registry = ScopeRegistry::new();
    let highlighter = Arc::new(TreeSitterHighlighter::from_shared_query(
        query,
        &mut registry,
    ));
    SyntaxLayer {
        tree,
        highlighter,
        ranges,
        depth: 1, // an injected layer — depth 0 (root) always short-circuits on empty ranges
    }
}

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
fn root_layer_with_empty_ranges_covers_every_line() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    // depth 0 in practice, but the empty-ranges short-circuit doesn't
    // actually key off `depth` — assert on the field it does check.
    let layer = injected_layer(vec![]);
    assert!(layer_covers_line(&layer, 0, 1));
    assert!(layer_covers_line(&layer, 10_000, 10_001));
}

#[test]
fn injected_layer_covers_a_line_inside_its_single_range() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let layer = injected_layer(vec![range(10, 20)]);
    assert!(layer_covers_line(&layer, 12, 15));
}

#[test]
fn injected_layer_does_not_cover_a_line_entirely_before_its_range() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let layer = injected_layer(vec![range(10, 20)]);
    assert!(!layer_covers_line(&layer, 0, 5));
}

#[test]
fn injected_layer_does_not_cover_a_line_entirely_after_its_range() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let layer = injected_layer(vec![range(10, 20)]);
    assert!(!layer_covers_line(&layer, 25, 30));
}

/// The multi-range case a real combined `markdown.inline` layer produces
/// (one range per paragraph) — the gap here is exactly what earlier
/// coverage was missing: every prior test used a root layer (empty
/// `ranges`, short-circuits before the binary search ever runs) or a
/// single-range layer, so `partition_point` always resolved to index 0
/// or 1. This exercises a line landing on the *second* range, and a line
/// falling in the gap between the two — real multi-candidate lookups.
#[test]
fn injected_layer_binary_search_finds_a_non_first_range() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let layer = injected_layer(vec![range(0, 10), range(20, 30), range(40, 50)]);
    assert!(
        layer_covers_line(&layer, 22, 25),
        "must find the third range, not just check index 0"
    );
    assert!(
        !layer_covers_line(&layer, 12, 15),
        "gap between range 0 and range 1 must not count as covered"
    );
}

#[test]
fn injected_layer_boundary_is_half_open() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let layer = injected_layer(vec![range(10, 20)]);
    // A line starting exactly at the range's end is adjacent, not
    // overlapping — `line_start < end_byte` must be strict.
    assert!(!layer_covers_line(&layer, 20, 25));
    // A line ending exactly at the range's start likewise doesn't
    // overlap — `partition_point`'s `start_byte < line_end` excludes it.
    assert!(!layer_covers_line(&layer, 5, 10));
    // But a line covering the range's first byte does overlap.
    assert!(layer_covers_line(&layer, 5, 11));
}
