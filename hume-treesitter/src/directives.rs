//! The predicates and directives tree-sitter leaves to the engine, read per
//! pattern when a query file loads. Tree-sitter evaluates the text
//! predicates (`#eq?`, `#match?`, `#any-of?` and their forms) itself.

use tree_sitter::{Query, QueryError, QueryErrorKind, QueryPredicate, QueryProperty};

/// What one pattern of a query file makes of its `#set!` settings and of
/// the predicates tree-sitter does not evaluate. Each method rejects its
/// input unless the kind overrides it.
pub(crate) trait PatternDirectives: Default {
    fn setting(&mut self, _setting: &QueryProperty) -> Result<(), String> {
        Err(unsupported("set!"))
    }

    fn predicate(
        &mut self,
        _capture_names: &[&str],
        predicate: &QueryPredicate,
    ) -> Result<(), String> {
        Err(unsupported(&predicate.operator))
    }
}

/// Every pattern of `query`, compiled from `source`, read as `P`. Fails, as
/// a `Predicate` error at the offending pattern's start, on `#is?` and
/// `#is-not?` and on whatever `P` rejects.
pub(crate) fn parse_patterns<P: PatternDirectives>(
    query: &Query,
    source: &str,
) -> Result<Vec<P>, QueryError> {
    (0..query.pattern_count())
        .map(|pattern| {
            parse_pattern(query, pattern)
                .map_err(|message| pattern_error(query, source, pattern, message))
        })
        .collect()
}

fn parse_pattern<P: PatternDirectives>(query: &Query, pattern: usize) -> Result<P, String> {
    if let Some((_, positive)) = query.property_predicates(pattern).first() {
        return Err(unsupported(if *positive { "is?" } else { "is-not?" }));
    }
    let mut parsed = P::default();
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
    let before = &source[..offset];
    let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
    QueryError {
        row: before.matches('\n').count(),
        column: offset - line_start,
        offset,
        message,
        kind: QueryErrorKind::Predicate,
    }
}
