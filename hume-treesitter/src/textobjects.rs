//! Tree-sitter structural text objects and navigation: kinds, spans, the
//! per-`(kind, span)` object table a compiled `textobjects.scm` resolves
//! to, and [`ObjectSpans`], which runs that query over a buffer's
//! syntax layers into a sorted list of cluster ranges.
//!
//! Freshness (the tree matches the text before a command runs) is
//! `Syntax::ensure_current`; selection policy (Move/Extend, count,
//! multi-cursor) is `hume-ops`'s `apply_text_object_by_mode` and
//! `apply_object_motion`; this module only collects spans and answers two
//! lookups over them.

// ── ObjectKind / ObjectSpan ──────────────────────────────────────────────

/// Emits an object-kind/span enum, its dense `ALL` slice, and its
/// `capture_name`/`from_capture_name` pair from one variant↦capture-name
/// list. `TextObjectsQuery`'s object table is sized
/// `[[ObjectDef; ObjectSpan::ALL.len()]; ObjectKind::ALL.len()]` and
/// indexed by `kind as usize`/`span as usize`, so a variant added to the enum
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
            /// against this list: a kind here with no commands there
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
        XmlElement = "xml-element",
    }
}

object_enum! {
    /// Which part of an object a capture spans. `Movement` is the queries'
    /// optional navigation-only capture (a function's name node, say),
    /// narrower than `Around`, consumed only by navigation, never by
    /// selection.
    ObjectSpan {
        Inside = "inside",
        Around = "around",
        Movement = "movement",
    }
}

/// Which set of spans a caller wants collected: the whole input to
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

/// The only direction enum for structural navigation. `hume-ops` takes a
/// `backward: bool` at its API boundary (the `apply_word_select`
/// convention) rather than importing this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

/// The object a `<kind>.<span>` name (`@function.inside`, a `#make-range!`
/// name) names, split on its last `.` (`@parameter.inside.extra` would split
/// on the last dot, but no installed query defines such a name). Names that
/// don't parse as `<kind>.<span>` (`@_helper`, `@function.x`) name nothing.
fn object_named(name: &str) -> Option<(ObjectKind, ObjectSpan)> {
    let (kind, span) = name.rsplit_once('.')?;
    Some((
        ObjectKind::from_capture_name(kind)?,
        ObjectSpan::from_capture_name(span)?,
    ))
}

/// A compiled `textobjects.scm` query plus a dense `(kind, span)` table of
/// how it defines each object and each pattern's directives, resolved once,
/// when the grammar loads.
pub struct TextObjectsQuery {
    query: tree_sitter::Query,
    objects: [[ObjectDef; ObjectSpan::ALL.len()]; ObjectKind::ALL.len()],
    patterns: Vec<ObjectPattern>,
}

/// How a [`TextObjectsQuery`] defines one `<kind>.<span>` object.
#[derive(Clone, Copy, Default)]
struct ObjectDef {
    /// The index of its `@<kind>.<span>` capture.
    capture: Option<u32>,
    /// Whether a pattern's `#make-range!` names it.
    made: bool,
}

impl TextObjectsQuery {
    /// A pattern using any engine predicate or directive but `#make-range!`
    /// and `#offset!`, or either of those with arguments it can't apply, is
    /// skipped, its error in `source` pushed to `skipped`.
    pub(crate) fn new(
        mut query: tree_sitter::Query,
        source: &str,
        skipped: &mut Vec<QueryError>,
    ) -> Self {
        let patterns: Vec<ObjectPattern> = parse_patterns(&mut query, source, skipped);
        let mut objects = [[ObjectDef::default(); ObjectSpan::ALL.len()]; ObjectKind::ALL.len()];
        for (idx, name) in query.capture_names().iter().enumerate() {
            if let Some((kind, span)) = object_named(name) {
                objects[kind as usize][span as usize].capture = Some(idx as u32);
            }
        }
        for &(kind, span, _) in patterns.iter().flat_map(|pattern| &pattern.ranges) {
            objects[kind as usize][span as usize].made = true;
        }
        Self {
            query,
            objects,
            patterns,
        }
    }

    /// The compiled query, for code that runs its matches itself.
    pub(crate) fn query(&self) -> &tree_sitter::Query {
        &self.query
    }

    fn object(&self, kind: ObjectKind, span: ObjectSpan) -> ObjectDef {
        self.objects[kind as usize][span as usize]
    }

    /// The capture index for a `<kind>.<span>` pair, if this query captures it.
    fn capture_index(&self, kind: ObjectKind, span: ObjectSpan) -> Option<u32> {
        self.object(kind, span).capture
    }

