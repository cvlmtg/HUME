use rustc_hash::FxHashMap;
use std::sync::atomic::AtomicBool;

use super::*;
use crate::registry::{GrammarBundle, OptionalQuery};
use test_fixtures::{grammar_query_path, require_fixture_file, require_grammars};

/// Load a real grammar fixture with an optional custom injections source
/// (overriding whatever `injections.scm` the fixture ships, if any),
/// keeps each test's injection query minimal and self-contained rather
/// than depending on upstream Helix query wording.
fn make_bundle(name: &str, symbol: &str, injections_src: Option<&str>) -> Arc<GrammarBundle> {
    let highlights_src =
        std::fs::read_to_string(grammar_query_path(name)).expect("read highlights.scm");
    crate::test_support::make_bundle(
        name,
        symbol,
        &highlights_src,
        injections_src
            .map(|src| (OptionalQuery::Injections, src))
            .as_slice(),
    )
}

fn parse(bundle: &GrammarBundle, source: &str) -> (tree_sitter::Parser, tree_sitter::Tree) {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(bundle.grammar.language()).unwrap();
    let tree = parser.parse(source, None).expect("parse");
    (parser, tree)
}

/// The layers `tree`, a layer of `bundle`, injects, with their own
/// injections, as a parse installs them.
fn injected(
    parser: &mut tree_sitter::Parser,
    tree: &tree_sitter::Tree,
    bundle: &GrammarBundle,
    rope: &ropey::Rope,
    langs: &FxHashMap<String, Arc<GrammarBundle>>,
) -> Vec<ParsedInjection> {
    let cancel = AtomicBool::new(false);
    let cx = InjectionContext {
        rope,
        langs,
        prior: None,
        cancel: &cancel,
    };
    let (_, layers) = resolve_and_parse_injections(parser, &cx, tree, bundle, 1, None);
    layers
}

// ── content_ranges (pure) ─────────────────────────────────────────────────

#[test]
fn content_ranges_excludes_only_unnamed_children_by_default() {
    require_grammars(&["json"]);
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
    require_grammars(&["json"]);
    let json = make_bundle("json", "tree_sitter_json", None);
    let (_parser, tree) = parse(&json, "[1,2]\n");
    let array = tree.root_node().named_child(0).expect("array node");

    let included = content_ranges(array, true);
    assert_eq!(included, vec![array.range()]);
}

// ── normalize_ranges (pure) ───────────────────────────────────────────────

use crate::test_support::range;

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
    require_grammars(&["json", "rust"]);
    let json = make_bundle(
        "json",
        "tree_sitter_json",
        Some(
            r#"((string_content) @injection.content (#set! injection.language "rust") (#set! injection.include-unnamed-children))"#,
        ),
    );
    let rust = make_bundle("rust", "tree_sitter_rust", None);
    let mut langs = FxHashMap::default();
    langs.insert("rust".to_owned(), Arc::clone(&rust));

    let source = "[\"hello\"]\n";
    let (mut parser, tree) = parse(&json, source);
    let rope = ropey::Rope::from_str(source);

    let out = injected(&mut parser, &tree, &json, &rope, &langs);
    assert_eq!(out.len(), 1, "expected exactly one injected layer");
    assert!(
        Arc::ptr_eq(&out[0].bundle, &rust),
        "expected the rust bundle to be resolved"
    );
    assert_eq!(out[0].depth, 1);
}

#[test]
fn unknown_injection_language_is_skipped_silently() {
    require_grammars(&["json", "rust"]);
    let json = make_bundle(
        "json",
        "tree_sitter_json",
        Some(
            r#"((string_content) @injection.content (#set! injection.language "no-such-language") (#set! injection.include-unnamed-children))"#,
        ),
    );
    // Non-empty but irrelevant: proves the lookup is by key,
    // not just "map happens to be empty".
    let mut langs = FxHashMap::default();
    langs.insert(
        "rust".to_owned(),
        make_bundle("rust", "tree_sitter_rust", None),
    );

    let source = "[\"hello\"]\n";
    let (mut parser, tree) = parse(&json, source);
    let rope = ropey::Rope::from_str(source);

    let out = injected(&mut parser, &tree, &json, &rope, &langs);
    assert!(
        out.is_empty(),
        "unresolvable injection language must produce no layer, got: {} layers",
        out.len()
    );
}

