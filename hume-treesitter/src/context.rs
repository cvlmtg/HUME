use std::ops::Range;
use std::sync::Arc;

use hume_rope::line::ContentLine;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Point, QueryCursor, QueryError, QueryPredicate, Tree};

use crate::directives::{MakeRange, PatternDirectives, parse_patterns, unsupported};
use crate::highlight::RopeProvider;
use crate::textobjects::{ObjectKind, ObjectSpan, TextObjectsQuery};

/// The query whose matches are the scopes a pane shows the first line of
/// above its text once that line has scrolled off.
pub struct ContextQuery {
    query: tree_sitter::Query,
    scopes: Scopes,
}

/// A scope a match names: the line heading it and where it ends.
#[derive(Clone, Copy)]
pub(crate) struct Scope {
    pub(crate) header: ContentLine,
    pub(crate) end: Point,
    pub(crate) end_byte: usize,
}

impl Scope {
    /// The scope `range` spans, headed by its own first line.
    pub(crate) fn of(range: tree_sitter::Range) -> Self {
        Self {
            header: ContentLine::new(range.start_point.row),
            end: range.end_point,
            end_byte: range.end_byte,
        }
    }
}

/// How a [`ContextQuery`] match names a scope and the node heading it.
enum Scopes {
    /// `context.scm`: `@context` captures a scope's node and each
    /// `#make-range! "context"` makes one; `@context.start` optionally names
    /// the node whose first line is the scope's header instead.
    Context {
        context: Option<u32>,
        start: Option<u32>,
        patterns: Vec<ContextPattern>,
    },
    /// `textobjects.scm`'s `function.around` and `class.around` objects,
    /// each captured node and each made range a scope headed by its own
    /// first line. [`ContextQuery::query`] is its query with the patterns
    /// naming neither disabled.
    TextObjects(Arc<TextObjectsQuery>),
}

/// The `#make-range! "context"` scopes one `context.scm` pattern makes.
#[derive(Default)]
struct ContextPattern {
    ranges: Vec<MakeRange>,
}

impl PatternDirectives for ContextPattern {
    fn predicate(&mut self, _: &[&str], predicate: &QueryPredicate) -> Result<(), String> {
        if &*predicate.operator != "make-range!" {
            return Err(unsupported(&predicate.operator));
        }
        let (name, range) = MakeRange::parse(&predicate.args)?;
        if name != "context" {
            return Err(format!(
                "#make-range! \"{name}\" names no scope; only \"context\" does"
            ));
        }
        self.ranges.push(range);
        Ok(())
    }
}

/// The text objects whose `around` span is a scope.
const SCOPE_KINDS: [ObjectKind; 2] = [ObjectKind::Function, ObjectKind::Class];

impl ContextQuery {
    /// What a `context.scm` needs to name a scope.
    pub(crate) const NEEDS: &str = r#"a @context capture or a #make-range! "context""#;

    /// A compiled `context.scm`, `None` when it names no scope: no
    /// `@context` capture and no `#make-range! "context"`. Fails, as a
    /// `Predicate` error at the offending pattern's start in `source`, on
    /// any engine predicate or directive but `#make-range! "context"`.
    pub(crate) fn new(query: tree_sitter::Query, source: &str) -> Result<Option<Self>, QueryError> {
        let patterns: Vec<ContextPattern> = parse_patterns(&query, source)?;
        let context = query.capture_index_for_name("context");
        if context.is_none() && patterns.iter().all(|pattern| pattern.ranges.is_empty()) {
            return Ok(None);
        }
        let start = query.capture_index_for_name("context.start");
        Ok(Some(Self {
            query,
            scopes: Scopes::Context {
                context,
                start,
                patterns,
            },
        }))
    }

    /// The scopes of a grammar without `context.scm`: its function and class
    /// text objects, `None` when it defines neither.
    ///
    /// A copy of `textobjects`'s query with every pattern that neither
    /// captures nor makes one disabled, so a header lookup does not run the
    /// other text objects' patterns on every line.
    pub(crate) fn from_textobjects(textobjects: &Arc<TextObjectsQuery>) -> Option<Self> {
        let scope_pattern = |pattern| {
            SCOPE_KINDS
                .iter()
                .any(|&kind| textobjects.pattern_defines(pattern, kind, ObjectSpan::Around))
        };
        let patterns = 0..textobjects.query().pattern_count();
        if !patterns.clone().any(scope_pattern) {
            return None;
        }
        let mut query = textobjects.query().deep_clone();
        for pattern in patterns.filter(|&pattern| !scope_pattern(pattern)) {
            query.disable_pattern(pattern);
        }
        Some(Self {
            query,
            scopes: Scopes::TextObjects(Arc::clone(textobjects)),
        })
    }

    /// Push the header line of every scope of `tree` containing `line`,
    /// whose bytes are `bytes`, onto `out`, in match order.
    pub(crate) fn header_lines(
        &self,
        cursor: &mut QueryCursor,
        tree: &Tree,
        rope: &ropey::Rope,
        bytes: Range<usize>,
        line: ContentLine,
        out: &mut Vec<ContentLine>,
    ) {
        self.for_each_scope(cursor, tree, rope, bytes, |scope| {
            out.extend(scope_header(scope, line));
        });
    }

    /// Call `f` with every scope of `tree` a match over `bytes` names, in
    /// match order.
    ///
    /// A text object's grouped capture (an attribute above a function) names
    /// each of its nodes as a scope of its own, so a caller keeping only
    /// scopes around a line keeps the item, never the node above it.
    pub(crate) fn for_each_scope(
        &self,
        cursor: &mut QueryCursor,
        tree: &Tree,
        rope: &ropey::Rope,
        bytes: Range<usize>,
        mut f: impl FnMut(Scope),
    ) {
        cursor.set_byte_range(bytes);
        let mut matches = cursor.matches(&self.query, tree.root_node(), RopeProvider(rope));
        while let Some(m) = matches.next() {
            match &self.scopes {
                Scopes::Context {
                    context,
                    start,
                    patterns,
                } => {
                    let header = start
                        .and_then(|start| m.nodes_for_capture_index(start).next())
                        .map(|start| ContentLine::new(start.start_position().row));
                    let mut headed = |range| {
                        let scope = Scope::of(range);
                        f(Scope {
                            header: header.unwrap_or(scope.header),
                            ..scope
                        });
                    };
                    if let Some(node) = context.and_then(|c| m.nodes_for_capture_index(c).next()) {
                        headed(node.range());
                    }
                    patterns[m.pattern_index]
                        .ranges
                        .iter()
                        .filter_map(|range| range.region(m))
                        .for_each(headed);
                }
                Scopes::TextObjects(objects) => {
                    for kind in SCOPE_KINDS {
                        let captured =
                            |capture| m.nodes_for_capture_index(capture).map(|node| node.range());
                        objects.regions(m, kind, ObjectSpan::Around, rope, captured, |range| {
                            f(Scope::of(range));
                        });
                    }
                }
            }
        }
    }
}

/// `header`'s first line when `line` is inside `scope` below that line. A
/// scope's last line is outside it, so its header drops once that line (a
/// closing `}`) reaches the header's row. A Markdown section ends at column 0
/// of the next heading, so its own last line stays inside.
fn scope_header(scope: Scope, line: ContentLine) -> Option<ContentLine> {
    (scope.header < line && line.index() < scope.end.row).then_some(scope.header)
}

#[cfg(test)]
mod tests;