    /// Whether this query defines a `<kind>.<span>` object, by a capture or
    /// by a `#make-range!`.
    pub(crate) fn defines(&self, kind: ObjectKind, span: ObjectSpan) -> bool {
        let object = self.object(kind, span);
        object.capture.is_some() || object.made
    }

    /// Whether `pattern` captures or makes a `kind.span` object.
    pub(crate) fn pattern_defines(
        &self,
        pattern: usize,
        kind: ObjectKind,
        span: ObjectSpan,
    ) -> bool {
        self.capture_index(kind, span).is_some_and(|capture| {
            self.query.capture_quantifiers(pattern)[capture as usize] != CaptureQuantifier::Zero
        }) || self.patterns[pattern].makes(kind, span)
    }

    /// Call `f` with each region match `m` makes the `kind.span` object of:
    /// each region `captured` gives for its capture, moved by the pattern's
    /// `#offset!` of it, and each `#make-range!` naming it. `m` is a match
    /// of this query or of a [`tree_sitter::Query::deep_clone`] of it.
    pub(crate) fn regions<I: IntoIterator<Item = tree_sitter::Range>>(
        &self,
        m: &QueryMatch<'_, '_>,
        kind: ObjectKind,
        span: ObjectSpan,
        rope: &Rope,
        captured: impl FnOnce(u32) -> I,
        mut f: impl FnMut(tree_sitter::Range),
    ) {
        let pattern = &self.patterns[m.pattern_index];
        if let Some(capture) = self.capture_index(kind, span) {
            captured(capture)
                .into_iter()
                .filter_map(|region| pattern.moved(capture, region, rope))
                .for_each(&mut f);
        }
        pattern
            .ranges_naming(kind, span)
            .filter_map(|range| range.region(m))
            .for_each(f);
    }
}

/// The directives one `textobjects.scm` pattern carries.
#[derive(Default)]
struct ObjectPattern {
    offsets: Vec<Offset>,
    ranges: Vec<(ObjectKind, ObjectSpan, MakeRange)>,
}

/// `#offset! @capture 0 start 0 end`: the capture's region moves `start`
/// bytes at its start and `end` bytes at its end, each along its own line.
struct Offset {
    capture: u32,
    start: isize,
    end: isize,
}

const OFFSET_ARGS: &str = "#offset! takes a text object capture and four numbers";

impl PatternDirectives for ObjectPattern {
    fn predicate(
        &mut self,
        capture_names: &[&str],
        predicate: &QueryPredicate,
    ) -> Result<(), String> {
        match (&*predicate.operator, &*predicate.args) {
            ("make-range!", args) => {
                let (name, range) = MakeRange::parse(args)?;
                let (kind, span) = object_named(name)
                    .ok_or_else(|| format!("#make-range! \"{name}\" names no text object"))?;
                self.ranges.push((kind, span, range));
            }
            (
                "offset!",
                [
                    Arg::Capture(capture),
                    Arg::String(start_row),
                    Arg::String(start),
                    Arg::String(end_row),
                    Arg::String(end),
                ],
            ) if object_named(capture_names[*capture as usize]).is_some() => {
                let number = |arg: &str| arg.parse::<isize>().map_err(|_| OFFSET_ARGS.to_owned());
                if number(start_row)? != 0 || number(end_row)? != 0 {
                    return Err(
                        "#offset! moves only along a line: its row offsets are 0".to_owned()
                    );
                }
                if self.offsets.iter().any(|offset| offset.capture == *capture) {
                    return Err("#offset! moves a capture once per pattern".to_owned());
                }
                self.offsets.push(Offset {
                    capture: *capture,
                    start: number(start)?,
                    end: number(end)?,
                });
            }
            ("offset!", _) => return Err(OFFSET_ARGS.to_owned()),
            (operator, _) => return Err(unsupported(operator)),
        }
        Ok(())
    }
}

impl ObjectPattern {
    /// Whether a `#make-range!` of this pattern names `kind.span`.
    fn makes(&self, kind: ObjectKind, span: ObjectSpan) -> bool {
        self.ranges_naming(kind, span).next().is_some()
    }

    /// This pattern's `#make-range!`s naming `kind.span`.
    fn ranges_naming(&self, kind: ObjectKind, span: ObjectSpan) -> impl Iterator<Item = MakeRange> {
        self.ranges
            .iter()
            .filter(move |&&(k, s, _)| (k, s) == (kind, span))
            .map(|&(_, _, range)| range)
    }

    /// `region` of `capture`, moved by this pattern's `#offset!` of it.
    /// `None` when a bound would leave its line or the text, or the region
    /// would be empty.
    fn moved(
        &self,
        capture: u32,
        region: tree_sitter::Range,
        rope: &Rope,
    ) -> Option<tree_sitter::Range> {
        let Some(offset) = self.offsets.iter().find(|offset| offset.capture == capture) else {
            return Some(region);
        };
        let (start_byte, start_point) =
            along_line(region.start_byte, region.start_point, offset.start, rope)?;
        let (end_byte, end_point) =
            along_line(region.end_byte, region.end_point, offset.end, rope)?;
        (start_byte < end_byte).then_some(tree_sitter::Range {
            start_byte,
            end_byte,
            start_point,
            end_point,
        })
    }
}

