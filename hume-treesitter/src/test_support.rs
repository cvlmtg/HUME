//! Test-only scaffolding shared across this crate's per-module test suites:
//! bundle builders, id/gen helpers, and range construction. Every caller
//! already calls `test_fixtures::require_grammars` before using these,
//! so none of them re-check fixture existence themselves.

use std::sync::{Arc, LazyLock, Mutex};

use hume_core::OptionalQuery;
use hume_editing::text::BufferText;
use hume_engine::theme::ScopeRegistry;

use crate::grammar::LoadedGrammar;
use crate::registry::{
    CompiledQueries, GrammarBundle, GrammarSnapshot, LanguageId, LanguageRegistry, compile_queries,
    next_config_gen,
};
use crate::syntax::Syntax;

/// The id `name` has across every test bundle, so two bundles built for one
/// name name the same language.
pub(crate) fn test_language_id(name: &str) -> LanguageId {
    static NAMES: LazyLock<Mutex<LanguageRegistry>> =
        LazyLock::new(|| Mutex::new(LanguageRegistry::for_testing()));
    NAMES.lock().unwrap_or_else(|e| e.into_inner()).intern(name)
}

/// Open `name`'s compiled grammar fixture.
pub(crate) fn open_grammar(name: &str, symbol: &str) -> LoadedGrammar {
    let path = test_fixtures::grammar_parser_path(name);
    LoadedGrammar::open(&path, symbol).expect("load grammar")
}

/// Build a `GrammarBundle` for `name`. `highlights_src` is compiled as the
/// highlight query; pass `""` when a test only needs a parse tree, not real
/// highlighting. Each `optional` source compiles as that kind's query.
pub(crate) fn make_bundle(
    name: &str,
    symbol: &str,
    highlights_src: &str,
    optional: &[(OptionalQuery, &str)],
) -> Arc<GrammarBundle> {
    let grammar = open_grammar(name, symbol);
    let CompiledQueries {
        highlighter,
        injections,
        textobjects,
        indents,
        context,
    } = compile_queries(
        grammar.language(),
        highlights_src,
        optional,
        &mut ScopeRegistry::new(),
    )
    .expect("compile queries");
    Arc::new(GrammarBundle {
        grammar,
        highlighter,
        injections,
        textobjects,
        indents,
        context,
        config_gen: next_config_gen(),
        language: test_language_id(name),
    })
}

/// The fetched Helix `kind` query for `language`.
pub(crate) fn fixture_query(language: &str, kind: OptionalQuery) -> String {
    let path = test_fixtures::require_helix_query(language, kind.file_name());
    std::fs::read_to_string(path).expect("read fixture query")
}

/// The `name` bundle carrying the fetched Helix `kind` query and no other.
pub(crate) fn bundle_with_fixture_query(
    name: &str,
    symbol: &str,
    kind: OptionalQuery,
) -> Arc<GrammarBundle> {
    test_fixtures::require_grammars(&[name]);
    make_bundle(name, symbol, "", &[(kind, &fixture_query(name, kind))])
}

/// The markdown grammar fixture with its highlights and injections queries.
pub(crate) fn fixture_markdown() -> Arc<GrammarBundle> {
    let highlights = test_fixtures::grammar_query_path("markdown");
    let injections =
        test_fixtures::require_helix_query("markdown", OptionalQuery::Injections.file_name());
    let read = |path: &std::path::Path| std::fs::read_to_string(path).expect("read markdown query");
    make_bundle(
        "markdown",
        "tree_sitter_markdown",
        &read(&highlights),
        &[(OptionalQuery::Injections, &read(&injections))],
    )
}

/// A snapshot with no grammars, for tests with no injections to resolve.
pub(crate) fn empty_langs() -> Arc<GrammarSnapshot> {
    Arc::new(GrammarSnapshot::default())
}

/// Parse `source` with `bundle`, no injections resolved. Returns the
/// `Syntax` rather than its layers: `layers()` borrows from it, so the caller
/// holds it.
pub(crate) fn syntax_with(bundle: Arc<GrammarBundle>, source: &str) -> (Syntax, BufferText) {
    let text = BufferText::from(source);
    let syn = Syntax::attach_sync(bundle, &text, &empty_langs());
    (syn, text)
}

/// Build a `tree_sitter::Range` from byte offsets, synthesizing single-line
/// row/column points (`column == byte offset`), good enough for tests that
/// only assert on byte ranges.
pub(crate) fn range(start: usize, end: usize) -> tree_sitter::Range {
    tree_sitter::Range {
        start_byte: start,
        end_byte: end,
        start_point: tree_sitter::Point {
            row: 0,
            column: start,
        },
        end_point: tree_sitter::Point {
            row: 0,
            column: end,
        },
    }
}
