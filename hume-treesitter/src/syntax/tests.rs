use rustc_hash::FxHashMap;
use std::sync::Arc;

use hume_editing::changeset::ChangeSetBuilder;
use hume_editing::text::Text;
use hume_engine::pipeline::BufferId;
use hume_engine::providers::SyntaxSpans;

use super::Syntax;
use crate::parse_worker::{ParseDone, ParseOutcome, ParsedLayers};
use crate::registry::GrammarBundle;
use crate::test_support::{empty_langs, fresh_bid};
use hume_test_fixtures::{grammar_query_path, skip_unless_grammars};

fn make_bundle(name: &str, symbol: &str) -> Arc<GrammarBundle> {
    crate::test_support::make_bundle(name, symbol, "", None)
}

/// Like `make_bundle`, but with a compiled injections query attached —
/// needed to exercise the injected-layer (`depth > 0`) path in `bake`.
fn make_bundle_with_injections(
    name: &str,
    symbol: &str,
    injections_src: &str,
) -> Arc<GrammarBundle> {
    crate::test_support::make_bundle(name, symbol, "", Some(injections_src))
}

/// Like `make_bundle`, but compiles the grammar's *real* `highlights.scm`
/// instead of an empty query — needed to assert `spans_for_line` actually
/// produces scopes, not just that a tree exists.
fn make_bundle_with_real_highlights(name: &str, symbol: &str) -> Arc<GrammarBundle> {
    let highlights_src =
        std::fs::read_to_string(grammar_query_path(name)).expect("highlights.scm should exist");
    crate::test_support::make_bundle(name, symbol, &highlights_src, None)
}

/// Real end-to-end parse via `do_parse`-equivalent: build a `ParseDone`
/// by parsing `text` directly with a fresh `tree_sitter::Parser`, so
/// tests exercise `Syntax::install` against a genuine tree rather than a
/// hand-rolled stand-in (independent oracle, not circular).
fn parse_done_for(
    bundle: &Arc<GrammarBundle>,
    bid: BufferId,
    text_gen: u64,
    text: &str,
) -> ParseDone {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(bundle.grammar.language())
        .expect("set language");
    let tree = parser.parse(text, None).expect("parse must succeed");
    ParseDone {
        bid,
        text_gen,
        bundle: Arc::clone(bundle),
        outcome: ParseOutcome::Ok(ParsedLayers {
            root: tree,
            injected: Vec::new(),
        }),
    }
}

// ── attach ────────────────────────────────────────────────────────────────

// No test exercises the `text.len_bytes() == 0` short-circuit branch in
// `attach`: `Text`'s public constructors always enforce the trailing-`\n`
// buffer invariant (see hume-editing/src/text.rs), so `len_bytes()` is
// never 0 for any `Text` reachable from a real `Buffer`. The branch is
// preserved verbatim from the pre-consolidation code (parse.rs) as
// defense-in-depth; it is not exercisable through the public `Text` API.

#[test]
fn attach_nonempty_text_returns_request_and_sets_in_flight() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let (syn, req) = Syntax::attach(bundle, bid, 0, &Text::from("{}\n"), &empty_langs());
    assert!(
        req.is_some(),
        "non-empty text must produce a full-parse request"
    );
    assert!(
        req.unwrap().old_tree.is_none(),
        "initial attach must request a full parse"
    );
    assert!(
        syn.is_in_flight(),
        "attach must record the request as in-flight"
    );
    assert_eq!(
        syn.parsed_gen(),
        0,
        "parsed_gen must not advance before install"
    );
}

// ── attach_sync ───────────────────────────────────────────────────────────

#[test]
fn attach_sync_parses_immediately_and_produces_real_highlight_spans() {
    if skip_unless_grammars(&["markdown"]) {
        return;
    }
    let bundle = make_bundle_with_real_highlights("markdown", "tree_sitter_markdown");
    let text = Text::from("# heading\n");
    let syn = Syntax::attach_sync(Arc::clone(&bundle), &text, &empty_langs());

    assert!(
        !syn.is_in_flight(),
        "attach_sync must return a fully-installed attachment — no async request left pending"
    );

    let mut spans = Vec::new();
    syn.spans_for_line(0, text.rope(), &mut spans);
    assert!(
        !spans.is_empty(),
        "a real markdown grammar must highlight a heading line immediately, not leave it plain"
    );
}

