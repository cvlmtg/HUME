use rustc_hash::FxHashMap;
use std::sync::Arc;

use hume_editing::changeset::{ChangeSet, ChangeSetBuilder};
use hume_editing::edit::TextChange;
use hume_editing::text::BufferText;
use hume_engine::pipeline::BufferId;
use hume_engine::providers::SyntaxSpans;
use hume_rope::offset::CharOffset;

use super::Syntax;
use crate::parse_worker::{ParseDone, ParseOutcome, ParsedLayers};
use crate::registry::GrammarBundle;
use crate::test_support::{empty_langs, fresh_bid};
use test_fixtures::{grammar_query_path, require_fixture_file, require_grammars};

fn make_bundle(name: &str, symbol: &str) -> Arc<GrammarBundle> {
    crate::test_support::make_bundle(name, symbol, "", None, None)
}

/// Like `make_bundle`, but with a compiled injections query attached,
/// needed to exercise the injected-layer (`depth > 0`) path in `bake`.
fn make_bundle_with_injections(
    name: &str,
    symbol: &str,
    injections_src: &str,
) -> Arc<GrammarBundle> {
    crate::test_support::make_bundle(name, symbol, "", Some(injections_src), None)
}

/// Like `make_bundle`, but compiles the grammar's *real* `highlights.scm`
/// instead of an empty query, needed to assert `spans_for_line` actually
/// produces scopes, not just that a tree exists.
fn make_bundle_with_real_highlights(name: &str, symbol: &str) -> Arc<GrammarBundle> {
    let highlights_src =
        std::fs::read_to_string(grammar_query_path(name)).expect("highlights.scm should exist");
    crate::test_support::make_bundle(name, symbol, &highlights_src, None, None)
}

/// Real end-to-end parse via `do_parse`-equivalent: build a `ParseDone`
/// by parsing `text` directly with a fresh `tree_sitter::Parser`, so
/// tests exercise `Syntax::install` against a genuine tree rather than a
/// hand-rolled stand-in.
fn parse_done_for(
    bundle: &Arc<GrammarBundle>,
    bid: BufferId,
    generation: u64,
    text: &str,
) -> ParseDone {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(bundle.grammar.language())
        .expect("set language");
    let tree = parser.parse(text, None).expect("parse must succeed");
    ParseDone {
        bid,
        generation,
        bundle: Arc::clone(bundle),
        outcome: ParseOutcome::Ok(ParsedLayers {
            root: tree,
            injected: Vec::new(),
        }),
    }
}

/// `before` with `inserted` added at char `at`, as the next text of its
/// lineage, and the change that makes it.
fn insert_into(before: &BufferText, at: usize, inserted: &str) -> (BufferText, ChangeSet) {
    let mut b = ChangeSetBuilder::new(before.end());
    b.retain_to(CharOffset::new(at));
    b.insert(inserted);
    let cs = b.finish();
    let after = cs
        .apply(before)
        .expect("an insertion applies to its own text");
    (after, cs)
}

/// Record the edit from `before` to `after` on `syn`.
fn record(syn: &mut Syntax, before: &BufferText, after: &BufferText, cs: &ChangeSet) {
    syn.record_edit(&TextChange::new(before, after, cs));
}

fn generation(text: &BufferText) -> u64 {
    text.generation()
}

// ── attach ────────────────────────────────────────────────────────────────

// No test exercises the `text.len_bytes() == 0` short-circuit branch in
// `attach`: `BufferText`'s public constructors always enforce the trailing-`\n`
// buffer invariant (see hume-editing/src/text.rs), so `len_bytes()` is
// never 0 for any `BufferText` reachable from a real `Buffer`. The branch is
// preserved verbatim from the pre-consolidation code (parse.rs) as
// defense-in-depth; it is not exercisable through the public `BufferText` API.

