use std::sync::Arc;

use rustc_hash::FxHashMap;

use hume_editing::text::BufferText;
use hume_test_fixtures::{helix_injections_path, helix_textobjects_path, require_grammars};

use super::*;
use crate::registry::GrammarBundle;
use crate::syntax::Syntax;
use crate::test_support::{empty_langs, make_bundle};

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
    for &kind in ObjectKind::ALL {
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
    for &kind in ObjectKind::ALL {
        for &span in ObjectSpan::ALL {
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
    let path = helix_textobjects_path("rust")
        .expect("rust helix-textobjects.scm fixture (run scripts/fetch-test-grammars.sh)");
    let source = std::fs::read_to_string(path).expect("read helix-textobjects.scm");
    let q = compile(&source);
    assert!(q.defines(ObjectKind::Function, ObjectSpan::Inside));
    assert!(q.defines(ObjectKind::Function, ObjectSpan::Around));
    assert!(q.defines(ObjectKind::Class, ObjectSpan::Inside));
    assert!(q.defines(ObjectKind::Parameter, ObjectSpan::Inside));
    assert!(q.defines(ObjectKind::Parameter, ObjectSpan::Around));
}

// ── ObjectSpans::enclosing / adjacent — synthetic spans ─────────────────────
//
// `spans` is private to this module and its descendants (this one included),
// so these build an `ObjectSpans` directly from a literal list rather than
// through a real query — pure lookup-logic tests with no grammar involved.

/// Build an `ObjectSpans` from an arbitrary-order list through the same
/// `ObjectSpans::finish` a real collection result goes through — `enclosing`
/// and `adjacent` both assume its sort-and-dedup, so a test double must not
/// re-implement that comparator independently.
fn spans_from(list: &[(usize, usize)]) -> ObjectSpans {
    ObjectSpans::finish(list.to_vec())
}

#[test]
fn enclosing_picks_the_smallest_containing_span() {
    let spans = spans_from(&[(0, 20), (5, 10)]);
    assert_eq!(spans.enclosing(7), Some((5, 10)));
}

#[test]
fn enclosing_returns_none_when_nothing_contains_pos() {
    let spans = spans_from(&[(0, 5)]);
    assert_eq!(spans.enclosing(10), None);
}

#[test]
fn enclosing_includes_pos_on_a_spans_last_char() {
    let spans = spans_from(&[(0, 5)]);
    assert_eq!(spans.enclosing(5), Some((0, 5)));
}

#[test]
fn adjacent_forward_picks_smallest_start_after_pos() {
    let spans = spans_from(&[(0, 5), (10, 15), (20, 25)]);
    assert_eq!(spans.adjacent(6, Direction::Forward), Some((10, 15)));
}

#[test]
fn adjacent_forward_ties_pick_largest_end() {
    let spans = spans_from(&[(10, 12), (10, 20)]);
    assert_eq!(spans.adjacent(5, Direction::Forward), Some((10, 20)));
}

#[test]
fn adjacent_backward_picks_largest_start_before_pos() {
    let spans = spans_from(&[(0, 5), (10, 15), (20, 25)]);
    assert_eq!(spans.adjacent(18, Direction::Backward), Some((10, 15)));
}

// Vim `[m`: a backward press from *inside* an object must land on that
// object's own start first, not skip past it to an earlier one — the
// reason `adjacent` is keyed on `start` in both directions rather than
// `end` for the backward case (Helix's own convention).
#[test]
fn adjacent_backward_from_inside_an_object_lands_on_its_own_start() {
    let spans = spans_from(&[(0, 5), (10, 20)]);
    assert_eq!(spans.adjacent(15, Direction::Backward), Some((10, 20)));
}

#[test]
fn adjacent_backward_ties_pick_largest_end() {
    let spans = spans_from(&[(0, 5), (0, 10)]);
    assert_eq!(spans.adjacent(8, Direction::Backward), Some((0, 10)));
}

#[test]
fn adjacent_returns_none_at_buffer_edges() {
    let forward_only = spans_from(&[(0, 5)]);
    assert_eq!(forward_only.adjacent(10, Direction::Forward), None);
    let backward_only = spans_from(&[(10, 15)]);
    assert_eq!(backward_only.adjacent(5, Direction::Backward), None);
}

// ── ObjectSpans::collect / collect_for_navigation — real rust fixture ──────

/// The real `rust` bundle carrying the fetched Helix `textobjects.scm` — no
/// injections; these tests probe single-layer trees.
fn rust_bundle_with_real_textobjects() -> Arc<GrammarBundle> {
    require_grammars(&["rust"]);
    let path = helix_textobjects_path("rust")
        .expect("rust helix-textobjects.scm fixture (run scripts/fetch-test-grammars.sh)");
    let source = std::fs::read_to_string(path).expect("read helix-textobjects.scm");
    make_bundle("rust", "tree_sitter_rust", "", None, Some(&source))
}

/// The real `markdown` bundle carrying the fetched Helix `injections.scm` —
/// the version PLUM actually installs (see `hume_test_fixtures`'s doc on
/// why that's distinct from the grammar's own bundled query).
fn markdown_bundle_with_helix_injections() -> Arc<GrammarBundle> {
    require_grammars(&["markdown"]);
    let path = helix_injections_path("markdown")
        .expect("markdown helix-injections.scm fixture (run scripts/fetch-test-grammars.sh)");
    let source = std::fs::read_to_string(path).expect("read helix-injections.scm");
    make_bundle("markdown", "tree_sitter_markdown", "", Some(&source), None)
}

/// The buffer text at `span`, inclusive end — for asserting on the actual
/// text a hull collected rather than hand-counted char offsets.
fn span_text(text: &BufferText, span: (usize, usize)) -> String {
    text.slice(span.0..span.1 + 1).to_string()
}

/// Parse `source` as rust with the real Helix `textobjects.scm` attached.
/// Returns the `Syntax` rather than its layers: `layers()` borrows from it,
/// so the caller has to hold it — every test below takes the borrow on its
/// own next line.
fn rust_syntax(source: &str) -> (Syntax, BufferText) {
    let text = BufferText::from(source);
    let syn = Syntax::attach_sync(rust_bundle_with_real_textobjects(), &text, &empty_langs());
    (syn, text)
}

#[test]
fn function_around_on_an_attributed_function_includes_the_attributes() {
    let source = "#[inline]\nfn foo() {\n    1\n}\n";
    let (syn, text) = rust_syntax(source);
    let layers = syn.layers().expect("layers installed");
    let pos = text.byte_to_char(source.find('1').unwrap());

    let around = ObjectSpans::collect(layers, &text, ObjectKind::Function, ObjectSpan::Around);
    let span = around.enclosing(pos).expect("function.around at the body");
    let got = span_text(&text, span);
    assert!(
        got.starts_with("#[inline]"),
        "must include the attribute: {got:?}"
    );
    assert!(
        got.trim_end().ends_with('}'),
        "must include the body: {got:?}"
    );
}

#[test]
fn parameter_around_probed_at_a_non_last_argument_includes_its_trailing_comma() {
    let source = "fn add(a: i32, b: i32) -> i32 { a + b }\n";
    let (syn, text) = rust_syntax(source);
    let layers = syn.layers().expect("layers installed");
    let pos = text.byte_to_char(source.find("a: i32").unwrap());

    let around = ObjectSpans::collect(layers, &text, ObjectKind::Parameter, ObjectSpan::Around);
    let around_span = around
        .enclosing(pos)
        .expect("parameter.around at the first argument");
    assert_eq!(span_text(&text, around_span), "a: i32,");
}

#[test]
fn parameter_inside_at_the_same_argument_excludes_the_comma() {
    let source = "fn add(a: i32, b: i32) -> i32 { a + b }\n";
    let (syn, text) = rust_syntax(source);
    let layers = syn.layers().expect("layers installed");
    let pos = text.byte_to_char(source.find("a: i32").unwrap());

    let inside = ObjectSpans::collect(layers, &text, ObjectKind::Parameter, ObjectSpan::Inside);
    let inside_span = inside
        .enclosing(pos)
        .expect("parameter.inside at the first argument");
    assert_eq!(span_text(&text, inside_span), "a: i32");
}

#[test]
fn collect_for_navigation_parameter_yields_inside_spans_no_trailing_comma() {
    let source = "fn add(a: i32, b: i32) -> i32 { a + b }\n";
    let (syn, text) = rust_syntax(source);
    let layers = syn.layers().expect("layers installed");
    let pos = text.byte_to_char(source.find("a: i32").unwrap());

    let nav = ObjectSpans::collect_for_navigation(layers, &text, ObjectKind::Parameter);
    let span = nav
        .enclosing(pos)
        .expect("navigation span at the first argument");
    assert_eq!(span_text(&text, span), "a: i32");
}

/// A query defining only `@parameter.around` (no `@parameter.inside`) must
/// still fall back to it for navigation, not yield nothing — Parameter's
/// priority is `Inside` first, but `Movement`/`Around` stay as fallbacks
/// rather than being dropped entirely when `Inside` is undefined.
#[test]
fn collect_for_navigation_parameter_falls_back_to_around_without_inside() {
    let bundle = crate::test_support::make_bundle(
        "rust",
        "tree_sitter_rust",
        "",
        None,
        Some("(parameters (_) @parameter.around)"),
    );
    let source = "fn add(a: i32, b: i32) -> i32 { a + b }\n";
    let text = BufferText::from(source);
    let syn = Syntax::attach_sync(bundle, &text, &empty_langs());
    let layers = syn.layers().expect("layers installed");
    let pos = text.byte_to_char(source.find("a: i32").unwrap());

    let nav = ObjectSpans::collect_for_navigation(layers, &text, ObjectKind::Parameter);
    assert!(
        nav.enclosing(pos).is_some(),
        "an around-only query must still produce a navigable span"
    );
}

// Pins a tree-sitter query-engine behavior, not a HUME one: it discards a
// quantified pattern's sub-match whose captures are a subset of a longer
// match already in progress. `(line_comment)+ @comment.around` therefore
// matches once, capturing every consecutive line comment under
// `@comment.around` — not once per line. If this ever contradicts, HARD
// STOP: do not patch it with a same-end dedup, which would break legitimate
// same-end nesting in delimiter-less languages.
#[test]
fn comment_around_on_the_last_line_of_a_block_is_the_whole_block() {
    let source = "// line one\n// line two\n// line three\nfn foo() {}\n";
    let (syn, text) = rust_syntax(source);
    let layers = syn.layers().expect("layers installed");
    let pos = text.byte_to_char(source.find("line three").unwrap());

    let around = ObjectSpans::collect(layers, &text, ObjectKind::Comment, ObjectSpan::Around);
    let span = around
        .enclosing(pos)
        .expect("comment.around from the last line");
    let got = span_text(&text, span);
    assert!(
        got.starts_with("// line one"),
        "must include the first line: {got:?}"
    );
    assert!(
        got.contains("// line two"),
        "must include the middle line: {got:?}"
    );
    assert!(
        got.ends_with("// line three"),
        "must end at the last line: {got:?}"
    );
}

// The `#[test]` pattern's `(#eq? @_test_attribute "test")` predicate only
// evaluates when the query cursor has a text provider — this is the proof
// `collect_hulls` wires one in, not just an assertion on the resulting span.
#[test]
fn test_around_spans_attribute_and_body() {
    let source = "#[test]\nfn it_works() {\n    assert!(true);\n}\n";
    let (syn, text) = rust_syntax(source);
    let layers = syn.layers().expect("layers installed");
    let pos = text.byte_to_char(source.find("assert!").unwrap());

    let around = ObjectSpans::collect(layers, &text, ObjectKind::Test, ObjectSpan::Around);
    let span = around.enclosing(pos).expect("test.around at the body");
    let got = span_text(&text, span);
    assert!(
        got.starts_with("#[test]"),
        "must include the attribute: {got:?}"
    );
    assert!(
        got.trim_end().ends_with('}'),
        "must include the body: {got:?}"
    );
}

// No pattern captures `function_item`'s body as `class.*` — only
// struct/enum/union/trait/impl do — so a cursor inside a method's body must
// resolve `class.inside` to the enclosing `impl`'s body, not the method's
// own block.
#[test]
fn class_inside_in_an_impl_method_body_picks_the_impl_body() {
    let source = "impl Foo {\n    fn bar() {\n        1\n    }\n}\n";
    let (syn, text) = rust_syntax(source);
    let layers = syn.layers().expect("layers installed");
    let pos = text.byte_to_char(source.find('1').unwrap());

    let inside = ObjectSpans::collect(layers, &text, ObjectKind::Class, ObjectSpan::Inside);
    let span = inside
        .enclosing(pos)
        .expect("class.inside enclosing the method body");
    let got = span_text(&text, span);
    assert!(
        got.contains("fn bar()"),
        "must be the impl's whole body, not just the method's block: {got:?}"
    );
}

// A layer defining `function.movement` on the name node: navigation must
// pick that narrower span, selection (`.around`) must stay unaffected.
#[test]
fn movement_capture_priority_affects_navigation_not_selection() {
    require_grammars(&["rust"]);
    let query_src =
        "(function_item name: (identifier) @function.movement)\n(function_item) @function.around";
    let bundle = make_bundle("rust", "tree_sitter_rust", "", None, Some(query_src));
    let source = "fn foo() {\n    1\n}\n";
    let text = BufferText::from(source);
    let syn = Syntax::attach_sync(bundle, &text, &empty_langs());
    let layers = syn.layers().expect("layers installed");
    let pos = text.byte_to_char(source.find('1').unwrap());

    let nav = ObjectSpans::collect_for_navigation(layers, &text, ObjectKind::Function);
    assert_eq!(
        nav.spans.len(),
        1,
        "expected exactly one function: {:?}",
        nav.spans
    );
    assert_eq!(span_text(&text, nav.spans[0]), "foo");

    let select = ObjectSpans::collect(layers, &text, ObjectKind::Function, ObjectSpan::Around);
    let select_span = select
        .enclosing(pos)
        .expect("selection span covers the body");
    let got = span_text(&text, select_span);
    assert!(
        got.starts_with("fn foo"),
        "selection must stay the whole function: {got:?}"
    );
}

// Proves the merge across layers, not an innermost-layer walk: a cursor
// inside a fenced Rust function selects it via the injected layer's own
// query; a cursor in markdown prose finds nothing (markdown defines no
// `function` objects); a cursor inside the fence but outside any function
// also finds nothing (the injected layer's query ran, it just found no
// enclosing match there).
#[test]
fn collect_merges_spans_across_root_and_injected_layers() {
    let markdown = markdown_bundle_with_helix_injections();
    let rust = rust_bundle_with_real_textobjects();
    let mut langs: FxHashMap<String, Arc<GrammarBundle>> = FxHashMap::default();
    langs.insert("rust".to_owned(), Arc::clone(&rust));
    let langs = Arc::new(langs);

    let source = "prose here\n\n```rust\nuse std::fmt;\n\nfn foo() {\n    1\n}\n```\n";
    let text = BufferText::from(source);
    let syn = Syntax::attach_sync(markdown, &text, &langs);
    let layers = syn.layers().expect("layers installed");

    let select = ObjectSpans::collect(layers, &text, ObjectKind::Function, ObjectSpan::Around);

    let pos_in_function = text.byte_to_char(source.find('1').unwrap());
    let span = select
        .enclosing(pos_in_function)
        .expect("must find the fenced rust function");
    assert!(span_text(&text, span).contains("fn foo"));

    let pos_in_prose = text.byte_to_char(source.find("prose").unwrap());
    assert_eq!(
        select.enclosing(pos_in_prose),
        None,
        "prose is not a function"
    );

    let pos_in_fence_not_function = text.byte_to_char(source.find("use std").unwrap());
    assert_eq!(
        select.enclosing(pos_in_fence_not_function),
        None,
        "inside the fence but outside any function"
    );
}