// ── frame_tick ────────────────────────────────────────────────────────────

#[test]
fn frame_tick_up_to_date_returns_no_request() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let (mut syn, _req) =
        Syntax::attach(Arc::clone(&bundle), bid, 0, &Text::from(""), &empty_langs());
    // parsed_gen == text_gen (0) already — up to date.
    let outcome = syn.frame_tick(bid, 0, &Text::from(""), &empty_langs());
    assert!(
        outcome.request.is_none(),
        "up-to-date buffer must not re-request"
    );
}

#[test]
fn frame_tick_dedups_while_in_flight_at_same_gen() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let (mut syn, req) = Syntax::attach(
        Arc::clone(&bundle),
        bid,
        1,
        &Text::from("{}\n"),
        &empty_langs(),
    );
    assert!(req.is_some());
    // parsed_gen is still 0 (attach doesn't install), text_gen is 1 —
    // frame_tick must see the existing in-flight request and dedup.
    let outcome = syn.frame_tick(bid, 1, &Text::from("{}\n"), &empty_langs());
    assert!(
        outcome.request.is_none(),
        "a request already in flight for this text_gen must not be re-posted"
    );
}

#[test]
fn frame_tick_reposts_after_further_edit() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let (mut syn, _req) = Syntax::attach(
        Arc::clone(&bundle),
        bid,
        1,
        &Text::from("{}\n"),
        &empty_langs(),
    );
    // Text advances to gen 2 before the gen-1 result arrives.
    let outcome = syn.frame_tick(bid, 2, &Text::from("{\"a\":1}\n"), &empty_langs());
    assert!(
        outcome.request.is_some(),
        "a newer text_gen than the in-flight one must trigger a fresh request"
    );
    assert_eq!(outcome.request.unwrap().text_gen, 2);
}

#[test]
fn frame_tick_old_tree_present_iff_chain_baked() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let (mut syn, _req) = Syntax::attach(
        Arc::clone(&bundle),
        bid,
        0,
        &Text::from("{}\n"),
        &empty_langs(),
    );
    let done = parse_done_for(&bundle, bid, 0, "{}\n");
    syn.install(done, 0);
    assert!(syn.layers().is_some(), "install must populate layers");

    // Record a contiguous edit and tick — chain bakes, tree_gen catches
    // up to text_gen, so old_tree must be Some.
    let rope_pre = ropey::Rope::from_str("{}\n");
    let mut b = ChangeSetBuilder::new(rope_pre.len_chars());
    b.retain(1);
    b.insert("\"a\":1");
    b.retain_rest();
    let cs = b.finish();
    syn.record_edit(1, &cs, &rope_pre);
    let outcome = syn.frame_tick(bid, 1, &Text::from("{\"a\":1}\n"), &empty_langs());
    assert!(
        outcome.request.unwrap().old_tree.is_some(),
        "a baked contiguous chain must produce an old_tree for incremental parse"
    );

    // clear_layers drops the committed tree — next tick must full-reparse.
    syn.clear_layers();
    syn.install(parse_done_for(&bundle, bid, 1, "{\"a\":1}\n"), 1);
    assert!(syn.layers().is_some());
    // Force a chain break: pending edit gen does not follow tree_gen+1.
    let rope2 = ropey::Rope::from_str("{\"a\":1}\n");
    let mut b2 = ChangeSetBuilder::new(rope2.len_chars());
    b2.retain(1);
    b2.insert("x");
    b2.retain_rest();
    let cs2 = b2.finish();
    // Skip a generation to break the chain (record at gen 3, not 2).
    syn.record_edit(3, &cs2, &rope2);
    let outcome2 = syn.frame_tick(bid, 3, &Text::from("{x\"a\":1}\n"), &empty_langs());
    assert!(
        outcome2.chain_break.is_some(),
        "a gapped chain must be reported as a break"
    );
    assert!(
        outcome2.request.unwrap().old_tree.is_none(),
        "a broken chain must fall back to a full reparse (no old_tree)"
    );
}