#[test]
fn attach_nonempty_text_returns_request_and_sets_in_flight() {
    require_grammars(&["json"]);
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let (syn, req) = Syntax::attach(bundle, bid, &BufferText::from("{}\n"), &empty_langs());
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
        None,
        "parsed_gen must not advance before install"
    );
}

// ── attach_sync ───────────────────────────────────────────────────────────

#[test]
fn attach_sync_parses_immediately_and_produces_real_highlight_spans() {
    require_grammars(&["markdown"]);
    let bundle = make_bundle_with_real_highlights("markdown", "tree_sitter_markdown");
    let text = BufferText::from("# heading\n");
    let syn = Syntax::attach_sync(Arc::clone(&bundle), &text, &empty_langs());

    assert!(
        !syn.is_in_flight(),
        "attach_sync must return a fully-installed attachment, no async request left pending"
    );

    let mut spans = Vec::new();
    syn.spans_for_line(
        hume_rope::line::ContentLine::new(0),
        text.rope(),
        &mut spans,
    );
    assert!(
        !spans.is_empty(),
        "a real markdown grammar must highlight a heading line immediately, not leave it plain"
    );
}

// ── frame_tick ────────────────────────────────────────────────────────────

#[test]
fn frame_tick_up_to_date_returns_no_request() {
    require_grammars(&["json"]);
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let text = BufferText::from("");
    let (mut syn, _req) = Syntax::attach(Arc::clone(&bundle), bid, &text, &empty_langs());
    // parsed_gen == the text's generation already: up to date.
    let outcome = syn.frame_tick(bid, &text, &empty_langs());
    assert!(
        outcome.request.is_none(),
        "up-to-date buffer must not re-request"
    );
}

#[test]
fn frame_tick_dedups_while_in_flight_at_same_gen() {
    require_grammars(&["json"]);
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let text = BufferText::from("{}\n");
    let (mut syn, req) = Syntax::attach(Arc::clone(&bundle), bid, &text, &empty_langs());
    assert!(req.is_some());
    // Nothing is parsed yet (attach doesn't install), so frame_tick must see
    // the existing in-flight request for this text and dedup.
    let outcome = syn.frame_tick(bid, &text, &empty_langs());
    assert!(
        outcome.request.is_none(),
        "a request already in flight for this generation must not be re-posted"
    );
}

#[test]
fn frame_tick_reposts_after_further_edit() {
    require_grammars(&["json"]);
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let text = BufferText::from("{}\n");
    let (mut syn, _req) = Syntax::attach(Arc::clone(&bundle), bid, &text, &empty_langs());
    // The text moves on before the first result arrives.
    let (next, _) = insert_into(&text, 1, "\"a\":1");
    let outcome = syn.frame_tick(bid, &next, &empty_langs());
    assert!(
        outcome.request.is_some(),
        "a newer generation than the in-flight one must trigger a fresh request"
    );
    assert_eq!(outcome.request.unwrap().generation, generation(&next));
}

