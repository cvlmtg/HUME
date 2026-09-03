//! Tree-sitter structural text objects and navigation: kinds, spans, the
//! per-`(kind, span)` capture-index table a compiled `textobjects.scm`
//! resolves to, and [`ObjectSpans`], which runs that query over a buffer's
//! syntax layers into a sorted list of inclusive char spans.
//!
//! Freshness (the tree matches the text before a command runs) is
//! `Syntax::ensure_current`; selection policy (Move/Extend, count,
//! multi-cursor) is `hume-ops`'s `apply_text_object_by_mode` and
//! `apply_object_motion` — this module only collects spans and answers two
//! lookups over them.

// ── ObjectKind / ObjectSpan ──────────────────────────────────────────────

/// Emits an object-kind/span enum, its dense `ALL` slice, and its
/// `capture_name`/`from_capture_name` pair from one variant↦capture-name
/// list. `TextObjectsQuery`'s capture table is sized
/// `[[Option<u32>; ObjectSpan::ALL.len()]; ObjectKind::ALL.len()]` and
/// indexed by `kind as usize`/`span as usize` — a variant added to the enum
/// but not to a hand-synced `ALL` list compiles fine and panics out of
/// bounds on the first query attach. One list generating all three closes
/// that: there is nothing left to forget to update in step.
macro_rules! object_enum {
    (
        $(#[$enum_doc:meta])*
        $enum_name:ident { $($variant:ident = $capture:literal),+ $(,)? }
    ) => {
        $(#[$enum_doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $enum_name {
            $($variant),+
        }

        impl $enum_name {
            /// Every variant, in declaration order. `pub` because
            /// `hume-editor` checks its own structural-command table
            /// against this list — a kind here with no commands there
            /// would otherwise ship silently.
            pub const ALL: &'static [$enum_name] = &[$($enum_name::$variant),+];

            /// The half of a capture name this type names, e.g.
            /// `@function.inside` has kind half `"function"` and span half
            /// `"inside"`. Single source of truth: also used in reverse by
            /// [`Self::from_capture_name`].
            fn capture_name(self) -> &'static str {
                match self {
                    $($enum_name::$variant => $capture),+
                }
            }

            fn from_capture_name(name: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|v| v.capture_name() == name)
            }
        }
    };
}

object_enum! {
    /// A structural object a `textobjects.scm` query may define, named after
    /// the `<kind>` half of its capture names (`function.inside`,
    /// `class.around`, …).
    ObjectKind {
        Function = "function",
        Class = "class",
        Parameter = "parameter",
        Comment = "comment",
        Test = "test",
        Entry = "entry",
    }
}

object_enum! {
    /// Which part of an object a capture spans. `Movement` is Helix's
    /// optional navigation-only capture (a function's name node, say) —
    /// narrower than `Around`, consumed only by navigation, never by
    /// selection.
    ObjectSpan {
        Inside = "inside",
        Around = "around",
        Movement = "movement",
    }
}

// ── SpanSelector ───────────────────────────────────────────────────────────

/// Which set of spans a caller wants collected — the whole input to
/// [`ObjectSpans::for_selector`], and therefore its memo key.
///
/// Selection (`m i f`, `m a c`) names an exact `<kind>.<span>` capture;
/// navigation (`goto-next-<kind>`) names only a kind and lets
/// [`ObjectSpans::collect_for_navigation`] pick the span per layer. Making
/// that a value rather than two call sites is what lets one cache serve both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanSelector {
    /// One `<kind>.<span>` capture, exactly as written.
    Exact(ObjectKind, ObjectSpan),
    /// The kind's best navigation span, per-layer priority.
    Navigation(ObjectKind),
}

// ── Direction ──────────────────────────────────────────────────────────────

/// The only direction enum for structural navigation. `hume-ops` takes a
/// `backward: bool` at its API boundary (the `apply_word_select`
/// convention) rather than importing this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

// ── TextObjectsQuery ───────────────────────────────────────────────────────

