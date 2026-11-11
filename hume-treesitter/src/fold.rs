//! Foldable regions of a syntax tree: a grammar's `folds.scm`, or without
//! one the scopes scope header rows come from.

use std::cmp::Reverse;
use std::ops::Range;
use std::sync::Arc;

use hume_engine::folds::Fold;
use hume_rope::line::ContentLine;
use hume_rope::offset::InclusiveRange;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, QueryCursor, Tree};

use crate::context::ContextQuery;
use crate::highlight::RopeProvider;
use crate::layers::{SyntaxLayer, SyntaxLayers};

/// The query whose matches are a grammar's foldable regions.
pub enum FoldQuery {
    /// `folds.scm`: each `@fold` node is a region.
    Folds {
        query: tree_sitter::Query,
        fold: u32,
    },
    /// Each scope is a region starting at its header line.
    Scopes(Arc<ContextQuery>),
}

impl FoldQuery {
    /// A compiled `folds.scm`, `None` when it captures no `@fold`.
    pub(crate) fn new(query: tree_sitter::Query) -> Option<Self> {
        let fold = query.capture_index_for_name("fold")?;
        Some(Self::Folds { query, fold })
    }

    /// Call `f` with every region of `tree` with a node in `bytes`, in
    /// match order.
    fn regions(
        &self,
        cursor: &mut QueryCursor,
        tree: &Tree,
        rope: &ropey::Rope,
        bytes: Range<usize>,
        f: &mut impl FnMut(Fold),
    ) {
        match self {
            Self::Folds { query, fold } => {
                cursor.set_byte_range(bytes);
                let mut matches = cursor.matches(query, tree.root_node(), RopeProvider(rope));
                while let Some(m) = matches.next() {
                    m.nodes_for_capture_index(*fold)
                        .filter_map(|node| fold_of(node, node, rope))
                        .for_each(&mut *f);
                }
            }
            Self::Scopes(scopes) => scopes.for_each_scope(cursor, tree, rope, bytes, |h, s| {
                fold_of(h, s, rope).into_iter().for_each(&mut *f);
            }),
        }
    }
}

impl SyntaxLayers {
    /// The regions of every layer holding `line`, innermost first, one per
    /// first line.
    pub fn folds_spanning(&self, rope: &ropey::Rope, line: ContentLine, out: &mut Vec<Fold>) {
        out.clear();
        let (bytes, covering) = self.covering_line(rope, line);
        layer_regions(
            &mut QueryCursor::new(),
            covering,
            rope,
            bytes,
            &mut |fold| {
                if fold.spans(line) {
                    out.push(fold);
                }
            },
        );
        canonical(out);
        out.reverse();
    }

    /// Every region of every layer, by first line, one per first line.
    pub fn all_folds(&self, rope: &ropey::Rope, out: &mut Vec<Fold>) {
        out.clear();
        let bytes = 0..rope.len_bytes();
        layer_regions(
            &mut QueryCursor::new(),
            self.layers.iter(),
            rope,
            bytes,
            &mut |fold| {
                out.push(fold);
            },
        );
        canonical(out);
    }

    /// Push the first line of every region of every layer starting in
    /// `lines` onto `out`, run on `cursor` in one query per layer.
    pub(crate) fn fold_starts(
        &self,
        cursor: &mut QueryCursor,
        rope: &ropey::Rope,
        lines: InclusiveRange<ContentLine>,
        out: &mut Vec<ContentLine>,
    ) {
        let (bytes, covering) = self.covering_lines(rope, lines);
        layer_regions(cursor, covering, rope, bytes, &mut |fold| {
            if lines.contains(fold.first()) {
                out.push(fold.first());
            }
        });
    }
}

fn layer_regions<'l>(
    cursor: &mut QueryCursor,
    layers: impl Iterator<Item = &'l SyntaxLayer>,
    rope: &ropey::Rope,
    bytes: Range<usize>,
    f: &mut impl FnMut(Fold),
) {
    for layer in layers {
        if let Some(query) = &layer.bundle.folds {
            query.regions(cursor, &layer.tree, rope, bytes.clone(), f);
        }
    }
}

/// Keep one region per first line, the one hiding the most, sorted by
/// first line: the region a fold line's caret closes is then also the one a
/// line inside it finds.
fn canonical(folds: &mut Vec<Fold>) {
    folds.sort_unstable_by_key(|f| (f.first(), Reverse(f.hidden().end)));
    folds.dedup_by_key(|f| f.first());
}

/// The region from `header`'s first line through `scope`'s last line.
///
/// A scope ending at column 0 ends on the line before (a Markdown section
/// runs to the next heading). Its last line stays shown when it holds only
/// the scope's closing token (`}`, `end`, `)`), possibly followed by more
/// text (`} else {`).
fn fold_of(header: Node, scope: Node, rope: &ropey::Rope) -> Option<Fold> {
    let first = header.start_position().row;
    let end = scope.end_position();
    let ends_at_line_start = end.column == 0 && end.row > first;
    let last = if ends_at_line_start {
        end.row - 1
    } else {
        end.row
    };
    let shows_last = !ends_at_line_start && closes_on_own_line(scope, end.row, rope);
    Fold::new(ContentLine::new(first), ContentLine::new(last), shows_last)
}

/// Whether `scope`'s last token is anonymous, starts on `row`, and only
/// whitespace precedes it there.
fn closes_on_own_line(scope: Node, row: usize, rope: &ropey::Rope) -> bool {
    let end = scope.end_byte();
    let Some(token) = scope.descendant_for_byte_range(end.saturating_sub(1), end) else {
        return false;
    };
    let start = token.start_position();
    let line_start = token.start_byte() - start.column;
    !token.is_named()
        && start.row == row
        && rope
            .byte_slice(line_start..token.start_byte())
            .chars()
            .all(char::is_whitespace)
}

#[cfg(test)]
mod tests;