#[test]
fn frame_tick_old_tree_present_iff_chain_baked() {
    require_grammars(&["json"]);
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let t0 = BufferText::from("{}\n");
    let (mut syn, _req) = Syntax::attach(Arc::clone(&bundle), bid, &t0, &empty_langs());
    syn.install(
        parse_done_for(&bundle, bid, generation(&t0), "{}\n"),
        generation(&t0),
    );
    assert!(syn.layers().is_some(), "install must populate layers");

    // Record a contiguous edit and tick: chain bakes, tree_gen catches
    // up to generation, so old_tree must be Some.
    let (t1, cs) = insert_into(&t0, 1, "\"a\":1");
    record(&mut syn, &t0, &t1, &cs);
    let outcome = syn.frame_tick(bid, &t1, &empty_langs());
    assert!(
        outcome.request.unwrap().old_tree.is_some(),
        "a baked contiguous chain must produce an old_tree for incremental parse"
    );

    syn.install(
        parse_done_for(&bundle, bid, generation(&t1), "{\"a\":1}\n"),
        generation(&t1),
    );
    assert!(syn.layers().is_some());
    // Force a chain break: the recorded edit starts from a text the tree
    // never saw.
    let (unseen, _) = insert_into(&t1, 1, "y");
    let (t3, cs3) = insert_into(&unseen, 1, "x");
    record(&mut syn, &unseen, &t3, &cs3);
    let outcome2 = syn.frame_tick(bid, &t3, &empty_langs());
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
    require_grammars(&["json"]);
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let t0 = BufferText::from("{}\n");
    let (mut syn, _req) = Syntax::attach(Arc::clone(&bundle), bid, &t0, &empty_langs());
    syn.install(
        parse_done_for(&bundle, bid, generation(&t0), "{}\n"),
        generation(&t0),
    );
    assert_eq!(syn.tree_gen(), generation(&t0));

    let (t1, cs) = insert_into(&t0, 1, "\"a\":1");
    record(&mut syn, &t0, &t1, &cs);
    assert_eq!(syn.pending_edits().len(), 1);

    let outcome = syn.frame_tick(bid, &t1, &empty_langs());
    assert!(
        outcome.chain_break.is_none(),
        "contiguous chain must not report a break"
    );
    assert_eq!(
        syn.tree_gen(),
        generation(&t1),
        "tree_gen must advance to the baked generation"
    );
    assert!(
        syn.pending_edits().is_empty(),
        "pending edits must be cleared after a successful bake"
    );

    // The baked root tree's end_byte must equal the
    // new text's byte length, computed from the string, not the tree.
    let expected_end_byte = "{\"a\":1}\n".len();
    let root = syn.layers().unwrap().root_tree().unwrap();
    assert_eq!(root.root_node().end_byte(), expected_end_byte);
}

#[test]
fn bake_accepts_a_chain_whose_generations_skip_numbers() {
    require_grammars(&["json"]);
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let t0 = BufferText::from("{}\n");
    let (mut syn, _req) = Syntax::attach(Arc::clone(&bundle), bid, &t0, &empty_langs());
    syn.install(
        parse_done_for(&bundle, bid, generation(&t0), "{}\n"),
        generation(&t0),
    );

    // A text computed and dropped uses up a generation between each pair.
    let _dropped = insert_into(&t0, 1, "z");
    let (t1, first) = insert_into(&t0, 1, "\"a\":1");
    let _dropped = insert_into(&t1, 1, "z");
    let (t2, second) = insert_into(&t1, 1, "b");
    assert!(generation(&t1) > generation(&t0) + 1 && generation(&t2) > generation(&t1) + 1);
    record(&mut syn, &t0, &t1, &first);
    record(&mut syn, &t1, &t2, &second);

    let outcome = syn.frame_tick(bid, &t2, &empty_langs());
    assert!(
        outcome.chain_break.is_none(),
        "edits that each start where the previous one ended form a chain"
    );
    assert_eq!(syn.tree_gen(), generation(&t2));
    assert!(outcome.request.unwrap().old_tree.is_some());
}

