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
/// Parse-tree nodes are nested or disjoint — never partially overlapping.
/// This provider flattens them so the **innermost** (shortest) capture wins
/// at every byte: e.g. `@string.escape` inside `@string` renders with the
/// escape scope, not the generic string scope. The output is always sorted
/// and non-overlapping.
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
    /// Scratch stack for the nested-capture flattener — retained to avoid reallocation.
    stack: Vec<(usize, ScopeId)>,
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
                stack: Vec::new(),
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
                stack: Vec::new(),
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
            ref mut stack,
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

        // Sort outermost-first (start asc, end desc), then flatten so the innermost
        // capture wins at every byte.
        raw.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        flatten_overlaps(raw, stack, out);
    }
}

// ---------------------------------------------------------------------------
// flatten_overlaps
// ---------------------------------------------------------------------------

/// Flatten nested/disjoint capture intervals so the innermost wins at every byte.
///
/// `raw` must already be sorted outermost-first (start asc, end desc) and is
/// drained by this call. `stack` is scratch storage (cleared on entry); on
/// return both `raw` and `stack` are empty. Non-overlapping, sorted intervals
/// are appended to `out`.
fn flatten_overlaps(
    raw: &mut Vec<(usize, usize, ScopeId)>,
    stack: &mut Vec<(usize, ScopeId)>,
    out: &mut Vec<(usize, usize, ScopeId)>,
) {
    debug_assert!(stack.is_empty());
    let mut pos = 0usize;
    for (start, end, scope) in raw.drain(..) {
        // Close intervals that end at or before `start`, innermost first.
        while let Some(&(top_end, top_scope)) = stack.last() {
            if top_end <= start {
                if pos < top_end {
                    out.push((pos, top_end, top_scope));
                    pos = top_end;
                }
                stack.pop();
            } else {
                break;
            }
        }
        // Fill the gap between `pos` and `start` with the enclosing scope.
        if let Some(&(_, top_scope)) = stack.last() && pos < start {
            out.push((pos, start, top_scope));
        }
        pos = start;
        stack.push((end, scope));
    }
    // Drain remaining open intervals, innermost first.
    while let Some((top_end, top_scope)) = stack.pop() {
        if pos < top_end {
            out.push((pos, top_end, top_scope));
            pos = top_end;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn s(n: u16) -> ScopeId {
        ScopeId(n)
    }

    fn run(mut raw: Vec<(usize, usize, ScopeId)>) -> Vec<(usize, usize, ScopeId)> {
        raw.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        let mut stack = Vec::new();
        let mut out = Vec::new();
        flatten_overlaps(&mut raw, &mut stack, &mut out);
        out
    }

    #[test]
    fn inner_wins_non_shared_start() {
        // Regression: outer @string [0,8), inner @string.escape [5,7).
        // Old sweep dropped the inner; stack flattener emits it correctly.
        let got = run(vec![(0, 8, s(0)), (5, 7, s(1))]);
        assert_eq!(got, vec![(0, 5, s(0)), (5, 7, s(1)), (7, 8, s(0))]);
    }

    #[test]
    fn inner_wins_shared_start() {
        // Shared start: outer [0,10), inner [0,4) — inner wins its region.
        let got = run(vec![(0, 10, s(0)), (0, 4, s(1))]);
        assert_eq!(got, vec![(0, 4, s(1)), (4, 10, s(0))]);
    }

    #[test]
    fn disjoint_gap_preserved() {
        // Two disjoint intervals; uncovered gap has no output.
        let got = run(vec![(0, 3, s(0)), (5, 8, s(1))]);
        assert_eq!(got, vec![(0, 3, s(0)), (5, 8, s(1))]);
    }

    #[test]
    fn three_level_nesting() {
        // [0,20) A, [5,15) B, [8,10) C — each level wins its region.
        let got = run(vec![(0, 20, s(0)), (5, 15, s(1)), (8, 10, s(2))]);
        assert_eq!(
            got,
            vec![
                (0, 5, s(0)),
                (5, 8, s(1)),
                (8, 10, s(2)),
                (10, 15, s(1)),
                (15, 20, s(0)),
            ]
        );
    }
}