/// A compiled `textobjects.scm` query plus a dense `(kind, span) → capture
/// index` table, resolved once at attach time by splitting each capture
/// name on its last `.` (`@parameter.inside.extra` would split on the last
/// dot, but no Helix query defines such a name today). Names that don't
/// parse as `<kind>.<span>` (`@_helper`, `@function.x`) map to nothing.
pub struct TextObjectsQuery {
    /// Read directly by [`collect_hulls`], which needs the compiled `Query`
    /// itself, not just this table.
    query: tree_sitter::Query,
    captures: [[Option<u32>; ObjectSpan::ALL.len()]; ObjectKind::ALL.len()],
}

impl TextObjectsQuery {
    pub(crate) fn new(query: tree_sitter::Query) -> Self {
        let mut captures = [[None; ObjectSpan::ALL.len()]; ObjectKind::ALL.len()];
        for (idx, name) in query.capture_names().iter().enumerate() {
            let Some((kind_name, span_name)) = name.rsplit_once('.') else {
                continue;
            };
            let (Some(kind), Some(span)) = (
                ObjectKind::from_capture_name(kind_name),
                ObjectSpan::from_capture_name(span_name),
            ) else {
                continue;
            };
            captures[kind as usize][span as usize] = Some(idx as u32);
        }
        Self { query, captures }
    }

    /// The capture index for a `<kind>.<span>` pair, if this query defines it.
    pub(crate) fn capture_index(&self, kind: ObjectKind, span: ObjectSpan) -> Option<u32> {
        self.captures[kind as usize][span as usize]
    }

    /// Whether this query defines a `<kind>.<span>` capture. Test-only —
    /// the collection paths want the index itself, not just its presence,
    /// so they call [`Self::capture_index`]; this exists because
    /// `.capture_index(..).is_some()` reads poorly in an assertion.
    #[cfg(test)]
    pub(crate) fn defines(&self, kind: ObjectKind, span: ObjectSpan) -> bool {
        self.capture_index(kind, span).is_some()
    }
}

// ── ObjectSpans ────────────────────────────────────────────────────────────

use std::sync::Arc;

use hume_editing::grapheme::prev_grapheme_boundary;
use hume_editing::text::BufferText;
use streaming_iterator::StreamingIterator;

use crate::highlight::RopeProvider;
use crate::layers::{SyntaxLayer, SyntaxLayers};

/// A structural object's captured region, hull-collected from a
/// `textobjects.scm` match and merged with every other match across a
/// buffer's syntax layers: a sorted, deduplicated list of inclusive char
/// spans. Owned rather than an iterator over the tree — `hume-editor` needs
/// `&state.buffers` and `&mut state.panes.state` at once when it applies the
/// resulting selection, so the tree borrow this collects from must end
/// before that, and N cursors × `count` navigation steps then probe a
/// vector instead of re-running the query per step.
///
/// `Default` is the empty set: a buffer with no syntax attached (no grammar,
/// or the first parse hasn't landed) has no layers to collect from, and the
/// editor's dispatch path needs a probe target that answers "no object
/// here" through the same `enclosing`/`adjacent` calls every other buffer
/// uses, rather than a second, `Option`-shaped "nothing to collect" case.
#[derive(Default)]
pub struct ObjectSpans {
    /// Inclusive `(start, end)` char spans, sorted by `(start, Reverse(end))`
    /// and deduplicated — `adjacent`'s `partition_point` walk depends on this
    /// exact ordering. `enclosing` is a full linear scan and doesn't need
    /// it, but keeps the same sorted-and-deduplicated data rather than a
    /// second representation.
    spans: Vec<(usize, usize)>,
}