#[test]
fn bake_mid_chain_gap_rejected() {
    require_grammars(&["json"]);
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let t0 = BufferText::from("{}\n");
    let (mut syn, _req) = Syntax::attach(Arc::clone(&bundle), bid, &t0, &empty_langs());
    syn.install(
        parse_done_for(&bundle, bid, generation(&t0), "{}\n"),
        generation(&t0),
    );

    // A broken link: the first edit ends at `t1` and the second starts at a
    // text never recorded, though the chain's two ends match the tree's
    // generation and the current one.
    let (t1, first) = insert_into(&t0, 1, "x");
    let (unseen, _) = insert_into(&t1, 1, "y");
    let (t3, second) = insert_into(&unseen, 1, "z");
    record(&mut syn, &t0, &t1, &first);
    record(&mut syn, &unseen, &t3, &second);

    let outcome = syn.frame_tick(bid, &t3, &empty_langs());
    assert!(
        outcome.chain_break.is_some(),
        "gapped chain must be rejected"
    );
    assert_eq!(
        syn.tree_gen(),
        generation(&t0),
        "gapped chain must NOT advance tree_gen"
    );
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
/// and checks the injected layer's cached `ranges` (the copy
/// `layer_covers_line` consults) actually moved with it instead of
/// staying pinned at the pre-edit byte offset.
#[test]
fn bake_refreshes_injected_layer_ranges_after_an_edit_shifts_them() {
    require_grammars(&["markdown", "rust"]);
    let inj_path = test_fixtures::grammar_query_path("markdown").with_file_name("injections.scm");
    require_fixture_file(&inj_path, "markdown injections.scm");
    let inj_src = std::fs::read_to_string(&inj_path).expect("read injections.scm");
    let markdown = make_bundle_with_injections("markdown", "tree_sitter_markdown", &inj_src);
    let rust = make_bundle("rust", "tree_sitter_rust");
    let mut langs_map = FxHashMap::default();
    langs_map.insert("rust".to_owned(), Arc::clone(&rust));
    let langs = Arc::new(langs_map);

    let bid = fresh_bid();
    let source = "```rust\nfn main() {}\n```\n";
    let t0 = BufferText::from(source);
    let (mut syn, _req) = Syntax::attach(Arc::clone(&markdown), bid, &t0, &langs);

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
            generation: generation(&t0),
            bundle: Arc::clone(&markdown),
            outcome: ParseOutcome::Ok(ParsedLayers { root, injected }),
        },
        generation(&t0),
    );

    // Insert text before the fenced code block. The rust layer's byte
    // range must shift forward by the inserted length once baked.
    let prefix = "more text\n";
    let (t1, cs) = insert_into(&t0, 0, prefix);
    record(&mut syn, &t0, &t1, &cs);

    let outcome = syn.frame_tick(bid, &t1, &langs);
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
    // The shift is exactly `prefix.len()` bytes,
    // computed from the inserted string, not re-derived from the tree.
    assert_eq!(
        rust_layer.ranges[0].start_byte,
        original_start + prefix.len(),
        "ranges must be refreshed to the tree's post-edit included_ranges, not left stale"
    );
}

// ── install ───────────────────────────────────────────────────────────────

#[test]
fn install_stale_generation_discarded() {
    require_grammars(&["json"]);
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let (mut syn, _req) = Syntax::attach(
        Arc::clone(&bundle),
        bid,
        &BufferText::from("{}\n"),
        &empty_langs(),
    );
    // current_generation has moved to 5; a done for gen 0 must be discarded.
    syn.install(parse_done_for(&bundle, bid, 0, "{}\n"), 5);
    assert_eq!(
        syn.parsed_gen(),
        None,
        "parsed_gen must NOT advance on a discarded stale result"
    );
    assert!(
        syn.layers().is_none(),
        "layers must not be installed from a stale result"
    );
}

#[test]
fn install_config_gen_mismatch_discarded_without_clearing_newer_in_flight() {
    require_grammars(&["json"]);
    let old_bundle = make_bundle("json", "tree_sitter_json");
    let new_bundle = make_bundle("json", "tree_sitter_json"); // distinct config_gen
    let bid = fresh_bid();
    // Attach with the NEW bundle (simulating a grammar swap already applied).
    let (mut syn, _req) = Syntax::attach(
        Arc::clone(&new_bundle),
        bid,
        &BufferText::from("{}\n"),
        &empty_langs(),
    );
    assert!(
        syn.is_in_flight(),
        "new attachment must have its own in-flight request"
    );

    // A done from the OLD bundle arrives late, same generation.
    let stale_done = parse_done_for(&old_bundle, bid, 1, "{}\n");
    syn.install(stale_done, 1);

    assert!(
        syn.is_in_flight(),
        "a done from a superseded attachment must not clear the new attachment's in_flight"
    );
    assert_eq!(
        syn.parsed_gen(),
        None,
        "stale-config result must not advance parsed_gen"
    );
    assert!(syn.layers().is_none());
}

