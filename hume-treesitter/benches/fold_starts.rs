use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use hume_core::{GrammarSource, OptionalQuery, QueryFiles};
use hume_editing::text::BufferText;
use hume_engine::builtins::fold_column::FoldStarts;
use hume_engine::folds::HiddenLines;
use hume_engine::providers::SyntaxSource;
use hume_engine::theme::ScopeRegistry;
use hume_rope::line::ContentLine;
use hume_rope::offset::InclusiveRange;
use hume_treesitter::registry::{GrammarBundle, GrammarSnapshot, LanguageRegistry};
use hume_treesitter::syntax::Syntax;
use test_fixtures::{
    grammar_parser_path, grammar_query_path, require_grammars, require_helix_query,
};

/// Methods in the benchmarked `impl`.
const METHODS: usize = 200;

/// Rows of one pane: the lines the fold column marks per frame.
const ROWS: usize = 50;

/// An `impl` of [`METHODS`] methods, each a loop of statements, and the
/// first line of the middle method.
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
    let first = format!("    fn m{}(&self) {{", METHODS / 2);
    let line = source
        .lines()
        .position(|l| l == first)
        .expect("the middle method");
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

/// A pane's worth of region starts, read in one query, from the regions of
/// `context.scm` and of the textobjects fallback of a grammar without one.
/// Then the same pane with every method's fold closed, where the render
/// pass reads one query per run of shown lines.
fn bench_fold_starts(c: &mut Criterion) {
    require_grammars(&["rust"]);
    let (source, first) = nested_source();
    let text = BufferText::from(source.as_str());
    let langs = Arc::new(GrammarSnapshot::default());
    let mut registry = LanguageRegistry::for_testing();
    for (name, kind) in [
        ("context", OptionalQuery::Context),
        ("textobjects", OptionalQuery::TextObjects),
    ] {
        let syntax = Syntax::attach_sync(bundle(&mut registry, kind), &text, &langs);
        let pane = InclusiveRange::new(first, first.advance(ROWS - 1));
        let mut out = Vec::new();
        syntax.fold_starts(pane, text.rope(), &mut out);
        assert!(
            out.contains(&first),
            "{name}: a region starts on the method's first line"
        );
        c.bench_function(&format!("fold_starts_pane_{name}"), |b| {
            b.iter(|| {
                out.clear();
                syntax.fold_starts(black_box(pane), text.rope(), &mut out);
            });
        });

        // Every region closed but the `impl`'s, as close-all and then
        // opening the outer fold leaves it.
        let rope = text.rope();
        let closed = syntax
            .layers()
            .expect("a parsed tree")
            .all_folds(rope)
            .into_iter()
            .filter(|fold| fold.first() != ContentLine::new(0));
        let hidden = HiddenLines::from_folds(closed);
        let drawn = InclusiveRange::new(
            first,
            hidden
                .line_after_shown(first, ROWS)
                .min(text.last_content_line()),
        );
        assert!(
            hidden.closed_at(first),
            "{name}: the method's fold is closed"
        );
        // Each run of shown lines ends on a closed method's first line.
        assert!(
            hidden.shown_runs(drawn).count() >= ROWS / 4,
            "{name}: the closed pane reads one run per method"
        );
        let mut starts = FoldStarts::default();
        c.bench_function(&format!("fold_starts_pane_closed_{name}"), |b| {
            b.iter(|| {
                starts.reset(hidden.clone());
                starts.read([black_box(drawn)], &syntax, rope);
            });
        });
    }
}

criterion_group!(benches, bench_fold_starts);
criterion_main!(benches);
