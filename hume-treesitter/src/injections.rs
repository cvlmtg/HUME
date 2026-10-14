use rustc_hash::FxHashMap;
use std::ops::{Range, RangeInclusive};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use streaming_iterator::StreamingIterator;

use crate::edited_bytes::{EditedBytes, push_merging};
use crate::highlight::RopeProvider;

use crate::parse_worker::{MAX_INJECTION_DEPTH, ParsedInjection, run_parse};
use crate::registry::GrammarBundle;

/// A compiled `injections.scm` query plus the per-pattern settings needed to
/// resolve embedded-language regions at parse time (fenced code blocks,
/// combined `markdown.inline` layers, doc-comment content, etc.).
///
/// Built once per grammar attach (in `attach_grammar`) and held by the
/// language's `GrammarBundle`, which every buffer of that language shares
/// through an `Arc`.
pub struct InjectionsQuery {
    pub(crate) query: Arc<tree_sitter::Query>,
    /// Capture index for `@injection.content`, if the query defines it.
    pub(crate) content_capture: Option<u32>,
    /// Capture index for `@injection.language`, if the query defines it.
    pub(crate) language_capture: Option<u32>,
    /// Index-aligned with `query.pattern_count()`.
    pub(crate) patterns: Vec<PatternConfig>,
}

/// Per-pattern `#set!` properties from an `injections.scm` query.
pub(crate) struct PatternConfig {
    /// Static `#set! injection.language "x"`, used when the pattern has no
    /// `@injection.language` capture (e.g. doc-comment content).
    pub(crate) language: Option<String>,
    /// `#set! injection.combined`: all matches of this pattern in one buffer
    /// parse as a single layer with multiple included ranges (required by
    /// `markdown.inline`, whose grammar expects the whole document's inline
    /// spans as one tree).
    pub(crate) combined: bool,
    /// `#set! injection.include-unnamed-children`: by default, a content
    /// node's *unnamed* (anonymous/punctuation) children are cut out of the
    /// injected range; this property includes them instead, so the full
    /// node span is injected untouched. Named children are never cut out
    /// (they're meaningful grammar constructs, not delimiters); only unnamed
    /// ones (parens, commas, markers) are excluded by default.
    pub(crate) include_unnamed_children: bool,
}

impl InjectionsQuery {
    /// Build from a compiled `injections.scm` query. Unknown captures and
    /// properties (the installed queries carry extras like
    /// `injection.filename`) are ignored; only the standard tree-sitter
    /// injection convention is interpreted.
    pub(crate) fn new(query: Arc<tree_sitter::Query>) -> Self {
        let content_capture = query.capture_index_for_name("injection.content");
        let language_capture = query.capture_index_for_name("injection.language");
        let patterns = (0..query.pattern_count())
            .map(|i| {
                let mut language = None;
                let mut combined = false;
                let mut include_unnamed_children = false;
                for prop in query.property_settings(i) {
                    match &*prop.key {
                        "injection.language" => {
                            language = prop.value.as_deref().map(str::to_owned);
                        }
                        "injection.combined" => combined = true,
                        "injection.include-unnamed-children" => include_unnamed_children = true,
                        _ => {}
                    }
                }
                PatternConfig {
                    language,
                    combined,
                    include_unnamed_children,
                }
            })
            .collect();
        Self {
            query,
            content_capture,
            language_capture,
            patterns,
        }
    }
}

// ── Injection resolution (worker thread) ──────────────────────────────────────

