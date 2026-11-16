use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use hume_core::{GrammarSource, OptionalQuery, QueryFiles};
use hume_editing::text::BufferText;
use hume_engine::providers::SyntaxSource;
use hume_engine::theme::ScopeRegistry;
use hume_rope::line::ContentLine;
use hume_treesitter::registry::{GrammarBundle, GrammarSnapshot, LanguageRegistry};
use hume_treesitter::syntax::Syntax;
use test_fixtures::{grammar_parser_path, grammar_query_path, require_grammars};

/// Functions of the benchmarked file, ten lines each.
const FUNCTIONS: usize = 500;

/// Lines highlighted per iteration, a screen.
const ROWS: usize = 50;

/// The rust fixture grammar, with its `locals.scm` when `locals`.
fn rust(locals: bool) -> Arc<GrammarBundle> {
    require_grammars(&["rust"]);
    let highlights = grammar_query_path("rust");
    let mut queries = QueryFiles::new(highlights.clone());
    if locals {
        queries = queries.with(
            OptionalQuery::Locals,
            highlights.with_file_name("locals.scm"),
        );
    }
    LanguageRegistry::for_testing()
        .attach_grammar(
            "rust",
            &GrammarSource {
                path: grammar_parser_path("rust"),
                symbol: "tree_sitter_rust".to_owned(),
                queries,
            },
            hume_core::ConfigOrigin::User,
            &mut ScopeRegistry::new(),
        )
        .expect("attach the grammar")
        .expect("no higher origin holds the grammar")
}

/// A file of `FUNCTIONS` functions, each with parameters, locals and uses.
fn source() -> String {
    (0..FUNCTIONS)
        .map(|i| {
            format!(
                "fn f{i}(alpha: u32, beta: u32) -> u32 {{\n    let gamma = alpha + beta;\n    let delta = gamma * alpha;\n    if delta > beta {{\n        return gamma;\n    }}\n    let epsilon = delta - gamma;\n    epsilon + alpha\n}}\n\n"
            )
        })
        .collect()
}

fn bench_locals(c: &mut Criterion) {
    let text = BufferText::from(source().as_str());
    let langs = Arc::new(GrammarSnapshot::default());
    for (name, locals) in [("without_locals", false), ("with_locals", true)] {
        let bundle = rust(locals);
        c.bench_function(&format!("parse_5k_rust_lines_{name}"), |b| {
            b.iter(|| Syntax::attach_sync(Arc::clone(&bundle), black_box(&text), &langs));
        });
        let syntax = Syntax::attach_sync(Arc::clone(&bundle), &text, &langs);
        let first = text.content_line_count().get() / 2;
        let mut out = Vec::new();
        c.bench_function(&format!("highlight_a_screen_{name}"), |b| {
            b.iter(|| {
                for line in first..first + ROWS {
                    out.clear();
                    syntax.spans_for_line(ContentLine::new(line), text.rope(), &mut out);
                }
            });
        });
    }
}

criterion_group!(benches, bench_locals);
criterion_main!(benches);
