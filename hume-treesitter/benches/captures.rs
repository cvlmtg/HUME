use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use hume_core::{GrammarSource, OptionalQuery, QueryFiles};
use hume_editing::text::BufferText;
use hume_engine::theme::ScopeRegistry;
use hume_rope::line::ContentLine;
use hume_rope::offset::InclusiveRange;
use hume_treesitter::registry::{GrammarBundle, GrammarSnapshot, LanguageRegistry};
use hume_treesitter::syntax::Syntax;
use test_fixtures::{
    grammar_parser_path, grammar_query_path, require_grammars, require_helix_query,
};

/// Paragraphs of the benchmarked document, each its own `markdown.inline`
/// layer and two lines long with its blank separator.
const PARAGRAPHS: usize = 2_500;

/// Lines of the queried range, as many as a screen of the plugin's draw.
const ROWS: usize = 50;

/// Markdown and `markdown.inline` attached to one registry.
fn bundles() -> (Arc<GrammarBundle>, Arc<GrammarBundle>) {
    require_grammars(&["markdown", "markdown.inline"]);
    let mut registry = LanguageRegistry::for_testing();
    let mut attach = |name: &str, symbol: &str, queries: QueryFiles| {
        registry
            .attach_grammar(
                name,
                &GrammarSource {
                    path: grammar_parser_path(name),
                    symbol: symbol.to_owned(),
                    queries,
                },
                hume_core::ConfigOrigin::User,
                &mut ScopeRegistry::new(),
            )
            .expect("attach the grammar")
            .expect("no higher origin holds the grammar")
    };
    let markdown = attach(
        "markdown",
        "tree_sitter_markdown",
        QueryFiles::new(grammar_query_path("markdown")).with(
            OptionalQuery::Injections,
            require_helix_query("markdown", OptionalQuery::Injections.file_name()),
        ),
    );
    let inline = attach(
        "markdown.inline",
        "tree_sitter_markdown_inline",
        QueryFiles::new(grammar_query_path("markdown.inline")),
    );
    (markdown, inline)
}

/// The emphasis nodes of the inline layers over a screen of a document with
/// one inline layer per paragraph.
fn bench_captures_in_a_range(c: &mut Criterion) {
    let (markdown, inline) = bundles();
    let langs = Arc::new(GrammarSnapshot::from_iter([(
        "markdown.inline".to_owned(),
        Arc::clone(&inline),
    )]));
    let source: String = (0..PARAGRAPHS)
        .map(|i| format!("paragraph {i} with *emphasis* in it\n\n"))
        .collect();
    let text = BufferText::from(source.as_str());
    let syntax = Syntax::attach_sync(markdown, &text, &langs);
    let layers = syntax.layers().expect("a parsed tree");
    let query = inline
        .capture_query("(emphasis) @e")
        .expect("the query compiles");

    let first = ContentLine::new(PARAGRAPHS);
    let range = InclusiveRange::new(first, first.advance(ROWS - 1));
    let mut out = Vec::new();
    query.collect(layers, text.rope(), range, &mut out);
    assert_eq!(
        out.len(),
        ROWS / 2,
        "one emphasis for each paragraph the range holds"
    );

    c.bench_function("captures_inline_range", |b| {
        b.iter(|| query.collect(layers, text.rope(), black_box(range), &mut out));
    });
}

criterion_group!(benches, bench_captures_in_a_range);
criterion_main!(benches);
