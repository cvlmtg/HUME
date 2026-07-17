use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use streaming_iterator::StreamingIterator;

use hume_engine::builtins::tree_sitter_hl::RopeProvider;

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
    langs: &HashMap<String, Arc<GrammarBundle>>,
    cancel: &AtomicBool,
    depth: u8,
) -> Vec<ParsedInjection> {
    let mut out = Vec::new();
    let Some(inj) = bundle.injections.as_ref() else {
        return out;
    };

    let mut non_combined: Vec<InjectionGroup> = Vec::new();
    let mut combined: HashMap<(usize, String), InjectionGroup> = HashMap::new();

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

    // HashMap iteration order is arbitrary — sort combined groups by their
    // (pattern_index, language) key so layer order (and thus same-depth `seq`
    // priority in `flatten_overlaps`) is stable across parses.
    let mut combined: Vec<_> = combined.into_iter().collect();
    combined.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

    for group in non_combined
        .into_iter()
        .chain(combined.into_iter().map(|(_, g)| g))
    {
        // Unknown injection language — skip silently. No lazy install: the
        // user opts into grammars explicitly via PLUM. Every entry in `langs`
        // is grammared by construction (it's built from the grammar table),
        // so no further "does it have a grammar" check is needed here.
        let Some(child_bundle) = langs.get(&group.language) else {
            continue;
        };
        let ranges = normalize_ranges(group.ranges);
        if ranges.is_empty() {
            continue;
        }

        if parser.set_language(child_bundle.grammar.language()).is_err() {
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
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;

    use hume_engine::builtins::tree_sitter_hl::TreeSitterHighlighter;
    use hume_engine::grammar::LoadedGrammar;
    use hume_engine::theme::ScopeRegistry;

    use super::*;
    use crate::registry::GrammarBundle;
    use crate::test_support::{grammar_injections_path, grammar_parser_path, grammar_query_path};

    /// Load a real grammar fixture with an optional custom injections source
    /// (overriding whatever `injections.scm` the fixture ships, if any) —
    /// keeps each test's injection query minimal and self-contained rather
    /// than depending on upstream Helix query wording.
    fn make_bundle(name: &str, symbol: &str, injections_src: Option<&str>) -> Arc<GrammarBundle> {
        let parser_path = grammar_parser_path(name);
        let grammar = LoadedGrammar::open(&parser_path, symbol).expect("load grammar");
        let highlights_src =
            std::fs::read_to_string(grammar_query_path(name)).expect("read highlights.scm");
        let query = Arc::new(
            tree_sitter::Query::new(grammar.language(), &highlights_src).expect("compile query"),
        );
        let mut registry = ScopeRegistry::new();
        let highlighter = Arc::new(TreeSitterHighlighter::from_shared_query(
            query,
            &mut registry,
        ));
        let injections = injections_src.map(|src| {
            let q = Arc::new(
                tree_sitter::Query::new(grammar.language(), src).expect("compile injections"),
            );
            InjectionsQuery::new(q)
        });
        Arc::new(GrammarBundle {
            grammar,
            highlighter,
            injections,
            config_gen: next_test_config_gen(),
        })
    }

    /// Distinct per call, mirroring `LanguageRegistry`'s `config_gen`
    /// invariant so tests that compare configs by gen see real identity.
    fn next_test_config_gen() -> u32 {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT_GEN: AtomicU32 = AtomicU32::new(0);
        NEXT_GEN.fetch_add(1, Ordering::Relaxed)
    }

    fn skip_if_missing(name: &str) -> bool {
        if !grammar_parser_path(name).exists() {
            eprintln!(
                "skipping: {name} grammar fixture not fetched — run scripts/fetch-test-grammars.sh"
            );
            true
        } else {
            false
        }
    }

    fn parse(bundle: &GrammarBundle, source: &str) -> (tree_sitter::Parser, tree_sitter::Tree) {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(bundle.grammar.language()).unwrap();
        let tree = parser.parse(source, None).expect("parse");
        (parser, tree)
    }

    // ── content_ranges (pure) ─────────────────────────────────────────────────

    #[test]
    fn content_ranges_excludes_only_unnamed_children_by_default() {
        if skip_if_missing("json") {
            return;
        }
        let json = make_bundle("json", "tree_sitter_json", None);
        // `[1,2]` parses as an `array` node whose direct children are, in
        // order: `[` (unnamed), `number` "1" (named), `,` (unnamed), `number`
        // "2" (named), `]` (unnamed). Excluding only unnamed children must
        // leave the two number spans (the named children) as content,
        // cutting out just the bracket/comma punctuation.
        let (_parser, tree) = parse(&json, "[1,2]\n");
        let array = tree.root_node().named_child(0).expect("array node");
        assert_eq!(array.kind(), "array");

        let content = content_ranges(array, false);
        let byte_ranges: Vec<(usize, usize)> =
            content.iter().map(|r| (r.start_byte, r.end_byte)).collect();
        assert_eq!(
            byte_ranges,
            vec![(1, 2), (3, 4)],
            "expected the two named `number` spans, excluding `[`, `,`, `]`; got {content:?}"
        );
    }

    #[test]
    fn content_ranges_include_unnamed_children_returns_whole_node() {
        if skip_if_missing("json") {
            return;
        }
        let json = make_bundle("json", "tree_sitter_json", None);
        let (_parser, tree) = parse(&json, "[1,2]\n");
        let array = tree.root_node().named_child(0).expect("array node");

        let included = content_ranges(array, true);
        assert_eq!(included, vec![array.range()]);
    }

    // ── normalize_ranges (pure) ───────────────────────────────────────────────

    fn range(start: usize, end: usize) -> tree_sitter::Range {
        tree_sitter::Range {
            start_byte: start,
            end_byte: end,
            start_point: tree_sitter::Point {
                row: 0,
                column: start,
            },
            end_point: tree_sitter::Point {
                row: 0,
                column: end,
            },
        }
    }

    #[test]
    fn normalize_ranges_drops_zero_width_and_sorts() {
        let got = normalize_ranges(vec![range(5, 8), range(3, 3), range(0, 2)]);
        assert_eq!(got, vec![range(0, 2), range(5, 8)]);
    }

    #[test]
    fn normalize_ranges_merges_overlapping_and_touching() {
        let got = normalize_ranges(vec![range(0, 5), range(3, 8), range(8, 10)]);
        assert_eq!(
            got,
            vec![range(0, 10)],
            "overlapping [0,5)+[3,8) and touching [8,10) must merge into one"
        );
    }

    // ── resolve_and_parse_injections ──────────────────────────────────────────

    #[test]
    fn static_language_override_wins_regardless_of_content() {
        if skip_if_missing("json") || skip_if_missing("rust") {
            return;
        }
        let json = make_bundle(
            "json",
            "tree_sitter_json",
            Some(
                r#"((string_content) @injection.content (#set! injection.language "rust") (#set! injection.include-unnamed-children))"#,
            ),
        );
        let rust = make_bundle("rust", "tree_sitter_rust", None);
        let mut langs = HashMap::new();
        langs.insert("rust".to_owned(), Arc::clone(&rust));

        let source = "[\"hello\"]\n";
        let (mut parser, tree) = parse(&json, source);
        let rope = ropey::Rope::from_str(source);
        let cancel = AtomicBool::new(false);

        let out =
            resolve_and_parse_injections(&mut parser, &tree, &json, &rope, &langs, &cancel, 1);
        assert_eq!(out.len(), 1, "expected exactly one injected layer");
        assert!(
            Arc::ptr_eq(&out[0].bundle, &rust),
            "expected the rust bundle to be resolved"
        );
        assert_eq!(out[0].depth, 1);
    }

    #[test]
    fn unknown_injection_language_is_skipped_silently() {
        if skip_if_missing("json") || skip_if_missing("rust") {
            return;
        }
        let json = make_bundle(
            "json",
            "tree_sitter_json",
            Some(
                r#"((string_content) @injection.content (#set! injection.language "no-such-language") (#set! injection.include-unnamed-children))"#,
            ),
        );
        // Non-empty but irrelevant: proves the lookup is genuinely by-key,
        // not just "map happens to be empty".
        let mut langs = HashMap::new();
        langs.insert(
            "rust".to_owned(),
            make_bundle("rust", "tree_sitter_rust", None),
        );

        let source = "[\"hello\"]\n";
        let (mut parser, tree) = parse(&json, source);
        let rope = ropey::Rope::from_str(source);
        let cancel = AtomicBool::new(false);

        let out =
            resolve_and_parse_injections(&mut parser, &tree, &json, &rope, &langs, &cancel, 1);
        assert!(
            out.is_empty(),
            "unresolvable injection language must produce no layer, got: {} layers",
            out.len()
        );
    }

    #[test]
    fn combined_merges_multiple_matches_into_one_layer() {
        if skip_if_missing("json") || skip_if_missing("rust") {
            return;
        }
        let json = make_bundle(
            "json",
            "tree_sitter_json",
            Some(
                r#"((string_content) @injection.content (#set! injection.language "rust") (#set! injection.combined) (#set! injection.include-unnamed-children))"#,
            ),
        );
        let rust = make_bundle("rust", "tree_sitter_rust", None);
        let mut langs = HashMap::new();
        langs.insert("rust".to_owned(), rust);

        // Three string literals — without `injection.combined` these would be
        // three separate layers; with it, exactly one layer with 3 ranges.
        let source = "[\"a\", \"b\", \"c\"]\n";
        let (mut parser, tree) = parse(&json, source);
        let rope = ropey::Rope::from_str(source);
        let cancel = AtomicBool::new(false);

        let out =
            resolve_and_parse_injections(&mut parser, &tree, &json, &rope, &langs, &cancel, 1);
        assert_eq!(
            out.len(),
            1,
            "combined matches of the same pattern+language must merge into one layer"
        );
        assert_eq!(
            out[0].ranges.len(),
            3,
            "one range per string literal, got: {:?}",
            out[0].ranges
        );
    }

    #[test]
    fn depth_cap_stops_recursion_at_max_depth() {
        if skip_if_missing("json") {
            return;
        }
        // Self-injecting: every `array` node re-parses its own (identical)
        // text as json again, matching `array` once more in the fresh tree —
        // unbounded without a depth cap, since the content never changes.
        let json = make_bundle(
            "json",
            "tree_sitter_json",
            Some(
                r#"((array) @injection.content (#set! injection.language "json") (#set! injection.include-unnamed-children))"#,
            ),
        );
        let mut langs = HashMap::new();
        langs.insert("json".to_owned(), Arc::clone(&json));

        let source = "[1]\n";
        let (mut parser, tree) = parse(&json, source);
        let rope = ropey::Rope::from_str(source);
        let cancel = AtomicBool::new(false);

        let out =
            resolve_and_parse_injections(&mut parser, &tree, &json, &rope, &langs, &cancel, 1);
        assert_eq!(
            out.len(),
            3,
            "expected one layer per depth (1, 2, 3), got: {}",
            out.len()
        );
        let max_depth = out.iter().map(|i| i.depth).max().unwrap();
        assert_eq!(
            max_depth, MAX_INJECTION_DEPTH,
            "recursion must stop exactly at the cap"
        );
    }

    #[test]
    fn dynamic_language_capture_reads_fenced_code_info_string() {
        if skip_if_missing("markdown") || skip_if_missing("rust") {
            return;
        }
        let Some(inj_path) = grammar_injections_path("markdown") else {
            eprintln!("skipping: markdown fixture has no injections.scm");
            return;
        };
        let inj_src = std::fs::read_to_string(inj_path).unwrap();
        let markdown = make_bundle("markdown", "tree_sitter_markdown", Some(&inj_src));
        let rust = make_bundle("rust", "tree_sitter_rust", None);
        let mut langs = HashMap::new();
        langs.insert("rust".to_owned(), Arc::clone(&rust));

        let source = "```rust\nfn main() {}\n```\n";
        let (mut parser, tree) = parse(&markdown, source);
        let rope = ropey::Rope::from_str(source);
        let cancel = AtomicBool::new(false);

        let out =
            resolve_and_parse_injections(&mut parser, &tree, &markdown, &rope, &langs, &cancel, 1);
        assert!(
            out.iter().any(|i| Arc::ptr_eq(&i.bundle, &rust)),
            "expected a rust layer resolved from the fenced code block's info string, got {} layers",
            out.len()
        );
    }

    #[test]
    fn dynamic_language_capture_unknown_info_string_no_layer() {
        if skip_if_missing("markdown") {
            return;
        }
        let Some(inj_path) = grammar_injections_path("markdown") else {
            eprintln!("skipping: markdown fixture has no injections.scm");
            return;
        };
        let inj_src = std::fs::read_to_string(inj_path).unwrap();
        let markdown = make_bundle("markdown", "tree_sitter_markdown", Some(&inj_src));
        let langs: HashMap<String, Arc<GrammarBundle>> = HashMap::new();

        let source = "```no-such-lang\nwhatever\n```\n";
        let (mut parser, tree) = parse(&markdown, source);
        let rope = ropey::Rope::from_str(source);
        let cancel = AtomicBool::new(false);

        let out =
            resolve_and_parse_injections(&mut parser, &tree, &markdown, &rope, &langs, &cancel, 1);
        assert!(
            out.is_empty(),
            "unknown fenced-code language must produce no layer, got: {} layers",
            out.len()
        );
    }
}
