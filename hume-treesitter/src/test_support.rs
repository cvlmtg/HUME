//! Test-only scaffolding shared across this crate's per-module test suites:
//! bundle builders, id/gen helpers, and range construction. Every caller
//! already calls `test_fixtures::require_grammars` before using these,
//! so none of them re-check fixture existence themselves.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use rustc_hash::FxHashMap;
use slotmap::SlotMap;

use hume_engine::pipeline::BufferId;
use hume_engine::theme::ScopeRegistry;

use crate::grammar::LoadedGrammar;
use crate::registry::{
    CompiledQueries, GrammarBundle, LanguageId, LanguageRegistry, OptionalQuery, compile_queries,
};

/// Distinct per call, mirroring `LanguageRegistry`'s `config_gen` invariant
/// so tests that compare bundles by gen see real identity.
pub(crate) fn next_test_config_gen() -> u32 {
    static NEXT_GEN: AtomicU32 = AtomicU32::new(0);
    NEXT_GEN.fetch_add(1, Ordering::Relaxed)
}

/// The id `name` has across every test bundle, so two bundles built for one
/// name name the same language.
pub(crate) fn test_language_id(name: &str) -> LanguageId {
    static NAMES: LazyLock<Mutex<LanguageRegistry>> =
        LazyLock::new(|| Mutex::new(LanguageRegistry::new()));
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
        config_gen: next_test_config_gen(),
        language: test_language_id(name),
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
