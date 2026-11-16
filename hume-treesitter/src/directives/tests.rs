use std::collections::BTreeSet;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor, QueryError};

use super::*;
use crate::test_support::open_grammar;

/// A query kind with no directives of its own.
#[derive(Default)]
struct Plain;

impl PatternDirectives for Plain {}

/// The patterns of `source` that match in rust `text` once `parse_patterns`
/// has read them, and what it skipped.
fn matching_patterns(source: &str, text: &str) -> (BTreeSet<usize>, Vec<QueryError>) {
    test_fixtures::require_grammars(&["rust"]);
    let grammar = open_grammar("rust", "tree_sitter_rust");
    let mut query = Query::new(grammar.language(), source).expect("the query compiles");
    let mut skipped = Vec::new();
    let patterns: Vec<Plain> = parse_patterns(&mut query, source, &mut skipped);
    assert_eq!(patterns.len(), query.pattern_count());
    let mut parser = Parser::new();
    parser
        .set_language(grammar.language())
        .expect("the grammar loads");
    let tree = parser.parse(text, None).expect("the text parses");
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), text.as_bytes());
    let mut matched = BTreeSet::new();
    while let Some(m) = matches.next() {
        matched.insert(m.pattern_index);
    }
    (matched, skipped)
}

#[test]
fn a_pattern_with_an_unsupported_predicate_never_matches_and_is_reported() {
    let source = "(block) @a\n((block) @b (#frobnicate! @b))\n";
    let (matched, skipped) = matching_patterns(source, "fn f() {}\n");
    assert_eq!(matched, BTreeSet::from([0]));
    let [error] = skipped.as_slice() else {
        panic!("expected one skipped pattern, got {skipped:?}");
    };
    assert_eq!((error.row, error.column), (1, 0));
    assert_eq!(error.message, "#frobnicate! is not supported");
}

#[test]
fn no_node_is_local() {
    let source = "((block) @a (#is-not? local))\n((block) @b (#is? local))\n";
    let (matched, skipped) = matching_patterns(source, "fn f() {}\n");
    assert_eq!(matched, BTreeSet::from([0]));
    assert!(skipped.is_empty(), "{skipped:?}");
}

#[test]
fn a_property_other_than_local_is_unsupported() {
    let source = r#"((block) @a (#is? "definition"))
((block) @b (#is-not? local "x"))
"#;
    let (matched, skipped) = matching_patterns(source, "fn f() {}\n");
    assert!(matched.is_empty(), "{matched:?}");
    let messages: Vec<_> = skipped.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(
        messages,
        ["#is? is not supported", "#is-not? is not supported"]
    );
}
