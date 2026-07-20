use std::sync::{Arc, Mutex};

use hume_engine::theme::ScopeRegistry;
use hume_engine::types::ScopeId;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Query, QueryCursor};

use crate::layers::{SyntaxLayers, layer_covers_line};

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
///
/// `pub(crate)` so other tree-sitter-driven query matching within this crate
/// (e.g. injection resolution in `injections.rs`) can reuse it instead of
/// duplicating.
pub(crate) struct RopeProvider<'a>(pub(crate) &'a ropey::Rope);

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

/// Built-in highlighter that drives one language's compiled tree-sitter
/// highlight query against a parse tree.
///
/// One instance is shared (via `Arc`) across every buffer of a given
/// language and every syntax layer using that language — capture names are
/// interned once, at construction. `collect_line_spans` is the sole entry
/// point; `layer_highlights_for_line` (below) drives it per buffer line,
/// merging captures across all of a buffer's syntax layers before flattening
/// overlaps.
///
/// # Byte offsets
///
/// tree-sitter returns *absolute* byte offsets within the full file.
/// `collect_line_spans` converts them to *line-relative* offsets.
pub struct TreeSitterHighlighter {
    query: Arc<Query>,
    /// Maps tree-sitter capture index → interned scope id (None = ignored).
    capture_scopes: Vec<Option<ScopeId>>,
    /// Reused query cursor — tree-sitter recommends reuse to amortise its
    /// internal allocation. `Mutex` because renders run through a shared
    /// `&TreeSitterHighlighter`; never contended in practice (rendering is
    /// single-threaded and layers are queried sequentially).
    cursor: Mutex<QueryCursor>,
}

impl TreeSitterHighlighter {
    /// Create a new provider using Helix-style pass-through scope names.
    ///
    /// Every tree-sitter capture name (e.g. `"keyword.function"`) is used
    /// directly as the engine scope name. The theme's dot-notation cascade
    /// (`keyword.function` → `keyword` → default) handles unknowns. This is
    /// the standard constructor for Helix-compatible `highlights.scm` queries.
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
        // Leading-underscore captures (e.g. `@_f`, `@_lib`) are Helix's
        // convention for pattern-internal predicate helpers, never meant to
        // be styled — map them to `None` so they never emit a span.
        let capture_scopes: Vec<Option<ScopeId>> = query
            .capture_names()
            .iter()
            .map(|name| {
                if name.starts_with('_') {
                    None
                } else {
                    Some(registry.intern_runtime(name))
                }
            })
            .collect();
        Self {
            query,
            capture_scopes,
            cursor: Mutex::new(QueryCursor::new()),
        }
    }

    /// Append this layer's raw (line-relative) capture intervals for
    /// `line_idx` into `raw`, tagged with `depth` for `flatten_overlaps`'s
    /// depth-first priority. Does not flatten overlaps — the caller merges
    /// captures from every layer covering the line before flattening once.
    ///
    /// Uses `cursor.captures()` (not `matches()`): Helix-style queries rely
    /// on later patterns overriding earlier ones for the *same* node — e.g.
    /// a catch-all `(symbol) @variable` followed by a more specific
    /// `(list . (symbol) @keyword)`. `matches()` yields matches ordered by
    /// each match's root node, so a list-rooted `@keyword` match (root at
    /// the opening paren) is emitted before the symbol-rooted `@variable`
    /// match nested inside it, and `flatten_overlaps`'s last-pushed-wins
    /// same-range tiebreak then picks `@variable` — silently losing every
    /// keyword/function capture. `captures()` yields captures ordered by
    /// node position with pattern order as the same-position tiebreak,
    /// matching Helix's own precedence semantics.
    pub(crate) fn collect_line_spans(
        &self,
        tree: &tree_sitter::Tree,
        rope: &ropey::Rope,
        line_start: usize,
        line_end: usize,
        depth: u8,
        raw: &mut Vec<(usize, usize, ScopeId, u8)>,
    ) {
        let mut cursor = self.cursor.lock().expect("query cursor lock poisoned");
        cursor.set_byte_range(line_start..line_end);

        let root = tree.root_node();
        let mut captures = cursor.captures(&self.query, root, RopeProvider(rope));
        while let Some((m, capture_index)) = captures.next() {
            let cap = m.captures[*capture_index];
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
                raw.push((rel_start, rel_end, scope, depth));
            }
        }
    }
}