/// `byte`, at `point`, moved `delta` bytes along its line. `None` when that
/// passes a line break or leaves the text.
fn along_line(byte: usize, point: Point, delta: isize, rope: &Rope) -> Option<(usize, Point)> {
    let column = point.column.checked_add_signed(delta)?;
    let moved = byte.checked_add_signed(delta)?;
    let crosses_break = moved > rope.len_bytes()
        || rope
            .bytes_at(byte)
            .take(moved.saturating_sub(byte))
            .any(|b| b == b'\n');
    (!crosses_break).then_some((moved, Point { column, ..point }))
}

// ── ObjectSpans ────────────────────────────────────────────────────────────

use std::sync::Arc;

use hume_editing::text::BufferText;
use hume_rope::cluster::{ClusterRange, ClusterStart};
use ropey::Rope;
use streaming_iterator::StreamingIterator;
use tree_sitter::{
    CaptureQuantifier, Point, QueryError, QueryMatch, QueryPredicate, QueryPredicateArg as Arg,
};

use crate::directives::{MakeRange, PatternDirectives, capture_hull, parse_patterns, unsupported};
use crate::highlight::RopeProvider;
use crate::layers::{SyntaxLayer, SyntaxLayers};

/// A structural object's captured region, hull-collected from a
/// `textobjects.scm` match and merged with every other match across a
/// buffer's syntax layers: a sorted, deduplicated list of cluster ranges.
/// Owned rather than an iterator over the tree: `hume-editor` needs
/// `&state.buffers` and `&mut state.panes.entries` at once when it applies the
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
    /// Cluster ranges, sorted by `(start, Reverse(end))` and
    /// deduplicated; `adjacent`'s `partition_point` walk depends on this
    /// exact ordering. `enclosing` is a full linear scan and doesn't need
    /// it, but keeps the same sorted-and-deduplicated data rather than a
    /// second representation.
    spans: Vec<ClusterRange>,
    /// Whether any layer's query defines the capture these spans were
    /// collected for, matches or not. `hume-editor` reads it to pick a
    /// lexical fallback per buffer rather than per probe.
    grammar_defines: bool,
}

