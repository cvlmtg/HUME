use std::sync::{Arc, Mutex};

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::providers::{HighlightSource, HighlightTier, SourceContext};
use crate::theme::ScopeRegistry;
use crate::types::{Scope, ScopeId};

// ---------------------------------------------------------------------------
// RopeProvider
// ---------------------------------------------------------------------------

/// Feeds rope chunks to tree-sitter query matching as a `TextProvider`.
///
/// Replaces a materialised `Vec<u8>` snapshot: the rope is the single source
/// of truth for buffer bytes.  Node byte ranges are expected to align with the
/// live rope at render time (the editor bakes the tree before each render).
/// `get_byte_slice` returns `None` on an out-of-range request, so a misaligned
/// range degrades to empty text rather than panicking.
struct RopeProvider<'a>(&'a ropey::Rope);

impl<'a> tree_sitter::TextProvider<&'a [u8]> for RopeProvider<'a> {
    type I = std::iter::Map<ropey::iter::Chunks<'a>, fn(&str) -> &[u8]>;

    fn text(&mut self, node: tree_sitter::Node) -> Self::I {
        let slice = self
            .0
            .get_byte_slice(node.start_byte()..node.end_byte())
            .unwrap_or_else(|| self.0.byte_slice(0..0));
        slice.chunks().map(str::as_bytes)
    }
}

// ---------------------------------------------------------------------------
// TreeSitterHighlighter
// ---------------------------------------------------------------------------

/// Built-in highlight provider that drives tree-sitter highlight queries.
///
/// # Usage
///
/// 1. Create with `TreeSitterHighlighter::new(language, query_source)`.
/// 2. After each re-parse, write the new tree to `SharedBuffer`.  Pass it via
///    `SourceContext` at render time — it is not stored here.
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
/// Individual parse-tree nodes are nested or disjoint, but captures from
/// *different* query patterns can produce partially-overlapping spans after
/// line clipping.  `highlights_for_line` uses a sweep-line algorithm that
/// tolerates any overlap and always emits sorted, non-overlapping output where
/// the **last-started** capture wins at every byte.
pub struct TreeSitterHighlighter {
    query: Arc<Query>,
    /// Maps tree-sitter capture index → interned scope id (None = ignored).
    capture_scopes: Vec<Option<ScopeId>>,
    /// Mutable state: must stay in sync with the rope/buffer.
    state: Mutex<TsState>,
}

struct TsState {
    /// Scratch buffer for raw captures — retained across calls to avoid reallocation.
    raw: Vec<(usize, usize, ScopeId)>,
    /// Reused query cursor — tree-sitter recommends reuse to amortise its internal allocation.
    cursor: QueryCursor,
    /// Scratch stack for the overlap flattener — retained to avoid reallocation.
    stack: Vec<(usize, ScopeId)>,
    /// Scratch event list for the overlap flattener's sweep line — retained to
    /// avoid reallocation (previously rebuilt every call, ~50 allocations/frame
    /// on a full screen).
    events: Vec<(usize, bool, ScopeId)>,
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
    ) -> Result<Self, tree_sitter::QueryError> {
        let query = Arc::new(Query::new(language, query_source)?);
        Ok(Self::from_shared_query(query, registry))
    }

    /// Create a provider from a pre-compiled, shared query.
    ///
    /// Use this when the `Query` has already been compiled at language
    /// registration time (e.g. from `GrammarBundle.query`) to avoid
    /// re-parsing the `highlights.scm` source per buffer open.
    pub fn from_shared_query(query: Arc<Query>, registry: &mut ScopeRegistry) -> Self {
        let capture_scopes: Vec<Option<ScopeId>> = query
            .capture_names()
            .iter()
            .map(|name| Some(registry.intern_runtime(name)))
            .collect();
        Self {
            query,
            capture_scopes,
            state: Mutex::new(TsState {
                raw: Vec::new(),
                cursor: QueryCursor::new(),
                stack: Vec::new(),
                events: Vec::new(),
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
                raw: Vec::new(),
                cursor: QueryCursor::new(),
                stack: Vec::new(),
                events: Vec::new(),
            }),
        })
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
            ref mut raw,
            ref mut cursor,
            ref mut stack,
            ref mut events,
        } = *state;
        cursor.set_byte_range(line_start..line_end);
        raw.clear();

        let root = tree.root_node();
        let mut matches = cursor.matches(&self.query, root, RopeProvider(ctx.rope));
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

        flatten_overlaps(raw, stack, events, out);
    }
}

// ---------------------------------------------------------------------------
// flatten_overlaps
// ---------------------------------------------------------------------------

