use std::hint::black_box;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use hume_core::{GrammarSource, OptionalQuery, QueryFiles};
use hume_editing::changeset::ChangeSetBuilder;
use hume_editing::edit::TextChange;
use hume_editing::indent::ElectricTrigger;
use hume_editing::text::BufferText;
use hume_editing::word::WordChars;
use hume_engine::theme::ScopeRegistry;
use hume_rope::offset::CharOffset;
use hume_treesitter::indent::{electric_indent, indent_for_new_line};
use hume_treesitter::registry::{GrammarBundle, GrammarSnapshot, LanguageRegistry};
use hume_treesitter::syntax::Syntax;
use test_fixtures::{
    grammar_parser_path, grammar_query_path, require_grammars, require_helix_indent_corpus,
    require_helix_query,
};

/// Lines the benchmarked buffer has at least.
const LINES: usize = 8_000;

/// A split before `y` puts the closure's parameters on two lines, so the
/// new line aligns under `x`; its own line and the three above it start
/// outside any alignment, so every baseline attempt mismatches.
const WORST_CASE: &str =
    "fn worst_case() {\n    a();\n    b();\n    c();\n    let f = |x, y| x;\n}\n";

/// Every bundle the benchmarks use, attached to one registry so that no
/// two of them share a grammar identity.
struct Grammars {
    /// Rust with the fixture indents query.
    rust: Arc<GrammarBundle>,
    /// [`Grammars::rust`] plus the fixture rust injections query, which injects
    /// rust into macro bodies.
    rust_injecting: Arc<GrammarBundle>,
    /// Markdown with the fixture injections query.
    markdown: Arc<GrammarBundle>,
    markdown_inline: Arc<GrammarBundle>,
}

static GRAMMARS: LazyLock<Grammars> = LazyLock::new(|| {
    require_grammars(&["rust", "markdown", "markdown.inline"]);
    let mut registry = LanguageRegistry::new();
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
    let rust_queries = QueryFiles::new(grammar_query_path("rust")).with(
        OptionalQuery::Indents,
        require_helix_query("rust", OptionalQuery::Indents.file_name()),
    );
    Grammars {
        rust: attach("rust", "tree_sitter_rust", rust_queries.clone()),
        rust_injecting: attach(
            "rust",
            "tree_sitter_rust",
            rust_queries.with(
                OptionalQuery::Injections,
                require_helix_query("rust", OptionalQuery::Injections.file_name()),
            ),
        ),
        markdown: attach(
            "markdown",
            "tree_sitter_markdown",
            QueryFiles::new(grammar_query_path("markdown")).with(
                OptionalQuery::Injections,
                require_helix_query("markdown", OptionalQuery::Injections.file_name()),
            ),
        ),
        markdown_inline: attach(
            "markdown.inline",
            "tree_sitter_markdown_inline",
            QueryFiles::new(grammar_query_path("markdown.inline")),
        ),
    }
});

type Langs = Arc<GrammarSnapshot>;

fn no_langs() -> Langs {
    Arc::new(GrammarSnapshot::default())
}

fn rust_corpus() -> String {
    std::fs::read_to_string(require_helix_indent_corpus("rust", "rust.rs"))
        .expect("read the indent corpus")
}

/// Copies of the rust indent corpus up to [`LINES`] lines, then
/// [`WORST_CASE`], and the char offset of its split.
fn worst_case_text() -> (BufferText, CharOffset) {
    let corpus = rust_corpus();
    let copies = LINES.div_ceil(corpus.lines().count());
    let mut source = corpus.repeat(copies);
    let split = source.chars().count() + WORST_CASE.find("y|").expect("split marker");
    source.push_str(WORST_CASE);
    (BufferText::from(source.as_str()), CharOffset::new(split))
}

/// The worst-case hybrid: the target and four mismatching baselines.
fn bench_worst_case_hybrid(c: &mut Criterion) {
    let (text, split) = worst_case_text();
    let syntax = Syntax::attach_sync(Arc::clone(&GRAMMARS.rust), &text, &no_langs());
    let layers = syntax.layers().expect("the text parses");
    let split = text.snap(split);
    assert_eq!(
        indent_for_new_line(layers, &text, split, 4).map(|indent| indent.width.get()),
        Some(13),
        "the new line falls back to its absolute width, aligned under `x`"
    );
    c.bench_function("indent_worst_case_hybrid", |b| {
        b.iter(|| black_box(indent_for_new_line(layers, &text, split, 4)));
    });
}

