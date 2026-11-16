use std::sync::Mutex;

use hume_engine::theme::ScopeRegistry;
use hume_engine::types::ScopeId;
use hume_rope::column::ByteCol;
use hume_rope::offset::ExclusiveRange;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, QueryError};

use crate::directives::{PatternDirectives, parse_patterns};
use crate::layers::{SyntaxLayer, SyntaxLayers};

// RopeProvider

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

// TreeSitterHighlighter

/// The nodes a `highlights.scm` pattern applies to, by whether the layer's
/// locals make them local: what its `#is?`/`#is-not? local` sets. A
/// highlights pattern takes no other predicate or directive of the engine's.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Locality {
    #[default]
    Any,
    NonLocal,
    LocalOnly,
}

impl Locality {
    /// Whether the pattern applies to a node `local` says is local or not,
    /// asked only when the answer matters.
    fn admits(self, local: impl FnOnce() -> bool) -> bool {
        match self {
            Self::Any => true,
            Self::NonLocal => !local(),
            Self::LocalOnly => local(),
        }
    }
}

impl PatternDirectives for Locality {
    fn local(&mut self, positive: bool) -> Result<(), String> {
        let wanted = if positive {
            Self::LocalOnly
        } else {
            Self::NonLocal
        };
        if *self != Self::Any && *self != wanted {
            return Err("#is? local and #is-not? local never hold together".to_owned());
        }
        *self = wanted;
        Ok(())
    }
}

/// Built-in highlighter that drives one language's compiled tree-sitter
/// highlight query against a parse tree.
///
/// One instance is shared (via `Arc`) across every buffer of a given
/// language and every syntax layer using that language: capture names are
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
    query: Query,
    /// Maps tree-sitter capture index → interned scope id (None = ignored).
    capture_scopes: Vec<Option<ScopeId>>,
    /// Per pattern, the nodes it applies to.
    localities: Vec<Locality>,
    /// Reused query cursor: tree-sitter recommends reuse to amortise its
    /// internal allocation. `Mutex` because renders run through a shared
    /// `&TreeSitterHighlighter`; never contended in practice (rendering is
    /// single-threaded and layers are queried sequentially).
    cursor: Mutex<QueryCursor>,
}

impl TreeSitterHighlighter {
    /// Create a provider from a query compiled once per grammar bundle and
    /// shared by every buffer's highlighting, so `highlights.scm` is not
    /// re-parsed per buffer open. A pattern using an engine predicate or
    /// directive other than `#is?`/`#is-not? local` is skipped, its error in
    /// `source` pushed to `skipped`.
    pub(crate) fn new(
        mut query: Query,
        source: &str,
        registry: &mut ScopeRegistry,
        skipped: &mut Vec<QueryError>,
    ) -> Self {
        let localities = parse_patterns(&mut query, source, skipped).parsed;
        // Leading-underscore captures (e.g. `@_f`, `@_lib`) are the queries'
        // convention for pattern-internal predicate helpers, never meant to
        // be styled, so map them to `None` so they never emit a span.
        let capture_scopes: Vec<Option<ScopeId>> = query
            .capture_names()
            .iter()
            .map(|name| {
                if name.starts_with('_') {
                    None
                } else {
                    Some(registry.intern(name))
                }
            })
            .collect();
        Self {
            query,
            capture_scopes,
            localities,
            cursor: Mutex::new(QueryCursor::new()),
        }
    }

