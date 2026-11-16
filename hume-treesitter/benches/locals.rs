use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use hume_core::{GrammarSource, OptionalQuery, QueryFiles};
use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::edit::TextChange;
use hume_editing::text::BufferText;
use hume_engine::providers::SyntaxSource;
use hume_engine::theme::ScopeRegistry;
use hume_rope::line::ContentLine;
use hume_rope::offset::CharOffset;
use hume_treesitter::registry::{GrammarBundle, GrammarSnapshot, LanguageRegistry};
use hume_treesitter::syntax::Syntax;
use test_fixtures::{
    grammar_parser_path, grammar_query_path, require_grammars, require_helix_query,
};

/// Functions of the benchmarked file, ten lines each.
const FUNCTIONS: usize = 500;

/// Lines highlighted per iteration, a screen.
const ROWS: usize = 50;

/// The rust fixture grammar, with its `locals.scm` when `locals`.
fn rust(locals: bool) -> Arc<GrammarBundle> {
    require_grammars(&["rust"]);
    let mut queries = QueryFiles::new(grammar_query_path("rust"));
    if locals {
        queries = queries.with(
            OptionalQuery::Locals,
            require_helix_query("rust", OptionalQuery::Locals.file_name()),
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

/// Selections typed at by the multi-cursor rows.
const CURSORS: usize = 200;

/// `text` with `inserted` at each of the ascending chars `at`, and the
/// change that makes it.
fn inserted(text: &BufferText, at: &[usize], inserted: &str) -> (BufferText, ChangeSet) {
    let mut builder = ChangeSetBuilder::new(text.end());
    for &at in at {
        builder.retain_to(CharOffset::new(at));
        builder.insert(inserted);
    }
    let change = builder.finish();
    let after = change.apply(text).expect("an insertion applies");
    (after, change)
}

/// `text` without the char at each of the ascending chars `at`, and the
/// change that makes it.
fn deleted(text: &BufferText, at: &[usize]) -> (BufferText, ChangeSet) {
    let mut builder = ChangeSetBuilder::new(text.end());
    for &at in at {
        builder.retain_to(CharOffset::new(at));
        builder.delete_to(CharOffset::new(at + 1));
    }
    let change = builder.finish();
    let after = change.apply(text).expect("a deletion applies");
    (after, change)
}

/// Reparse after typing one char at the first `needle` of the source, or
/// at `CURSORS` of them, then after deleting it, as the editor reparses
/// after each keystroke. Typed after `CURSORS` parameter names, the char
/// renames each parameter, so its uses stop resolving and every rebuilt
/// scope holds fewer resolved references than before.
fn bench_reparse(c: &mut Criterion) {
    let source = source();
    let langs = Arc::new(GrammarSnapshot::default());
    let middle = |needle: &str| {
        let half = source.len() / 2;
        vec![
            half + source[half..]
                .find(needle)
                .expect("the needle is in the source"),
        ]
    };
    let cursors: Vec<usize> = source
        .match_indices("    let delta")
        .map(|(at, _)| at)
        .take(CURSORS)
        .collect();
    let parameters: Vec<usize> = source
        .match_indices("(alpha:")
        .map(|(at, _)| at + "(alpha".len())
        .take(CURSORS)
        .collect();
    for (place, at, typed) in [
        ("in_a_function", middle("    let delta"), " "),
        ("at_the_top_level", middle("fn f"), "\n"),
        ("at_200_cursors", cursors, " "),
        ("at_200_parameter_names", parameters, "x"),
    ] {
        // Each typed char lands after those typed before it.
        let typed_at: Vec<usize> = at.iter().enumerate().map(|(i, at)| at + i).collect();
        for (name, locals) in [("without_locals", false), ("with_locals", true)] {
            let mut text = BufferText::from(source.as_str());
            let mut syntax = Syntax::attach_sync(rust(locals), &text, &langs);
            c.bench_function(&format!("reparse_after_a_keystroke_{place}_{name}"), |b| {
                b.iter(|| {
                    for edit in [true, false] {
                        let (after, change) = if edit {
                            inserted(&text, &at, typed)
                        } else {
                            deleted(&text, &typed_at)
                        };
                        syntax.record_edit(&TextChange::new(&text, &after, &change));
                        syntax.ensure_current(&after, &langs);
                        text = after;
                    }
                });
            });
        }
    }
}

criterion_group!(benches, bench_locals, bench_reparse);
criterion_main!(benches);
