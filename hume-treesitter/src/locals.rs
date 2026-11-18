//! A grammar's `locals.scm`: the scopes, definitions and references of a
//! layer's tree. A reference resolves to the definition of its name it can
//! see, takes that definition's highlight, and is local, which decides a
//! highlights pattern's `#is?`/`#is-not? local`.
//!
//! A reference resolves by name to a definition that ends before it,
//! searching from its innermost scope outward, the latest such definition
//! in a scope winning. A scope set `#set! local.scope-inherits false` does
//! not see its parents' definitions. A definition's highlight is its capture
//! name after `local.definition.`, `@local.definition.variable.parameter`
//! giving its references `variable.parameter`; a bare `@local.definition`
//! makes them local and leaves their highlight alone. A node any pattern
//! captures under another name, `@_` among them, is not a reference,
//! whatever the order of the patterns.
//!
//! A reparse rebuilds only the scopes an edit can have changed: the
//! innermost scope around each byte the edit or the reparse touched and
//! around every definition or reference holding such a byte, since a
//! definition changes what references see only inside its own scope, and a
//! name is its node's text. A change outside every scope rebuilds the whole
//! layer.

use std::cmp::Reverse;
use std::ops::RangeInclusive;
use std::sync::Arc;

use rustc_hash::FxHashMap;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, QueryProperty};

use hume_engine::theme::ScopeRegistry;
use hume_engine::types::ScopeId;
use hume_rope::offset::ExclusiveRange;

use crate::directives::{PatternDirectives, parse_patterns};
use crate::edits::ByteMap;
use crate::highlight::RopeProvider;
use crate::injections::node_text;
use crate::layers::LayerChange;
use crate::registry::GrammarBundle;
use crate::registry::QuerySkips;

/// A compiled `locals.scm`.
pub struct LocalsQuery {
    query: Query,
    /// Per capture index, what the capture names.
    roles: Vec<CaptureRole>,
    patterns: Vec<LocalsPattern>,
}

/// What a `locals.scm` capture makes of its node.
#[derive(Clone, Copy, PartialEq)]
enum CaptureRole {
    Scope,
    /// A definition, with the highlight its references take.
    Definition(Option<ScopeId>),
    Reference,
    /// Any other name, `@_` among them: the node is not a reference.
    Discard,
}

/// What one `locals.scm` pattern sets.
#[derive(Default)]
struct LocalsPattern {
    /// `#set! local.scope-inherits` to anything but `"true"`: the pattern's
    /// `@local.scope` does not see its parents' definitions.
    no_inheritance: bool,
}

impl PatternDirectives for LocalsPattern {
    fn setting(&mut self, setting: &QueryProperty) -> Result<(), String> {
        match (&*setting.key, setting.capture_id) {
            ("local.scope-inherits", None) => {
                self.no_inheritance = setting.value.as_deref().is_some_and(|v| v != "true");
                Ok(())
            }
            _ => Err(format!("#set! {} is not a locals property", setting.key)),
        }
    }
}

impl LocalsQuery {
    /// What a `locals.scm` needs to name anything.
    pub(crate) const NEEDS: &str = "a @local.scope, @local.definition or @local.reference capture";

    /// A compiled `locals.scm`, its definition highlights interned in
    /// `registry`. `None` when no pattern it keeps captures a scope,
    /// definition or reference. A pattern using an engine predicate or
    /// directive is skipped, its error in `source` pushed to `skips.patterns`.
    /// A definition highlight the registry refuses leaves the definition
    /// without one and is pushed to `skips.captures`.
    pub(crate) fn new(
        mut query: Query,
        source: &str,
        registry: &mut ScopeRegistry,
        skips: &mut QuerySkips,
    ) -> Option<Self> {
        let patterns = parse_patterns::<LocalsPattern>(&mut query, source, &mut skips.patterns);
        let roles: Vec<CaptureRole> = query
            .capture_names()
            .iter()
            .map(|name| match *name {
                "local.scope" => CaptureRole::Scope,
                "local.reference" => CaptureRole::Reference,
                "local.definition" => CaptureRole::Definition(None),
                name => name
                    .strip_prefix("local.definition.")
                    .map_or(CaptureRole::Discard, |highlight| {
                        CaptureRole::Definition(skips.intern(registry, name, highlight))
                    }),
            })
            .collect();
        if !roles
            .iter()
            .enumerate()
            .any(|(idx, role)| *role != CaptureRole::Discard && patterns.captures(idx))
        {
            return None;
        }
        Some(Self {
            query,
            roles,
            patterns: patterns.parsed,
        })
    }