/// Byte ranges for `node`'s injectable content. When `include_unnamed_children`
/// is false (the default), the node's *unnamed* (anonymous: punctuation,
/// delimiters, markers) children are cut out of the range, leaving the gaps;
/// named children (meaningful grammar constructs) are never cut out; they
/// stay part of the surrounding content segment. When true, or the node has
/// no children, returns the node's own full range as a single entry.
fn content_ranges(
    node: tree_sitter::Node,
    include_unnamed_children: bool,
) -> Vec<tree_sitter::Range> {
    if include_unnamed_children || node.child_count() == 0 {
        return vec![node.range()];
    }
    let mut ranges = Vec::new();
    let mut prev_end_byte = node.start_byte();
    let mut prev_end_point = node.start_position();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() {
            continue; // stays part of the surrounding content, not a cut point
        }
        if child.start_byte() > prev_end_byte {
            ranges.push(tree_sitter::Range {
                start_byte: prev_end_byte,
                end_byte: child.start_byte(),
                start_point: prev_end_point,
                end_point: child.start_position(),
            });
        }
        prev_end_byte = child.end_byte();
        prev_end_point = child.end_position();
    }
    if prev_end_byte < node.end_byte() {
        ranges.push(tree_sitter::Range {
            start_byte: prev_end_byte,
            end_byte: node.end_byte(),
            start_point: prev_end_point,
            end_point: node.end_position(),
        });
    }
    ranges
}

/// Sort `ranges` by start byte, drop zero-width entries, and merge
/// overlapping or touching ranges. `Parser::set_included_ranges` requires
/// strictly increasing, non-overlapping ranges.
fn normalize_ranges(mut ranges: Vec<tree_sitter::Range>) -> Vec<tree_sitter::Range> {
    ranges.retain(|r| r.start_byte < r.end_byte);
    ranges.sort_unstable_by_key(|r| r.start_byte);
    let mut out: Vec<tree_sitter::Range> = Vec::with_capacity(ranges.len());
    for r in ranges {
        if let Some(last) = out.last_mut()
            && r.start_byte <= last.end_byte
        {
            if r.end_byte > last.end_byte {
                last.end_byte = r.end_byte;
                last.end_point = r.end_point;
            }
            continue;
        }
        out.push(r);
    }
    out
}

/// Read a small node's text directly from the rope. Only used for
/// `@injection.language` capture nodes (a few bytes: a fenced code block's
/// info string), so a one-off allocation here is not a hot-path concern:
/// this whole function runs once per parse, not per render frame.
fn node_text(node: tree_sitter::Node, rope: &ropey::Rope) -> String {
    rope.byte_slice(node.start_byte()..node.end_byte()).into()
}

/// The smallest range holding both `a` and `b`.
fn hull(a: tree_sitter::Range, b: tree_sitter::Range) -> tree_sitter::Range {
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

/// One match of a layer's injections query. It depends only on the layer's
/// tree and text, not on which grammars are installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InjectionSite {
    pattern_index: usize,
    /// The pattern's static `#set! injection.language`, else the lowercased
    /// text of its `@injection.language` capture.
    language: String,
    /// The hull of every node the match captured.
    span: tree_sitter::Range,
    /// The content ranges, unnormalized. Never empty.
    ranges: Vec<tree_sitter::Range>,
}

/// A layer's injection sites, ordered by the start byte of each site's
/// first content range, then by pattern index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InjectionSites(Vec<InjectionSite>);

impl InjectionSite {
    fn order(&self) -> (usize, usize) {
        (self.ranges[0].start_byte, self.pattern_index)
    }
}

impl InjectionSites {
    /// `sites` in order, each once. Sites tied on the order keep the order
    /// they are given in.
    fn canonical(mut sites: Vec<InjectionSite>) -> Self {
        sites.sort_by_key(InjectionSite::order);
        let mut unique: Vec<InjectionSite> = Vec::with_capacity(sites.len());
        for site in sites {
            let seen = unique
                .iter()
                .rev()
                .take_while(|kept| kept.order() == site.order())
                .any(|kept| *kept == site);
            if !seen {
                unique.push(site);
            }
        }
        Self(unique)
    }

    /// Move every site's span and ranges through `edit`, as `Tree::edit`
    /// moves a tree's included ranges.
    pub(crate) fn edit(&mut self, edit: &tree_sitter::InputEdit) {
        for site in &mut self.0 {
            edit.edit_range(&mut site.span);
            for range in &mut site.ranges {
                edit.edit_range(range);
            }
        }
    }
}

