use rustc_hash::FxHashSet;
use tree_sitter::{
    Language, Query, QueryError, QueryPredicate, QueryPredicateArg as Arg, QueryProperty,
};

use crate::directives::{PatternDirectives, parse_patterns, unsupported};

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
            _ => return Err(unsupported(operator)),
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

/// What one `indents.scm` pattern sets and the predicates the engine
/// evaluates for it.
#[derive(Default)]
struct IndentPattern {
    /// `#set! "scope" "header"`.
    header: bool,
    predicates: Vec<Predicate>,
}

impl PatternDirectives for IndentPattern {
    fn setting(&mut self, setting: &QueryProperty) -> Result<(), String> {
        match (&*setting.key, setting.value.as_deref(), setting.capture_id) {
            ("scope", Some("header"), None) => {
                self.header = true;
                Ok(())
            }
            _ => Err(format!(
                "#set! {} is not an indents property; only #set! \"scope\" \"header\" is",
                setting.key
            )),
        }
    }

    fn predicate(&mut self, _: &[&str], predicate: &QueryPredicate) -> Result<(), String> {
        self.predicates.push(Predicate::parse(predicate)?);
        Ok(())
    }
}

/// A compiled `indents.scm`: the query plus, resolved once at load, the
/// engine role of each capture, each pattern's header setting and
/// engine-evaluated predicates, and the texts that could still grow into
/// another of the grammar's tokens.
pub struct IndentQuery {
    pub(super) query: Query,
    captures: Vec<Option<IndentCapture>>,
    patterns: Vec<IndentPattern>,
    /// Every proper prefix of the grammar's anonymous token kinds.
    token_prefixes: FxHashSet<Box<str>>,
}

impl IndentQuery {
    /// What an `indents.scm` needs to indent anything.
    pub(crate) const NEEDS: &str = "an indent capture such as @indent or @outdent";

    /// A compiled `indents.scm`, `None` when no pattern it keeps has an
    /// indent capture. A
    /// pattern using a predicate the engine does not evaluate, or a `#set!`
    /// other than `"scope" "header"`, is skipped, its error in `source`
    /// pushed to `skipped`.
    pub(crate) fn new(
        mut query: Query,
        source: &str,
        language: &Language,
        skipped: &mut Vec<QueryError>,
    ) -> Option<Self> {
        let captures: Vec<Option<IndentCapture>> = query
            .capture_names()
            .iter()
            .map(|name| IndentCapture::from_name(name))
            .collect();
        let patterns = parse_patterns(&mut query, source, skipped);
        if !captures
            .iter()
            .enumerate()
            .any(|(idx, capture)| capture.is_some() && patterns.captures(idx))
        {
            return None;
        }
        Some(Self {
            query,
            captures,
            patterns: patterns.parsed,
            token_prefixes: token_prefixes(language),
        })
    }

    pub(super) fn capture(&self, index: u32) -> Option<IndentCapture> {
        self.captures[index as usize]
    }

    /// Whether `pattern` sets `#set! "scope" "header"`: its indent captures
    /// open their scope on the captured node's parent's first line.
    pub(super) fn is_header(&self, pattern: usize) -> bool {
        self.patterns[pattern].header
    }

    pub(super) fn predicates(&self, pattern: usize) -> &[Predicate] {
        &self.patterns[pattern].predicates
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
