//! The predicates and directives tree-sitter leaves to the engine, read per
//! pattern when a query file loads, and the regions a match's captures
//! make. Tree-sitter evaluates the text predicates (`#eq?`, `#match?`,
//! `#any-of?` and their forms) itself.
//!
//! A pattern using one HUME does not apply is disabled, so it never matches
//! instead of matching as if the predicate held, and reported; the rest of
//! its file still loads.

use hume_rope::column::ByteCol;
use hume_rope::lines::advance_byte_point;
use tree_sitter::{
    Query, QueryError, QueryErrorKind, QueryMatch, QueryPredicate, QueryPredicateArg as Arg,
    QueryProperty,
};

/// What one pattern of a query file makes of its `#set!` settings, its
/// `#is?`/`#is-not? local` and the predicates tree-sitter does not
/// evaluate. Each method rejects its input unless the kind overrides it.
pub(crate) trait PatternDirectives: Default {
    fn setting(&mut self, _setting: &QueryProperty) -> Result<(), String> {
        Err(unsupported("set!"))
    }

    /// `#is? local` when `positive`, `#is-not? local` otherwise.
    fn local(&mut self, positive: bool) -> Result<(), String> {
        Err(unsupported(if positive { "is?" } else { "is-not?" }))
    }

    fn predicate(
        &mut self,
        _capture_names: &[&str],
        predicate: &QueryPredicate,
    ) -> Result<(), String> {
        Err(unsupported(&predicate.operator))
    }
}

/// Every pattern of `query`, compiled from `source`, read as `P`. A pattern
/// with a predicate or directive `P` rejects, or a property predicate on
/// anything but `local`, is disabled and read as `P::default()`, and a
/// `Predicate` error at its start is pushed to `skipped`.
pub(crate) fn parse_patterns<P: PatternDirectives>(
    query: &mut Query,
    source: &str,
    skipped: &mut Vec<QueryError>,
) -> Vec<P> {
    let mut patterns = Vec::with_capacity(query.pattern_count());
    for pattern in 0..query.pattern_count() {
        let parsed = parse_pattern(query, pattern).unwrap_or_else(|message| {
            skipped.push(pattern_error(query, source, pattern, message));
            query.disable_pattern(pattern);
            P::default()
        });
        patterns.push(parsed);
    }
    patterns
}

fn parse_pattern<P: PatternDirectives>(query: &Query, pattern: usize) -> Result<P, String> {
    let mut parsed = P::default();
    for (property, positive) in query.property_predicates(pattern) {
        let local =
            &*property.key == "local" && property.value.is_none() && property.capture_id.is_none();
        if !local {
            return Err(unsupported(if *positive { "is?" } else { "is-not?" }));
        }
        parsed.local(*positive)?;
    }
    for setting in query.property_settings(pattern) {
        parsed.setting(setting)?;
    }
    for predicate in query.general_predicates(pattern) {
        parsed.predicate(query.capture_names(), predicate)?;
    }
    Ok(parsed)
}

/// The message for a predicate or directive a query kind does not support.
pub(crate) fn unsupported(operator: &str) -> String {
    format!("#{operator} is not supported")
}

/// A `Predicate` error at `pattern`'s start, with tree-sitter's own row
/// and byte-column convention.
fn pattern_error(query: &Query, source: &str, pattern: usize, message: String) -> QueryError {
    let offset = query.start_byte_for_pattern(pattern);
    let (row, column) = advance_byte_point(0, ByteCol::default(), &source[..offset]);
    QueryError {
        row,
        column: column.index(),
        offset,
        message,
        kind: QueryErrorKind::Predicate,
    }
}

/// `#make-range! "name" @start @end`: a region from `@start`'s start
/// through `@end`'s end.
#[derive(Clone, Copy)]
pub(crate) struct MakeRange {
    start: u32,
    end: u32,
}

impl MakeRange {
    /// The name a `#make-range!` gives, and its range.
    pub(crate) fn parse(args: &[Arg]) -> Result<(&str, Self), String> {
        match args {
            [Arg::String(name), Arg::Capture(start), Arg::Capture(end)] => Ok((
                name,
                Self {
                    start: *start,
                    end: *end,
                },
            )),
            _ => Err("#make-range! takes a name and two captures".to_owned()),
        }
    }

    /// The region `m` makes, ending at `@start`'s end when `@end` captured
    /// nothing. `None` when `@start` captured nothing or either capture's
    /// nodes are not contiguous (see [`capture_hull`]), or the region is
    /// empty.
    pub(crate) fn region(self, m: &QueryMatch<'_, '_>) -> Option<tree_sitter::Range> {
        let start = capture_hull(m, self.start)?;
        let end = match m.nodes_for_capture_index(self.end).next() {
            Some(_) => capture_hull(m, self.end)?,
            None => start,
        };
        (start.start_byte < end.end_byte).then_some(tree_sitter::Range {
            start_byte: start.start_byte,
            start_point: start.start_point,
            end_byte: end.end_byte,
            end_point: end.end_point,
        })
    }
}

/// The hull of every node captured under `capture_idx`, from the first
/// node's start to the furthest end, or `None` if the capture is empty or
/// its nodes are not contiguous.
///
/// A quantified capture legitimately spans several nodes (a function's
/// leading attributes, a run of line comments). But an unanchored quantifier
/// lets tree-sitter match across unrelated siblings: rust `test.around`'s
/// attribute group can skip over one test's body and latch onto the next
/// test's `#[test]`. So consecutive nodes must overlap or nest, or the second
/// must be the first's `next_sibling()` (the adjacency a `.` anchor enforces).
/// A failing match is dropped whole, not trimmed to its contiguous prefix,
/// since the prefix would still tag an unrelated function as a test.
pub(crate) fn capture_hull(m: &QueryMatch, capture_idx: u32) -> Option<tree_sitter::Range> {
    let mut nodes = m.nodes_for_capture_index(capture_idx);
    let first = nodes.next()?;
    let mut prev = first;
    let mut covered = first.range();
    for node in nodes {
        debug_assert!(
            node.start_byte() >= prev.start_byte(),
            "tree-sitter yields one capture's nodes in document order"
        );
        let contiguous = node.start_byte() <= covered.end_byte
            || prev.next_sibling().is_some_and(|sib| sib.id() == node.id());
        if !contiguous {
            return None;
        }
        covered = hull(covered, node.range());
        prev = node;
    }
    Some(covered)
}

/// The smallest range holding both `a` and `b`.
pub(crate) fn hull(a: tree_sitter::Range, b: tree_sitter::Range) -> tree_sitter::Range {
    let (start_byte, start_point) = if b.start_byte < a.start_byte {
        (b.start_byte, b.start_point)
    } else {
        (a.start_byte, a.start_point)
    };
    let (end_byte, end_point) = if b.end_byte > a.end_byte {
        (b.end_byte, b.end_point)
    } else {
        (a.end_byte, a.end_point)
    };
    tree_sitter::Range {
        start_byte,
        end_byte,
        start_point,
        end_point,
    }
}

#[cfg(test)]
mod tests;