    /// Append `layer`'s raw (line-relative) highlight intervals for the
    /// line at `line_start..line_end` into `raw`, tagged with the layer's
    /// depth for `flatten_overlaps`'s depth-first priority. Does not flatten
    /// overlaps; the caller merges every layer covering the line before
    /// flattening once.
    ///
    /// A capture whose pattern's `#is?`/`#is-not? local` disagrees with the
    /// layer's locals is left out. Each resolved local reference starting on
    /// the line follows the captures with its definition's highlight, so it
    /// wins the same-range tiebreak over the captures of its node.
    ///
    /// Uses `cursor.captures()` (not `matches()`): the highlight queries rely
    /// on later patterns overriding earlier ones for the *same* node, e.g.
    /// a catch-all `(symbol) @variable` followed by a more specific
    /// `(list . (symbol) @keyword)`. `matches()` yields matches ordered by
    /// each match's root node, so a list-rooted `@keyword` match (root at
    /// the opening paren) is emitted before the symbol-rooted `@variable`
    /// match nested inside it, and `flatten_overlaps`'s last-pushed-wins
    /// same-range tiebreak then picks `@variable`, silently losing every
    /// keyword/function capture. `captures()` yields captures ordered by
    /// node position with pattern order as the same-position tiebreak,
    /// the precedence the queries are written for.
    pub(crate) fn collect_line_spans(
        &self,
        layer: &SyntaxLayer,
        rope: &ropey::Rope,
        line_start: usize,
        line_end: usize,
        raw: &mut Vec<(ByteCol, ByteCol, u8, ScopeId)>,
    ) {
        let mut cursor = self.cursor.lock().expect("query cursor lock poisoned");
        cursor.set_byte_range(line_start..line_end);

        let root = layer.tree.root_node();
        // A capture node can span multiple lines (markdown fenced blocks,
        // paragraphs); `set_byte_range` only filters which nodes match, it
        // does not clip a matched node's end to the queried line. Clamp to
        // this line's content (newline excluded), so a span is a range of
        // its own line and a caller slicing the line by it stays inside.
        // Depends only on `line_start`/`line_end`/`rope`, not on the
        // capture, so it is hoisted above the loop instead of recomputed per capture.
        let content_len = (line_end - line_start).saturating_sub(usize::from(
            line_end > line_start && rope.byte(line_end - 1) == b'\n',
        ));
        let mut push = |abs_start: usize, abs_end: usize, scope: ScopeId| {
            let rel_start = ByteCol::new(abs_start.saturating_sub(line_start));
            let rel_end = ByteCol::new(abs_end.saturating_sub(line_start).min(content_len));
            if rel_start < rel_end {
                raw.push((rel_start, rel_end, layer.depth, scope));
            }
        };
        let mut captures = cursor.captures(&self.query, root, RopeProvider(rope));
        while let Some((m, capture_index)) = captures.next() {
            let cap = m.captures()[*capture_index];
            let Some(scope) = self
                .capture_scopes
                .get(cap.index as usize)
                .copied()
                .flatten()
            else {
                continue;
            };
            let node = cap.node;
            if !self.localities[m.pattern_index]
                .admits(|| layer.locals.is_local(node.start_byte(), node.end_byte()))
            {
                continue;
            }
            push(node.start_byte(), node.end_byte(), scope);
        }
        for (range, scope) in layer.locals.references_in(line_start, line_end) {
            push(range.start, range.end, scope);
        }
    }
}

/// Build sorted, non-overlapping highlight spans for `line_idx` across every
/// syntax layer that covers it.
///
/// Collects each covering layer's raw captures (tagged with the layer's
/// depth) then flattens once via [`hume_engine::interval_sweep::flatten_overlapping_spans`]
/// (`TieBreak::LastPushed`: deepest layer wins, then last-opened within the
/// same depth: `stack` stays sorted ascending by `(depth, seq)`, so
/// `stack.last()` is always the highest-priority active span regardless of
/// collection order; a nested injection's captures can be collected before
/// or after its parent's, only `depth` determines priority). `raw`/`stack`/
/// `events` are caller-owned scratch (`Syntax`'s `FlattenScratch`), cleared
/// on entry.
///
/// Deliberate non-optimization: every line re-runs the query from the tree
/// root (clipped by `set_byte_range`, so cost is O(tree depth + line
/// content)) rather than batching one query per viewport. Per-line keeps
/// this a pure function of `(tree, rope, line)`; batching, even hidden
/// behind this same signature, would add a fill-before-read protocol that
/// every caller inherits. Starting the query below the root is not an
/// option either: patterns rooted at ancestor nodes silently stop
/// matching. If this ever needs revisiting, fix order: a pre-bucketed
/// viewport query first, a span cache second.
pub fn layer_highlights_for_line(
    layers: &SyntaxLayers,
    line_idx: hume_rope::line::ContentLine,
    rope: &ropey::Rope,
    raw: &mut Vec<(ByteCol, ByteCol, u8, ScopeId)>,
    stack: &mut Vec<(u8, u32, ScopeId)>,
    events: &mut Vec<(ByteCol, bool, u32, u8, ScopeId)>,
    out: &mut Vec<(ExclusiveRange<ByteCol>, ScopeId)>,
) {
    let (bytes, covering) = layers.covering_line(rope, line_idx);
    raw.clear();
    for layer in covering {
        layer
            .bundle
            .highlighter
            .collect_line_spans(layer, rope, bytes.start, bytes.end, raw);
    }

    hume_engine::interval_sweep::flatten_overlapping_spans(
        raw,
        stack,
        events,
        out,
        hume_engine::interval_sweep::TieBreak::LastPushed,
    );
}

#[cfg(test)]
mod tests;