    fn role(&self, capture: u32) -> CaptureRole {
        self.roles[capture as usize]
    }
}

/// The bytes of a node of a layer's tree.
type NodeBytes = ExclusiveRange<usize>;

/// The locals of one layer's tree, from its grammar's `locals.scm`, moved
/// through each change until the next parse rebuilds the scopes it can have
/// changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayerLocals {
    /// Every scope, sorted by start, then by end descending, so a scope's
    /// enclosing scopes come before it.
    scopes: Vec<ScopeEntry>,
    /// Every definition, sorted by range.
    definitions: Vec<Definition>,
    /// Every reference that resolved, sorted by range, with its
    /// definition's highlight.
    resolved: Vec<(NodeBytes, Option<ScopeId>)>,
}

#[derive(Clone, Debug, PartialEq)]
struct ScopeEntry {
    range: NodeBytes,
    inherits: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Definition {
    range: NodeBytes,
    /// The innermost scope it was defined in, `None` for the layer's own.
    scope: Option<NodeBytes>,
    name: Arc<str>,
    highlight: Option<ScopeId>,
}

/// A scope open during a walk over a layer's captures.
struct OpenScope {
    /// `None` for the layer's own scope.
    range: Option<NodeBytes>,
    inherits: bool,
    /// Each name defined so far, with where its latest definition ends and
    /// that definition's highlight.
    definitions: FxHashMap<Arc<str>, (usize, Option<ScopeId>)>,
}

impl OpenScope {
    fn new(range: Option<NodeBytes>, inherits: bool) -> Self {
        Self {
            range,
            inherits,
            definitions: FxHashMap::default(),
        }
    }

    fn ends_by(&self, byte: usize) -> bool {
        self.range.is_some_and(|range| range.end <= byte)
    }
}

fn key(range: &NodeBytes) -> (usize, usize) {
    (range.start, range.end)
}

fn node_bytes(node: tree_sitter::Node<'_>) -> NodeBytes {
    ExclusiveRange::new(node.start_byte(), node.end_byte())
}

/// Whether `inner` lies inside `outer`, ends included.
fn within(inner: &NodeBytes, outer: &NodeBytes) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

impl LayerLocals {
    /// The locals `query` finds in `tree`, parsed from `rope`.
    fn built(
        query: &LocalsQuery,
        cursor: &mut QueryCursor,
        tree: &tree_sitter::Tree,
        rope: &ropey::Rope,
    ) -> Self {
        let mut locals = Self::default();
        locals.append(
            query,
            cursor,
            tree,
            rope,
            None,
            &mut vec![OpenScope::new(None, true)],
        );
        locals
    }

