use std::sync::Arc;

use hume_core::{BufferId, OptionalQuery};
use hume_editing::text::BufferText;
use hume_engine::providers::SyntaxSource;
use hume_rope::line::ContentLine;
use test_fixtures::require_grammars;

use super::*;
use crate::registry::{GrammarBundle, GrammarSnapshot};
use crate::syntax::Syntax;
use crate::test_support::{
    bundle_with_fixture_query, empty_langs, fixture_query, make_bundle, open_grammar, syntax_with,
};
use crate::textobjects::{ObjectKind, ObjectSpan, ObjectSpans, SpanSelector};

fn compile(source: &str) -> ContextQuery {
    require_grammars(&["rust"]);
    let grammar = open_grammar("rust", "tree_sitter_rust");
    ContextQuery::new(tree_sitter::Query::new(grammar.language(), source).expect("compile query"))
}

/// Rust with the fixture `kind` query and no other optional query.
fn rust_bundle(kind: OptionalQuery) -> Arc<GrammarBundle> {
    bundle_with_fixture_query("rust", "tree_sitter_rust", kind)
}

/// The index of `source`'s first line starting with `prefix`.
fn line_of(source: &str, prefix: &str) -> ContentLine {
    ContentLine::new(
        source
            .lines()
            .position(|l| l.starts_with(prefix))
            .expect("a line with that prefix"),
    )
}

fn lines(source: &str, prefixes: &[&str]) -> Vec<ContentLine> {
    prefixes.iter().map(|p| line_of(source, p)).collect()
}

fn headers(syn: &Syntax, text: &BufferText, line: ContentLine) -> Vec<ContentLine> {
    let mut out = Vec::new();
    syn.header_lines(line, text.rope(), &mut out);
    out
}

/// An `impl` holding a `fn` with a long body.
fn impl_with_long_fn() -> String {
    let lets: String = (0..20)
        .map(|i| format!("        let a{i} = {i};\n"))
        .collect();
    format!("impl Foo {{\n    fn bar(&self) {{\n{lets}    }}\n}}\n")
}

#[test]
fn context_query_resolves_its_captures() {
    let fixture = compile(&fixture_query("rust", OptionalQuery::Context));
    assert!(fixture.context.is_some());
    assert!(fixture.start.is_none());

    let with_start = compile("(function_item name: (_) @context.start) @context");
    assert!(with_start.context.is_some());
    assert!(with_start.start.is_some());
}

#[test]
fn impl_and_fn_headers_for_a_line_deep_in_a_body() {
    let source = impl_with_long_fn();
    let (syn, text) = syntax_with(rust_bundle(OptionalQuery::Context), &source);
    let last_let = line_of(&source, "        let a19");
    assert_eq!(
        headers(&syn, &text, last_let),
        lines(&source, &["impl Foo", "    fn bar"])
    );
}

#[test]
fn header_on_the_line_itself_is_not_a_scope() {
    let source = impl_with_long_fn();
    let (syn, text) = syntax_with(rust_bundle(OptionalQuery::Context), &source);
    assert_eq!(
        headers(&syn, &text, line_of(&source, "    fn bar")),
        lines(&source, &["impl Foo"])
    );
}

#[test]
fn closing_brace_line_is_outside_its_scope() {
    let source = impl_with_long_fn();
    let (syn, text) = syntax_with(rust_bundle(OptionalQuery::Context), &source);
    assert_eq!(
        headers(&syn, &text, line_of(&source, "    }")),
        lines(&source, &["impl Foo"])
    );

    let source = "#[test]\nfn t() {\n    a();\n    b();\n}\n";
    let (syn, text) = syntax_with(rust_bundle(OptionalQuery::TextObjects), source);
    assert_eq!(headers(&syn, &text, line_of(source, "}")), Vec::new());
}

#[test]
fn else_branch_lists_fn_if_and_else() {
    let source = "fn f(x: bool) {\n    if x {\n        a();\n    } else {\n        b();\n        c();\n    }\n}\n";
    let (syn, text) = syntax_with(rust_bundle(OptionalQuery::Context), source);
    assert_eq!(
        headers(&syn, &text, line_of(source, "        c();")),
        lines(source, &["fn f", "    if x", "    } else"])
    );
}