/// Build sorted, non-overlapping highlight spans for `line_idx` across every
/// syntax layer that covers it, for consumption by the engine's
/// `rebuild_tier_bufs` (reached via the `SyntaxSpans` trait).
///
/// Collects each covering layer's raw captures (tagged with the layer's
/// depth) then flattens once — `flatten_overlaps` resolves overlaps by
/// deepest-layer-wins, so a nested injection's captures always take priority
/// over its parent's, regardless of collection order. `raw`/`stack`/`events`
/// are caller-owned scratch (`Syntax`'s `FlattenScratch`), cleared on entry.
///
/// Deliberate non-optimization: every line re-runs the query from the tree
/// root (clipped by `set_byte_range`, so cost is O(tree depth + line
/// content)) rather than batching one query per viewport. Per-line keeps
/// this a pure function of `(tree, rope, line)` — batching, even hidden
/// behind this same signature, would add a fill-before-read protocol that
/// every caller inherits. Starting the query below the root is not an
/// option either — patterns rooted at ancestor nodes silently stop
/// matching. Revisit bar and fix order: see "Syntax highlight query
/// granularity" in docs/ROADMAP.md (pre-bucketed viewport query first, span
/// cache second).
pub fn layer_highlights_for_line(
    layers: &SyntaxLayers,
    line_idx: usize,
    rope: &ropey::Rope,
    raw: &mut Vec<(usize, usize, ScopeId, u8)>,
    stack: &mut Vec<(u8, u32, ScopeId)>,
    events: &mut Vec<(usize, bool, u32, ScopeId, u8)>,
    out: &mut Vec<(usize, usize, ScopeId)>,
) {
    let line_start = rope.line_to_byte(line_idx);
    let line_end = if line_idx + 1 < rope.len_lines() {
        rope.line_to_byte(line_idx + 1)
    } else {
        rope.len_bytes()
    };

    raw.clear();
    for layer in &layers.layers {
        if layer_covers_line(layer, line_start, line_end) {
            layer.highlighter.collect_line_spans(
                &layer.tree,
                rope,
                line_start,
                line_end,
                layer.depth,
                raw,
            );
        }
    }

    flatten_overlaps(raw, stack, events, out);
}

// ---------------------------------------------------------------------------
// flatten_overlaps
// ---------------------------------------------------------------------------

