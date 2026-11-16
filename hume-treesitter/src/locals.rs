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
//! innermost scope around each byte the edit or the reparse touched, since
//! a definition changes what references see only inside its own scope. A
//! change outside every scope rebuilds the whole layer.

use std::cmp::Reverse;
use std::ops::RangeInclusive;
use std::sync::Arc;

use rustc_hash::FxHashMap;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, QueryError, QueryProperty};

use hume_engine::theme::ScopeRegistry;
use hume_engine::types::ScopeId;
use hume_rope::offset::ExclusiveRange;

use crate::directives::{PatternDirectives, parse_patterns};
use crate::edits::ByteMap;
use crate::highlight::RopeProvider;
use crate::injections::node_text;
use crate::layers::{LayerPrior, dirty_bytes};
use crate::registry::GrammarBundle;

/// A compiled `locals.scm`.
pub struct LocalsQuery {
    query: Query,
    /// Per capture index, what the capture names.
    roles: Vec<CaptureRole>,
    /// Per pattern, whether its `@local.scope` sees its parents'
    /// definitions.
    inherits: Vec<bool>,
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
    /// `#set! local.scope-inherits` to anything but `"true"`.
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
    /// `registry`. `None` when it captures no scope, definition or
    /// reference. A pattern using an engine predicate or directive is
    /// skipped, its error in `source` pushed to `skipped`.
    pub(crate) fn new(
        mut query: Query,
        source: &str,
        registry: &mut ScopeRegistry,
        skipped: &mut Vec<QueryError>,
    ) -> Option<Self> {
        let patterns: Vec<LocalsPattern> = parse_patterns(&mut query, source, skipped);
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
                        CaptureRole::Definition(Some(registry.intern(highlight)))
                    }),
            })
            .collect();
        if roles.iter().all(|role| *role == CaptureRole::Discard) {
            return None;
        }
        Some(Self {
            query,
            roles,
            inherits: patterns.iter().map(|p| !p.no_inheritance).collect(),
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
    name: Box<str>,
    highlight: Option<ScopeId>,
}

/// A scope open during [`LayerLocals::rebuild`]'s walk.
struct OpenScope {
    /// `None` for the layer's own scope.
    range: Option<NodeBytes>,
    inherits: bool,
    /// Each name defined so far, with where its latest definition ends and
    /// that definition's highlight.
    definitions: FxHashMap<Box<str>, (usize, Option<ScopeId>)>,
}

impl OpenScope {
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
    /// Rebuild the locals `query` finds in `region` of `tree`, parsed from
    /// `rope`, or in the whole tree for `None`, keeping every entry outside
    /// it. `region` is a scope of `tree` and of these locals' scopes alike,
    /// so a reference inside it sees the definitions of the scopes around it
    /// that these locals hold, and one outside sees nothing inside it.
    fn rebuild(
        &mut self,
        query: &LocalsQuery,
        tree: &tree_sitter::Tree,
        rope: &ropey::Rope,
        region: Option<NodeBytes>,
    ) {
        let mut scopes = vec![OpenScope {
            range: None,
            inherits: true,
            definitions: FxHashMap::default(),
        }];
        if let Some(region) = region {
            scopes.extend(
                self.scopes
                    .iter()
                    .filter(|s| within(&region, &s.range) && s.range != region)
                    .map(|s| OpenScope {
                        range: Some(s.range),
                        inherits: s.inherits,
                        definitions: FxHashMap::default(),
                    }),
            );
            for definition in self
                .definitions
                .iter()
                .take_while(|d| d.range.start < region.start)
                .filter(|d| d.range.end <= region.start)
            {
                if let Some(open) = scopes.iter_mut().find(|s| s.range == definition.scope) {
                    open.definitions.insert(
                        definition.name.clone(),
                        (definition.range.end, definition.highlight),
                    );
                }
            }
        }
        let seeded = scopes.len();

        let mut new_scopes = Vec::new();
        let mut new_definitions = Vec::new();
        let mut new_resolved = Vec::new();
        let mut discarded = Vec::new();
        let mut cursor = QueryCursor::new();
        if let Some(region) = region {
            cursor.set_byte_range(region.start..region.end);
        }
        let mut captures = cursor.captures(&query.query, tree.root_node(), RopeProvider(rope));
        while let Some((m, index)) = captures.next() {
            let capture = m.captures()[*index];
            let node = capture.node;
            let range = node_bytes(node);
            if region.is_some_and(|region| !within(&range, &region)) {
                continue;
            }
            while scopes.len() > seeded && scopes.last().is_some_and(|s| s.ends_by(range.start)) {
                scopes.pop();
            }
            match query.role(capture.index) {
                CaptureRole::Scope => {
                    let inherits = query.inherits[m.pattern_index];
                    new_scopes.push(ScopeEntry { range, inherits });
                    scopes.push(OpenScope {
                        range: Some(range),
                        inherits,
                        definitions: FxHashMap::default(),
                    });
                }
                CaptureRole::Definition(highlight) => {
                    let name: Box<str> = node_text(node, rope).into();
                    let innermost = scopes.last_mut().expect("the layer's scope stays open");
                    innermost
                        .definitions
                        .insert(name.clone(), (range.end, highlight));
                    new_definitions.push(Definition {
                        range,
                        scope: innermost.range,
                        name,
                        highlight,
                    });
                }
                CaptureRole::Reference => {
                    if let Some(highlight) = resolve(&scopes, &node_text(node, rope), range.start) {
                        new_resolved.push((range, highlight));
                    }
                }
                CaptureRole::Discard => discarded.push(key(&range)),
            }
        }
        new_scopes.sort_by_key(|s| (s.range.start, Reverse(s.range.end)));
        new_definitions.sort_by_key(|d| key(&d.range));
        discarded.sort_unstable();
        // A definition's own node is never a reference to it, and a node a
        // discard captures is no reference at all.
        new_resolved.retain(|(range, _)| {
            new_definitions
                .binary_search_by_key(&key(range), |d| key(&d.range))
                .is_err()
                && discarded.binary_search(&key(range)).is_err()
        });
        new_resolved.sort_by_key(|(range, _)| key(range));

        let Some(region) = region else {
            *self = Self {
                scopes: new_scopes,
                definitions: new_definitions,
                resolved: new_resolved,
            };
            return;
        };
        let before =
            |start: usize, end: usize| (start, Reverse(end)) < (region.start, Reverse(region.end));
        let first = self
            .scopes
            .partition_point(|s| before(s.range.start, s.range.end));
        let last = self.scopes.partition_point(|s| s.range.start < region.end);
        self.scopes.splice(first..last, new_scopes);
        splice_within(&mut self.definitions, region, new_definitions, |d| &d.range);
        splice_within(&mut self.resolved, region, new_resolved, |(range, _)| range);
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

/// `entries`, sorted by the range `range_of` reads, with those inside
/// `region` replaced by `new`, sorted the same way. An entry starting
/// inside a scope ends inside it.
fn splice_within<T>(
    entries: &mut Vec<T>,
    region: NodeBytes,
    new: Vec<T>,
    range_of: impl Fn(&T) -> &NodeBytes,
) {
    let first = entries.partition_point(|e| range_of(e).start < region.start);
    let last = entries.partition_point(|e| range_of(e).start < region.end);
    entries.splice(first..last, new);
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
/// `dirty`, widened over every scope of `prior` whose start or end lies in
/// it, the innermost scope of `tree` holding it, the outermost kept where
/// two nest. `None` when one has no such scope: the whole layer is rebuilt.
fn dirty_scopes(
    query: &LocalsQuery,
    prior: &LayerLocals,
    tree: &tree_sitter::Tree,
    rope: &ropey::Rope,
    dirty: &[RangeInclusive<usize>],
) -> Option<Vec<NodeBytes>> {
    if !query.roles.contains(&CaptureRole::Scope) {
        return None;
    }
    let mut cursor = QueryCursor::new();
    let mut regions = Vec::new();
    for region in dirty {
        let (mut lo, mut hi) = (*region.start(), *region.end());
        loop {
            let crossing = |s: &&ScopeEntry| {
                let (start, end) = key(&s.range);
                (lo..=hi).contains(&start) || (lo..=hi).contains(&end)
            };
            let (wider_lo, wider_hi) = prior
                .scopes
                .iter()
                .filter(crossing)
                .fold((lo, hi), |(l, h), s| {
                    (l.min(s.range.start), h.max(s.range.end))
                });
            if (wider_lo, wider_hi) == (lo, hi) {
                break;
            }
            (lo, hi) = (wider_lo, wider_hi);
        }
        cursor.set_byte_range(lo..hi + 1);
        let mut innermost: Option<NodeBytes> = None;
        let mut captures = cursor.captures(&query.query, tree.root_node(), RopeProvider(rope));
        while let Some((m, index)) = captures.next() {
            let capture = m.captures()[*index];
            let range = node_bytes(capture.node);
            let holds = range.start <= lo && hi < range.end;
            if query.role(capture.index) == CaptureRole::Scope
                && holds
                && innermost.is_none_or(|i| within(&range, &i))
            {
                innermost = Some(range);
            }
        }
        regions.push(innermost?);
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

/// The locals of `tree`, a layer parsed from `rope` with `bundle`: empty
/// for a grammar without a `locals.scm`. With `prior`, the layer's locals
/// before this parse, edited to it, only the scopes an edit can have
/// changed are rebuilt.
pub(crate) fn layer_locals(
    bundle: &GrammarBundle,
    tree: &tree_sitter::Tree,
    rope: &ropey::Rope,
    prior: Option<&LayerPrior<'_>>,
) -> Arc<LayerLocals> {
    let Some(query) = bundle.locals.as_ref() else {
        return Arc::default();
    };
    let Some(prior) = prior else {
        let mut locals = LayerLocals::default();
        locals.rebuild(query, tree, rope, None);
        return Arc::new(locals);
    };
    let dirty = dirty_bytes(prior, tree);
    if dirty.is_empty() {
        return Arc::clone(prior.locals);
    }
    let mut locals = LayerLocals::clone(prior.locals);
    match dirty_scopes(query, &locals, tree, rope, &dirty) {
        Some(regions) => {
            for region in regions {
                locals.rebuild(query, tree, rope, Some(region));
            }
        }
        None => locals.rebuild(query, tree, rope, None),
    }
    Arc::new(locals)
}

#[cfg(test)]
mod tests;
