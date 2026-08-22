//! Test-only scaffolding shared across this crate's per-module test suites:
//! bundle builders, id/gen helpers, and range construction. Every caller
//! already gates on `hume_test_fixtures::skip_unless_grammars` before using
//! these, so none of them re-check fixture existence themselves.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use rustc_hash::FxHashMap;
use slotmap::SlotMap;

use hume_engine::pipeline::BufferId;
use hume_engine::theme::ScopeRegistry;

use crate::grammar::LoadedGrammar;
use crate::highlight::TreeSitterHighlighter;
use crate::injections::InjectionsQuery;
use crate::registry::GrammarBundle;

/// Distinct per call, mirroring `LanguageRegistry`'s `config_gen` invariant
/// so tests that compare bundles by gen see real identity.
pub(crate) fn next_test_config_gen() -> u32 {
    static NEXT_GEN: AtomicU32 = AtomicU32::new(0);
    NEXT_GEN.fetch_add(1, Ordering::Relaxed)
}

/// Open `name`'s compiled grammar fixture.
pub(crate) fn open_grammar(name: &str, symbol: &str) -> LoadedGrammar {
    let path = hume_test_fixtures::grammar_parser_path(name);
    LoadedGrammar::open(&path, symbol).expect("load grammar")
}

/// Build a `GrammarBundle` for `name`. `highlights_src` is compiled as the
/// highlight query — pass `""` when a test only needs a parse tree, not real
/// highlighting. `injections_src`, if given, compiles as the grammar's
/// `injections.scm` query.
pub(crate) fn make_bundle(
    name: &str,
    symbol: &str,
    highlights_src: &str,
    injections_src: Option<&str>,
) -> Arc<GrammarBundle> {
    let grammar = open_grammar(name, symbol);
    let query = Arc::new(
        tree_sitter::Query::new(grammar.language(), highlights_src).expect("compile highlights"),
    );
    let mut registry = ScopeRegistry::new();
    let highlighter = Arc::new(TreeSitterHighlighter::from_shared_query(
        query,
        &mut registry,
    ));
    let injections = injections_src.map(|src| {
        let q =
            Arc::new(tree_sitter::Query::new(grammar.language(), src).expect("compile injections"));
        InjectionsQuery::new(q)
    });
    Arc::new(GrammarBundle {
        grammar,
        highlighter,
        injections,
        config_gen: next_test_config_gen(),
    })
}

/// A fresh, never-before-seen `BufferId`.
pub(crate) fn fresh_bid() -> BufferId {
    let mut sm: SlotMap<BufferId, ()> = SlotMap::with_key();
    sm.insert(())
}

/// An empty grammar-name → bundle map, for tests with no injections to resolve.
pub(crate) fn empty_langs() -> Arc<FxHashMap<String, Arc<GrammarBundle>>> {
    Arc::new(FxHashMap::default())
}

/// Build a `tree_sitter::Range` from byte offsets, synthesizing single-line
/// row/column points (`column == byte offset`) — good enough for tests that
/// only assert on byte ranges.
pub(crate) fn range(start: usize, end: usize) -> tree_sitter::Range {
    tree_sitter::Range {
        start_byte: start,
        end_byte: end,
        start_point: tree_sitter::Point {
            row: 0,
            column: start, // column-name-safe: tree-sitter's Point::column is a byte offset
        },
        end_point: tree_sitter::Point {
            row: 0,
            column: end, // column-name-safe: tree-sitter's Point::column is a byte offset
        },
    }
}
