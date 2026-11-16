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
//! makes them local and leaves their highlight alone.

use std::sync::Arc;

use rustc_hash::FxHashMap;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Query, QueryCursor, QueryError, QueryProperty};

use hume_engine::theme::ScopeRegistry;
use hume_engine::types::ScopeId;

use crate::directives::{PatternDirectives, parse_patterns};
use crate::highlight::RopeProvider;
use crate::injections::node_text;
use crate::registry::GrammarBundle;

/// A compiled `locals.scm`.
pub struct LocalsQuery {
    query: Query,
    scope: Option<u32>,
    reference: Option<u32>,
    /// Per capture index, `Some` for a definition capture, holding the
    /// highlight its references take.
    definitions: Vec<Option<Option<ScopeId>>>,
    /// Per pattern, whether its `@local.scope` sees its parents'
    /// definitions.
    inherits: Vec<bool>,
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
        let definitions: Vec<Option<Option<ScopeId>>> = query
            .capture_names()
            .iter()
            .map(|name| match *name {
                "local.definition" => Some(None),
                name => name
                    .strip_prefix("local.definition.")
                    .map(|highlight| Some(registry.intern(highlight))),
            })
            .collect();
        let scope = query.capture_index_for_name("local.scope");
        let reference = query.capture_index_for_name("local.reference");
        if scope.is_none() && reference.is_none() && definitions.iter().all(Option::is_none) {
            return None;
        }
        Some(Self {
            query,
            scope,
            reference,
            definitions,
            inherits: patterns.iter().map(|p| !p.no_inheritance).collect(),
        })
    }
}

/// The locals of one layer's tree, from its grammar's `locals.scm`, built
/// once per parse and moved through each edit until the next.
#[derive(Clone, Debug, Default)]
pub struct LayerLocals {
    /// Every definition node and every reference that resolved, sorted.
    local_nodes: Vec<tree_sitter::Range>,
    /// Each resolved reference whose definition has a highlight, with that
    /// highlight, sorted by start.
    references: Vec<(tree_sitter::Range, ScopeId)>,
}

/// A scope open during [`LayerLocals::of`]'s walk.
struct OpenScope {
    end: usize,
    inherits: bool,
    /// Each name defined so far, with where its latest definition ends and
    /// that definition's highlight.
    definitions: FxHashMap<Box<str>, (usize, Option<ScopeId>)>,
}

impl LayerLocals {
    /// The locals `query` finds in `tree`, parsed from `rope`, walking the
    /// whole tree once in document order.
    pub(crate) fn of(query: &LocalsQuery, tree: &tree_sitter::Tree, rope: &ropey::Rope) -> Self {
        let mut scopes = vec![OpenScope {
            end: usize::MAX,
            inherits: true,
            definitions: FxHashMap::default(),
        }];
        let mut definition_nodes = Vec::new();
        let mut resolved = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut captures = cursor.captures(&query.query, tree.root_node(), RopeProvider(rope));
        while let Some((m, index)) = captures.next() {
            let capture = m.captures()[*index];
            let node = capture.node;
            let start = node.start_byte();
            while scopes.len() > 1 && scopes.last().is_some_and(|s| s.end <= start) {
                scopes.pop();
            }
            if Some(capture.index) == query.scope {
                scopes.push(OpenScope {
                    end: node.end_byte(),
                    inherits: query.inherits[m.pattern_index],
                    definitions: FxHashMap::default(),
                });
            } else if let Some(&Some(highlight)) = query.definitions.get(capture.index as usize) {
                let innermost = scopes.last_mut().expect("the root scope stays open");
                innermost
                    .definitions
                    .insert(node_text(node, rope).into(), (node.end_byte(), highlight));
                definition_nodes.push(node.range());
            } else if Some(capture.index) == query.reference
                && let Some(highlight) = resolve(&scopes, &node_text(node, rope), start)
            {
                resolved.push((node.range(), highlight));
            }
        }
        let key = |r: &tree_sitter::Range| (r.start_byte, r.end_byte);
        definition_nodes.sort_by_key(key);
        // A definition's own node is never a reference to it.
        resolved.retain(|(range, _)| {
            definition_nodes
                .binary_search_by_key(&key(range), key)
                .is_err()
        });
        resolved.sort_by_key(|(range, _)| key(range));
        let mut local_nodes = definition_nodes;
        local_nodes.extend(resolved.iter().map(|&(range, _)| range));
        local_nodes.sort_by_key(key);
        local_nodes.dedup();
        Self {
            local_nodes,
            references: resolved
                .into_iter()
                .filter_map(|(range, highlight)| Some((range, highlight?)))
                .collect(),
        }
    }

    /// Whether the node spanning `start..end` is local.
    pub(crate) fn is_local(&self, start: usize, end: usize) -> bool {
        self.local_nodes
            .binary_search_by_key(&(start, end), |r| (r.start_byte, r.end_byte))
            .is_ok()
    }

    /// The resolved references starting in `start..end`, each with its
    /// definition's highlight.
    pub(crate) fn references_in(
        &self,
        start: usize,
        end: usize,
    ) -> impl Iterator<Item = &(tree_sitter::Range, ScopeId)> {
        let first = self
            .references
            .partition_point(|(r, _)| r.start_byte < start);
        self.references[first..]
            .iter()
            .take_while(move |(r, _)| r.start_byte < end)
    }

    /// Move every range through `edit`, as `Tree::edit` moves a tree.
    pub(crate) fn edit(&mut self, edit: &tree_sitter::InputEdit) {
        for range in &mut self.local_nodes {
            edit.edit_range(range);
        }
        for (range, _) in &mut self.references {
            edit.edit_range(range);
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

/// The locals of `tree`, a layer parsed from `rope` with `bundle`: empty
/// for a grammar without a `locals.scm`.
pub(crate) fn layer_locals(
    bundle: &GrammarBundle,
    tree: &tree_sitter::Tree,
    rope: &ropey::Rope,
) -> Arc<LayerLocals> {
    Arc::new(
        bundle
            .locals
            .as_ref()
            .map_or_else(LayerLocals::default, |query| {
                LayerLocals::of(query, tree, rope)
            }),
    )
}

#[cfg(test)]
mod tests;