#[test]
fn combined_merges_multiple_matches_into_one_layer() {
    require_grammars(&["json", "rust"]);
    let json = make_bundle(
        "json",
        "tree_sitter_json",
        Some(
            r#"((string_content) @injection.content (#set! injection.language "rust") (#set! injection.combined) (#set! injection.include-unnamed-children))"#,
        ),
    );
    let rust = make_bundle("rust", "tree_sitter_rust", None);
    let mut langs = FxHashMap::default();
    langs.insert("rust".to_owned(), rust);

    // Three string literals. Without `injection.combined` these would be
    // three separate layers; with it, exactly one layer with 3 ranges.
    let source = "[\"a\", \"b\", \"c\"]\n";
    let (mut parser, tree) = parse(&json, source);
    let rope = ropey::Rope::from_str(source);

    let out = injected(&mut parser, &tree, &json, &rope, &langs);
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
    require_grammars(&["json"]);
    // Self-injecting: every `array` node re-parses its own (identical)
    // text as json again, matching `array` once more in the fresh tree,
    // unbounded without a depth cap, since the content never changes.
    let json = make_bundle(
        "json",
        "tree_sitter_json",
        Some(
            r#"((array) @injection.content (#set! injection.language "json") (#set! injection.include-unnamed-children))"#,
        ),
    );
    let mut langs = FxHashMap::default();
    langs.insert("json".to_owned(), Arc::clone(&json));

    let source = "[1]\n";
    let (mut parser, tree) = parse(&json, source);
    let rope = ropey::Rope::from_str(source);

    let out = injected(&mut parser, &tree, &json, &rope, &langs);
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
    // Below the cap each layer's tree holds one array; at the cap a layer
    // has no sites.
    let sites: Vec<(u8, usize)> = out
        .iter()
        .map(|layer| (layer.depth, layer.sites.0.len()))
        .collect();
    assert_eq!(sites, vec![(3, 0), (2, 1), (1, 1)]);
}

#[test]
fn dynamic_language_capture_reads_fenced_code_info_string() {
    require_grammars(&["markdown", "rust"]);
    let inj_path = grammar_query_path("markdown").with_file_name("injections.scm");
    require_fixture_file(&inj_path, "markdown injections.scm");
    let inj_src = std::fs::read_to_string(inj_path).unwrap();
    let markdown = make_bundle("markdown", "tree_sitter_markdown", Some(&inj_src));
    let rust = make_bundle("rust", "tree_sitter_rust", None);
    let mut langs = FxHashMap::default();
    langs.insert("rust".to_owned(), Arc::clone(&rust));

    let source = "```rust\nfn main() {}\n```\n";
    let (mut parser, tree) = parse(&markdown, source);
    let rope = ropey::Rope::from_str(source);

    let out = injected(&mut parser, &tree, &markdown, &rope, &langs);
    assert!(
        out.iter().any(|i| Arc::ptr_eq(&i.bundle, &rust)),
        "expected a rust layer resolved from the fenced code block's info string, got {} layers",
        out.len()
    );
}

#[test]
fn dynamic_language_capture_unknown_info_string_no_layer() {
    require_grammars(&["markdown"]);
    let inj_path = grammar_query_path("markdown").with_file_name("injections.scm");
    require_fixture_file(&inj_path, "markdown injections.scm");
    let inj_src = std::fs::read_to_string(inj_path).unwrap();
    let markdown = make_bundle("markdown", "tree_sitter_markdown", Some(&inj_src));
    let langs: FxHashMap<String, Arc<GrammarBundle>> = FxHashMap::default();

    let source = "```no-such-lang\nwhatever\n```\n";
    let (mut parser, tree) = parse(&markdown, source);
    let rope = ropey::Rope::from_str(source);

    let out = injected(&mut parser, &tree, &markdown, &rope, &langs);
    assert!(
        out.is_empty(),
        "unknown fenced-code language must produce no layer, got: {} layers",
        out.len()
    );
}