impl ObjectSpans {
    /// Every `<kind>.<span>` object across every layer whose bundle defines
    /// that capture, merged into one list.
    ///
    /// There is no innermost-layer walk and no "does this layer cover the
    /// cursor" test: an injected layer's captured nodes always lie inside
    /// the parent node that hosts the injection, so `enclosing`'s
    /// smallest-span and `adjacent`'s nearest-start already prefer the
    /// innermost object once every layer's spans are merged into one list,
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
        Self::collect_with(layers, text, kind, |query| {
            query.defines(kind, span).then_some(span)
        })
    }

    /// Navigation spans for `kind`: per layer, the first span in priority
    /// order that layer's query defines. `.movement` exists for the
    /// languages where `.around` is a poor navigation target (a whole
    /// function body vs. just its name).
    ///
    /// `Parameter` reorders rather than following the default priority:
    /// `Inside` first, since the queries' `parameter.around` hull is the
    /// argument *plus its trailing comma*, a wart `m i a` / `m a a` reject for
    /// selection (`around_from_inner` recomputes the separator itself),
    /// while `parameter.inside` is exactly the span `m i a` selects, so
    /// `goto-next-argument` lands on that same trimmed span. `Movement` and
    /// `Around` stay as fallbacks (reordered, not dropped) for a query that
    /// defines only one of them: a grammar with `@parameter.around` but no
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
        Self::collect_with(layers, text, kind, |query| {
            priority.into_iter().find(|&span| query.defines(kind, span))
        })
    }

    /// [`Self::collect`] / [`Self::collect_for_navigation`] as `selector`
    /// asks, memoized on `layers`.
    ///
    /// The entry point every structural command goes through. Collection
    /// walks each layer's whole tree (`collect_hulls` cannot clip with
    /// `set_byte_range` without truncating grouped hulls), so repeating a
    /// command (key repeat on `goto-next-*`, a macro or `.`-repeat step)
    /// would otherwise re-walk an unchanged tree every keypress.
    ///
    /// The memo lives on `SyntaxLayers` and dies with it; see
    /// `SyntaxLayers::textobject_memo` for why that placement is what makes
    /// invalidation total. The two collectors below stay public, pure and
    /// uncached. The tests use them as the reference this path is checked
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
    /// textobjects query, collected at the `kind` span `pick` picks for that
    /// query; a layer whose `pick` returns `None` contributes nothing.
    fn collect_with(
        layers: &SyntaxLayers,
        text: &BufferText,
        kind: ObjectKind,
        pick: impl Fn(&TextObjectsQuery) -> Option<ObjectSpan>,
    ) -> Self {
        let mut spans = Vec::new();
        let mut grammar_defines = false;
        for layer in &layers.layers {
            if let Some(query) = layer.bundle.textobjects.as_ref()
                && let Some(span) = pick(query)
            {
                grammar_defines = true;
                collect_hulls(query, kind, span, layer, text, &mut spans);
            }
        }
        Self {
            grammar_defines,
            ..Self::finish(spans)
        }
    }

    fn finish(mut spans: Vec<ClusterRange>) -> Self {
        spans.sort_by(|a, b| a.start().cmp(&b.start()).then(b.end().cmp(&a.end())));
        spans.dedup();
        Self {
            spans,
            grammar_defines: false,
        }
    }

    /// Whether some syntax layer's query defines the object these spans
    /// were collected for, even where it matched nothing. `false` for the
    /// empty default.
    pub fn grammar_defines(&self) -> bool {
        self.grammar_defines
    }

    /// The smallest span containing `pos`.
    pub fn enclosing(&self, pos: ClusterStart) -> Option<ClusterRange> {
        self.spans
            .iter()
            .copied()
            .filter(|span| span.contains(pos))
            .min_by_key(|span| {
                let chars = span.chars();
                chars.end.chars_since(chars.start)
            })
    }

    /// The next/previous object relative to `pos`.
    ///
    /// **Start-keyed in both directions**, not `end` for the backward
    /// case: a backward press from inside an object must land on that
    /// object's own start first, then walk further back on a
    /// repeat. Keying backward on `end < pos` can never select
    /// the object currently enclosing the cursor, since its end is `>= pos`
    /// by definition.
    ///
    /// `Forward`: smallest `start > pos`, ties -> largest `end`.
    /// `Backward`: largest `start < pos`, ties -> largest `end`.
    pub fn adjacent(&self, pos: ClusterStart, dir: Direction) -> Option<ClusterRange> {
        match dir {
            Direction::Forward => {
                // First span past every `start <= pos` entry. Since ties
                // are pre-sorted by descending `end`, that span is already
                // the largest-end winner within its start.
                let idx = self.spans.partition_point(|span| span.start() <= pos);
                self.spans.get(idx).copied()
            }
            Direction::Backward => {
                // First span with `start >= pos`; step back one to the
                // largest `start < pos`, then walk to the first span
                // sharing that start (the descending-`end` sort puts the
                // largest-end tie-break winner there).
                let idx = self.spans.partition_point(|span| span.start() < pos);
                let target_start = self.spans[..idx].last()?.start();
                let run_start =
                    self.spans[..idx].partition_point(|span| span.start() < target_start);
                self.spans.get(run_start).copied()
            }
        }
    }
}

/// Run `query`'s matches over `layer`'s tree and push the clusters holding
/// each region a match makes the `kind.span` object of. A zero-width region
/// (a `MISSING` node standing in for absent syntax) contributes nothing. A
/// region that starts or ends inside a cluster (a grammar that makes a
/// combining mark its own token, an `#offset!` landing inside a character)
/// widens to the whole cluster.
///
/// `set_byte_range` is never used here, unlike the highlighter:
/// the cursor prunes children outside its range, which truncates a grouped
/// hull (the trailing comma a `parameter.around` pattern captures after
/// the argument, the leading attributes a `function.around` pattern
/// captures before the function) rather than merely skipping matches that
/// don't touch a queried region. So this always walks the whole tree.
fn collect_hulls(
    query: &TextObjectsQuery,
    kind: ObjectKind,
    span: ObjectSpan,
    layer: &SyntaxLayer,
    text: &BufferText,
    out: &mut Vec<ClusterRange>,
) {
    let mut cursor = tree_sitter::QueryCursor::new();
    let root = layer.tree.root_node();
    let mut matches = cursor.matches(&query.query, root, RopeProvider(text.rope()));
    while let Some(m) = matches.next() {
        let captured = |capture| capture_hull(m, capture);
        query.regions(m, kind, span, text.rope(), captured, |region| {
            // Callers hand over only a tree parsed from `text`; a stale one
            // could name bytes past its end.
            assert!(
                region.end_byte <= text.len_bytes(),
                "text-object span end {} exceeds buffer length {}: tree is stale",
                region.end_byte,
                text.len_bytes()
            );
            out.extend(text.covering_bytes(region.start_byte..region.end_byte));
        });
    }
}

#[cfg(test)]
mod tests;
