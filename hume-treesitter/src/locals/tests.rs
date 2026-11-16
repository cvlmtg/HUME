use hume_engine::theme::ScopeRegistry;
use tree_sitter::{InputEdit, Parser, Point, Query};

use super::*;
use crate::test_support::open_grammar;

const SCOPES: &str = "(function_item) @local.scope\n(block) @local.scope\n";
const DEFINITIONS: &str = "(parameter pattern: (identifier) @local.definition.variable.parameter)
(let_declaration pattern: (identifier) @local.definition.variable)\n";
const REFERENCES: &str = "(identifier) @local.reference\n";

/// The locals `source`, a rust `locals.scm`, finds in `text`, and the
/// registry its highlights were interned in.
fn locals(source: &str, text: &str) -> (LayerLocals, ScopeRegistry) {
    test_fixtures::require_grammars(&["rust"]);
    let grammar = open_grammar("rust", "tree_sitter_rust");
    let mut registry = ScopeRegistry::new();
    let mut skipped = Vec::new();
    let query = LocalsQuery::new(
        Query::new(grammar.language(), source).expect("the query compiles"),
        source,
        &mut registry,
        &mut skipped,
    )
    .expect("the query names locals");
    assert!(skipped.is_empty(), "{skipped:?}");
    let mut parser = Parser::new();
    parser
        .set_language(grammar.language())
        .expect("the grammar loads");
    let tree = parser.parse(text, None).expect("the text parses");
    let rope = ropey::Rope::from_str(text);
    (LayerLocals::of(&query, &tree, &rope), registry)
}

/// Each resolved reference with a highlight, as `(byte offset, highlight)`.
fn references(locals: &LayerLocals) -> Vec<(usize, ScopeId)> {
    locals
        .references_in(0, usize::MAX)
        .map(|(range, highlight)| (range.start_byte, *highlight))
        .collect()
}

fn all() -> String {
    format!("{SCOPES}{DEFINITIONS}{REFERENCES}")
}

#[test]
fn a_parameter_resolves_in_its_function_and_not_in_another() {
    let text = "fn f(a: u8) -> u8 { a }\nfn g() -> u8 { a }\n";
    let (locals, mut registry) = locals(&all(), text);
    let parameter = registry.intern("variable.parameter");
    let use_in_f = text.find("{ a }").unwrap() + 2;
    assert_eq!(references(&locals), [(use_in_f, parameter)]);
    let use_in_g = text.rfind("{ a }").unwrap() + 2;
    assert!(locals.is_local(use_in_f, use_in_f + 1));
    assert!(!locals.is_local(use_in_g, use_in_g + 1));
}

#[test]
fn a_definition_is_local_but_no_reference_to_itself() {
    let text = "fn f(a: u8) {}\n";
    let (locals, _) = locals(&all(), text);
    let definition = text.find('a').unwrap();
    assert!(locals.is_local(definition, definition + 1));
    assert!(references(&locals).is_empty());
}

#[test]
fn a_scope_that_does_not_inherit_sees_none_of_its_parents_definitions() {
    let text = "fn f() { let x = 1; fn g() -> i32 { x } }\n";
    let inner_use = text.rfind('x').unwrap();
    let inheriting = all();
    let (locals_in, _) = locals(&inheriting, text);
    assert!(locals_in.is_local(inner_use, inner_use + 1));
    let closed = format!(
        "((function_item) @local.scope (#set! local.scope-inherits false))\n(block) @local.scope\n{DEFINITIONS}{REFERENCES}"
    );
    let (locals_out, _) = locals(&closed, text);
    assert!(!locals_out.is_local(inner_use, inner_use + 1));
}

#[test]
fn a_reference_sees_only_the_definitions_before_it() {
    let text = "fn f(x: u8) { x; let x = 1; x; }\n";
    let (locals, mut registry) = locals(&all(), text);
    let parameter = registry.intern("variable.parameter");
    let variable = registry.intern("variable");
    let uses: Vec<usize> = text.match_indices("x;").map(|(i, _)| i).collect();
    assert_eq!(
        references(&locals),
        [(uses[0], parameter), (uses[1], variable)]
    );
}

#[test]
fn a_reference_before_any_definition_stays_unresolved() {
    let text = "fn f() { x; let x = 1; x; }\n";
    let (locals, _) = locals(&all(), text);
    let first = text.find("x;").unwrap();
    let last = text.rfind("x;").unwrap();
    assert!(!locals.is_local(first, first + 1));
    assert!(locals.is_local(last, last + 1));
}

#[test]
fn a_bare_definition_makes_its_references_local_without_a_highlight() {
    let source =
        format!("{SCOPES}(let_declaration pattern: (identifier) @local.definition)\n{REFERENCES}");
    let text = "fn f() { let x = 1; x; }\n";
    let (locals, _) = locals(&source, text);
    let reference = text.rfind('x').unwrap();
    assert!(locals.is_local(reference, reference + 1));
    assert!(references(&locals).is_empty());
}

#[test]
fn an_edit_moves_every_range() {
    let text = "fn f(a: u8) -> u8 { a }\n";
    let (mut locals, _) = locals(&all(), text);
    let before = references(&locals);
    locals.edit(&InputEdit {
        start_byte: 0,
        old_end_byte: 0,
        new_end_byte: 2,
        start_position: Point::new(0, 0),
        old_end_position: Point::new(0, 0),
        new_end_position: Point::new(0, 2),
    });
    let after: Vec<_> = before.iter().map(|&(at, h)| (at + 2, h)).collect();
    assert_eq!(references(&locals), after);
    let definition = text.find('a').unwrap() + 2;
    assert!(locals.is_local(definition, definition + 1));
}
