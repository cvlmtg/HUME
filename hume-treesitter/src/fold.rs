//! Foldable regions of a syntax tree: a grammar's `folds.scm`, or without
//! one the scopes scope header rows come from.

use std::cmp::Reverse;
use std::ops::Range;
use std::sync::Arc;

use hume_engine::folds::Fold;
use hume_rope::line::ContentLine;
use hume_rope::lines::{is_blank, leading_whitespace_end};
use hume_rope::offset::InclusiveRange;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, QueryCursor, QueryError, QueryPredicate, QueryPredicateArg as Arg, Tree};

use crate::context::{ContextQuery, Scope};
use crate::directives::{PatternDirectives, parse_patterns, unsupported};
use crate::highlight::RopeProvider;
use crate::layers::{SyntaxLayer, SyntaxLayers};

/// The query whose matches are a grammar's foldable regions.
pub enum FoldQuery {
    /// `folds.scm`: each `@fold` node is a region.
    Folds {
        query: tree_sitter::Query,
        fold: u32,
        patterns: Vec<FoldPattern>,
    },
    /// Each scope is a region starting at its header line.
    Scopes(Arc<ContextQuery>),
}

/// The directives one `folds.scm` pattern carries.
#[derive(Default)]
pub struct FoldPattern {
    /// `#trim! @fold`: the region ends at its last non-blank line.
    trim: bool,
    /// `#not-has-parent? @c kind…`: no node of capture `c` sits directly
    /// under a node of one of the kinds.
    not_has_parent: Vec<(u32, Box<[Box<str>]>)>,
}

const NOT_HAS_PARENT_ARGS: &str = "#not-has-parent? takes a capture and node kinds";

impl PatternDirectives for FoldPattern {
    fn predicate(
        &mut self,
        capture_names: &[&str],
        predicate: &QueryPredicate,
    ) -> Result<(), String> {
        match (&*predicate.operator, &*predicate.args) {
            ("trim!", [Arg::Capture(capture)]) if capture_names[*capture as usize] == "fold" => {
                self.trim = true;
            }
            ("trim!", _) => return Err("#trim! takes only the @fold capture".to_owned()),
            ("not-has-parent?", [Arg::Capture(capture), kinds @ ..]) if !kinds.is_empty() => {
                let kinds = kinds
                    .iter()
                    .map(|kind| match kind {
                        Arg::String(kind) => Some(kind.clone()),
                        Arg::Capture(_) => None,
                    })
                    .collect::<Option<_>>()
                    .ok_or(NOT_HAS_PARENT_ARGS)?;
                self.not_has_parent.push((*capture, kinds));
            }
            ("not-has-parent?", _) => return Err(NOT_HAS_PARENT_ARGS.to_owned()),
            (operator, _) => return Err(unsupported(operator)),
        }
        Ok(())
    }
}

impl FoldPattern {
    /// `fold`, ending at its last non-blank line when the pattern trims.
    fn trimmed(&self, fold: Fold, rope: &ropey::Rope) -> Option<Fold> {
        if !self.trim {
            return Some(fold);
        }
        let last = InclusiveRange::new(fold.first().advance(1), fold.last())
            .iter()
            .rev()
            .find(|&line| !is_blank(rope, line))
            .unwrap_or(fold.first());
        Fold::new(fold.first(), last, fold.shows_last() && last == fold.last())
    }

    fn admits(&self, m: &tree_sitter::QueryMatch<'_, '_>) -> bool {
        self.not_has_parent.iter().all(|(capture, kinds)| {
            m.nodes_for_capture_index(*capture).all(|node| {
                node.parent()
                    .is_none_or(|parent| !kinds.iter().any(|kind| **kind == *parent.kind()))
            })
        })
    }
}

impl FoldQuery {
    /// What a `folds.scm` needs to name a region.
    pub(crate) const NEEDS: &str = "a @fold capture";

    /// A compiled `folds.scm`, `None` when no pattern it keeps captures
    /// `@fold`. A pattern
    /// using any engine predicate or directive but `#trim! @fold` and
    /// `#not-has-parent? @c kind…` is skipped, its error in `source` pushed
    /// to `skipped`.
    pub(crate) fn new(
        mut query: tree_sitter::Query,
        source: &str,
        skipped: &mut Vec<QueryError>,
    ) -> Option<Self> {
        let patterns = parse_patterns(&mut query, source, skipped);
        let fold = patterns.capture_named(&query, "fold")?;
        Some(Self::Folds {
            query,
            fold,
            patterns: patterns.parsed,
        })
    }