/// A one-char edit at the buffer start, the synchronous reparse, then the
/// worst-case hybrid. Iterations alternate inserting and deleting the char.
fn bench_edit_reparse_hybrid(c: &mut Criterion) {
    let langs = no_langs();
    let (mut text, split) = worst_case_text();
    let mut syntax = Syntax::attach_sync(Arc::clone(&GRAMMARS.rust), &text, &langs);
    let mut padded = false;
    c.bench_function("indent_edit_reparse_hybrid", |b| {
        b.iter(|| {
            let mut builder = ChangeSetBuilder::new(text.end());
            if padded {
                builder.delete_to(CharOffset::new(1));
            } else {
                builder.insert(" ");
            }
            let change = builder.finish();
            let next = change.apply(&text).expect("a one-char edit applies");
            syntax.record_edit(&TextChange::new(&text, &next, &change));
            text = next;
            padded = !padded;
            black_box(syntax.ensure_current(&text, &langs));
            let split = text.snap(split.shift(isize::from(padded)));
            let layers = syntax.layers().expect("the text parses");
            black_box(indent_for_new_line(layers, &text, split, 4))
        });
    });
}

/// A function whose body ends with a fresh line, indented one level
/// deeper than the closing brace typed on it ends up.
const FRESH_LINE: &str = "fn electric() {\n    a();\n    \n}\n";

/// `prefix`, [`FRESH_LINE`], then `suffix`, and the char offset of the
/// fresh line's indent end.
fn with_fresh_line(prefix: &str, suffix: &str) -> (BufferText, CharOffset) {
    let at = prefix.chars().count() + FRESH_LINE.find("    \n}").expect("fresh line") + 4;
    (
        BufferText::from(format!("{prefix}{FRESH_LINE}{suffix}").as_str()),
        CharOffset::new(at),
    )
}

/// What a timed keystroke covers: typing `}`, recording the edit and the
/// synchronous reparse, then, with `ElectricIndent`, the dedent the editor
/// computes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Keystroke {
    Reparse,
    ElectricIndent,
}

/// One keystroke on the fresh line, typed at `at` and timed as `measured`
/// says. Only the keystroke is timed; deleting the `}` and reparsing again
/// restore the text between iterations.
fn bench_keystroke(
    c: &mut Criterion,
    name: &str,
    measured: Keystroke,
    bundle: &Arc<GrammarBundle>,
    langs: &Langs,
    (mut text, at): (BufferText, CharOffset),
) {
    let mut syntax = Syntax::attach_sync(Arc::clone(bundle), &text, langs);
    let edit = |syntax: &mut Syntax, text: &mut BufferText, brace: bool| {
        let mut builder = ChangeSetBuilder::new(text.end());
        builder.retain_to(at);
        if brace {
            builder.insert("}");
        } else {
            builder.delete_to(at.shift(1));
        }
        let change = builder.finish();
        let next = change.apply(text).expect("a one-char edit applies");
        syntax.record_edit(&TextChange::new(text, &next, &change));
        *text = next;
        black_box(syntax.ensure_current(text, langs));
    };
    let dedent = |syntax: &Syntax, text: &BufferText| {
        let line = text.char_to_line(at);
        let cursor = text.snap(at.shift(1));
        let layers = syntax.layers().expect("the text parses");
        electric_indent(
            layers,
            text,
            line,
            cursor,
            ElectricTrigger::Typed,
            WordChars::default(),
            4,
        )
    };
    edit(&mut syntax, &mut text, true);
    assert_eq!(
        dedent(&syntax, &text).map(|width| width.get()),
        Some(0),
        "the closing brace dedents the fresh line"
    );
    edit(&mut syntax, &mut text, false);
    c.bench_function(name, |b| {
        b.iter_custom(|iters| {
            let mut timed = Duration::ZERO;
            for _ in 0..iters {
                let start = Instant::now();
                edit(&mut syntax, &mut text, true);
                if measured == Keystroke::ElectricIndent {
                    black_box(dedent(&syntax, &text));
                }
                timed += start.elapsed();
                edit(&mut syntax, &mut text, false);
            }
            timed
        });
    });
}