/// Every match of `inj` over `tree` that names a language and captures
/// content, in match order: over the whole tree, or one bounded run per
/// region of `within`, which finds the matches whose root node (the parent
/// of a pattern without a single root) meets the region.
fn query_sites(
    inj: &InjectionsQuery,
    tree: &tree_sitter::Tree,
    rope: &ropey::Rope,
    within: Option<&[Range<usize>]>,
) -> Vec<InjectionSite> {
    let Some(content_idx) = inj.content_capture else {
        return Vec::new();
    };
    let mut sites = Vec::new();
    let mut cursor = tree_sitter::QueryCursor::new();
    match within {
        None => push_sites(inj, content_idx, &mut cursor, tree, rope, &mut sites),
        Some(regions) => {
            for region in regions {
                cursor.set_byte_range(region.clone());
                push_sites(inj, content_idx, &mut cursor, tree, rope, &mut sites);
            }
        }
    }
    sites
}

/// Push onto `sites` every match `cursor` finds of `inj` over `tree` that
/// names a language and captures content.
fn push_sites(
    inj: &InjectionsQuery,
    content_idx: u32,
    cursor: &mut tree_sitter::QueryCursor,
    tree: &tree_sitter::Tree,
    rope: &ropey::Rope,
    sites: &mut Vec<InjectionSite>,
) {
    let mut matches = cursor.matches(&inj.query, tree.root_node(), RopeProvider(rope));
    while let Some(m) = matches.next() {
        let pattern = &inj.patterns[m.pattern_index];

        // Static `#set! injection.language` takes priority over a dynamic
        // `@injection.language` capture (patterns rarely carry both).
        let language = pattern.language.clone().or_else(|| {
            inj.language_capture.and_then(|idx| {
                m.captures()
                    .iter()
                    .find(|c| c.index == idx)
                    .map(|c| node_text(c.node, rope).to_lowercase())
            })
        });
        let Some(language) = language else { continue };

        let ranges: Vec<tree_sitter::Range> = m
            .captures()
            .iter()
            .filter(|c| c.index == content_idx)
            .flat_map(|c| content_ranges(c.node, pattern.include_unnamed_children))
            .collect();
        if ranges.is_empty() {
            continue;
        }
        let span = m
            .captures()
            .iter()
            .map(|c| c.node.range())
            .reduce(hull)
            .expect("a match with content ranges captured its node");
        sites.push(InjectionSite {
            pattern_index: m.pattern_index,
            language,
            span,
            ranges,
        });
    }
}

/// A layer's committed sites and tree, edited to the text being parsed,
/// and where they were found.
pub(crate) struct SitePrior<'p> {
    /// The layer's committed tree, edited to the text being parsed.
    pub(crate) old_tree: &'p tree_sitter::Tree,
    /// The sites of `old_tree`, edited with it.
    pub(crate) sites: &'p InjectionSites,
    /// The bytes from the layer's first range's start to its last one's
    /// end.
    pub(crate) hull: RangeInclusive<usize>,
    /// Whether the new tree is a reparse of `old_tree`, rather than
    /// `old_tree` itself.
    pub(crate) reparsed: bool,
    /// The bytes the edits changed.
    pub(crate) edited: &'p EditedBytes,
}

/// The bytes where a layer's sites can differ from `prior`'s, as sorted,
/// disjoint closed intervals: the edited bytes in the layer's hull and,
/// when `tree` was reparsed from `prior.old_tree`, every range the two
/// trees differ in, its end byte included.
fn dirty_bytes(prior: &SitePrior<'_>, tree: &tree_sitter::Tree) -> Vec<RangeInclusive<usize>> {
    let mut dirty: Vec<RangeInclusive<usize>> = prior.edited.within(prior.hull.clone()).collect();
    if prior.reparsed {
        dirty.extend(
            prior
                .old_tree
                .changed_ranges(tree)
                .map(|range| range.start_byte..=range.end_byte),
        );
    }
    dirty.sort_unstable_by_key(|region| *region.start());
    let mut merged: Vec<RangeInclusive<usize>> = Vec::with_capacity(dirty.len());
    for region in dirty {
        push_merging(&mut merged, region);
    }
    merged
}

