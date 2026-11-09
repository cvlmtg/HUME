use std::ops::Range;

use hume_rope::line::ContentLine;
use streaming_iterator::StreamingIterator;
use tree_sitter::{CaptureQuantifier, Node, QueryCursor, Tree};

use crate::highlight::RopeProvider;
use crate::textobjects::{ObjectKind, ObjectSpan, TextObjectsQuery};

/// The query whose matches are the scopes a pane shows the first line of
/// above its text once that line has scrolled off.
pub struct ContextQuery {
    query: tree_sitter::Query,
    scopes: Scopes,
}

/// How a [`ContextQuery`] match names a scope and the node heading it.
enum Scopes {
    /// `context.scm`: `@context` captures a scope's node; `@context.start`
    /// optionally names the node whose first line is the scope's header
    /// instead.
    Context {
        context: Option<u32>,
        start: Option<u32>,
    },
    /// `textobjects.scm`'s `function.around` and `class.around` captures,
    /// each headed by the captured node holding the line's start, so a
    /// leading grouped node (an attribute above a function) never heads one.
    TextObjects { around: [Option<u32>; 2] },
}

impl ContextQuery {
    /// A compiled `context.scm`.
    pub(crate) fn new(query: tree_sitter::Query) -> Self {
        let context = query.capture_index_for_name("context");
        let start = query.capture_index_for_name("context.start");
        Self {
            query,
            scopes: Scopes::Context { context, start },
        }
    }

    /// The scopes of a grammar without `context.scm`: its function and class
    /// text objects, `None` when it defines neither.
    ///
    /// A copy of `textobjects`'s query with every pattern capturing neither
    /// disabled, so a header lookup does not run the other text objects'
    /// patterns on every line.
    pub(crate) fn from_textobjects(textobjects: &TextObjectsQuery) -> Option<Self> {
        let around = [ObjectKind::Function, ObjectKind::Class]
            .map(|kind| textobjects.capture_index(kind, ObjectSpan::Around));
        if around.iter().all(Option::is_none) {
            return None;
        }
        let mut query = textobjects.query().deep_clone();
        for pattern in 0..query.pattern_count() {
            let quantifiers = query.capture_quantifiers(pattern);
            let captures_a_scope = around
                .iter()
                .flatten()
                .any(|&index| quantifiers[index as usize] != CaptureQuantifier::Zero);
            if !captures_a_scope {
                query.disable_pattern(pattern);
            }
        }
        Some(Self {
            query,
            scopes: Scopes::TextObjects { around },
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
        let line_start = bytes.start;
        cursor.set_byte_range(bytes);
        let mut matches = cursor.matches(&self.query, tree.root_node(), RopeProvider(rope));
        while let Some(m) = matches.next() {
            match self.scopes {
                Scopes::Context { context, start } => {
                    let Some(scope) = context.and_then(|c| m.nodes_for_capture_index(c).next())
                    else {
                        continue;
                    };
                    let header = start
                        .and_then(|start| m.nodes_for_capture_index(start).next())
                        .unwrap_or(scope);
                    out.extend(scope_header(header, scope, line));
                }
                Scopes::TextObjects { around } => {
                    for index in around.into_iter().flatten() {
                        let item = m.nodes_for_capture_index(index).find(|node| {
                            node.start_byte() <= line_start && line_start < node.end_byte()
                        });
                        out.extend(item.and_then(|item| scope_header(item, item, line)));
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
fn scope_header(header: Node, scope: Node, line: ContentLine) -> Option<ContentLine> {
    let header = ContentLine::new(header.start_position().row);
    (header < line && line.index() < scope.end_position().row).then_some(header)
}

#[cfg(test)]
mod tests;