// ── bake (via record_edit + frame_tick) ──────────────────────────────────

#[test]
fn bake_contiguous_chain_advances_tree_gen_and_clears_pending() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let (mut syn, _req) = Syntax::attach(
        Arc::clone(&bundle),
        bid,
        0,
        &Text::from("{}\n"),
        &empty_langs(),
    );
    syn.install(parse_done_for(&bundle, bid, 0, "{}\n"), 0);
    assert_eq!(syn.tree_gen(), 0);

    let rope_pre = ropey::Rope::from_str("{}\n");
    let mut b = ChangeSetBuilder::new(rope_pre.len_chars());
    b.retain(1);
    b.insert("\"a\":1");
    b.retain_rest();
    let cs = b.finish();
    syn.record_edit(1, &cs, &rope_pre);
    assert_eq!(syn.pending_edits().len(), 1);

    let outcome = syn.frame_tick(bid, 1, &Text::from("{\"a\":1}\n"), &empty_langs());
    assert!(
        outcome.chain_break.is_none(),
        "contiguous chain must not report a break"
    );
    assert_eq!(
        syn.tree_gen(),
        1,
        "tree_gen must advance to the baked generation"
    );
    assert!(
        syn.pending_edits().is_empty(),
        "pending edits must be cleared after a successful bake"
    );

    // Independent oracle: the baked root tree's end_byte must equal the
    // new text's byte length — computed from the string, not the tree.
    let expected_end_byte = "{\"a\":1}\n".len();
    let root = syn.layers().unwrap().root_tree().unwrap();
    assert_eq!(root.root_node().end_byte(), expected_end_byte);
}

#[test]
fn bake_mid_chain_gap_rejected() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let (mut syn, _req) = Syntax::attach(
        Arc::clone(&bundle),
        bid,
        0,
        &Text::from("{}\n"),
        &empty_langs(),
    );
    syn.install(parse_done_for(&bundle, bid, 0, "{}\n"), 0);

    // Fabricate a gapped chain directly: recorded gens 1 and 3 (a gap at
    // 2), matching endpoints against tree_gen(=0)+1 ..= text_gen(=3).
    let rope_pre = ropey::Rope::from_str("{}\n");
    let mut b = ChangeSetBuilder::new(rope_pre.len_chars());
    b.retain(1);
    b.insert("x");
    b.retain_rest();
    let cs = b.finish();
    syn.record_edit(1, &cs, &rope_pre);
    syn.record_edit(3, &cs, &rope_pre);

    let outcome = syn.frame_tick(bid, 3, &Text::from("{x}\n"), &empty_langs());
    assert!(
        outcome.chain_break.is_some(),
        "gapped chain must be rejected"
    );
    assert_eq!(syn.tree_gen(), 0, "gapped chain must NOT advance tree_gen");
    assert!(
        syn.pending_edits().is_empty(),
        "broken chain must still clear pending_edits so the caller falls back to a full reparse"
    );
    assert!(
        outcome.request.unwrap().old_tree.is_none(),
        "broken chain must request a full reparse"
    );
}

