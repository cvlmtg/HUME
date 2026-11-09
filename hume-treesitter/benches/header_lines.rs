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
use test_fixtures::{
    grammar_parser_path, grammar_query_path, require_grammars, require_helix_query,
};

/// Methods in the benchmarked `impl`.
const METHODS: usize = 200;

/// An `impl` of [`METHODS`] methods, each a loop of statements, and a line
/// inside the middle method's loop: three scopes deep.
fn nested_source() -> (String, ContentLine) {
    let method = |i: usize| {
        let body: String = (0..20)
            .map(|j| format!("            let v{j} = x + {j};\n"))
            .collect();
        format!("    fn m{i}(&self) {{\n        for x in 0..{i} {{\n{body}        }}\n    }}\n\n")
    };
    let source = format!(
        "impl S {{\n{}}}\n",
        (0..METHODS).map(method).collect::<String>()
    );
    let deep = format!("    fn m{}(&self) {{", METHODS / 2);
    let line = source
        .lines()
        .position(|l| l == deep)
        .expect("the middle method")
        + 10;
    (source, ContentLine::new(line))
}

/// Rust with the fixture `kind` query as its only optional query.
fn bundle(registry: &mut LanguageRegistry, kind: OptionalQuery) -> Arc<GrammarBundle> {
    registry
        .attach_grammar(
            "rust",
            &GrammarSource {
                path: grammar_parser_path("rust"),
                symbol: "tree_sitter_rust".to_owned(),
                queries: QueryFiles::new(grammar_query_path("rust"))
                    .with(kind, require_helix_query("rust", kind.file_name())),
            },
            hume_core::ConfigOrigin::User,
            &mut ScopeRegistry::new(),
        )
        .expect("attach the grammar")
        .expect("no higher origin holds the grammar")
}

/// One line's header lookup, from `context.scm` and from the textobjects
/// fallback of a grammar without one.
fn bench_header_lines(c: &mut Criterion) {
    require_grammars(&["rust"]);
    let (source, line) = nested_source();
    let text = BufferText::from(source.as_str());
    let langs = Arc::new(GrammarSnapshot::default());
    let mut registry = LanguageRegistry::for_testing();
    for (name, kind) in [
        ("context", OptionalQuery::Context),
        ("textobjects", OptionalQuery::TextObjects),
    ] {
        let syntax = Syntax::attach_sync(bundle(&mut registry, kind), &text, &langs);
        let mut out = Vec::new();
        syntax.header_lines(line, text.rope(), &mut out);
        assert!(
            out.len() >= 2,
            "{name}: the line is inside the impl and a method"
        );
        c.bench_function(&format!("header_lines_{name}"), |b| {
            b.iter(|| syntax.header_lines(black_box(line), text.rope(), &mut out));
        });
    }
}

criterion_group!(benches, bench_header_lines);
criterion_main!(benches);