impl ObjectSpans {
    /// Every `<kind>.<span>` object across every layer whose bundle defines
    /// that capture, merged into one list.
    ///
    /// There is no innermost-layer walk and no "does this layer cover the
    /// cursor" test: an injected layer's captured nodes always lie inside
    /// the parent node that hosts the injection, so `enclosing`'s
    /// smallest-span and `adjacent`'s nearest-start already prefer the
    /// innermost object once every layer's spans are merged into one list —
    /// and a layer without a `textobjects` query (Rust's `comment`
    /// injection, markdown prose) simply contributes nothing. The outward
    /// fallback to an enclosing language's own objects is a consequence of
    /// the merge, not a mechanism this function implements.
    pub fn collect(
        layers: &SyntaxLayers,
        text: &BufferText,
        kind: ObjectKind,
        span: ObjectSpan,
    ) -> Self {
        Self::collect_with(layers, text, |query| query.capture_index(kind, span))
    }

    /// Navigation spans for `kind`: per layer, the first span in priority
    /// order that layer's query defines — Helix's rule that `.movement`
    /// exists precisely for the languages where `.around` is a poor
    /// navigation target (a whole function body vs. just its name).
    ///
    /// `Parameter` reorders rather than following the default priority:
    /// `Inside` first, since Helix's `parameter.around` hull is the argument
    /// *plus its trailing comma* — a wart `m i a` / `m a a` reject for
    /// selection (`around_from_inner` recomputes the separator itself) —
    /// while `parameter.inside` is exactly the span `m i a` selects, so
    /// `goto-next-argument` lands on that same trimmed span. `Movement` and
    /// `Around` stay as fallbacks (reordered, not dropped) for a query that
    /// defines only one of them — a grammar with `@parameter.around` but no
    /// `@parameter.inside` still gets a navigable span rather than a silent
    /// no-op.
    pub fn collect_for_navigation(
        layers: &SyntaxLayers,
        text: &BufferText,
        kind: ObjectKind,
    ) -> Self {
        const DEFAULT_PRIORITY: [ObjectSpan; 3] =
            [ObjectSpan::Movement, ObjectSpan::Around, ObjectSpan::Inside];
        const PARAMETER_PRIORITY: [ObjectSpan; 3] =
            [ObjectSpan::Inside, ObjectSpan::Movement, ObjectSpan::Around];
        let priority = if kind == ObjectKind::Parameter {
            PARAMETER_PRIORITY
        } else {
            DEFAULT_PRIORITY
        };
        Self::collect_with(layers, text, |query| {
            priority
                .into_iter()
                .find_map(|span| query.capture_index(kind, span))
        })
    }

    /// [`Self::collect`] / [`Self::collect_for_navigation`] as `selector`
    /// asks, memoized on `layers`.
    ///
    /// The entry point every structural command goes through. Collection
    /// walks each layer's whole tree — `collect_hulls` cannot clip with
    /// `set_byte_range` without truncating grouped hulls — so repeating a
    /// command (key repeat on `goto-next-*`, a macro or `.`-repeat step)
    /// would otherwise re-walk an unchanged tree every keypress.
    ///
    /// The memo lives on `SyntaxLayers` and dies with it; see
    /// `SyntaxLayers::textobject_memo` for why that placement is what makes
    /// invalidation total. The two collectors below stay public, pure and
    /// uncached — they are the independent oracle this path is tested
    /// against.
    ///
    /// Not in tension with `highlight.rs`'s documented refusal to cache
    /// spans: that is the per-line highlight query, which `set_byte_range`
    /// already clips to `O(tree depth + line)`. This one is unclipped and
    /// whole-tree.
    pub fn for_selector(
        layers: &SyntaxLayers,
        text: &BufferText,
        selector: SpanSelector,
    ) -> Arc<Self> {
        let mut memo = layers
            .textobject_memo
            .lock()
            .expect("textobject memo lock poisoned");
        if let Some((cached, spans)) = memo.as_ref()
            && *cached == selector
        {
            return Arc::clone(spans);
        }
        let spans = Arc::new(match selector {
            SpanSelector::Exact(kind, span) => Self::collect(layers, text, kind, span),
            SpanSelector::Navigation(kind) => Self::collect_for_navigation(layers, text, kind),
        });
        *memo = Some((selector, Arc::clone(&spans)));
        spans
    }