/// Flatten capture intervals into sorted, non-overlapping output.
///
/// Uses a sweep-line over start/end events so partial overlaps (which can
/// arise when captures from different query patterns are line-clipped, or
/// when a nested injection's range overlaps its parent layer's) are handled
/// correctly.  `raw` is drained; `stack` and `events` are scratch storage
/// cleared on entry.  Non-overlapping, sorted intervals are appended to `out`.
///
/// Priority is **deepest layer wins**, then **last-opened** (most recently
/// started) within the same depth, matching tree-sitter's own priority model
/// generalized across injection layers. `stack` is kept sorted ascending by
/// `(depth, seq)` at all times — insertion happens at the correct sorted
/// position rather than always at the end — so `stack.last()` is always the
/// highest-priority active interval regardless of collection order (a nested
/// injection's captures can be collected before or after its parent's; only
/// `depth` determines priority, never collection order).
fn flatten_overlaps(
    raw: &mut Vec<(usize, usize, ScopeId, u8)>,
    stack: &mut Vec<(u8, u32, ScopeId)>,
    events: &mut Vec<(usize, bool, u32, ScopeId, u8)>,
    out: &mut Vec<(usize, usize, ScopeId)>,
) {
    debug_assert!(stack.is_empty());
    debug_assert!(events.is_empty());
    if raw.is_empty() {
        return;
    }

    // Build a sorted event list: (pos, is_end, seq, scope, depth). `seq` is
    // the interval's index in `raw` — unique per interval, used to pop the
    // exact matching stack entry (never ambiguous, unlike matching by scope
    // value when two active intervals share a scope).
    // End events sort before start events at the same position so a closing
    // interval is popped before a new one is pushed at the same byte.
    for (seq, &(start, end, scope, depth)) in raw.iter().enumerate() {
        let seq = seq as u32;
        events.push((start, false, seq, scope, depth)); // Start
        events.push((end, true, seq, scope, depth)); // End
    }
    raw.clear();
    // Sort purely by (pos, ends-before-starts) — priority among
    // simultaneously active intervals is resolved by the sorted-stack
    // insertion below, not by event processing order.
    events.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

    let mut pos = 0usize;
    for &(event_pos, is_end, seq, scope, depth) in events.iter() {
        // Emit the gap before this event using the currently active scope.
        if let Some(&(_, _, active_scope)) = stack.last()
            && pos < event_pos
        {
            out.push((pos, event_pos, active_scope));
        }
        pos = event_pos;

        if is_end {
            let idx = stack.iter().position(|&(d, s, _)| d == depth && s == seq);
            debug_assert!(
                idx.is_some(),
                "end event with no matching start on the stack — a zero-width \
                 interval would sort its end before its own start at the same \
                 position; callers must filter those out before collection"
            );
            if let Some(idx) = idx {
                stack.remove(idx);
            }
        } else {
            // Insert in ascending (depth, seq) order so `stack.last()` stays
            // the highest-priority active interval regardless of arrival order.
            let insert_at = stack.partition_point(|&(d, s, _)| (d, s) < (depth, seq));
            stack.insert(insert_at, (depth, seq, scope));
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

    /// Run `flatten_overlaps` with every interval at depth 0 (single-layer,
    /// the pre-injection behavior) — `d()` below builds multi-depth input
    /// for the depth-priority tests.
    fn run(raw: Vec<(usize, usize, ScopeId)>) -> Vec<(usize, usize, ScopeId)> {
        run_d(raw.into_iter().map(|(a, b, c)| (a, b, c, 0)).collect())
    }

    fn run_d(mut raw: Vec<(usize, usize, ScopeId, u8)>) -> Vec<(usize, usize, ScopeId)> {
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
    fn same_start_three_way_tie_resolves_by_seq() {
        // Three captures share a start byte with distinct ends and distinct
        // scopes. The highest-seq (last-captured) interval must win at byte
        // 0; as it and the middle one end, each remaining interval takes over
        // in seq order. `sort_unstable_by` does not guarantee tie order by
        // API contract — the explicit `seq` field makes this deterministic
        // rather than an accident of the current sort implementation.
        let got = run(vec![(0, 5, s(0)), (0, 8, s(1)), (0, 6, s(2))]);
        assert_eq!(got, vec![(0, 6, s(2)), (6, 8, s(1))]);
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

        let mut first = vec![(0, 3, s(0), 0)];
        flatten_overlaps(&mut first, &mut stack, &mut events, &mut out);
        assert_eq!(out, vec![(0, 3, s(0))]);

        out.clear();
        let mut second = vec![(10, 12, s(1), 0)];
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

    // ── Depth priority (multi-layer / injections) ────────────────────────────

    #[test]
    fn deeper_layer_wins_regardless_of_collection_order() {
        // A depth-0 (root) span [0,10) fully contains a depth-2 (nested
        // injection) span [3,7) — collected in REVERSE depth order (the
        // depth-2 entry pushed into `raw` before the depth-0 one), simulating
        // a nested injection gathered ahead of its shallower parent. The
        // deeper span must still win its region regardless of collection order.
        let got = run_d(vec![(3, 7, s(2), 2), (0, 10, s(0), 0)]);
        assert_eq!(got, vec![(0, 3, s(0)), (3, 7, s(2)), (7, 10, s(0))]);
    }

    #[test]
    fn shallower_layer_started_later_still_loses_to_deeper() {
        // Depth-1 span [0,10) started AFTER (higher seq) a depth-3 span
        // [2,5) in collection order — plain seq-order priority (pre-depth-
        // aware flattening) would have picked the depth-1 span at [2,5)
        // since it has the higher seq; depth must win instead.
        let got = run_d(vec![(2, 5, s(3), 3), (0, 10, s(1), 1)]);
        assert_eq!(got, vec![(0, 2, s(1)), (2, 5, s(3)), (5, 10, s(1))]);
    }
}
