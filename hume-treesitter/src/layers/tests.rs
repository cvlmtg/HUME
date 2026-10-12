use test_fixtures::require_grammars;

use super::{SyntaxLayer, SyntaxLayers, layer_covers_line};
use crate::test_support::{make_bundle, open_grammar, range, test_language_id};

// `layer_covers_line` only ever reads `ranges`. The tree's actual
// content is irrelevant, so every test case shares one parsed layer and
// just varies `ranges`/`line_start`/`line_end`.
fn injected_layer(ranges: Vec<tree_sitter::Range>) -> SyntaxLayer {
    let grammar = open_grammar("json", "tree_sitter_json");
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(grammar.language())
        .expect("set language");
    let tree = parser.parse("{}\n", None).expect("parse");
    let bundle = make_bundle("json", "tree_sitter_json", "", None, None);
    SyntaxLayer {
        tree,
        bundle,
        ranges,
        depth: 1, // an injected layer; depth 0 (root) always short-circuits on empty ranges
    }
}

#[test]
fn root_layer_with_empty_ranges_covers_every_line() {
    require_grammars(&["json"]);
    // depth 0 in practice, but the empty-ranges short-circuit doesn't
    // actually key off `depth`, so assert on the field it does check.
    let layer = injected_layer(vec![]);
    assert!(layer_covers_line(&layer, 0, 1));
    assert!(layer_covers_line(&layer, 10_000, 10_001));
}

#[test]
fn injected_layer_covers_a_line_inside_its_single_range() {
    require_grammars(&["json"]);
    let layer = injected_layer(vec![range(10, 20)]);
    assert!(layer_covers_line(&layer, 12, 15));
}

#[test]
fn injected_layer_does_not_cover_a_line_entirely_before_its_range() {
    require_grammars(&["json"]);
    let layer = injected_layer(vec![range(10, 20)]);
    assert!(!layer_covers_line(&layer, 0, 5));
}

#[test]
fn injected_layer_does_not_cover_a_line_entirely_after_its_range() {
    require_grammars(&["json"]);
    let layer = injected_layer(vec![range(10, 20)]);
    assert!(!layer_covers_line(&layer, 25, 30));
}

/// The multi-range case a real combined `markdown.inline` layer produces
/// (one range per paragraph). A root layer (empty `ranges`) short-circuits
/// before the binary search runs, and a single-range layer only resolves
/// `partition_point` to index 0 or 1. This exercises a line landing on the *second* range, and a line
/// falling in the gap between the two: real multi-candidate lookups.
#[test]
fn injected_layer_binary_search_finds_a_non_first_range() {
    require_grammars(&["json"]);
    let layer = injected_layer(vec![range(0, 10), range(20, 30), range(40, 50)]);
    assert!(
        layer_covers_line(&layer, 22, 25),
        "must find the second range, not just check index 0"
    );
    assert!(
        !layer_covers_line(&layer, 12, 15),
        "gap between range 0 and range 1 must not count as covered"
    );
}

#[test]
fn injected_layer_boundary_is_half_open() {
    require_grammars(&["json"]);
    let layer = injected_layer(vec![range(10, 20)]);
    // A line starting exactly at the range's end is adjacent, not
    // overlapping: `line_start < end_byte` must be strict.
    assert!(!layer_covers_line(&layer, 20, 25));
    // A line ending exactly at the range's start likewise doesn't
    // overlap: `partition_point`'s `start_byte < line_end` excludes it.
    assert!(!layer_covers_line(&layer, 5, 10));
    // But a line covering the range's first byte does overlap.
    assert!(layer_covers_line(&layer, 5, 11));
}

fn layer_of(name: &str, symbol: &str, ranges: Vec<tree_sitter::Range>, depth: u8) -> SyntaxLayer {
    let grammar = open_grammar(name, symbol);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(grammar.language())
        .expect("set language");
    SyntaxLayer {
        tree: parser.parse("", None).expect("parse"),
        bundle: make_bundle(name, symbol, "", None, None),
        ranges,
        depth,
    }
}

fn json() -> crate::registry::LanguageId {
    test_language_id("json")
}

fn rust() -> crate::registry::LanguageId {
    test_language_id("rust")
}

fn markdown() -> crate::registry::LanguageId {
    test_language_id("markdown")
}

#[test]
fn the_root_layer_contains_every_span() {
    require_grammars(&["markdown"]);
    let layers = SyntaxLayers::new(vec![layer_of(
        "markdown",
        "tree_sitter_markdown",
        vec![],
        0,
    )]);

    assert_eq!(layers.languages_containing(0..1), [markdown()]);
    assert_eq!(layers.languages_containing(500..900), [markdown()]);
}

#[test]
fn a_span_inside_one_included_range_is_in_that_layer() {
    require_grammars(&["markdown", "rust"]);
    let layers = SyntaxLayers::new(vec![
        layer_of("markdown", "tree_sitter_markdown", vec![], 0),
        layer_of(
            "rust",
            "tree_sitter_rust",
            vec![range(10, 20), range(30, 40)],
            1,
        ),
    ]);

    assert_eq!(layers.languages_containing(10..20), [rust(), markdown()]);
    assert_eq!(layers.languages_containing(32..35), [rust(), markdown()]);
    assert_eq!(layers.languages_containing(5..12), [markdown()]);
    assert_eq!(layers.languages_containing(18..21), [markdown()]);
}

#[test]
fn a_span_straddling_a_gap_between_ranges_is_not_in_that_layer() {
    require_grammars(&["markdown", "rust"]);
    let layers = SyntaxLayers::new(vec![
        layer_of("markdown", "tree_sitter_markdown", vec![], 0),
        layer_of(
            "rust",
            "tree_sitter_rust",
            vec![range(10, 20), range(30, 40)],
            1,
        ),
    ]);

    assert_eq!(layers.languages_containing(15..35), [markdown()]);
}

#[test]
fn deeper_layers_come_first_whatever_their_order_in_the_vector() {
    require_grammars(&["markdown", "rust", "json"]);
    let layers = SyntaxLayers::new(vec![
        layer_of("markdown", "tree_sitter_markdown", vec![], 0),
        layer_of("json", "tree_sitter_json", vec![range(12, 18)], 2),
        layer_of("rust", "tree_sitter_rust", vec![range(10, 20)], 1),
    ]);

    assert_eq!(
        layers.languages_containing(13..15),
        [json(), rust(), markdown()]
    );
}

#[test]
fn of_two_layers_at_one_depth_the_later_comes_first() {
    require_grammars(&["markdown", "rust", "json"]);
    let layers = SyntaxLayers::new(vec![
        layer_of("markdown", "tree_sitter_markdown", vec![], 0),
        layer_of("rust", "tree_sitter_rust", vec![range(10, 20)], 1),
        layer_of("json", "tree_sitter_json", vec![range(10, 20)], 1),
    ]);

    assert_eq!(
        layers.languages_containing(11..12),
        [json(), rust(), markdown()]
    );
}
