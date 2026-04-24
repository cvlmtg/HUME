use std::sync::Mutex;

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
/// 1. Create with `TreeSitterHighlighter::new(id, language, query_source)`.
/// 2. After each edit, call `update(source_bytes, new_tree)` to keep the
///    source snapshot and parse tree in sync with the buffer.
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
    query: Query,
    /// Maps tree-sitter capture index → interned scope id (None = ignored).
    capture_scopes: Vec<Option<ScopeId>>,
    /// Mutable state: must stay in sync with the rope/buffer.
    state: Mutex<TsState>,
}

struct TsState {
    /// Full file bytes. Updated on every edit via `update()`.
    source: Vec<u8>,
    /// Latest parse tree.
    tree: tree_sitter::Tree,
    /// Scratch buffer for raw captures — retained across calls to avoid reallocation.
    raw: Vec<(usize, usize, ScopeId)>,
    /// Reused query cursor — tree-sitter recommends reuse to amortise its internal allocation.
    cursor: QueryCursor,
}

impl TreeSitterHighlighter {
    /// Create a new provider.
    ///
    /// `scope_map` maps tree-sitter capture names (e.g. `"keyword"`) to
    /// engine scope names (e.g. `Scope("keyword")`). Captures not in the map
    /// are silently ignored.
    ///
    /// `registry` is the session-wide [`ScopeRegistry`]. Each scope in
    /// `scope_map` is interned here so it can be resolved in O(1) at render
    /// time via [`crate::theme::Theme::resolve`].
    pub fn new(
        language: &Language,
        query_source: &str,
        scope_map: &[(&str, Scope)],
        registry: &mut ScopeRegistry,
        initial_source: Vec<u8>,
        initial_tree: tree_sitter::Tree,
    ) -> Result<Self, tree_sitter::QueryError> {
        let query = Query::new(language, query_source)?;
        let capture_names = query.capture_names();
        // Intern each mapped scope into the registry; unmapped captures stay None.
        let capture_scopes: Vec<Option<ScopeId>> = capture_names
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
                tree: initial_tree,
                raw: Vec::new(),
                cursor: QueryCursor::new(),
            }),
        })
    }

    /// Update the source snapshot and parse tree after an edit.
    /// Call this before the next render frame.
    pub fn update(&self, new_source: Vec<u8>, new_tree: tree_sitter::Tree) {
        let mut state = self.state.lock().expect("highlight state lock poisoned");
        state.source = new_source;
        state.tree = new_tree;
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
            ref tree,
            ref source,
            ref mut raw,
            ref mut cursor,
        } = *state;
        cursor.set_byte_range(line_start..line_end);
        raw.clear();

        // tree-sitter 0.24 returns a StreamingIterator, not a regular Iterator.
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
    // they live in the integration test suite rather than here.
    // This module is exercised by example binaries and integration tests.
}