    /// Shared walk behind [`Self::collect`] and
    /// [`Self::collect_for_navigation`]: every layer whose bundle defines a
    /// textobjects query, hulled at the capture index `pick` picks for that
    /// query — a layer whose `pick` returns `None` contributes nothing.
    fn collect_with(
        layers: &SyntaxLayers,
        text: &BufferText,
        pick: impl Fn(&TextObjectsQuery) -> Option<u32>,
    ) -> Self {
        let mut spans = Vec::new();
        for layer in &layers.layers {
            if let Some(query) = layer.bundle.textobjects.as_ref()
                && let Some(idx) = pick(query)
            {
                collect_hulls(query, idx, layer, text, &mut spans);
            }
        }
        Self::finish(spans)
    }

    fn finish(mut spans: Vec<(usize, usize)>) -> Self {
        spans.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        spans.dedup();
        Self { spans }
    }

    /// The smallest span containing `pos` (`start <= pos <= end`).
    pub fn enclosing(&self, pos: usize) -> Option<(usize, usize)> {
        self.spans
            .iter()
            .copied()
            .filter(|&(start, end)| start <= pos && pos <= end)
            .min_by_key(|&(start, end)| end - start)
    }

    /// The next/previous object relative to `pos`.
    ///
    /// **Start-keyed in both directions** — not `end` for the backward
    /// case, as Helix does: a backward press from inside an object must
    /// land on that object's own start first (Vim `[m`), then walk further
    /// back on a repeat. Keying backward on `end < pos` can never select
    /// the object currently enclosing the cursor, since its end is `>= pos`
    /// by definition.
    ///
    /// `Forward`: smallest `start > pos`, ties -> largest `end`.
    /// `Backward`: largest `start < pos`, ties -> largest `end`.
    pub fn adjacent(&self, pos: usize, dir: Direction) -> Option<(usize, usize)> {
        match dir {
            Direction::Forward => {
                // First span past every `start <= pos` entry. Since ties
                // are pre-sorted by descending `end`, that span is already
                // the largest-end winner within its start.
                let idx = self.spans.partition_point(|&(start, _)| start <= pos);
                self.spans.get(idx).copied()
            }
            Direction::Backward => {
                // First span with `start >= pos`; step back one to the
                // largest `start < pos`, then walk to the first span
                // sharing that start (the descending-`end` sort puts the
                // largest-end tie-break winner there).
                let idx = self.spans.partition_point(|&(start, _)| start < pos);
                let (target_start, _) = *self.spans[..idx].last()?;
                let run_start =
                    self.spans[..idx].partition_point(|&(start, _)| start < target_start);
                self.spans.get(run_start).copied()
            }
        }
    }
}

