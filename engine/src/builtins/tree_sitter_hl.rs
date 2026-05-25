use std::sync::{Arc, Mutex};

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::providers::{HighlightSource, HighlightTier, SourceContext};
use crate::theme::ScopeRegistry;
use crate::types::{Scope, ScopeId};

// ---------------------------------------------------------------------------
// TreeSitterHighlighter
// ---------------------------------------------------------------------------

/// Built-in highlight provider that drives tree-sitter highlight queries.
///
/// # Usage
///
/// 1. Create with `TreeSitterHighlighter::new(language, query_source)`.
/// 2. After each re-parse, call `refresh_source(bytes)` to keep the cached
///    source snapshot in sync with the buffer. The parse tree is read from
///    `SourceContext.tree` at query time — `SharedBuffer.tree` is the single
///    authoritative owner.
/// 3. Register with `ProviderSet::add_highlight_source`.
///
/// # Byte offsets
///
/// tree-sitter returns *absolute* byte offsets within the full file. The
/// provider converts them to *line-relative* offsets (as required by the
/// `HighlightSource` contract) using `ctx.line_start_byte`.
///
/// # Overlapping captures
///
/// tree-sitter queries can produce overlapping captures (e.g. an outer
/// `@type` and an inner `@type.builtin`). This provider resolves overlaps by
/// keeping the shorter (more specific) interval when two intervals share a
/// starting byte, and by trimming the longer one when a shorter one is
/// contained within it. The output is always sorted and non-overlapping.
pub struct TreeSitterHighlighter {
    query: Arc<Query>,
    /// Maps tree-sitter capture index → interned scope id (None = ignored).
    capture_scopes: Vec<Option<ScopeId>>,
    /// Mutable state: must stay in sync with the rope/buffer.
    state: Mutex<TsState>,
}

struct TsState {
    /// Full file bytes, refreshed on every re-parse via `refresh_source`.
    /// Kept here because `cursor.matches` requires a contiguous `&[u8]` for
    /// query predicates, which ropey cannot provide directly.
    source: Vec<u8>,
    /// Scratch buffer for raw captures — retained across calls to avoid reallocation.
    raw: Vec<(usize, usize, ScopeId)>,
    /// Reused query cursor — tree-sitter recommends reuse to amortise its internal allocation.
    cursor: QueryCursor,
}

impl TreeSitterHighlighter {
    /// Create a new provider using Helix-style pass-through scope names.
    ///
    /// Every tree-sitter capture name (e.g. `"keyword.function"`) is used
    /// directly as the engine scope name. The theme's dot-notation cascade
    /// (`keyword.function` → `keyword` → default) handles unknowns. This is
    /// the standard constructor for Helix-compatible `highlights.scm` queries.
    ///
    /// Use [`new_with_scope_map`] when explicit capture→scope remapping is needed.
    pub fn new(
        language: &Language,
        query_source: &str,
        registry: &mut ScopeRegistry,
        initial_source: Vec<u8>,
    ) -> Result<Self, tree_sitter::QueryError> {
        let query = Arc::new(Query::new(language, query_source)?);
        Ok(Self::from_shared_query(query, registry, initial_source))
    }

    /// Create a provider from a pre-compiled, shared query.
    ///
    /// Use this when the `Query` has already been compiled at language
    /// registration time (e.g. from `GrammarBundle.query`) to avoid
    /// re-parsing the `highlights.scm` source per buffer open.
    pub fn from_shared_query(
        query: Arc<Query>,
        registry: &mut ScopeRegistry,
        initial_source: Vec<u8>,
    ) -> Self {
        let capture_scopes: Vec<Option<ScopeId>> = query
            .capture_names()
            .iter()
            .map(|name| Some(registry.intern_runtime(name)))
            .collect();
        Self {
            query,
            capture_scopes,
            state: Mutex::new(TsState {
                source: initial_source,
                raw: Vec::new(),
                cursor: QueryCursor::new(),
            }),
        }
    }