/// The sites of `tree`: a full query without `prior`, else `prior`'s sites
/// with those that can have changed queried again.
///
/// A match depends only on its root node's subtree: its structure, and the
/// text its predicates and language capture read. A bounded run over the
/// dirty bytes D, each region queried from one byte before it, finds the
/// matches whose root meets the region, on the old tree and the new one
/// alike. A match whose root misses D has the same root subtree and text in
/// both trees: it is carried, and the new run does not find it again. A
/// match whose root meets D is found by the old-tree run, so it is dropped,
/// and by the new-tree run if it still holds. The changed ranges promise
/// that a byte outside them has the same ancestor symbols in both trees,
/// not the same extents; a node whose extent or children change has a
/// changed byte inside or next to its old or new extent. The byte before a
/// region is queried with it, and D's closed ends already hold the byte
/// after each change. D also takes the edited bytes in the layer's hull,
/// since tree-sitter can leave an edit out of the changed ranges (a
/// deleted info string) and a node's text includes the gaps between the
/// layer's ranges. A carried site's span must also clear D: the old-tree
/// run reads the new text, so a predicate over a captured node an edit
/// changed can fail there and leave a stale site undropped.
fn resolve_sites(
    inj: &InjectionsQuery,
    tree: &tree_sitter::Tree,
    rope: &ropey::Rope,
    prior: Option<SitePrior<'_>>,
) -> InjectionSites {
    let Some(prior) = prior else {
        return InjectionSites::canonical(query_sites(inj, tree, rope, None));
    };
    let dirty = dirty_bytes(&prior, tree);
    if dirty.is_empty() {
        return prior.sites.clone();
    }
    let regions: Vec<Range<usize>> = dirty
        .iter()
        .map(|region| region.start().saturating_sub(1)..region.end() + 1)
        .collect();
    let dropped = query_sites(inj, prior.old_tree, rope, Some(&regions));
    let carried = prior.sites.0.iter().filter(|site| {
        dirty.iter().all(|region| {
            site.span.end_byte < *region.start() || site.span.start_byte > *region.end()
        }) && !dropped.contains(site)
    });
    let new = if prior.reparsed {
        query_sites(inj, tree, rope, Some(&regions))
    } else {
        dropped.clone()
    };
    InjectionSites::canonical(carried.cloned().chain(new).collect())
}

/// One resolved (but not yet parsed) injection: the grammar to parse it
/// with and the normalized, non-empty byte ranges to include. Carrying the
/// resolved `bundle` (not just its name) makes a second, redundant `langs`
/// lookup at consumption time structurally impossible instead of merely
/// unreachable.
struct InjectionGroup {
    bundle: Arc<GrammarBundle>,
    ranges: Vec<tree_sitter::Range>,
}

/// The layers `sites` inject with the grammars in `langs`, in the order
/// they are installed: each non-combined site in site order, then one
/// layer per combined pattern and language, merging all of its sites, in
/// (pattern index, language) order.
fn group_sites(
    inj: &InjectionsQuery,
    sites: &InjectionSites,
    langs: &FxHashMap<String, Arc<GrammarBundle>>,
) -> Vec<InjectionGroup> {
    let mut non_combined: Vec<InjectionGroup> = Vec::new();
    let mut combined: FxHashMap<(usize, &str), InjectionGroup> = FxHashMap::default();
    for site in &sites.0 {
        // Unknown injection language: skip silently. No lazy install: the
        // user opts into grammars explicitly via PLUM. Every entry in `langs`
        // is grammared, since it's built from the grammar table.
        // Resolved here, before `combined` insertion, so its FxHashMap only
        // ever keys on the trusted installed-grammar names. A dynamic
        // `@injection.language` capture is raw buffer text, and unfiltered
        // attacker-chosen keys in an unkeyed hash invite collision DoS.
        let Some(bundle) = langs.get(&site.language) else {
            continue;
        };
        if inj.patterns[site.pattern_index].combined {
            combined
                .entry((site.pattern_index, site.language.as_str()))
                .or_insert_with(|| InjectionGroup {
                    bundle: Arc::clone(bundle),
                    ranges: Vec::new(),
                })
                .ranges
                .extend_from_slice(&site.ranges);
        } else {
            non_combined.push(InjectionGroup {
                bundle: Arc::clone(bundle),
                ranges: site.ranges.clone(),
            });
        }
    }

    // FxHashMap iteration order is arbitrary, so sort combined groups by their
    // (pattern_index, language) key so layer order (and thus same-depth `seq`
    // priority in `flatten_overlaps`) is stable across parses.
    let mut combined: Vec<_> = combined.into_iter().collect();
    combined.sort_unstable_by_key(|&(key, _)| key);

    non_combined
        .into_iter()
        .chain(combined.into_iter().map(|(_, group)| group))
        .filter_map(|group| {
            let ranges = normalize_ranges(group.ranges);
            (!ranges.is_empty()).then_some(InjectionGroup {
                bundle: group.bundle,
                ranges,
            })
        })
        .collect()
}