/// Edits recorded while the very first parse is still in flight can't be
/// baked (`bake`'s early-out never clears pending when `layers` is still
/// `None`), so they survive until the first successful install, which
/// must drain them.
#[test]
fn install_matching_done_clears_in_flight_and_drains_pending() {
    require_grammars(&["json"]);
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let t0 = BufferText::from("{}\n");
    let (mut syn, _req0) = Syntax::attach(Arc::clone(&bundle), bid, &t0, &empty_langs());
    assert!(
        syn.is_in_flight(),
        "attach must post the initial full-parse request"
    );

    // An edit lands while the initial parse is still in flight.
    let (t1, cs) = insert_into(&t0, 1, "x");
    record(&mut syn, &t0, &t1, &cs);

    // frame_tick at the new gen: bake early-outs (layers still None from
    // the in-flight initial parse), so pending_edits survives; a fresh
    // request for the new gen is posted and recorded as in-flight.
    let outcome = syn.frame_tick(bid, &t1, &empty_langs());
    assert!(
        outcome.request.is_some(),
        "the edit must trigger a fresh request"
    );
    assert_eq!(
        syn.pending_edits().len(),
        1,
        "bake must not clear pending while layers is None"
    );

    // The new request's done arrives and matches the current gen.
    syn.install(
        parse_done_for(&bundle, bid, generation(&t1), "{x}\n"),
        generation(&t1),
    );

    assert!(!syn.is_in_flight(), "a matching done must clear in_flight");
    assert_eq!(syn.parsed_gen(), Some(generation(&t1)));
    assert_eq!(syn.tree_gen(), generation(&t1));
    assert!(
        syn.pending_edits().is_empty(),
        "a successful install must drain pending edits at or below the installed gen"
    );
    assert!(syn.layers().is_some());
}