    /// These locals, edited to `tree`, with the locals `query` finds in each
    /// of `regions` in place of theirs. `regions` are sorted, disjoint scopes
    /// of `tree` and of these locals alike, that no definition or reference
    /// of either crosses, so a reference inside one sees the definitions of
    /// the scopes around it that these locals hold, and one outside sees
    /// nothing inside it.
    fn rebuilt(
        &self,
        query: &LocalsQuery,
        cursor: &mut QueryCursor,
        tree: &tree_sitter::Tree,
        rope: &ropey::Rope,
        regions: &[NodeBytes],
    ) -> Self {
        let mut out = Self {
            scopes: Vec::with_capacity(self.scopes.len()),
            definitions: Vec::with_capacity(self.definitions.len()),
            resolved: Vec::with_capacity(self.resolved.len()),
        };
        let (mut scopes, mut definitions, mut resolved) =
            (&self.scopes[..], &self.definitions[..], &self.resolved[..]);
        let mut open = vec![OpenScope::new(None, true)];
        // `out.definitions` from `unseeded` on are not yet in `open`.
        let mut unseeded = 0;
        for &region in regions {
            let before = scopes.partition_point(|s| {
                (s.range.start, Reverse(s.range.end)) < (region.start, Reverse(region.end))
            });
            open.extend(
                scopes[..before]
                    .iter()
                    .map(|s| OpenScope::new(Some(s.range), s.inherits)),
            );
            out.scopes.extend_from_slice(&scopes[..before]);
            scopes = &scopes[before..];
            open.retain(|s| {
                s.range
                    .is_none_or(|range| within(&region, &range) && range != region)
            });
            let kept = definitions.partition_point(|d| d.range.start < region.start);
            out.definitions.extend_from_slice(&definitions[..kept]);
            definitions = &definitions[kept..];
            let kept = resolved.partition_point(|(r, _)| r.start < region.start);
            out.resolved.extend_from_slice(&resolved[..kept]);
            resolved = &resolved[kept..];

            for definition in &out.definitions[unseeded..] {
                debug_assert!(
                    definition.range.end <= region.start,
                    "definition {:?} crosses region {region:?}",
                    definition.range
                );
                // Of two scopes of one range, the later opened is the
                // innermost, which a walk defines in.
                if let Some(scope) = open.iter_mut().rev().find(|s| s.range == definition.scope) {
                    scope.definitions.insert(
                        definition.name.clone(),
                        (definition.range.end, definition.highlight),
                    );
                }
            }
            out.append(query, cursor, tree, rope, Some(region), &mut open);
            unseeded = out.definitions.len();

            scopes = &scopes[scopes.partition_point(|s| s.range.start < region.end)..];
            definitions =
                &definitions[definitions.partition_point(|d| d.range.start < region.end)..];
            resolved = &resolved[resolved.partition_point(|(r, _)| r.start < region.end)..];
        }
        out.scopes.extend_from_slice(scopes);
        out.definitions.extend_from_slice(definitions);
        out.resolved.extend_from_slice(resolved);
        out
    }

    /// Append the locals `query` finds in `region` of `tree`, parsed from
    /// `rope`, or in the whole tree for `None`, to these, which hold nothing
    /// after `region`'s start. `open` holds the scopes around `region`,
    /// innermost last, with the definitions a reference inside it sees; a
    /// definition found directly in one of them is added to it.
    fn append(
        &mut self,
        query: &LocalsQuery,
        cursor: &mut QueryCursor,
        tree: &tree_sitter::Tree,
        rope: &ropey::Rope,
        region: Option<NodeBytes>,
        open: &mut Vec<OpenScope>,
    ) {
        let seeded = open.len();
        let (first_scope, first_definition) = (self.scopes.len(), self.definitions.len());
        let mut resolved = Vec::new();
        let mut discarded = Vec::new();
        cursor.set_byte_range(region.map_or(0..usize::MAX, |region| region.start..region.end));
        let mut captures = cursor.captures(&query.query, tree.root_node(), RopeProvider(rope));
        while let Some((m, index)) = captures.next() {
            let capture = m.captures()[*index];
            let node = capture.node;
            let range = node_bytes(node);
            if region.is_some_and(|region| !within(&range, &region)) {
                continue;
            }
            while open.len() > seeded && open.last().is_some_and(|s| s.ends_by(range.start)) {
                open.pop();
            }
            match query.role(capture.index) {
                CaptureRole::Scope => {
                    let inherits = !query.patterns[m.pattern_index].no_inheritance;
                    self.scopes.push(ScopeEntry { range, inherits });
                    open.push(OpenScope::new(Some(range), inherits));
                }
                CaptureRole::Definition(highlight) => {
                    let name: Arc<str> = node_text(node, rope).into();
                    let innermost = open.last_mut().expect("the layer's scope stays open");
                    innermost
                        .definitions
                        .insert(name.clone(), (range.end, highlight));
                    self.definitions.push(Definition {
                        range,
                        scope: innermost.range,
                        name,
                        highlight,
                    });
                }
                CaptureRole::Reference => {
                    if let Some(highlight) = resolve(open, &node_text(node, rope), range.start) {
                        resolved.push((range, highlight));
                    }
                }
                CaptureRole::Discard => discarded.push(key(&range)),
            }
        }
        open.truncate(seeded);
        self.scopes[first_scope..].sort_by_key(|s| (s.range.start, Reverse(s.range.end)));
        let definitions = &mut self.definitions[first_definition..];
        definitions.sort_by_key(|d| key(&d.range));
        discarded.sort_unstable();
        // A definition's own node is never a reference to it, and a node a
        // discard captures is no reference at all.
        resolved.retain(|(range, _)| {
            definitions
                .binary_search_by_key(&key(range), |d| key(&d.range))
                .is_err()
                && discarded.binary_search(&key(range)).is_err()
        });
        resolved.sort_by_key(|(range, _)| key(range));
        self.resolved.extend(resolved);
    }