#[test]
fn injections_at_one_node_order_by_pattern_and_disjoint_ones_by_position() {
    require_grammars(&["rust", "json"]);
    let injections =
        test_fixtures::require_helix_query("rust", OptionalQuery::Injections.file_name());
    let injections = std::fs::read_to_string(injections).expect("read the helix injections query");
    let root = make_bundle("rust", "tree_sitter_rust", Some(&injections));
    let rust = make_bundle("rust", "tree_sitter_rust", None);
    // No html grammar is fetched; json stands in for it under that name.
    let html = make_bundle("json", "tree_sitter_json", None);
    let langs = Arc::new(FxHashMap::from_iter([
        ("rust".to_owned(), Arc::clone(&rust)),
        ("html".to_owned(), Arc::clone(&html)),
    ]));

    // Every macro body matches the pattern injecting rust; `html!`'s also
    // matches the later pattern injecting html.
    let source = "fn m() {\n    html! { x };\n    vec![1];\n    html! { y };\n}\n";
    let text = hume_editing::text::BufferText::from(source);
    let syntax = crate::syntax::Syntax::attach_sync(root, &text, &langs);
    let layers: Vec<(&str, Vec<&str>)> = syntax
        .layers()
        .expect("installed")
        .layers
        .iter()
        .skip(1)
        .map(|layer| {
            let name = if Arc::ptr_eq(&layer.bundle, &rust) {
                "rust"
            } else if Arc::ptr_eq(&layer.bundle, &html) {
                "html"
            } else {
                "other"
            };
            let texts = layer
                .ranges
                .iter()
                .map(|range| &source[range.start_byte..range.end_byte])
                .collect();
            (name, texts)
        })
        .collect();
    assert_eq!(
        layers,
        vec![
            ("rust", vec![" x "]),
            ("html", vec![" x "]),
            ("rust", vec!["1"]),
            ("rust", vec![" y "]),
            ("html", vec![" y "]),
        ]
    );
}

#[test]
fn an_edit_before_a_fence_moves_the_sites_with_the_text() {
    require_grammars(&["markdown", "rust"]);
    let markdown = fixture_markdown();
    let langs = Arc::new(FxHashMap::from_iter([(
        "rust".to_owned(),
        make_bundle("rust", "tree_sitter_rust", None),
    )]));

    let before = hume_editing::text::BufferText::from("Text.\n\n```rust\nfn a() {}\n```\n");
    let mut syntax = crate::syntax::Syntax::attach_sync(Arc::clone(&markdown), &before, &langs);
    // Inside the paragraph, so it gains a row and moves the fence down
    // without any site starting or ending at the edit.
    let mut builder = hume_editing::changeset::ChangeSetBuilder::new(before.end());
    builder.retain_to(hume_rope::offset::CharOffset::new("Te".len()));
    builder.insert("x\ny");
    let change = builder.finish();
    let after = change.apply(&before).expect("the insertion applies");
    syntax.record_edit(&hume_editing::edit::TextChange::new(
        &before, &after, &change,
    ));
    let baked = syntax.frame_tick(hume_engine::pipeline::BufferId::default(), &after, &langs);
    assert!(baked.chain_break.is_none());

    let source = after.to_string();
    let (_parser, fresh) = parse(&markdown, &source);
    let inj = markdown.injections.as_ref().expect("an injections query");
    let expected = resolve_sites(inj, &fresh, after.rope(), None);
    let fence = expected
        .0
        .iter()
        .find(|site| site.language == "rust")
        .expect("the fence's site");
    // The fence's body follows `Tex`, `yxt.`, a blank line and its opener.
    assert_eq!(fence.ranges[0].start_point.row, 4);
    let layers = syntax.layers().expect("installed");
    assert_eq!(layers.layers[0].sites, expected);
}

// ── PriorInjections::reuse ────────────────────────────────────────────────

/// `ab\n  fn a() {}\n`: the rust layer covers `fn a() {}`, bytes 5..14,
/// from column 2 of row 1, whose start is byte 3.
const REUSE_SOURCE: &str = "ab\n  fn a() {}\n";

fn reuse_range() -> tree_sitter::Range {
    tree_sitter::Range {
        start_byte: 5,
        end_byte: 14,
        start_point: tree_sitter::Point { row: 1, column: 2 },
        end_point: tree_sitter::Point { row: 1, column: 11 },
    }
}

/// The committed rust layer of [`REUSE_SOURCE`] at depth 1.
fn reuse_prior(rust: &Arc<GrammarBundle>) -> PriorInjections {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(rust.grammar.language()).unwrap();
    parser.set_included_ranges(&[reuse_range()]).unwrap();
    let tree = parser.parse(REUSE_SOURCE, None).expect("parse");
    PriorInjections::from_iter([PriorInjection {
        bundle: Arc::clone(rust),
        depth: 1,
        tree,
        ranges: vec![reuse_range()],
        sites: InjectionSites::default(),
    }])
}