/// All prior `bake` coverage used JSON (no injections), so the
/// `layer.depth > 0` ranges-refresh branch never ran. This installs a
/// real markdown root + rust fenced-code injection layer, edits text
/// *before* the fenced block (shifting the injection forward), bakes,
/// and checks the injected layer's cached `ranges` — the copy
/// `layer_covers_line` consults — actually moved with it instead of
/// staying pinned at the pre-edit byte offset.
#[test]
fn bake_refreshes_injected_layer_ranges_after_an_edit_shifts_them() {
    if skip_unless_grammars(&["markdown", "rust"]) {
        return;
    }
    let inj_path =
        hume_test_fixtures::grammar_query_path("markdown").with_file_name("injections.scm");
    if hume_test_fixtures::skip_unless_file(&inj_path, "markdown injections.scm") {
        return;
    }
    let inj_src = std::fs::read_to_string(&inj_path).expect("read injections.scm");
    let markdown = make_bundle_with_injections("markdown", "tree_sitter_markdown", &inj_src);
    let rust = make_bundle("rust", "tree_sitter_rust");
    let mut langs_map = FxHashMap::default();
    langs_map.insert("rust".to_owned(), Arc::clone(&rust));
    let langs = Arc::new(langs_map);

    let bid = fresh_bid();
    let source = "```rust\nfn main() {}\n```\n";
    let (mut syn, _req) =
        Syntax::attach(Arc::clone(&markdown), bid, 0, &Text::from(source), &langs);

    // Parse root + resolve injections directly (mirrors `do_parse` in
    // `parse_worker.rs`, inlined so the test controls the exact result).
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(markdown.grammar.language())
        .expect("set language");
    let root = parser.parse(source, None).expect("parse root");
    let rope = ropey::Rope::from_str(source);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let injected = crate::injections::resolve_and_parse_injections(
        &mut parser,
        &root,
        &markdown,
        &rope,
        &langs,
        &cancel,
        1,
    );
    assert_eq!(injected.len(), 1, "expected one rust injection layer");
    let original_start = injected[0].ranges[0].start_byte;

    syn.install(
        ParseDone {
            bid,
            text_gen: 0,
            bundle: Arc::clone(&markdown),
            outcome: ParseOutcome::Ok(ParsedLayers { root, injected }),
        },
        0,
    );

    // Insert text before the fenced code block — the rust layer's byte
    // range must shift forward by the inserted length once baked.
    let prefix = "more text\n";
    let rope_pre = rope;
    let mut b = ChangeSetBuilder::new(rope_pre.len_chars());
    b.insert(prefix);
    b.retain_rest();
    let cs = b.finish();
    syn.record_edit(1, &cs, &rope_pre);

    let new_source = format!("{prefix}{source}");
    let outcome = syn.frame_tick(bid, 1, &Text::from(new_source.as_str()), &langs);
    assert!(
        outcome.chain_break.is_none(),
        "contiguous chain must not report a break"
    );

    let rust_layer = syn
        .layers()
        .unwrap()
        .layers
        .iter()
        .find(|l| l.depth > 0)
        .expect("rust injected layer must survive the bake");
    // Independent oracle: the shift is exactly `prefix.len()` bytes,
    // computed from the inserted string — not re-derived from the tree.
    assert_eq!(
        rust_layer.ranges[0].start_byte,
        original_start + prefix.len(),
        "ranges must be refreshed to the tree's post-edit included_ranges, not left stale"
    );
}

// ── install ───────────────────────────────────────────────────────────────

#[test]
fn install_stale_text_gen_discarded() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let (mut syn, _req) = Syntax::attach(
        Arc::clone(&bundle),
        bid,
        0,
        &Text::from("{}\n"),
        &empty_langs(),
    );
    // current_text_gen has moved to 5; a done for gen 0 must be discarded.
    syn.install(parse_done_for(&bundle, bid, 0, "{}\n"), 5);
    assert_eq!(
        syn.parsed_gen(),
        0,
        "parsed_gen must NOT advance on a discarded stale result"
    );
    assert!(
        syn.layers().is_none(),
        "layers must not be installed from a stale result"
    );
}

#[test]
fn install_config_gen_mismatch_discarded_without_clearing_newer_in_flight() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let old_bundle = make_bundle("json", "tree_sitter_json");
    let new_bundle = make_bundle("json", "tree_sitter_json"); // distinct config_gen
    let bid = fresh_bid();
    // Attach with the NEW bundle (simulating a grammar swap already applied).
    let (mut syn, _req) = Syntax::attach(
        Arc::clone(&new_bundle),
        bid,
        1,
        &Text::from("{}\n"),
        &empty_langs(),
    );
    assert!(
        syn.is_in_flight(),
        "new attachment must have its own in-flight request"
    );

    // A done from the OLD bundle arrives late, same text_gen.
    let stale_done = parse_done_for(&old_bundle, bid, 1, "{}\n");
    syn.install(stale_done, 1);

    assert!(
        syn.is_in_flight(),
        "a done from a superseded attachment must not clear the new attachment's in_flight"
    );
    assert_eq!(
        syn.parsed_gen(),
        0,
        "stale-config result must not advance parsed_gen"
    );
    assert!(syn.layers().is_none());
}

