use std::ops::Range;

use hume_rope::line::ContentLine;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, QueryCursor, Tree};

use crate::highlight::RopeProvider;
use crate::textobjects::{ObjectKind, ObjectSpan, TextObjectsQuery};

/// A compiled `context.scm` query: the scopes whose first line a pane shows
/// above its text once that line has scrolled off. `@context` captures a
/// scope's node; `@context.start` optionally names the node whose first line
/// is the scope's header instead.
pub struct ContextQuery {
    query: tree_sitter::Query,
    context: Option<u32>,
    start: Option<u32>,
}

impl ContextQuery {
    pub(crate) fn new(query: tree_sitter::Query) -> Self {
        let context = query.capture_index_for_name("context");
        let start = query.capture_index_for_name("context.start");
        Self {
            query,
            context,
            start,
        }
    }

    /// Push the header line of every `@context` scope of `tree` containing
    /// `line`, whose bytes are `bytes`, onto `out`, in match order.
    pub(crate) fn header_lines(
        &self,
        cursor: &mut QueryCursor,
        tree: &Tree,
        rope: &ropey::Rope,
        bytes: Range<usize>,
        line: ContentLine,
        out: &mut Vec<ContentLine>,
    ) {
        let Some(context) = self.context else {
            return;
        };
        cursor.set_byte_range(bytes);
        let mut matches = cursor.matches(&self.query, tree.root_node(), RopeProvider(rope));
        while let Some(m) = matches.next() {
            let Some(scope) = m.nodes_for_capture_index(context).next() else {
                continue;
            };
            let header = self
                .start
                .and_then(|start| m.nodes_for_capture_index(start).next())
                .unwrap_or(scope);
            out.extend(scope_header(header, scope, line));
        }
    }
}

/// [`ContextQuery::header_lines`] for a grammar without `context.scm`: its
/// `function.around` and `class.around` captures are the scopes, each headed
/// by the captured node holding the line's start, so a leading grouped node
/// (an attribute above a function) never heads one.
///
/// Clipped to `bytes` like the context query and run on the caller's cursor:
/// it neither walks the whole tree nor reads the structural commands' memo.
pub(crate) fn textobject_header_lines(
    query: &TextObjectsQuery,
    cursor: &mut QueryCursor,
    tree: &Tree,
    rope: &ropey::Rope,
    bytes: Range<usize>,
    line: ContentLine,
    out: &mut Vec<ContentLine>,
) {
    let indices = [ObjectKind::Function, ObjectKind::Class]
        .map(|kind| query.capture_index(kind, ObjectSpan::Around));
    if indices.iter().all(Option::is_none) {
        return;
    }
    let line_start = bytes.start;
    cursor.set_byte_range(bytes);
    let mut matches = cursor.matches(query.query(), tree.root_node(), RopeProvider(rope));
    while let Some(m) = matches.next() {
        for index in indices.into_iter().flatten() {
            let item = m
                .nodes_for_capture_index(index)
                .find(|node| node.start_byte() <= line_start && line_start < node.end_byte());
            out.extend(item.and_then(|item| scope_header(item, item, line)));
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