/// The bytes an insertion of `len` bytes at `at` edits.
fn edited_by_insertion(at: usize, len: usize) -> crate::edited_bytes::EditedBytes {
    let point = tree_sitter::Point { row: 0, column: 0 };
    let mut edited = crate::edited_bytes::EditedBytes::default();
    edited.record(&tree_sitter::InputEdit {
        start_byte: at,
        old_end_byte: at,
        new_end_byte: at + len,
        start_position: point,
        old_end_position: point,
        new_end_position: point,
    });
    edited
}

#[test]
fn an_exact_untouched_prior_is_reused_unchanged() {
    require_grammars(&["rust"]);
    let rust = make_bundle("rust", "tree_sitter_rust", None);
    let prior = reuse_prior(&rust);
    // Row 0 only: bytes 0..=1.
    let edited = edited_by_insertion(0, 1);
    assert!(matches!(
        prior.reuse(&rust, 1, &[reuse_range()], &edited),
        Reuse::Unchanged(site) if std::ptr::eq(site.old_tree, &prior.layers[0].tree)
    ));
}

#[test]
fn an_exact_prior_edited_inside_its_range_is_reparsed_against() {
    require_grammars(&["rust"]);
    let rust = make_bundle("rust", "tree_sitter_rust", None);
    let prior = reuse_prior(&rust);
    let edited = edited_by_insertion(8, 1);
    assert!(matches!(
        prior.reuse(&rust, 1, &[reuse_range()], &edited),
        Reuse::Incremental(site) if std::ptr::eq(site.old_tree, &prior.layers[0].tree)
    ));
}

#[test]
fn an_edit_on_the_row_before_the_range_is_not_unchanged() {
    require_grammars(&["rust"]);
    let rust = make_bundle("rust", "tree_sitter_rust", None);
    let prior = reuse_prior(&rust);
    // Byte 3 starts row 1, two columns before the range.
    let edited = edited_by_insertion(3, 1);
    assert!(matches!(
        prior.reuse(&rust, 1, &[reuse_range()], &edited),
        Reuse::Incremental(_)
    ));
}

#[test]
fn other_ranges_or_depth_parse_fresh() {
    require_grammars(&["rust"]);
    let rust = make_bundle("rust", "tree_sitter_rust", None);
    let prior = reuse_prior(&rust);
    let edited = crate::edited_bytes::EditedBytes::default();
    let longer = tree_sitter::Range {
        end_byte: 15,
        end_point: tree_sitter::Point { row: 1, column: 12 },
        ..reuse_range()
    };
    assert!(matches!(
        prior.reuse(&rust, 1, &[longer], &edited),
        Reuse::Fresh
    ));
    assert!(matches!(
        prior.reuse(&rust, 2, &[reuse_range()], &edited),
        Reuse::Fresh
    ));
}

#[test]
fn another_bundle_of_the_same_grammar_parses_fresh() {
    require_grammars(&["rust"]);
    let rust = make_bundle("rust", "tree_sitter_rust", None);
    let prior = reuse_prior(&rust);
    let rebound = make_bundle("rust", "tree_sitter_rust", None);
    let edited = crate::edited_bytes::EditedBytes::default();
    assert!(matches!(
        prior.reuse(&rebound, 1, &[reuse_range()], &edited),
        Reuse::Fresh
    ));
}

// ── resolve_sites ─────────────────────────────────────────────────────────

/// A text before and after one replacement, with what a parse of the new
/// text against the old one has: the old tree and its sites edited to the
/// new text, the bytes the edit changed, and the new tree.
struct Reparsed {
    old: tree_sitter::Tree,
    old_sites: InjectionSites,
    edited: crate::edited_bytes::EditedBytes,
    rope: ropey::Rope,
    new: tree_sitter::Tree,
}

/// `before` with `deleted` bytes at byte `at` replaced by `inserted`,
/// reparsed with `bundle`. ASCII only, so bytes are chars.
fn reparse(
    bundle: &GrammarBundle,
    before: &str,
    at: usize,
    deleted: usize,
    inserted: &str,
) -> Reparsed {
    let inj = bundle.injections.as_ref().expect("an injections query");
    let text = hume_editing::text::BufferText::from(before);
    let (mut parser, mut old) = parse(bundle, before);
    let mut old_sites = resolve_sites(inj, &old, text.rope(), None);
    let mut builder = hume_editing::changeset::ChangeSetBuilder::new(text.end());
    builder.retain_to(hume_rope::offset::CharOffset::new(at));
    builder.delete_to(hume_rope::offset::CharOffset::new(at + deleted));
    builder.insert(inserted);
    let change = builder.finish();
    let mut edited = crate::edited_bytes::EditedBytes::default();
    for edit in crate::edits::input_edits_from_changeset(&change, text.rope()) {
        old.edit(&edit);
        old_sites.edit(&edit);
        edited.record(&edit);
    }
    let after = change.apply(&text).expect("the replacement applies");
    let rope = after.rope().clone();
    let new = parser.parse(after.to_string(), Some(&old)).expect("parse");
    Reparsed {
        old,
        old_sites,
        edited,
        rope,
        new,
    }
}

