use std::sync::{Arc, Mutex};

use hume_engine::theme::ScopeRegistry;
use hume_engine::types::ScopeId;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor};

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
        raw: &mut Vec<(usize, usize, u8, ScopeId)>,
    ) {
        let mut cursor = self.cursor.lock().expect("query cursor lock poisoned");
        cursor.set_byte_range(line_start..line_end);

        let root = tree.root_node();
        // A capture node can span multiple lines (markdown fenced blocks,
        // paragraphs); `set_byte_range` only filters which nodes match, it
        // does not clip a matched node's end to the queried line. Clamp to
        // this line's content (newline excluded) so consumers that slice
        // their own copy of the line by these offsets — the hover popup,
        // via `MarkupSyntax::styled_row` — never index past its end.
        // Depends only on `line_start`/`line_end`/`rope`, not on the
        // capture — hoisted above the loop instead of recomputed per capture.
        let content_len = (line_end - line_start).saturating_sub(usize::from(
            line_end > line_start && rope.byte(line_end - 1) == b'\n',
        ));
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
            let rel_end = abs_end.saturating_sub(line_start).min(content_len);
            if rel_start < rel_end {
                raw.push((rel_start, rel_end, depth, scope));
            }
        }
    }
}

/// Build sorted, non-overlapping highlight spans for `line_idx` across every
/// syntax layer that covers it, for consumption by the engine's
/// `rebuild_line_decorations` (reached via the `SyntaxSpans` trait).
///
/// Collects each covering layer's raw captures (tagged with the layer's
/// depth) then flattens once via [`hume_engine::interval_sweep::flatten_overlapping_spans`]
/// (`TieBreak::LastPushed`: deepest layer wins, then last-opened within the
/// same depth — `stack` stays sorted ascending by `(depth, seq)`, so
/// `stack.last()` is always the highest-priority active span regardless of
/// collection order; a nested injection's captures can be collected before
/// or after its parent's, only `depth` determines priority). `raw`/`stack`/
/// `events` are caller-owned scratch (`Syntax`'s `FlattenScratch`), cleared
/// on entry.
///
/// Deliberate non-optimization: every line re-runs the query from the tree
/// root (clipped by `set_byte_range`, so cost is O(tree depth + line
/// content)) rather than batching one query per viewport. Per-line keeps
/// this a pure function of `(tree, rope, line)` — batching, even hidden
/// behind this same signature, would add a fill-before-read protocol that
/// every caller inherits. Starting the query below the root is not an
/// option either — patterns rooted at ancestor nodes silently stop
/// matching. If this ever needs revisiting, fix order: a pre-bucketed
/// viewport query first, a span cache second.
pub fn layer_highlights_for_line(
    layers: &SyntaxLayers,
    line_idx: usize,
    rope: &ropey::Rope,
    raw: &mut Vec<(usize, usize, u8, ScopeId)>,
    stack: &mut Vec<(u8, u32, ScopeId)>,
    events: &mut Vec<(usize, bool, u32, u8, ScopeId)>,
    out: &mut Vec<(usize, usize, ScopeId)>,
) {
    let line_start = rope.line_to_byte(line_idx);
    let line_end = hume_rope::line_end_exclusive_byte(rope, line_idx);

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

    hume_engine::interval_sweep::flatten_overlapping_spans(
        raw,
        stack,
        events,
        out,
        hume_engine::interval_sweep::TieBreak::LastPushed,
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
