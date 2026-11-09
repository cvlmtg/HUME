use test_fixtures::{require_grammars, require_helix_query};

use super::*;
use crate::test_support::open_grammar;
use hume_core::OptionalQuery;

fn compile(source: &str) -> ContextQuery {
    require_grammars(&["rust"]);
    let grammar = open_grammar("rust", "tree_sitter_rust");
    ContextQuery::new(tree_sitter::Query::new(grammar.language(), source).expect("compile query"))
}

#[test]
fn context_query_resolves_its_captures() {
    require_grammars(&["rust"]);
    let path = require_helix_query("rust", OptionalQuery::Context.file_name());
    let fixture = compile(&std::fs::read_to_string(path).expect("read context.scm"));
    assert!(fixture.context.is_some());
    assert!(fixture.start.is_none());

    let with_start = compile("(function_item name: (_) @context.start) @context");
    assert!(with_start.context.is_some());
    assert!(with_start.start.is_some());
}
