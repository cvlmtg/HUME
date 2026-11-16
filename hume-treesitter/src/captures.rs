//! Captures of a caller-supplied query over every layer of one grammar: the
//! nodes a plugin asks for by name, as char ranges.

use std::cmp::Reverse;
use std::sync::Arc;

use hume_rope::line::{ContentLine, join_line_ranges};
use hume_rope::offset::{CharOffset, ExclusiveRange};
use ropey::Rope;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, QueryError};

use crate::directives::{PatternDirectives, parse_patterns};
use crate::highlight::RopeProvider;
use crate::layers::SyntaxLayers;
use crate::registry::GrammarBundle;

/// A query compiled by one `GrammarBundle`. It reads only the layers parsed
/// with that bundle, so it never runs over a tree of another grammar build.
pub struct CaptureQuery {
    query: Query,
    /// Capture names by capture index.
    names: Vec<Arc<str>>,
    /// The `config_gen` of the bundle that compiled it.
    config_gen: u32,
}

/// A captured node, `range` in chars of the text its layer was parsed from.
/// `name` indexes [`CaptureQuery::names`] of the query that produced it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Capture {
    pub name: u32,
    pub range: ExclusiveRange<CharOffset>,
}

/// A capture query's pattern, which takes no predicate or directive of the
/// engine's.
#[derive(Default)]
struct NoDirectives;

impl PatternDirectives for NoDirectives {}

impl CaptureQuery {
    /// `source` compiled against `bundle`'s grammar. A pattern using a
    /// predicate or directive tree-sitter leaves to the engine fails it,
    /// with the error at the first such pattern's start.
    pub(crate) fn new(bundle: &GrammarBundle, source: &str) -> Result<Self, QueryError> {
        let mut query = Query::new(bundle.grammar.language(), source)?;
        let mut skipped = Vec::new();
        parse_patterns::<NoDirectives>(&mut query, source, &mut skipped);
        if let Some(error) = skipped.into_iter().next() {
            return Err(error);
        }
        let names = query
            .capture_names()
            .iter()
            .map(|&n| Arc::from(n))
            .collect();
        Ok(Self {
            query,
            names,
            config_gen: bundle.config_gen,
        })
    }

    /// The capture names, by [`Capture::name`].
    pub fn names(&self) -> &[Arc<str>] {
        &self.names
    }

    /// Replace `out` with the captures of the matches over each layer of
    /// `layers` parsed with this query's bundle over the lines of `lines`,
    /// injected layers included, predicates evaluated against `rope`. The
    /// ranges may come in any order and overlap; no line is walked twice.
    ///
    /// `out` is sorted by start, then by end descending, so an enclosing node
    /// precedes the nodes it starts with, then by name. Each distinct capture
    /// is there once, however many matches or ranges reach it.
    ///
    /// A match counts when the node its pattern is rooted at intersects one
    /// of the ranges, with every capture of the match, inside the lines or
    /// not. A pattern rooted at a table returns the whole table, one rooted
    /// at a delimiter only the delimiters on the lines.
    pub fn collect(
        &self,
        layers: &SyntaxLayers,
        rope: &Rope,
        lines: &[ExclusiveRange<ContentLine>],
        out: &mut Vec<Capture>,
    ) {
        out.clear();
        let mut cursor = QueryCursor::new();
        for lines in join_line_ranges(lines.iter().copied()) {
            let (bytes, covering) = layers.covering_lines(rope, lines);
            cursor.set_byte_range(bytes);
            for layer in covering.filter(|layer| layer.bundle.config_gen == self.config_gen) {
                let mut matches =
                    cursor.matches(&self.query, layer.tree.root_node(), RopeProvider(rope));
                while let Some(m) = matches.next() {
                    out.extend(m.captures().iter().map(|capture| Capture {
                        name: capture.index,
                        range: ExclusiveRange::new(
                            CharOffset::new(rope.byte_to_char(capture.node.start_byte())),
                            CharOffset::new(rope.byte_to_char(capture.node.end_byte())),
                        ),
                    }));
                }
            }
        }
        out.sort_by_key(|capture| {
            (
                capture.range.start,
                Reverse(capture.range.end),
                capture.name,
            )
        });
        out.dedup();
    }
}

#[cfg(test)]
mod tests;