#[test]
fn multi_line_signature_yields_its_first_line_only() {
    let source = "fn f(\n    a: u32,\n    b: u32,\n) {\n    body();\n    more();\n}\n";
    let (syn, text) = syntax_with(rust_bundle(OptionalQuery::Context), source);
    assert_eq!(
        headers(&syn, &text, line_of(source, "    more();")),
        lines(source, &["fn f("])
    );
}

#[test]
fn two_scopes_starting_on_one_line_merge() {
    let source = "fn main() {\n    foo(bar(\n        1,\n        2,\n    ));\n}\n";
    let (syn, text) = syntax_with(rust_bundle(OptionalQuery::Context), source);
    assert_eq!(
        headers(&syn, &text, line_of(source, "        2,")),
        lines(source, &["fn main", "    foo(bar("])
    );
}

#[test]
fn injected_layer_adds_its_own_headers() {
    require_grammars(&["markdown", "rust"]);
    let injections = fixture_query("markdown", OptionalQuery::Injections);
    let context = fixture_query("markdown", OptionalQuery::Context);
    let markdown = make_bundle(
        "markdown",
        "tree_sitter_markdown",
        "",
        &[
            (OptionalQuery::Injections, &injections),
            (OptionalQuery::Context, &context),
        ],
    );
    let langs = Arc::new(GrammarSnapshot::from_iter([(
        "rust".to_owned(),
        rust_bundle(OptionalQuery::Context),
    )]));
    let source = "## Usage\n\n```rust\nfn demo() {\n    let x = 1;\n    let y = 2;\n}\n```\n";
    let text = BufferText::from(source);
    let syn = Syntax::attach_sync(markdown, &text, &langs);
    assert_eq!(
        headers(&syn, &text, line_of(source, "    let y")),
        lines(source, &["## Usage", "fn demo"])
    );
}

#[test]
fn fallback_header_is_the_item_not_its_attribute() {
    let source = "#[test]\nfn t() {\n    a();\n    b();\n}\n";
    let (syn, text) = syntax_with(rust_bundle(OptionalQuery::TextObjects), source);
    assert_eq!(
        headers(&syn, &text, line_of(source, "    b();")),
        lines(source, &["fn t"])
    );
}

#[test]
fn fallback_leaves_the_textobject_memo_alone() {
    let source = "#[test]\nfn t() {\n    a();\n    b();\n}\n";
    let (syn, text) = syntax_with(rust_bundle(OptionalQuery::TextObjects), source);
    let layers = syn.layers().expect("layers installed");
    let selector = SpanSelector::Exact(ObjectKind::Function, ObjectSpan::Inside);
    let spans = ObjectSpans::for_selector(layers, &text, selector);
    for _ in 0..3 {
        headers(&syn, &text, line_of(source, "    b();"));
    }
    let memo = layers.textobject_memo.lock().expect("memo lock");
    let (cached, kept) = memo.as_ref().expect("the memo holds the collected spans");
    assert_eq!(*cached, selector);
    assert!(Arc::ptr_eq(kept, &spans));
}

#[test]
fn no_layers_yields_nothing() {
    let source = impl_with_long_fn();
    let text = BufferText::from(source.as_str());
    let (syn, _request) = Syntax::attach(
        rust_bundle(OptionalQuery::Context),
        BufferId::default(),
        &text,
        &empty_langs(),
    );
    assert!(syn.layers().is_none());
    assert_eq!(
        headers(&syn, &text, line_of(&source, "        let a19")),
        Vec::new()
    );
}

#[test]
fn header_lines_replace_what_out_held() {
    let source = impl_with_long_fn();
    let stale = ContentLine::new(source.lines().count());
    let last_let = line_of(&source, "        let a19");

    let (syn, text) = syntax_with(rust_bundle(OptionalQuery::Context), &source);
    let mut out = vec![stale];
    syn.header_lines(last_let, text.rope(), &mut out);
    assert_eq!(out, lines(&source, &["impl Foo", "    fn bar"]));

    let (unparsed, _request) = Syntax::attach(
        rust_bundle(OptionalQuery::Context),
        BufferId::default(),
        &text,
        &empty_langs(),
    );
    let mut out = vec![stale];
    unparsed.header_lines(last_let, text.rope(), &mut out);
    assert_eq!(out, Vec::new());
}