    /// Whether the node spanning `start..end` is local.
    pub(crate) fn is_local(&self, start: usize, end: usize) -> bool {
        let at = (start, end);
        self.definitions
            .binary_search_by_key(&at, |d| key(&d.range))
            .is_ok()
            || self
                .resolved
                .binary_search_by_key(&at, |(range, _)| key(range))
                .is_ok()
    }

    /// The resolved references starting in `start..end` whose definition
    /// has a highlight, each with it.
    pub(crate) fn references_in(
        &self,
        start: usize,
        end: usize,
    ) -> impl Iterator<Item = (NodeBytes, ScopeId)> + '_ {
        let first = self.resolved.partition_point(|(r, _)| r.start < start);
        self.resolved[first..]
            .iter()
            .take_while(move |(r, _)| r.start < end)
            .filter_map(|&(range, highlight)| Some((range, highlight?)))
    }

    /// Move every range through the change `map` describes, as `Tree::edit`
    /// moves a tree through its edits.
    pub(crate) fn edit(&mut self, map: &ByteMap) {
        let moved = |range: &mut NodeBytes| {
            *range = ExclusiveRange::new(map.map(range.start), map.map(range.end));
        };
        for scope in &mut self.scopes {
            moved(&mut scope.range);
        }
        for definition in &mut self.definitions {
            moved(&mut definition.range);
            if let Some(scope) = &mut definition.scope {
                moved(scope);
            }
        }
        for (range, _) in &mut self.resolved {
            moved(range);
        }
    }
}

/// The highlight of the definition of `name` a reference at `start` sees
/// from the innermost of `scopes`: `Some(None)` for a definition without
/// one, `None` when it sees none. The nearest scope defining `name` decides.
fn resolve(scopes: &[OpenScope], name: &str, start: usize) -> Option<Option<ScopeId>> {
    for scope in scopes.iter().rev() {
        if let Some(&(end, highlight)) = scope.definitions.get(name) {
            return (end <= start).then_some(highlight);
        }
        if !scope.inherits {
            break;
        }
    }
    None
}

/// The scopes of `tree` to rebuild from `prior`'s locals: for each region
/// `dirty`, widened until no scope of `prior` starts or ends in it and no
/// definition or reference of `prior` or `tree` overlaps it without lying
/// inside it, the innermost scope of `tree` holding it, the outermost kept
/// where two nest. `None` when one has no such scope: the whole layer is
/// rebuilt.
fn dirty_scopes(
    query: &LocalsQuery,
    cursor: &mut QueryCursor,
    prior: &LayerLocals,
    tree: &tree_sitter::Tree,
    rope: &ropey::Rope,
    dirty: &[RangeInclusive<usize>],
) -> Option<Vec<NodeBytes>> {
    if !query.roles.contains(&CaptureRole::Scope) {
        return None;
    }
    let mut regions = Vec::new();
    for region in dirty {
        // Half-open, so a scope it absorbs bounds it by its own end.
        let mut bytes = ExclusiveRange::new(*region.start(), *region.end() + 1);
        let holder = loop {
            bytes = widened_over_prior(prior, bytes);
            let (holder, wider) = holder_in_tree(query, cursor, tree, rope, bytes);
            if wider == bytes {
                break holder;
            }
            bytes = wider;
        };
        regions.push(holder?);
    }
    regions.sort_by_key(|r| (r.start, Reverse(r.end)));
    let mut kept: Vec<NodeBytes> = Vec::with_capacity(regions.len());
    for region in regions {
        if kept.last().is_none_or(|last| !within(&region, last)) {
            kept.push(region);
        }
    }
    Some(kept)
}