impl Reparsed {
    /// The sites of the new tree, resolved from `prior_sites` as the old
    /// tree's.
    fn resolve(&self, inj: &InjectionsQuery, prior_sites: &InjectionSites) -> InjectionSites {
        resolve_sites(
            inj,
            &self.new,
            &self.rope,
            Some(SitePrior {
                old_tree: &self.old,
                sites: prior_sites,
                hull: 0..=self.rope.len_bytes(),
                reparsed: true,
                edited: &self.edited,
            }),
        )
    }

    /// The new tree's sites by a full query.
    fn fresh(&self, inj: &InjectionsQuery) -> InjectionSites {
        resolve_sites(inj, &self.new, &self.rope, None)
    }
}

fn fixture_markdown() -> Arc<GrammarBundle> {
    let inj_path = grammar_query_path("markdown").with_file_name("injections.scm");
    require_fixture_file(&inj_path, "markdown injections.scm");
    let inj_src = std::fs::read_to_string(inj_path).unwrap();
    make_bundle("markdown", "tree_sitter_markdown", Some(&inj_src))
}

#[test]
fn without_dirty_bytes_the_prior_sites_are_kept_as_given() {
    require_grammars(&["markdown"]);
    let markdown = fixture_markdown();
    let inj = markdown.injections.as_ref().expect("an injections query");
    let source = "```rust\nfn a() {}\n```\n";
    let (_parser, tree) = parse(&markdown, source);
    let rope = ropey::Rope::from_str(source);
    let mut given = resolve_sites(inj, &tree, &rope, None);
    given.0[0].language = "elsewhere".to_owned();
    let edited = crate::edited_bytes::EditedBytes::default();
    let resolved = resolve_sites(
        inj,
        &tree,
        &rope,
        Some(SitePrior {
            old_tree: &tree,
            sites: &given,
            hull: 0..=rope.len_bytes(),
            reparsed: false,
            edited: &edited,
        }),
    );
    assert_eq!(resolved, given);
}

#[test]
fn an_edit_in_one_fence_requeries_that_fence_only() {
    require_grammars(&["markdown"]);
    let markdown = fixture_markdown();
    let inj = markdown.injections.as_ref().expect("an injections query");
    let before = "```rust\nfn a() {}\n```\n\nText.\n\n```rust\nfn b() {}\n```\n";
    let at = before.find("a()").expect("fn a");
    let reparsed = reparse(&markdown, before, at, 0, "x");
    // The second fence's site, outside the edit, is carried as given.
    let relabel = |sites: &mut InjectionSites| {
        let last = sites.0.last_mut().expect("sites");
        assert_eq!(last.language, "rust");
        last.language = "elsewhere".to_owned();
    };
    let mut given = reparsed.old_sites.clone();
    relabel(&mut given);
    let mut expected = reparsed.fresh(inj);
    relabel(&mut expected);
    assert_eq!(reparsed.resolve(inj, &given), expected);
}

/// A fence whose two functions are each rewritten to the same byte: the
/// two dirty regions lie apart, and the run over each finds the fence's
/// match, so it is resolved once.
#[test]
fn a_match_two_dirty_regions_meet_is_resolved_once() {
    require_grammars(&["markdown"]);
    let markdown = fixture_markdown();
    let inj = markdown.injections.as_ref().expect("an injections query");
    let source = "```rust\nfn a() {}\nfn b() {}\n```\n";
    let (_parser, tree) = parse(&markdown, source);
    let rope = ropey::Rope::from_str(source);
    let fresh = resolve_sites(inj, &tree, &rope, None);
    assert_eq!(fresh.0.len(), 1, "one fence");
    let point = |byte: usize| {
        let row_start = source[..byte].rfind('\n').map_or(0, |newline| newline + 1);
        tree_sitter::Point {
            row: source[..byte].matches('\n').count(),
            column: byte - row_start,
        }
    };
    let mut edited = crate::edited_bytes::EditedBytes::default();
    for name in ["b()", "a()"] {
        let at = source.find(name).expect("the function");
        edited.record(&tree_sitter::InputEdit {
            start_byte: at,
            old_end_byte: at + 1,
            new_end_byte: at + 1,
            start_position: point(at),
            old_end_position: point(at + 1),
            new_end_position: point(at + 1),
        });
    }
    let dirty: Vec<_> = edited.within(0..=rope.len_bytes()).collect();
    assert_eq!(dirty.len(), 2, "the regions lie apart: {dirty:?}");
    let regions: Vec<Range<usize>> = dirty
        .iter()
        .map(|region| region.start() - 1..region.end() + 1)
        .collect();
    assert_eq!(
        query_sites(inj, &tree, &rope, Some(&regions)).len(),
        2,
        "each region finds the fence's match"
    );
    for reparsed in [false, true] {
        let resolved = resolve_sites(
            inj,
            &tree,
            &rope,
            Some(SitePrior {
                old_tree: &tree,
                sites: &fresh,
                hull: 0..=rope.len_bytes(),
                reparsed,
                edited: &edited,
            }),
        );
        assert_eq!(resolved, fresh, "reparsed: {reparsed}");
    }
}

