use rustc_hash::FxHashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use streaming_iterator::StreamingIterator;

use crate::highlight::RopeProvider;

use crate::parse_worker::{MAX_INJECTION_DEPTH, ParsedInjection, run_parse};
use crate::registry::GrammarBundle;

// ── InjectionsQuery ────────────────────────────────────────────────────────────

/// A compiled `injections.scm` query plus the per-pattern settings needed to
/// resolve embedded-language regions at parse time (fenced code blocks,
/// combined `markdown.inline` layers, doc-comment content, etc.).
///
/// Built once per grammar attach (in `attach_grammar`), shared across every
/// buffer of that language via `Arc`, mirroring `GrammarBundle.query`.
pub struct InjectionsQuery {
    pub query: Arc<tree_sitter::Query>,
    /// Capture index for `@injection.content`, if the query defines it.
    pub content_capture: Option<u32>,
    /// Capture index for `@injection.language`, if the query defines it.
    pub language_capture: Option<u32>,
    /// Index-aligned with `query.pattern_count()`.
    pub patterns: Vec<PatternConfig>,
}

/// Per-pattern `#set!` properties from an `injections.scm` query.
pub struct PatternConfig {
    /// Static `#set! injection.language "x"` — used when the pattern has no
    /// `@injection.language` capture (e.g. doc-comment content).
    pub language: Option<String>,
    /// `#set! injection.combined` — all matches of this pattern in one buffer
    /// parse as a single layer with multiple included ranges (required by
    /// `markdown.inline`, whose grammar expects the whole document's inline
    /// spans as one tree).
    pub combined: bool,
    /// `#set! injection.include-unnamed-children` — by default, a content
    /// node's *unnamed* (anonymous/punctuation) children are cut out of the
    /// injected range; this property includes them instead, so the full
    /// node span is injected untouched. Named children are never cut out —
    /// they're meaningful grammar constructs, not delimiters — only unnamed
    /// ones (parens, commas, markers) are excluded by default.
    pub include_unnamed_children: bool,
}

impl InjectionsQuery {
    /// Build from a compiled `injections.scm` query. Unknown captures and
    /// properties (Helix queries carry extras like `injection.filename`) are
    /// silently ignored — only the standard tree-sitter injection convention
    /// is interpreted.
    pub fn new(query: Arc<tree_sitter::Query>) -> Self {
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
/// is false (the default), the node's *unnamed* (anonymous — punctuation,
/// delimiters, markers) children are cut out of the range, leaving the gaps;
/// named children (meaningful grammar constructs) are never cut out — they
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
/// `@injection.language` capture nodes (a few bytes — a fenced code block's
/// info string), so a one-off allocation here is not a hot-path concern —
/// this whole function runs once per parse, not per render frame.
fn node_text(node: tree_sitter::Node, rope: &ropey::Rope) -> String {
    rope.byte_slice(node.start_byte()..node.end_byte()).into()
}

/// One resolved (but not yet parsed) injection site: the language to parse
/// it with and the byte ranges to include.
struct InjectionGroup {
    language: String,
    ranges: Vec<tree_sitter::Range>,
}

/// Resolve and parse every embedded-language injection reachable from `tree`,
/// recursing into each injected layer's own injections up to
/// `MAX_INJECTION_DEPTH`.
///
/// `depth` is the depth being resolved *into* (1 for the root's direct
/// injections). Runs on the parse worker thread — `parser` is reused across
/// layers (language + included-ranges are reconfigured for each); `cancel`
/// is checked by the same progress callback as the root parse.
pub(crate) fn resolve_and_parse_injections(
    parser: &mut tree_sitter::Parser,
    tree: &tree_sitter::Tree,
    bundle: &GrammarBundle,
    rope: &ropey::Rope,
    langs: &FxHashMap<String, Arc<GrammarBundle>>,
    cancel: &AtomicBool,
    depth: u8,
) -> Vec<ParsedInjection> {
    let mut out = Vec::new();
    let Some(inj) = bundle.injections.as_ref() else {
        return out;
    };

    let mut non_combined: Vec<InjectionGroup> = Vec::new();
    let mut combined: FxHashMap<(usize, String), InjectionGroup> = FxHashMap::default();

    let mut cursor = tree_sitter::QueryCursor::new();
    let root = tree.root_node();
    let mut matches = cursor.matches(&inj.query, root, RopeProvider(rope));
    while let Some(m) = matches.next() {
        let pattern = &inj.patterns[m.pattern_index];

        // Static `#set! injection.language` takes priority over a dynamic
        // `@injection.language` capture (patterns rarely carry both).
        let language = pattern.language.clone().or_else(|| {
            inj.language_capture.and_then(|idx| {
                m.captures
                    .iter()
                    .find(|c| c.index == idx)
                    .map(|c| node_text(c.node, rope).to_lowercase())
            })
        });
        let Some(language) = language else { continue };

        // Unknown injection language — skip silently. No lazy install: the
        // user opts into grammars explicitly via PLUM. Every entry in `langs`
        // is grammared by construction (it's built from the grammar table).
        // Checked here, before `combined` insertion, so its FxHashMap only
        // ever keys on the trusted installed-grammar names — a dynamic
        // `@injection.language` capture is raw buffer text, and unfiltered
        // attacker-chosen keys in an unkeyed hash invite collision DoS.
        if !langs.contains_key(&language) {
            continue;
        }

        let Some(content_idx) = inj.content_capture else {
            continue;
        };
        let mut ranges: Vec<tree_sitter::Range> = m
            .captures
            .iter()
            .filter(|c| c.index == content_idx)
            .flat_map(|c| content_ranges(c.node, pattern.include_unnamed_children))
            .collect();
        if ranges.is_empty() {
            continue;
        }

        if pattern.combined {
            combined
                .entry((m.pattern_index, language.clone()))
                .or_insert_with(|| InjectionGroup {
                    language,
                    ranges: Vec::new(),
                })
                .ranges
                .append(&mut ranges);
        } else {
            non_combined.push(InjectionGroup { language, ranges });
        }
    }

    // FxHashMap iteration order is arbitrary — sort combined groups by their
    // (pattern_index, language) key so layer order (and thus same-depth `seq`
    // priority in `flatten_overlaps`) is stable across parses.
    let mut combined: Vec<_> = combined.into_iter().collect();
    combined.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

    for group in non_combined
        .into_iter()
        .chain(combined.into_iter().map(|(_, g)| g))
    {
        // Always present — unknown languages were filtered before grouping.
        let Some(child_bundle) = langs.get(&group.language) else {
            continue;
        };
        let ranges = normalize_ranges(group.ranges);
        if ranges.is_empty() {
            continue;
        }

        if parser
            .set_language(child_bundle.grammar.language())
            .is_err()
        {
            continue; // ABI mismatch on the injected grammar
        }
        if parser.set_included_ranges(&ranges).is_err() {
            continue; // non-monotonic ranges (shouldn't happen post-normalize)
        }

        let Some(child_tree) = run_parse(parser, rope, None, cancel) else {
            continue; // cancelled mid-parse
        };

        if depth < MAX_INJECTION_DEPTH {
            out.extend(resolve_and_parse_injections(
                parser,
                &child_tree,
                child_bundle,
                rope,
                langs,
                cancel,
                depth + 1,
            ));
        }

        out.push(ParsedInjection {
            bundle: Arc::clone(child_bundle),
            tree: child_tree,
            ranges,
            depth,
        });
    }

    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
