use std::ops::Range;

use hume_rope::line::ContentLine;
use streaming_iterator::StreamingIterator;
use tree_sitter::{CaptureQuantifier, Point, QueryCursor, Tree};

use crate::highlight::RopeProvider;
use crate::textobjects::{ObjectKind, ObjectPattern, ObjectSpan, TextObjectsQuery};

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
    /// `context.scm`: `@context` captures a scope's node; `@context.start`
    /// optionally names the node whose first line is the scope's header
    /// instead.
    Context { context: u32, start: Option<u32> },
    /// `textobjects.scm`'s `function.around` and `class.around` objects,
    /// each captured node and each made range a scope headed by its own
    /// first line.
    TextObjects {
        around: [Option<u32>; 2],
        patterns: Vec<ObjectPattern>,
    },
}

/// The text objects whose `around` span is a scope.
const SCOPE_KINDS: [ObjectKind; 2] = [ObjectKind::Function, ObjectKind::Class];

impl ContextQuery {
    /// A compiled `context.scm`, `None` when it captures no `@context`.
    pub(crate) fn new(query: tree_sitter::Query) -> Option<Self> {
        let context = query.capture_index_for_name("context")?;
        let start = query.capture_index_for_name("context.start");
        Some(Self {
            query,
            scopes: Scopes::Context { context, start },
        })
    }

    /// The scopes of a grammar without `context.scm`: its function and class
    /// text objects, `None` when it defines neither.
    ///
    /// A copy of `textobjects`'s query with every pattern that neither
    /// captures nor makes one disabled, so a header lookup does not run the
    /// other text objects' patterns on every line.
    pub(crate) fn from_textobjects(textobjects: &TextObjectsQuery) -> Option<Self> {
        if !SCOPE_KINDS
            .iter()
            .any(|&kind| textobjects.defines(kind, ObjectSpan::Around))
        {
            return None;
        }
        let around = SCOPE_KINDS.map(|kind| textobjects.capture_index(kind, ObjectSpan::Around));
        let patterns = textobjects.patterns();
        let mut query = textobjects.query().deep_clone();
        for (pattern, directives) in patterns.iter().enumerate() {
            let quantifiers = query.capture_quantifiers(pattern);
            let captures_a_scope = around
                .iter()
                .flatten()
                .any(|&index| quantifiers[index as usize] != CaptureQuantifier::Zero);
            let makes_a_scope = SCOPE_KINDS
                .iter()
                .any(|&kind| directives.makes(kind, ObjectSpan::Around));
            if !captures_a_scope && !makes_a_scope {
                query.disable_pattern(pattern);
            }
        }
        Some(Self {
            query,
            scopes: Scopes::TextObjects {
                around,
                patterns: patterns.to_vec(),
            },
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
                &Scopes::Context { context, start } => {
                    let Some(scope) = m.nodes_for_capture_index(context).next() else {
                        continue;
                    };
                    let scope = Scope::of(scope.range());
                    let header = start
                        .and_then(|start| m.nodes_for_capture_index(start).next())
                        .map_or(scope.header, |start| {
                            ContentLine::new(start.start_position().row)
                        });
                    f(Scope { header, ..scope });
                }
                Scopes::TextObjects { around, patterns } => {
                    let pattern = &patterns[m.pattern_index];
                    for &index in around.iter().flatten() {
                        m.nodes_for_capture_index(index)
                            .filter_map(|node| pattern.moved(index, node.range(), rope))
                            .for_each(|range| f(Scope::of(range)));
                    }
                    for kind in SCOPE_KINDS {
                        pattern.made(kind, ObjectSpan::Around, m, |range| f(Scope::of(range)));
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