#[test]
fn a_new_html_block_joins_the_carried_ones_in_one_layer() {
    require_grammars(&["markdown", "json"]);
    let injections =
        test_fixtures::require_helix_query("markdown", OptionalQuery::Injections.file_name());
    let injections = std::fs::read_to_string(injections).expect("read the helix injections query");
    let markdown = make_bundle("markdown", "tree_sitter_markdown", Some(&injections));
    let inj = markdown.injections.as_ref().expect("an injections query");
    // No html grammar is fetched; json stands in for it under that name.
    let langs = FxHashMap::from_iter([(
        "html".to_owned(),
        make_bundle("json", "tree_sitter_json", None),
    )]);
    let before = "<div>one</div>\n\nText.\n\n<div>two</div>\n";
    let at = before.find("Text").expect("the paragraph");
    let reparsed = reparse(&markdown, before, at, 0, "<p>new</p>\n\n");
    let resolved = reparsed.resolve(inj, &reparsed.old_sites);
    let fresh = reparsed.fresh(inj);
    assert_eq!(resolved, fresh);
    let html = resolved
        .0
        .iter()
        .filter(|site| site.language == "html")
        .count();
    assert_eq!(html, 3, "all three blocks");
    let layers = |sites: &InjectionSites| -> Vec<Vec<tree_sitter::Range>> {
        group_sites(inj, sites, &langs)
            .into_iter()
            .map(|group| group.ranges)
            .collect()
    };
    assert_eq!(layers(&resolved).len(), 1, "one combined html layer");
    assert_eq!(layers(&resolved), layers(&fresh));
}

#[test]
fn renaming_a_macro_requeries_the_patterns_rooted_at_it() {
    require_grammars(&["rust"]);
    let injections =
        test_fixtures::require_helix_query("rust", OptionalQuery::Injections.file_name());
    let injections = std::fs::read_to_string(injections).expect("read the helix injections query");
    let rust = make_bundle("rust", "tree_sitter_rust", Some(&injections));
    let inj = rust.injections.as_ref().expect("an injections query");
    let before = "fn m() {\n    vec![1];\n}\n";
    let at = before.find("vec").expect("the macro");
    // The rust pattern's captures miss the macro name, the html one's
    // hold it.
    let reparsed = reparse(&rust, before, at, "vec".len(), "html");
    let resolved = reparsed.resolve(inj, &reparsed.old_sites);
    let languages: Vec<&str> = resolved
        .0
        .iter()
        .map(|site| site.language.as_str())
        .collect();
    assert_eq!(languages, vec!["rust", "html"]);
    assert_eq!(resolved, reparsed.fresh(inj));
}

#[test]
fn a_match_lost_outside_its_captures_is_dropped() {
    require_grammars(&["markdown"]);
    let markdown = make_bundle(
        "markdown",
        "tree_sitter_markdown",
        Some(
            r#"(fenced_code_block (info_string) (code_fence_content) @injection.content (#set! injection.language "rust"))"#,
        ),
    );
    let inj = markdown.injections.as_ref().expect("an injections query");
    let before = "```rust\nfn a() {}\n```\n";
    // The info string, which the pattern needs but does not capture.
    let reparsed = reparse(&markdown, before, "```".len(), "rust".len(), "");
    assert_eq!(reparsed.old_sites.0.len(), 1);
    assert_eq!(
        reparsed.resolve(inj, &reparsed.old_sites),
        InjectionSites::default()
    );
}
