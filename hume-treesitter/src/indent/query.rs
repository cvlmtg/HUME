use rustc_hash::FxHashSet;
use tree_sitter::{
    Language, Query, QueryError, QueryErrorKind, QueryPredicate, QueryPredicateArg as Arg,
};

hume_core::named_enum! {
    /// What an indents-query capture name means to the engine. Any other
    /// name (`@expr-start`, `@_helper`) is a helper the patterns' predicates
    /// refer to.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum IndentCapture {
        Indent => "indent",
        IndentAlways => "indent.always",
        Outdent => "outdent",
        OutdentAlways => "outdent.always",
        Align => "align",
        Anchor => "anchor",
        Extend => "extend",
        ExtendPreventOnce => "extend.prevent-once",
        Opaque => "opaque",
    }
}

/// A predicate tree-sitter leaves to the engine. Each reads the first node
/// of its captures.
pub(super) enum Predicate {
    /// `#not-kind-eq? @c "kind"`.
    NotKindEq { capture: u32, kind: Box<str> },
    /// `#same-line? @a @b` (`same`) or `#not-same-line? @a @b`: whether the
    /// two nodes start on one line.
    SameLine { a: u32, b: u32, same: bool },
    /// `#one-line? @c` (`one`) or `#not-one-line? @c`: whether the node
    /// starts and ends on one line.
    OneLine { capture: u32, one: bool },
}

impl Predicate {
    fn parse(predicate: &QueryPredicate) -> Result<Self, String> {
        let operator = &*predicate.operator;
        let expected = match operator {
            "not-kind-eq?" => "a capture and a node kind",
            "same-line?" | "not-same-line?" => "two captures",
            "one-line?" | "not-one-line?" => "one capture",
            _ => return Err(format!("#{operator} is not an indents predicate")),
        };
        match (operator, &*predicate.args) {
            ("not-kind-eq?", [Arg::Capture(capture), Arg::String(kind)]) => Ok(Self::NotKindEq {
                capture: *capture,
                kind: kind.clone(),
            }),
            ("same-line?" | "not-same-line?", [Arg::Capture(a), Arg::Capture(b)]) => {
                Ok(Self::SameLine {
                    a: *a,
                    b: *b,
                    same: operator == "same-line?",
                })
            }
            ("one-line?" | "not-one-line?", [Arg::Capture(capture)]) => Ok(Self::OneLine {
                capture: *capture,
                one: operator == "one-line?",
            }),
            _ => Err(format!("#{operator} takes {expected}")),
        }
    }
}

/// A compiled `indents.scm`: the query plus, resolved once at load, the
/// engine role of each capture, whether each pattern sets the header scope,
/// each pattern's engine-evaluated predicates, and the texts that could
/// still grow into another of the grammar's tokens. Tree-sitter evaluates
/// the text predicates (`#eq?`, `#match?`, `#any-of?`) itself.
pub struct IndentQuery {
    pub(super) query: Query,
    captures: Vec<Option<IndentCapture>>,
    header: Vec<bool>,
    predicates: Vec<Vec<Predicate>>,
    /// Every proper prefix of the grammar's anonymous token kinds.
    token_prefixes: FxHashSet<Box<str>>,
}

impl IndentQuery {
    /// Fails, as a `Predicate` error at the offending pattern's start in
    /// `source`, on any predicate the engine does not evaluate, on `#is?`
    /// and `#is-not?`, and on any `#set!` other than `"scope" "header"`.
    pub(crate) fn new(query: Query, source: &str, language: &Language) -> Result<Self, QueryError> {
        let captures = query
            .capture_names()
            .iter()
            .map(|name| IndentCapture::from_name(name))
            .collect();
        let mut header = Vec::with_capacity(query.pattern_count());
        let mut predicates = Vec::with_capacity(query.pattern_count());
        for pattern in 0..query.pattern_count() {
            let error = |message| pattern_error(&query, source, pattern, message);
            if let Some((_, positive)) = query.property_predicates(pattern).first() {
                let operator = if *positive { "is?" } else { "is-not?" };
                return Err(error(format!("#{operator} is not an indents predicate")));
            }
            let mut is_header = false;
            for setting in query.property_settings(pattern) {
                match (&*setting.key, setting.value.as_deref(), setting.capture_id) {
                    ("scope", Some("header"), None) => is_header = true,
                    _ => {
                        return Err(error(format!(
                            "#set! {} is not an indents property; only #set! \"scope\" \"header\" is",
                            setting.key
                        )));
                    }
                }
            }
            header.push(is_header);
            predicates.push(
                query
                    .general_predicates(pattern)
                    .iter()
                    .map(Predicate::parse)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(error)?,
            );
        }
        Ok(Self {
            query,
            captures,
            header,
            predicates,
            token_prefixes: token_prefixes(language),
        })
    }

    pub(super) fn capture(&self, index: u32) -> Option<IndentCapture> {
        self.captures[index as usize]
    }

    /// Whether `pattern` sets `#set! "scope" "header"`: its indent captures
    /// open their scope on the captured node's parent's first line.
    pub(super) fn is_header(&self, pattern: usize) -> bool {
        self.header[pattern]
    }

    pub(super) fn predicates(&self, pattern: usize) -> &[Predicate] {
        &self.predicates[pattern]
    }

    /// Whether `text` is a proper prefix of one of the grammar's anonymous
    /// tokens: typing on could still turn it into that token, as `.` into
    /// `..`.
    pub(super) fn is_token_prefix(&self, text: &str) -> bool {
        self.token_prefixes.contains(text)
    }
}

/// Every proper, non-empty prefix of `language`'s visible anonymous node
/// kinds, which are the literal tokens of its grammar.
fn token_prefixes(language: &Language) -> FxHashSet<Box<str>> {
    let count =
        u16::try_from(language.node_kind_count()).expect("tree-sitter numbers node kinds in u16");
    (0..count)
        .filter(|&id| !language.node_kind_is_named(id) && language.node_kind_is_visible(id))
        .filter_map(|id| language.node_kind_for_id(id))
        .flat_map(|kind| {
            kind.char_indices()
                .skip(1)
                .map(move |(end, _)| Box::from(&kind[..end]))
        })
        .collect()
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