#[test]
fn install_parse_failed_advances_parsed_gen_only() {
    require_grammars(&["json"]);
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let (mut syn, _req) = Syntax::attach(
        Arc::clone(&bundle),
        bid,
        &BufferText::from("{}\n"),
        &empty_langs(),
    );
    let done = ParseDone {
        bid,
        generation: 0,
        bundle: Arc::clone(&bundle),
        outcome: ParseOutcome::ParseFailed,
    };
    syn.install(done, 0);
    assert_eq!(
        syn.parsed_gen(),
        Some(0),
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

/// A generation that failed to parse must still be installable if a later
/// result for that *same* generation succeeds, whether a retried
/// `ensure_current` call or a slow async result that finally lands. Keying
/// the "already installed" guard on `parsed_gen` (which `ParseFailed` also
/// advances) would discard this success permanently, leaving `layers` stuck
/// on whatever they were before the failure until an unrelated edit bumps
/// `generation` past this generation entirely.
#[test]
fn install_recovers_from_a_parse_failed_for_the_same_generation() {
    require_grammars(&["json"]);
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let (mut syn, _req) = Syntax::attach(
        Arc::clone(&bundle),
        bid,
        &BufferText::from("{}\n"),
        &empty_langs(),
    );
    // Gen 0's own first parse attempt fails (e.g. cancelled mid-flight).
    syn.install(
        ParseDone {
            bid,
            generation: 0,
            bundle: Arc::clone(&bundle),
            outcome: ParseOutcome::ParseFailed,
        },
        0,
    );
    assert_eq!(syn.parsed_gen(), Some(0));
    assert!(syn.layers().is_none());

    // A later result for the SAME generation succeeds and must actually land.
    syn.install(parse_done_for(&bundle, bid, 0, "{}\n"), 0);

    assert_eq!(
        syn.tree_gen(),
        0,
        "the successful retry must advance tree_gen"
    );
    assert!(
        syn.layers().is_some(),
        "the successful retry must install layers, not be discarded as \
         'already installed' because gen 0 was already attempted"
    );
}

// ── is_current ────────────────────────────────────────────────────────────

/// The freshness predicate every caller gates on. After a `ParseFailed` for
/// a generation, `parsed_gen` has advanced to it but `layers`/`tree_gen`
/// still describe the *previous* generation, so `parsed_gen` alone reports
/// "current" over a tree that predates the edit, which is exactly the state
/// that lets a structural query hand `byte_to_char` an offset past the
/// buffer's end.
#[test]
fn is_current_is_false_when_a_failed_parse_advanced_parsed_gen_over_older_layers() {
    require_grammars(&["json"]);
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let t0 = BufferText::from("{}\n");
    let (mut syn, _req) = Syntax::attach(Arc::clone(&bundle), bid, &t0, &empty_langs());
    // The first text lands: layers installed, tree_gen == parsed_gen.
    syn.install(
        parse_done_for(&bundle, bid, generation(&t0), "{}\n"),
        generation(&t0),
    );
    assert!(syn.is_current(generation(&t0)), "the first text is current");

    // An edit moves the text on, then the new text's parse fails.
    let (t1, cs) = insert_into(&t0, 1, "\"a\":1");
    record(&mut syn, &t0, &t1, &cs);
    syn.install(
        ParseDone {
            bid,
            generation: generation(&t1),
            bundle: Arc::clone(&bundle),
            outcome: ParseOutcome::ParseFailed,
        },
        generation(&t1),
    );

    // `parsed_gen` alone would now claim the new text is current...
    assert_eq!(syn.parsed_gen(), Some(generation(&t1)));
    // ...but the committed layers still describe the first.
    assert_eq!(syn.tree_gen(), generation(&t0));
    assert!(
        !syn.is_current(generation(&t1)),
        "layers predate the new text: is_current must not report it as current"
    );
}

// ── ensure_current ────────────────────────────────────────────────────────

#[test]
fn ensure_current_parses_a_never_parsed_attachment() {
    require_grammars(&["json"]);
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let text = BufferText::from("{}\n");
    let (mut syn, _req) = Syntax::attach(Arc::clone(&bundle), bid, &text, &empty_langs());

    let outcome = syn.ensure_current(bid, &text, &empty_langs());
    assert!(
        outcome.is_none(),
        "the first-ever parse is not a chain break"
    );
    assert!(
        syn.layers().is_some(),
        "ensure_current must install a tree even though nothing was ever \
         installed before: parsed_gen and generation are both 0 here, which \
         must not be mistaken for 'already up to date'"
    );
    assert_eq!(
        syn.layers()
            .unwrap()
            .root_tree()
            .unwrap()
            .root_node()
            .end_byte(),
        "{}\n".len(),
        "the installed root must span the source string"
    );
    assert_eq!(syn.parsed_gen(), Some(0));
}

#[test]
fn ensure_current_reparses_a_stale_tree_after_a_recorded_edit() {
    require_grammars(&["json"]);
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let t0 = BufferText::from("{}\n");
    let (mut syn, _req) = Syntax::attach(Arc::clone(&bundle), bid, &t0, &empty_langs());
    syn.install(
        parse_done_for(&bundle, bid, generation(&t0), "{}\n"),
        generation(&t0),
    );

    // Record an edit but skip frame_tick: ensure_current must bake and
    // reparse on its own, with no async round trip.
    let (new_text, cs) = insert_into(&t0, 1, "\"a\":1");
    record(&mut syn, &t0, &new_text, &cs);

    let outcome = syn.ensure_current(bid, &new_text, &empty_langs());
    assert!(outcome.is_none(), "a contiguous chain is not a break");

    // Computed from the new string, not the tree.
    let expected_end_byte = "{\"a\":1}\n".len();
    assert_eq!(
        syn.layers()
            .unwrap()
            .root_tree()
            .unwrap()
            .root_node()
            .end_byte(),
        expected_end_byte
    );
    assert_eq!(syn.parsed_gen(), Some(generation(&new_text)));
    assert_eq!(syn.tree_gen(), generation(&new_text));
    assert!(syn.pending_edits().is_empty());
}

#[test]
fn ensure_current_up_to_date_does_not_reparse() {
    require_grammars(&["json"]);
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let t0 = BufferText::from("{}\n");
    let (mut syn, _req) = Syntax::attach(Arc::clone(&bundle), bid, &t0, &empty_langs());
    syn.install(
        parse_done_for(&bundle, bid, generation(&t0), "{}\n"),
        generation(&t0),
    );

    // A text of the same generation that would parse to a different tree if
    // ensure_current actually reparsed: the gen-gate short-circuits first.
    let other = BufferText::from("{\"a\":1}\n");
    assert_eq!(generation(&other), generation(&t0));
    let outcome = syn.ensure_current(bid, &other, &empty_langs());
    assert!(outcome.is_none());
    assert_eq!(
        syn.tree_gen(),
        generation(&t0),
        "up-to-date attachment must not reparse"
    );
    assert_eq!(
        syn.layers()
            .unwrap()
            .root_tree()
            .unwrap()
            .root_node()
            .end_byte(),
        "{}\n".len(),
        "the committed tree must still be the original, unreparsed one"
    );
}

#[test]
fn ensure_current_reports_a_broken_chain_and_full_reparses() {
    require_grammars(&["json"]);
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let t0 = BufferText::from("{}\n");
    let (mut syn, _req) = Syntax::attach(Arc::clone(&bundle), bid, &t0, &empty_langs());
    syn.install(
        parse_done_for(&bundle, bid, generation(&t0), "{}\n"),
        generation(&t0),
    );

    // Record an edit that starts at a text the tree never saw.
    let (unseen, _) = insert_into(&t0, 1, "y");
    let (new_text, cs) = insert_into(&unseen, 1, "x");
    record(&mut syn, &unseen, &new_text, &cs);

    let outcome = syn.ensure_current(bid, &new_text, &empty_langs());
    let brk = outcome.expect("a gapped chain must be reported");
    assert_eq!(brk.tree_gen, generation(&t0));
    assert_eq!(brk.generation, generation(&new_text));
    assert_eq!(
        syn.layers()
            .unwrap()
            .root_tree()
            .unwrap()
            .root_node()
            .end_byte(),
        "{xy}\n".len(),
        "a broken chain must still land on a full reparse of the new text"
    );
    assert_eq!(syn.parsed_gen(), Some(generation(&new_text)));
}

#[test]
fn install_discards_a_result_for_an_already_installed_generation() {
    require_grammars(&["json"]);
    let bundle = make_bundle("json", "tree_sitter_json");
    let bid = fresh_bid();
    let t0 = BufferText::from("{}\n");
    let (mut syn, _req) = Syntax::attach(Arc::clone(&bundle), bid, &t0, &empty_langs());
    // Reach the next text synchronously, as a structural command would.
    syn.install(
        parse_done_for(&bundle, bid, generation(&t0), "{}\n"),
        generation(&t0),
    );
    let (t1, cs) = insert_into(&t0, 1, "\"a\":1");
    record(&mut syn, &t0, &t1, &cs);
    syn.ensure_current(bid, &t1, &empty_langs());
    assert_eq!(syn.parsed_gen(), Some(generation(&t1)));

    // A late asynchronous result for the SAME generation, built from
    // different (shorter) text, must not overwrite what ensure_current
    // already installed.
    let late_done = parse_done_for(&bundle, bid, generation(&t1), "{}\n");
    syn.install(late_done, generation(&t1));

    assert_eq!(
        syn.layers()
            .unwrap()
            .root_tree()
            .unwrap()
            .root_node()
            .end_byte(),
        "{\"a\":1}\n".len(),
        "a redundant same-generation result must not replace the already-installed tree"
    );
    assert!(
        !syn.is_in_flight(),
        "the redundant result must still be treated as answering any in-flight request"
    );
}