    /// Create a new provider with an explicit capture-name → scope-name map.
    ///
    /// Captures not present in `scope_map` are silently ignored. Use this for
    /// grammars where tree-sitter capture names don't match the engine scope
    /// convention directly.
    pub fn new_with_scope_map(
        language: &Language,
        query_source: &str,
        scope_map: &[(&str, Scope)],
        registry: &mut ScopeRegistry,
        initial_source: Vec<u8>,
    ) -> Result<Self, tree_sitter::QueryError> {
        let query = Arc::new(Query::new(language, query_source)?);
        let capture_scopes: Vec<Option<ScopeId>> = query
            .capture_names()
            .iter()
            .map(|name| {
                scope_map
                    .iter()
                    .find(|(n, _)| *n == *name)
                    .map(|(_, s)| registry.intern(s.0))
            })
            .collect();
        Ok(Self {
            query,
            capture_scopes,
            state: Mutex::new(TsState {
                source: initial_source,
                raw: Vec::new(),
                cursor: QueryCursor::new(),
            }),
        })
    }

    /// Refresh the internal source snapshot after a re-parse.
    ///
    /// Reuses the existing `Vec<u8>` allocation (clear + extend_from_slice) so
    /// the steady-state per-frame cost is a copy, not an alloc.
    ///
    /// The parse tree is not stored here — `SharedBuffer.tree` is the single
    /// authoritative owner, passed to `highlights_for_line` via `SourceContext.tree`.
    pub fn refresh_source(&self, bytes: &[u8]) {
        let mut state = self.state.lock().expect("highlight state lock poisoned");
        state.source.clear();
        state.source.extend_from_slice(bytes);
    }
}

impl HighlightSource for TreeSitterHighlighter {
    fn tier(&self) -> HighlightTier {
        HighlightTier::Syntax
    }

    fn highlights_for_line(
        &self,
        line_idx: usize,
        ctx: &SourceContext,
        out: &mut Vec<(usize, usize, ScopeId)>,
    ) {
        let Some(tree) = ctx.tree else { return };
        let mut state = self.state.lock().expect("highlight state lock poisoned");

        // Compute the absolute byte range for this line.
        let line_start = ctx.line_start_byte;
        let line_end = if line_idx + 1 < ctx.rope.len_lines() {
            ctx.rope.line_to_byte(line_idx + 1)
        } else {
            ctx.rope.len_bytes()
        };

        // Destructure into split borrows so the compiler sees all fields as disjoint.
        let TsState {
            ref source,
            ref mut raw,
            ref mut cursor,
        } = *state;
        cursor.set_byte_range(line_start..line_end);
        raw.clear();

        let root = tree.root_node();
        let source_bytes = source.as_slice();
        let mut matches = cursor.matches(&self.query, root, source_bytes);
        while let Some(m) = matches.next() {
            for cap in m.captures {
                let Some(scope) = self
                    .capture_scopes
                    .get(cap.index as usize)
                    .copied()
                    .flatten()
                else {
                    continue;
                };
                let node = cap.node;
                let abs_start = node.start_byte();
                let abs_end = node.end_byte();
                let rel_start = abs_start.saturating_sub(line_start);
                let rel_end = abs_end.saturating_sub(line_start);
                if rel_start < rel_end {
                    raw.push((rel_start, rel_end, scope));
                }
            }
        }

        if raw.is_empty() {
            return;
        }

        // Sort by (start, length ascending — shorter = more specific wins).
        raw.sort_by_key(|&(start, end, _)| (start, end - start));

        // Resolve overlaps: keep the first (most specific) interval at each
        // byte position. Trim or drop intervals that are fully subsumed.
        let mut max_end: usize = 0;
        for (start, end, scope) in raw.drain(..) {
            if start >= max_end {
                out.push((start, end, scope));
                max_end = end;
            } else if end <= max_end {
                // Fully contained within a previous interval — skip.
            } else {
                // Partially overlapping — trim start to max_end.
                out.push((max_end, end, scope));
                max_end = end;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    // Tree-sitter integration tests require a compiled language grammar, so
    // they live in the integration test suite (`engine/tests/grammar_integration.rs`).
    // This module is exercised by those integration tests.
}