/// Whether `range` overlaps `bytes` without lying inside it.
fn crosses(range: &NodeBytes, bytes: &NodeBytes) -> bool {
    range.start < bytes.end && bytes.start < range.end && !within(range, bytes)
}

/// `bytes` widened over every scope of `prior` starting or ending in it and
/// every definition and reference of `prior` crossing it, until none is
/// left.
fn widened_over_prior(prior: &LayerLocals, mut bytes: NodeBytes) -> NodeBytes {
    loop {
        let in_bytes = |byte: usize| bytes.start <= byte && byte < bytes.end;
        let scopes = prior
            .scopes
            .iter()
            .map(|s| s.range)
            .filter(|range| in_bytes(range.start) || in_bytes(range.end));
        let named = prior
            .definitions
            .iter()
            .map(|d| d.range)
            .chain(prior.resolved.iter().map(|&(range, _)| range))
            .filter(|range| crosses(range, &bytes));
        let wider = scopes.chain(named).fold(bytes, hull);
        if wider == bytes {
            return bytes;
        }
        bytes = wider;
    }
}

/// The innermost scope of `tree` holding `bytes`, and `bytes` widened over
/// every definition and reference of `tree` crossing it.
fn holder_in_tree(
    query: &LocalsQuery,
    cursor: &mut QueryCursor,
    tree: &tree_sitter::Tree,
    rope: &ropey::Rope,
    bytes: NodeBytes,
) -> (Option<NodeBytes>, NodeBytes) {
    cursor.set_byte_range(bytes.start..bytes.end);
    let mut innermost: Option<NodeBytes> = None;
    let mut wider = bytes;
    let mut captures = cursor.captures(&query.query, tree.root_node(), RopeProvider(rope));
    while let Some((m, index)) = captures.next() {
        let capture = m.captures()[*index];
        let range = node_bytes(capture.node);
        match query.role(capture.index) {
            CaptureRole::Scope => {
                if within(&bytes, &range) && innermost.is_none_or(|i| within(&range, &i)) {
                    innermost = Some(range);
                }
            }
            CaptureRole::Definition(_) | CaptureRole::Reference => {
                if crosses(&range, &bytes) {
                    wider = hull(wider, range);
                }
            }
            CaptureRole::Discard => {}
        }
    }
    (innermost, wider)
}

/// The smallest range holding `a` and `b`.
fn hull(a: NodeBytes, b: NodeBytes) -> NodeBytes {
    ExclusiveRange::new(a.start.min(b.start), a.end.max(b.end))
}

/// The locals of `tree`, a layer parsed from `rope` with `bundle`: empty
/// for a grammar without a `locals.scm`. With `change`, whose prior holds
/// the layer's locals before this parse, edited to it, only the scopes an
/// edit can have changed are rebuilt.
pub(crate) fn layer_locals(
    bundle: &GrammarBundle,
    tree: &tree_sitter::Tree,
    rope: &ropey::Rope,
    change: Option<&LayerChange<'_>>,
) -> Arc<LayerLocals> {
    let Some(query) = bundle.locals.as_ref() else {
        return Arc::default();
    };
    let mut cursor = QueryCursor::new();
    let scoped = match change.map(|change| (change.prior.locals, change.dirty())) {
        Some((prior, [])) => return Arc::clone(prior),
        Some((prior, dirty)) => dirty_scopes(query, &mut cursor, prior, tree, rope, dirty)
            .map(|regions| (prior, regions)),
        None => None,
    };
    Arc::new(match scoped {
        Some((prior, regions)) => prior.rebuilt(query, &mut cursor, tree, rope, &regions),
        None => LayerLocals::built(query, &mut cursor, tree, rope),
    })
}

#[cfg(test)]
mod tests;