/// An injected layer of the committed layers, edited to the text being
/// parsed.
pub(crate) struct PriorInjection {
    pub(crate) bundle: Arc<GrammarBundle>,
    pub(crate) depth: u8,
    pub(crate) tree: tree_sitter::Tree,
    /// Never empty, as no injected layer's ranges are.
    pub(crate) ranges: Vec<tree_sitter::Range>,
    pub(crate) sites: InjectionSites,
}

impl PriorInjection {
    fn key(&self) -> (u8, usize) {
        (self.depth, self.ranges[0].start_byte)
    }

    /// What the sites of this layer's next tree resolve from.
    fn site_prior<'p>(&'p self, edited: &'p EditedBytes, reparsed: bool) -> SitePrior<'p> {
        let last = self
            .ranges
            .last()
            .expect("an injected layer's ranges are never empty");
        SitePrior {
            old_tree: &self.tree,
            sites: &self.sites,
            hull: self.ranges[0].start_byte..=last.end_byte,
            reparsed,
            edited,
        }
    }
}

/// The committed layers' injected layers, sorted by depth, then by the
/// start byte of their first range.
pub(crate) struct PriorInjections {
    layers: Vec<PriorInjection>,
}

impl FromIterator<PriorInjection> for PriorInjections {
    fn from_iter<I: IntoIterator<Item = PriorInjection>>(layers: I) -> Self {
        let mut layers: Vec<PriorInjection> = layers.into_iter().collect();
        layers.sort_by_key(PriorInjection::key);
        Self { layers }
    }
}

/// How a layer about to be parsed uses the committed layers.
enum Reuse<'p> {
    /// Install the prior layer's tree as it is, unparsed.
    Unchanged(SitePrior<'p>),
    /// Reparse against the prior layer's tree.
    Incremental(SitePrior<'p>),
    /// Parse with no old tree.
    Fresh,
}

impl PriorInjections {
    /// The prior layer of the same bundle, compared by identity, at the
    /// same depth and over the same ranges, bytes and points alike, is
    /// installed unchanged when `edited` does not touch its ranges and
    /// reparsed against otherwise. A language name may since be bound to
    /// another grammar, and tree-sitter refuses an old tree of another
    /// language. Equal ranges leave the old tree no included-range
    /// difference from the new parse. `ranges` is never empty.
    ///
    /// A parse reads only its grammar, its ranges, the text inside them
    /// and the columns the ranges start at, all unchanged; the baked tree,
    /// holding the same nodes at the positions the edits moved them to, is
    /// a parse of the new text.
    fn reuse<'p>(
        &'p self,
        bundle: &Arc<GrammarBundle>,
        depth: u8,
        ranges: &[tree_sitter::Range],
        edited: &'p EditedBytes,
    ) -> Reuse<'p> {
        let key = (depth, ranges[0].start_byte);
        let from = self.layers.partition_point(|prior| prior.key() < key);
        let Some(prior) = self.layers[from..]
            .iter()
            .take_while(|prior| prior.key() == key)
            .find(|prior| Arc::ptr_eq(&prior.bundle, bundle) && prior.ranges == ranges)
        else {
            return Reuse::Fresh;
        };
        if edited.touches(ranges) {
            Reuse::Incremental(prior.site_prior(edited, true))
        } else {
            Reuse::Unchanged(prior.site_prior(edited, false))
        }
    }
}