/// Flatten capture intervals into sorted, non-overlapping output.
///
/// Uses a sweep-line over start/end events so partial overlaps (which can
/// arise when captures from different query patterns are line-clipped) are
/// handled correctly.  `raw` is drained; `stack` and `events` are scratch
/// storage cleared on entry.  Non-overlapping, sorted intervals are appended
/// to `out`.
///
/// When intervals overlap, the **last-opened** (most recently started) scope
/// wins at any given byte, matching tree-sitter's own priority model.
fn flatten_overlaps(
    raw: &mut Vec<(usize, usize, ScopeId)>,
    stack: &mut Vec<(usize, ScopeId)>,
    events: &mut Vec<(usize, bool, ScopeId)>,
    out: &mut Vec<(usize, usize, ScopeId)>,
) {
    debug_assert!(stack.is_empty());
    debug_assert!(events.is_empty());
    if raw.is_empty() {
        return;
    }

    // Build a sorted event list: (pos, is_end, scope).
    // End events sort before start events at the same position so a closing
    // interval is popped before a new one is pushed at the same byte.
    for &(start, end, scope) in raw.iter() {
        events.push((start, false, scope)); // Start
        events.push((end, true, scope)); // End
    }
    raw.clear();
    // Sort: by position, ends before starts at the same position.
    events.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

    let mut pos = 0usize;
    for &(event_pos, is_end, scope) in events.iter() {
        // Emit the gap before this event using the currently active scope.
        if let Some(&(_, active_scope)) = stack.last()
            && pos < event_pos
        {
            out.push((pos, event_pos, active_scope));
        }
        pos = event_pos;

        if is_end {
            // Pop this scope from the active set (find by value, last occurrence).
            if let Some(idx) = stack.iter().rposition(|&(_, s)| s == scope) {
                stack.remove(idx);
            }
        } else {
            stack.push((event_pos, scope));
        }
    }
    stack.clear();
    events.clear();

    // Merge adjacent segments that share the same scope — they can arise when
    // an overlapping interval ends while another with the same scope is still
    // active (e.g. A=[0,5), B=[3,8): at pos=5 A ends, B continues, producing
    // (3,5,B) then (5,8,B) without a merge pass).
    //
    // `dedup_by`'s closure receives `(a, b)` where `b` is the retained
    // predecessor; returning `true` drops `a` after folding its end into `b`.
    out.dedup_by(|next, prev| {
        if prev.2 == next.2 && prev.1 == next.0 {
            prev.1 = next.1; // extend the retained segment
            true
        } else {
            false
        }
    });
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
        let mut stack = Vec::new();
        let mut events = Vec::new();
        let mut out = Vec::new();
        flatten_overlaps(&mut raw, &mut stack, &mut events, &mut out);
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

    #[test]
    fn partial_overlap_last_started_wins() {
        // A=[0,5), B=[3,8): overlapping, not nested.  In the overlap region
        // [3,5) B started last so it wins; B continues alone in [5,8).
        let got = run(vec![(0, 5, s(0)), (3, 8, s(1))]);
        assert_eq!(got, vec![(0, 3, s(0)), (3, 8, s(1))]);
    }

    #[test]
    fn identical_ranges_last_pushed_wins() {
        // Two captures of the same byte range: last-opened (s(1)) wins entirely.
        let got = run(vec![(0, 5, s(0)), (0, 5, s(1))]);
        assert_eq!(got, vec![(0, 5, s(1))]);
    }

    #[test]
    fn scratch_reuse_across_calls_leaves_no_stale_events() {
        // Regression for O1: `events`/`stack` are caller-owned scratch reused
        // across calls (mirroring TsState in the real highlighter). A second,
        // disjoint call through the same scratch must not see the first
        // call's intervals leak through.
        let mut stack = Vec::new();
        let mut events = Vec::new();
        let mut out = Vec::new();

        let mut first = vec![(0, 3, s(0))];
        flatten_overlaps(&mut first, &mut stack, &mut events, &mut out);
        assert_eq!(out, vec![(0, 3, s(0))]);

        out.clear();
        let mut second = vec![(10, 12, s(1))];
        flatten_overlaps(&mut second, &mut stack, &mut events, &mut out);
        assert_eq!(out, vec![(10, 12, s(1))]);
    }

    #[test]
    fn three_segment_chain_merges_into_one() {
        // Same scope, three adjacent segments: [0,2)+[2,5)+[5,9) → one [0,9).
        // Exercises the dedup_by merge pass over more than one join.
        let got = run(vec![(0, 2, s(0)), (2, 5, s(0)), (5, 9, s(0))]);
        assert_eq!(got, vec![(0, 9, s(0))]);
    }
}