/// A fresh line at the end of a [`LINES`]-line rust buffer.
fn rust_buffer() -> (BufferText, CharOffset) {
    let corpus = rust_corpus();
    with_fresh_line(&corpus.repeat(LINES.div_ceil(corpus.lines().count())), "")
}

fn bench_electric_keystroke(c: &mut Criterion) {
    bench_keystroke(
        c,
        "indent_electric_keystroke",
        Keystroke::ElectricIndent,
        &GRAMMARS.rust,
        &no_langs(),
        rust_buffer(),
    );
}

fn bench_reparse_keystroke_rust(c: &mut Criterion) {
    bench_keystroke(
        c,
        "syntax_reparse_keystroke_rust",
        Keystroke::Reparse,
        &GRAMMARS.rust,
        &no_langs(),
        rust_buffer(),
    );
}

/// The rust buffer with rust injected into its macro bodies.
fn bench_reparse_keystroke_rust_nested(c: &mut Criterion) {
    let rust = &GRAMMARS.rust_injecting;
    bench_keystroke(
        c,
        "syntax_reparse_keystroke_rust_nested",
        Keystroke::Reparse,
        rust,
        &Arc::new(GrammarSnapshot::from_iter([(
            "rust".to_owned(),
            Arc::clone(rust),
        )])),
        rust_buffer(),
    );
}

/// Fenced rust blocks the benchmarked markdown buffer holds.
const FENCES: usize = 30;

/// A fresh line in the last of [`FENCES`] fenced rust blocks of a markdown
/// buffer, each a copy of the rust corpus: the reparse brings every
/// injected layer up to date.
fn markdown_buffer() -> (BufferText, CharOffset) {
    let fence = format!("Some *prose*.\n\n```rust\n{}```\n\n", rust_corpus());
    let prefix = format!("{}```rust\n", fence.repeat(FENCES - 1));
    with_fresh_line(&prefix, "```\n")
}

/// The injected languages of the markdown buffer, with `rust` as the
/// fenced blocks' grammar.
fn markdown_langs(rust: &Arc<GrammarBundle>) -> Langs {
    Arc::new(GrammarSnapshot::from_iter([
        ("rust".to_owned(), Arc::clone(rust)),
        (
            "markdown.inline".to_owned(),
            Arc::clone(&GRAMMARS.markdown_inline),
        ),
    ]))
}

fn bench_electric_keystroke_in_markdown(c: &mut Criterion) {
    bench_keystroke(
        c,
        "indent_electric_keystroke_markdown",
        Keystroke::ElectricIndent,
        &GRAMMARS.markdown,
        &markdown_langs(&GRAMMARS.rust),
        markdown_buffer(),
    );
}

fn bench_reparse_keystroke_markdown(c: &mut Criterion) {
    bench_keystroke(
        c,
        "syntax_reparse_keystroke_markdown",
        Keystroke::Reparse,
        &GRAMMARS.markdown,
        &markdown_langs(&GRAMMARS.rust),
        markdown_buffer(),
    );
}

/// The keystroke and its dedent in the markdown buffer, with rust also
/// injected into the fenced blocks' macro bodies, as an installed rust
/// grammar does.
fn bench_electric_keystroke_in_markdown_nested(c: &mut Criterion) {
    bench_keystroke(
        c,
        "indent_electric_keystroke_markdown_nested",
        Keystroke::ElectricIndent,
        &GRAMMARS.markdown,
        &markdown_langs(&GRAMMARS.rust_injecting),
        markdown_buffer(),
    );
}

/// The keystroke's reparse alone in the buffer of
/// [`bench_electric_keystroke_in_markdown_nested`].
fn bench_reparse_keystroke_markdown_nested(c: &mut Criterion) {
    bench_keystroke(
        c,
        "syntax_reparse_keystroke_markdown_nested",
        Keystroke::Reparse,
        &GRAMMARS.markdown,
        &markdown_langs(&GRAMMARS.rust_injecting),
        markdown_buffer(),
    );
}

criterion_group!(
    benches,
    bench_worst_case_hybrid,
    bench_edit_reparse_hybrid,
    bench_electric_keystroke,
    bench_electric_keystroke_in_markdown,
    bench_electric_keystroke_in_markdown_nested,
    bench_reparse_keystroke_markdown,
    bench_reparse_keystroke_markdown_nested,
    bench_reparse_keystroke_rust,
    bench_reparse_keystroke_rust_nested
);
criterion_main!(benches);