/// Edits recorded while the very first parse is still in flight can't be
/// baked (`bake`'s early-out never clears pending when `layers` is still
/// `None`), so they survive until the first successful install — which
/// must drain them. A done can only install when `done.text_gen` equals
/// the *current* text_gen, and pending-edit gens are always ≤ the current
/// text_gen at record time — so `retain(g > text_gen)` always empties the
/// list on a real successful install; there is no reachable case where it
/// retains an entry.
#[test]
fn install_matching_done_clears_in_flight_and_drains_pending() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let (mut syn, _req0) = Syntax::attach(
        Arc::clone(&bundle),
        bid,
        0,
        &Text::from("{}\n"),
        &empty_langs(),
    );
    assert!(
        syn.is_in_flight(),
        "attach must post the initial full-parse request"
    );

    // An edit lands while the initial parse (gen 0) is still in flight.
    let rope_pre = ropey::Rope::from_str("{}\n");
    let mut b = ChangeSetBuilder::new(rope_pre.len_chars());
    b.retain(1);
    b.insert("x");
    b.retain_rest();
    let cs = b.finish();
    syn.record_edit(1, &cs, &rope_pre);

    // frame_tick at the new gen: bake early-outs (layers still None from
    // the in-flight initial parse), so pending_edits survives; a fresh
    // request for gen 1 is posted and recorded as in-flight.
    let outcome = syn.frame_tick(bid, 1, &Text::from("{x}\n"), &empty_langs());
    assert!(
        outcome.request.is_some(),
        "gen-1 edit must trigger a fresh request"
    );
    assert_eq!(
        syn.pending_edits().len(),
        1,
        "bake must not clear pending while layers is None"
    );

    // The gen-1 request's done arrives and matches the current gen.
    syn.install(parse_done_for(&bundle, bid, 1, "{x}\n"), 1);

    assert!(!syn.is_in_flight(), "a matching done must clear in_flight");
    assert_eq!(syn.parsed_gen(), 1);
    assert_eq!(syn.tree_gen(), 1);
    assert!(
        syn.pending_edits().is_empty(),
        "a successful install must drain pending edits at or below the installed gen"
    );
    assert!(syn.layers().is_some());
}

#[test]
fn install_parse_failed_advances_parsed_gen_only() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let (mut syn, _req) = Syntax::attach(
        Arc::clone(&bundle),
        bid,
        0,
        &Text::from("{}\n"),
        &empty_langs(),
    );
    let done = ParseDone {
        bid,
        text_gen: 0,
        bundle: Arc::clone(&bundle),
        outcome: ParseOutcome::ParseFailed,
    };
    syn.install(done, 0);
    assert_eq!(
        syn.parsed_gen(),
        0,
        "parsed_gen must advance even on ParseFailed"
    );
    assert_eq!(
        syn.tree_gen(),
        0,
        "tree_gen must NOT advance on ParseFailed"
    );
    assert!(
        syn.layers().is_none(),
        "layers must stay unset on ParseFailed"
    );
}

// ── clear_layers ──────────────────────────────────────────────────────────

#[test]
fn clear_layers_keeps_attachment_next_tick_full_reparses() {
    if skip_unless_grammars(&["json"]) {
        return;
    }
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let (mut syn, _req) = Syntax::attach(
        Arc::clone(&bundle),
        bid,
        0,
        &Text::from("{}\n"),
        &empty_langs(),
    );
    syn.install(parse_done_for(&bundle, bid, 0, "{}\n"), 0);
    assert!(syn.layers().is_some());

    syn.clear_layers();
    assert!(
        syn.layers().is_none(),
        "clear_layers must drop committed layers"
    );
    assert_eq!(
        syn.bundle().config_gen,
        bundle.config_gen,
        "attachment must survive clear_layers"
    );

    let outcome = syn.frame_tick(bid, 1, &Text::from("{\"a\":1}\n"), &empty_langs());
    assert!(
        outcome.request.unwrap().old_tree.is_none(),
        "with layers cleared, the next tick must request a full reparse"
    );
}
