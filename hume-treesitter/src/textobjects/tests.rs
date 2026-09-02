use hume_test_fixtures::require_grammars;

use super::*;

fn compile(source: &str) -> TextObjectsQuery {
    require_grammars(&["rust"]);
    let grammar = crate::test_support::open_grammar("rust", "tree_sitter_rust");
    let query = tree_sitter::Query::new(grammar.language(), source).expect("compile query");
    TextObjectsQuery::new(query)
}

#[test]
fn every_kind_span_capture_name_resolves_to_its_own_index() {
    let q = compile(
        "(function_item) @function.inside
         (function_item) @function.around
         (struct_item) @class.inside
         (struct_item) @class.around
         (parameters) @parameter.inside
         (parameters) @parameter.around
         (line_comment) @comment.inside
         (line_comment) @comment.around
         (function_item) @test.inside
         (function_item) @test.around
         (field_expression) @entry.inside
         (field_expression) @entry.around",
    );
    for kind in ObjectKind::ALL {
        for span in [ObjectSpan::Inside, ObjectSpan::Around] {
            assert!(
                q.defines(kind, span),
                "expected {kind:?}.{span:?} to be defined"
            );
        }
    }
}

#[test]
fn underscore_prefixed_and_unknown_suffix_captures_resolve_to_nothing() {
    let q = compile(
        "(function_item) @_helper
         (function_item) @function.x
         (function_item) @function.inside",
    );
    // `@_helper` never parses as `<kind>.<span>` at all (no dot to split on).
    // `@function.x` splits fine but `x` isn't a known ObjectSpan — neither
    // should register a capture beyond the one legitimate `function.inside`.
    assert!(q.defines(ObjectKind::Function, ObjectSpan::Inside));
    assert!(!q.defines(ObjectKind::Function, ObjectSpan::Around));
    assert!(!q.defines(ObjectKind::Function, ObjectSpan::Movement));
}

#[test]
fn defines_is_false_for_a_pair_the_query_omits() {
    let q = compile("(function_item) @function.inside");
    assert!(q.defines(ObjectKind::Function, ObjectSpan::Inside));
    for kind in ObjectKind::ALL {
        for span in ObjectSpan::ALL {
            if (kind, span) == (ObjectKind::Function, ObjectSpan::Inside) {
                continue;
            }
            assert!(
                !q.defines(kind, span),
                "expected {kind:?}.{span:?} to be undefined"
            );
        }
    }
}

#[test]
fn real_helix_rust_textobjects_defines_expected_pairs() {
    require_grammars(&["rust"]);
    let path = hume_test_fixtures::helix_textobjects_path("rust")
        .expect("rust helix-textobjects.scm fixture (run scripts/fetch-test-grammars.sh)");
    let source = std::fs::read_to_string(path).expect("read helix-textobjects.scm");
    let q = compile(&source);
    assert!(q.defines(ObjectKind::Function, ObjectSpan::Inside));
    assert!(q.defines(ObjectKind::Function, ObjectSpan::Around));
    assert!(q.defines(ObjectKind::Class, ObjectSpan::Inside));
    assert!(q.defines(ObjectKind::Parameter, ObjectSpan::Inside));
    assert!(q.defines(ObjectKind::Parameter, ObjectSpan::Around));
}