    /// Call `f` with every region of `tree` with a node in `bytes` whose
    /// first line `wanted` holds, in match order. A region's extent is
    /// worked out only once `wanted` holds its first line.
    fn regions(
        &self,
        cursor: &mut QueryCursor,
        tree: &Tree,
        rope: &ropey::Rope,
        bytes: Range<usize>,
        wanted: &impl Fn(ContentLine) -> bool,
        f: &mut impl FnMut(Fold),
    ) {
        let root = tree.root_node();
        match self {
            Self::Folds {
                query,
                fold,
                patterns,
            } => {
                cursor.set_byte_range(bytes);
                let mut matches = cursor.matches(query, root, RopeProvider(rope));
                while let Some(m) = matches.next() {
                    let pattern = &patterns[m.pattern_index];
                    if !pattern.admits(m) {
                        continue;
                    }
                    m.nodes_for_capture_index(*fold)
                        .map(|node| Scope::of(node.range()))
                        .filter(|scope| wanted(scope.header))
                        .filter_map(|scope| fold_of(scope, root, rope))
                        .filter_map(|fold| pattern.trimmed(fold, rope))
                        .for_each(&mut *f);
                }
            }
            Self::Scopes(scopes) => scopes.for_each_scope(cursor, tree, rope, bytes, |scope| {
                if wanted(scope.header) {
                    fold_of(scope, root, rope).into_iter().for_each(&mut *f);
                }
            }),
        }
    }
}

impl SyntaxLayers {
    /// Whether any layer's grammar names foldable regions.
    pub fn can_fold(&self) -> bool {
        self.layers.iter().any(|layer| layer.bundle.folds.is_some())
    }

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
            &|first| first <= line,
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
    pub fn all_folds(&self, rope: &ropey::Rope) -> Vec<Fold> {
        let mut folds = Vec::new();
        layer_regions(
            &mut QueryCursor::new(),
            self.layers.iter(),
            rope,
            0..rope.len_bytes(),
            &|_| true,
            &mut |fold| folds.push(fold),
        );
        canonical(&mut folds);
        folds
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
        layer_regions(
            cursor,
            covering,
            rope,
            bytes,
            &|first| lines.contains(first),
            &mut |fold| out.push(fold.first()),
        );
    }
}

fn layer_regions<'l>(
    cursor: &mut QueryCursor,
    layers: impl Iterator<Item = &'l SyntaxLayer>,
    rope: &ropey::Rope,
    bytes: Range<usize>,
    wanted: &impl Fn(ContentLine) -> bool,
    f: &mut impl FnMut(Fold),
) {
    for layer in layers {
        if let Some(query) = &layer.bundle.folds {
            query.regions(cursor, &layer.tree, rope, bytes.clone(), wanted, f);
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

/// The region from `scope`'s header line through its last line, in the
/// tree whose root is `root`.
///
/// A scope ending at column 0 ends on the line before (a Markdown section
/// runs to the next heading). Its last line stays shown when it holds only
/// the scope's closing token (`}`, `end`, `)`), possibly followed by more
/// text (`} else {`).
fn fold_of(scope: Scope, root: Node, rope: &ropey::Rope) -> Option<Fold> {
    let first = scope.header;
    let end = scope.end;
    let ends_at_line_start = end.column == 0 && end.row > first.index();
    let last = ContentLine::new(if ends_at_line_start {
        end.row - 1
    } else {
        end.row
    });
    let shows_last = !ends_at_line_start && closes_on_own_line(scope, root, rope);
    Fold::new(first, last, shows_last)
}

/// Whether the token ending `scope` under `root` is anonymous, starts on
/// `scope`'s last row, and only whitespace precedes it there.
fn closes_on_own_line(scope: Scope, root: Node, rope: &ropey::Rope) -> bool {
    let (end, row) = (scope.end_byte, scope.end.row);
    let Some(token) = root.descendant_for_byte_range(end.saturating_sub(1), end) else {
        return false;
    };
    !token.is_named()
        && token.start_position().row == row
        && leading_whitespace_end(rope, ContentLine::new(row))
            .offset()
            .index()
            == rope.byte_to_char(token.start_byte())
}

#[cfg(test)]
mod tests;