/// The hull — `min(start_byte) ‥ max(end_byte)` — of every node `m` captured
/// under `capture_idx`, **iff those nodes describe one contiguous region**;
/// `None` otherwise (an empty capture, or a non-contiguous one — see below).
///
/// A quantified capture (`(attribute_item)* @function.around`, `(line_comment)+
/// @comment.around`) genuinely can span several nodes for one real object —
/// a function's leading attributes, a run of line comments — and hulling
/// those is exactly what a grouped Helix pattern is written to mean. But an
/// *unanchored* quantifier (no `.` between two of its repetitions, or
/// between its last repetition and what follows) lets tree-sitter match
/// across unrelated intervening siblings: the rust `test.around` pattern's
/// `[(attribute_item)|(line_comment)]*` group, with no anchor before it, can
/// skip clean over one `#[test] fn`'s whole body and latch onto the *next*
/// test's own `#[test]` attribute, reporting one match whose captured nodes
/// span both tests. Hulling that blindly silently selects text the query
/// never actually described as one object.
///
/// So two consecutively captured nodes must be contiguous to hull together:
/// either they overlap/nest (a capture applied to both a parent and a
/// child, or twice to the same node, as `parameter.around`'s wrapping group
/// capture and its inner `","?` do), or the second is literally the first's
/// `next_sibling()` — the same adjacency tree-sitter's own `.` anchor
/// enforces, checked here only across a quantifier's *own* unanchored gaps.
/// A match failing this is **dropped whole, never trimmed** to its
/// contiguous prefix: trimming would still fabricate an object out of
/// content the query never grouped — the bogus `test.around` match's
/// trailing run is `[a later attribute, that later fn]`, and keeping even
/// that would tag an unrelated function as a test.
fn capture_hull(m: &tree_sitter::QueryMatch, capture_idx: u32) -> Option<(usize, usize)> {
    let mut nodes = m.nodes_for_capture_index(capture_idx);
    let first = nodes.next()?;
    let mut prev = first;
    let (mut hull_start, mut hull_end) = (first.start_byte(), first.end_byte());
    for node in nodes {
        debug_assert!(
            node.start_byte() >= prev.start_byte(),
            "tree-sitter yields one capture's nodes in document order"
        );
        let contiguous = node.start_byte() <= hull_end
            || prev.next_sibling().is_some_and(|sib| sib.id() == node.id());
        if !contiguous {
            return None;
        }
        hull_start = hull_start.min(node.start_byte());
        hull_end = hull_end.max(node.end_byte());
        prev = node;
    }
    Some((hull_start, hull_end))
}

/// Run `query`'s matches over `layer`'s tree and, for every match that
/// captures `capture_idx`, push [`capture_hull`]'s result as an inclusive
/// char span. A match without the capture, or whose captured nodes aren't
/// contiguous, contributes nothing; so does a zero-width hull (a `MISSING`
/// node standing in for absent syntax).
///
/// `set_byte_range` is deliberately never used here, unlike the highlighter:
/// the cursor prunes children outside its range, which truncates a grouped
/// hull — the trailing comma a `parameter.around` pattern captures after
/// the argument, the leading attributes a `function.around` pattern
/// captures before the function — rather than merely skipping matches that
/// don't touch a queried region. So this always walks the whole tree.
fn collect_hulls(
    query: &TextObjectsQuery,
    capture_idx: u32,
    layer: &SyntaxLayer,
    text: &BufferText,
    out: &mut Vec<(usize, usize)>,
) {
    let mut cursor = tree_sitter::QueryCursor::new();
    let root = layer.tree.root_node();
    let mut matches = cursor.matches(&query.query, root, RopeProvider(text.rope()));
    while let Some(m) = matches.next() {
        let Some((start_byte, end_byte)) = capture_hull(m, capture_idx) else {
            continue;
        };
        if start_byte >= end_byte {
            continue; // zero-width hull (MISSING nodes) — not a real object
        }
        // A stale tree (an edit recorded but not yet baked/reparsed) would
        // let a node's byte range run past the live buffer's own length —
        // `Syntax::ensure_current` makes that impossible by construction, so
        // a violation here is a bug, not a case to paper over silently.
        debug_assert!(
            end_byte <= text.len_bytes(),
            "text-object span end {end_byte} exceeds buffer length {} — tree is stale",
            text.len_bytes()
        );
        let start = text.byte_to_char(start_byte);
        let end_exclusive = text.byte_to_char(end_byte);
        let end = prev_grapheme_boundary(text, end_exclusive);
        // The byte-space guard above doesn't survive the grapheme-boundary
        // step: a hull whose byte range covers only a combining mark or ZWJ
        // continuation (its own token in some grammars) converts to a
        // one-char span, and stepping back to that cluster's start can land
        // `end` before `start` — `enclosing`'s `end - start` would then
        // underflow. Same "not a real object" treatment as the byte-space
        // degenerate case above.
        if end < start {
            continue;
        }
        out.push((start, end));
    }
}

#[cfg(test)]
mod tests;