/// `group`'s ranges parsed with its grammar against `old_tree`. `None` when
/// the parser rejects the grammar or the ranges, or the parse is cancelled.
fn parse_group(
    parser: &mut tree_sitter::Parser,
    cx: &InjectionContext<'_>,
    group: &InjectionGroup,
    old_tree: Option<&tree_sitter::Tree>,
) -> Option<tree_sitter::Tree> {
    // An ABI mismatch on the injected grammar.
    parser.set_language(group.bundle.grammar.language()).ok()?;
    // Non-monotonic ranges, which normalization rules out.
    parser.set_included_ranges(&group.ranges).ok()?;
    run_parse(parser, cx.rope, old_tree, cx.cancel)
}

/// What resolving a layer's injections reads besides the layer itself.
pub(crate) struct InjectionContext<'a> {
    pub(crate) rope: &'a ropey::Rope,
    /// Every grammared language, by the name an injection gives.
    pub(crate) langs: &'a FxHashMap<String, Arc<GrammarBundle>>,
    /// The committed layers' injected layers, when they are edited to
    /// `rope`'s text, and the bytes those edits changed.
    pub(crate) prior: Option<(&'a PriorInjections, &'a EditedBytes)>,
    /// Checked by the same progress callback as the root parse.
    pub(crate) cancel: &'a AtomicBool,
}

/// The injection sites of `tree`, a layer of `bundle`'s grammar, resolved
/// from `prior` as [`resolve_sites`] does, and every layer they inject,
/// parsed, its own injections resolved the same way and listed right
/// before it.
///
/// `depth` is the depth being resolved *into* (1 for the root's direct
/// injections). A layer at `MAX_INJECTION_DEPTH`, or of a grammar without
/// an injections query, has no sites and injects nothing. Each injected
/// layer reuses or reparses against the prior layer
/// [`PriorInjections::reuse`] picks, and parses in full without one.
/// `parser` is reused across layers (language + included-ranges are
/// reconfigured for each).
pub(crate) fn resolve_and_parse_injections(
    parser: &mut tree_sitter::Parser,
    cx: &InjectionContext<'_>,
    tree: &tree_sitter::Tree,
    bundle: &GrammarBundle,
    depth: u8,
    prior: Option<SitePrior<'_>>,
) -> (InjectionSites, Vec<ParsedInjection>) {
    let Some(inj) = bundle
        .injections
        .as_ref()
        .filter(|_| depth <= MAX_INJECTION_DEPTH)
    else {
        return (InjectionSites::default(), Vec::new());
    };
    let sites = resolve_sites(inj, tree, cx.rope, prior);

    let mut out = Vec::new();
    for group in group_sites(inj, &sites, cx.langs) {
        let reuse = cx.prior.map_or(Reuse::Fresh, |(prior, edited)| {
            prior.reuse(&group.bundle, depth, &group.ranges, edited)
        });
        let (child_tree, child_prior) = match reuse {
            Reuse::Unchanged(prior) => (prior.old_tree.clone(), Some(prior)),
            Reuse::Incremental(prior) => {
                let Some(tree) = parse_group(parser, cx, &group, Some(prior.old_tree)) else {
                    continue;
                };
                (tree, Some(prior))
            }
            Reuse::Fresh => {
                let Some(tree) = parse_group(parser, cx, &group, None) else {
                    continue;
                };
                (tree, None)
            }
        };

        let (child_sites, descendants) = resolve_and_parse_injections(
            parser,
            cx,
            &child_tree,
            &group.bundle,
            depth + 1,
            child_prior,
        );
        out.extend(descendants);
        out.push(ParsedInjection {
            bundle: group.bundle,
            tree: child_tree,
            ranges: group.ranges,
            depth,
            sites: child_sites,
        });
    }

    (sites, out)
}

#[cfg(test)]
mod tests;
