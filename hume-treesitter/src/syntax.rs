use std::collections::HashMap;
use std::sync::Arc;

use hume_editing::changeset::ChangeSet;
use hume_editing::text::Text;
use hume_engine::pipeline::BufferId;
use hume_engine::syntax_layers::{SyntaxLayer, SyntaxLayers};

use crate::edits::input_edits_from_changeset;
use crate::parse_worker::{ParseDone, ParseOutcome, ParseRequest};
use crate::registry::GrammarBundle;

/// Diagnostic info for a broken pending-edit chain: a text mutation bumped
/// `text_gen` without recording an `InputEdit` between two recorded edits.
/// The editor logs this at `Severity::Trace`; the state machine itself has
/// no message-log access.
#[derive(Debug)]
pub struct ChainBreak {
    pub tree_gen: u64,
    pub text_gen: u64,
    pub first: Option<u64>,
    pub last: Option<u64>,
}

/// Result of one `Syntax::frame_tick` call.
pub struct FrameTickOutcome {
    /// The caller MUST post this request to the parse backend when `Some`.
    pub request: Option<ParseRequest>,
    /// Set when this tick found a broken pending-edit chain.
    pub chain_break: Option<ChainBreak>,
}

/// All per-buffer tree-sitter state: the attached grammar, committed parse
/// layers, generation bookkeeping, pending `InputEdit`s awaiting a bake, and
/// the in-flight request generation.
///
/// Replaces three previously hand-synced homes (editor-side generations,
/// engine-side trees, backend-side in-flight map) with one type — desync is
/// unrepresentable because there is only one place to look.
pub struct Syntax {
    /// The attached root grammar bundle. Immutable for this attachment's
    /// lifetime — a grammar swap replaces the whole `Syntax` via a fresh
    /// `attach` call, it never mutates this field in place.
    bundle: Arc<GrammarBundle>,
    /// Committed parse layers. `None` until the first `ParseDone` installs.
    layers: Option<SyntaxLayers>,
    /// `text_gen` of the most recently installed (or failed) parse result.
    /// Equal to `Buffer.text_gen` means the installed tree is up to date.
    parsed_gen: u64,
    /// Text generation whose coordinates the committed `layers` describe.
    /// Advances on every successful bake and on every precise parse install.
    /// Distinct from `parsed_gen`: edits can outpace the worker, so
    /// `tree_gen` advances every frame (via bake) while `parsed_gen` only
    /// advances when the worker delivers a result.
    tree_gen: u64,
    /// Edits recorded since the last bake or install, `(text_gen, edit)`
    /// pairs in order. A contiguous chain from `tree_gen + 1` to the current
    /// `text_gen` enables in-place baking; a gap forces a full reparse.
    pending_edits: Vec<(u64, tree_sitter::InputEdit)>,
    /// `text_gen` of the posted-but-unanswered parse request, if any. No
    /// `config_gen` slot is needed: `bundle` never changes within one
    /// attachment, so the posted config is always `bundle.config_gen`.
    in_flight: Option<u64>,
}

impl Syntax {
    /// Create a fresh attachment. Empty text short-circuits: `parsed_gen` is
    /// set to `text_gen` immediately, no request is built, `in_flight` stays
    /// `None`. Otherwise returns the initial full-parse request — the caller
    /// MUST post it to the parse backend.
    pub fn attach(
        bundle: Arc<GrammarBundle>,
        bid: BufferId,
        text_gen: u64,
        text: &Text,
        langs: &Arc<HashMap<String, Arc<GrammarBundle>>>,
    ) -> (Self, Option<ParseRequest>) {
        let mut syn = Self {
            bundle: Arc::clone(&bundle),
            layers: None,
            parsed_gen: 0,
            tree_gen: 0,
            pending_edits: Vec::new(),
            in_flight: None,
        };

        if text.len_bytes() == 0 {
            syn.parsed_gen = text_gen;
            return (syn, None);
        }

        let req = ParseRequest {
            bid,
            text_gen,
            bundle,
            text: text.clone(),
            old_tree: None,
            langs: Arc::clone(langs),
        };
        syn.in_flight = Some(text_gen);
        (syn, Some(req))
    }

    /// Record one batch of `InputEdit`s translated from a `ChangeSet` against
    /// the pre-edit rope. Called from the `doc_ops` chokepoint immediately
    /// after every text mutation.
    pub fn record_edit(&mut self, text_gen: u64, cs: &ChangeSet, rope_pre: &ropey::Rope) {
        for edit in input_edits_from_changeset(cs, rope_pre) {
            self.pending_edits.push((text_gen, edit));
        }
    }

    /// Per-frame driver. In order: gen-gate (already up to date → no
    /// request), bake pending edits into the committed layers, in-flight
    /// dedup (a request for this exact `text_gen` is already posted → no
    /// request), then build the next incremental request and record it as
    /// in-flight. The caller MUST post a returned request.
    pub fn frame_tick(
        &mut self,
        bid: BufferId,
        text_gen: u64,
        text: &Text,
        langs: &Arc<HashMap<String, Arc<GrammarBundle>>>,
    ) -> FrameTickOutcome {
        if self.parsed_gen == text_gen {
            return FrameTickOutcome {
                request: None,
                chain_break: None,
            };
        }

        let chain_break = self.bake(text_gen);

        if self.in_flight == Some(text_gen) {
            return FrameTickOutcome {
                request: None,
                chain_break,
            };
        }

        let old_tree = if self.tree_gen == text_gen {
            self.layers
                .as_ref()
                .and_then(SyntaxLayers::root_tree)
                .cloned()
        } else {
            None
        };

        let req = ParseRequest {
            bid,
            text_gen,
            bundle: Arc::clone(&self.bundle),
            text: text.clone(),
            old_tree,
            langs: Arc::clone(langs),
        };
        self.in_flight = Some(text_gen);
        FrameTickOutcome {
            request: Some(req),
            chain_break,
        }
    }

    /// Bake `pending_edits` into the committed `layers`. No-op (and no
    /// `ChainBreak`) when there is no committed tree yet or nothing pending —
    /// checked *before* the chain-contiguity test so a reloaded buffer (layers
    /// cleared, stale pending) never trace-logs or clears pending here.
    ///
    /// On a complete chain (`tree_gen + 1 ..= text_gen`, no gaps): applies
    /// every recorded `InputEdit` to every layer's tree, refreshes injected
    /// layers' cached `ranges`, advances `tree_gen`, clears `pending_edits`.
    ///
    /// On a broken chain: clears `pending_edits` (so the caller's `old_tree ==
    /// None` path posts a full reparse) and leaves `tree_gen` untouched,
    /// returning the break info for the caller to log.
    fn bake(&mut self, text_gen: u64) -> Option<ChainBreak> {
        if self.pending_edits.is_empty() || self.layers.is_none() {
            return None;
        }

        let tree_gen = self.tree_gen;
        let chain_ok = self.pending_edits[0].0 == tree_gen + 1
            && self
                .pending_edits
                .last()
                .expect("checked non-empty above")
                .0
                == text_gen
            && self.pending_edits.windows(2).all(|w| w[1].0 - w[0].0 <= 1);

        if chain_ok {
            let edits: Vec<tree_sitter::InputEdit> =
                self.pending_edits.iter().map(|(_, e)| *e).collect();
            let layers = &mut self.layers.as_mut().expect("checked above").layers;
            for layer in layers.iter_mut() {
                for edit in &edits {
                    layer.tree.edit(edit);
                }
                // `ranges` is a separate cached copy (consulted by
                // `layer_covers_line`) that must be refreshed to match the
                // tree's shifted included ranges. The root layer's ranges are
                // always empty (whole-buffer) and need no refresh.
                if layer.depth > 0 {
                    layer.ranges = layer.tree.included_ranges();
                }
            }
            self.tree_gen = text_gen;
            self.pending_edits.clear();
            None
        } else {
            let break_info = ChainBreak {
                tree_gen,
                text_gen,
                first: self.pending_edits.first().map(|(g, _)| *g),
                last: self.pending_edits.last().map(|(g, _)| *g),
            };
            self.pending_edits.clear();
            Some(break_info)
        }
    }

    /// Install a `ParseDone` result.
    ///
    /// Clears `in_flight` when `done` matches the posted request (`text_gen`
    /// equal, and `config_gen` equal — a done from a *previous* attachment
    /// fails the config match and must not clear a newer attachment's
    /// in-flight record). Discards the parse outcome itself (without
    /// touching `parsed_gen`) on a config-gen mismatch (grammar swapped
    /// in flight) or a stale `text_gen` (text moved on since submission).
    pub fn install(&mut self, done: ParseDone, current_text_gen: u64) {
        let ParseDone {
            text_gen,
            bundle,
            outcome,
            ..
        } = done;

        if self.in_flight == Some(text_gen) && bundle.config_gen == self.bundle.config_gen {
            self.in_flight = None;
        }

        if bundle.config_gen != self.bundle.config_gen {
            return;
        }
        if text_gen != current_text_gen {
            return;
        }

        match outcome {
            ParseOutcome::Ok(parsed) => {
                let root_highlighter = Arc::clone(&bundle.highlighter);
                let mut layers = Vec::with_capacity(1 + parsed.injected.len());
                layers.push(SyntaxLayer {
                    tree: parsed.root,
                    highlighter: root_highlighter,
                    ranges: Vec::new(),
                    depth: 0,
                });
                for injected in parsed.injected {
                    layers.push(SyntaxLayer {
                        tree: injected.tree,
                        highlighter: Arc::clone(&injected.bundle.highlighter),
                        ranges: injected.ranges,
                        depth: injected.depth,
                    });
                }
                self.layers = Some(SyntaxLayers { layers });
                self.pending_edits.retain(|(g, _)| *g > text_gen);
                self.tree_gen = text_gen;
            }
            ParseOutcome::ParseFailed => {
                // Advance parsed_gen so this generation is not retried every
                // frame; tree_gen/layers stay as-is (next edit bumps
                // text_gen and triggers a fresh attempt).
            }
        }

        self.parsed_gen = text_gen;
    }

    /// Committed layers for the renderer. `None` until the first install.
    pub fn layers(&self) -> Option<&SyntaxLayers> {
        self.layers.as_ref()
    }

    /// Drop the committed layers, keeping the attachment and generations
    /// (buffer reload: content replaced wholesale). The next `frame_tick`
    /// full-reparses (`tree_gen != text_gen` → `old_tree = None`).
    pub fn clear_layers(&mut self) {
        self.layers = None;
    }

    /// The attached root grammar bundle — read by `sweep_buffers_for_grammars`
    /// to check whether it has an injections query.
    pub fn bundle(&self) -> &Arc<GrammarBundle> {
        &self.bundle
    }

    pub fn parsed_gen(&self) -> u64 {
        self.parsed_gen
    }

    pub fn tree_gen(&self) -> u64 {
        self.tree_gen
    }

    pub fn pending_edits(&self) -> &[(u64, tree_sitter::InputEdit)] {
        &self.pending_edits
    }

    pub fn is_in_flight(&self) -> bool {
        self.in_flight.is_some()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use hume_editing::changeset::ChangeSetBuilder;
    use hume_editing::text::Text;
    use hume_engine::builtins::tree_sitter_hl::TreeSitterHighlighter;
    use hume_engine::grammar::LoadedGrammar;
    use hume_engine::pipeline::BufferId;
    use hume_engine::theme::ScopeRegistry;
    use slotmap::SlotMap;

    use super::Syntax;
    use crate::parse_worker::{ParseDone, ParseOutcome, ParsedLayers};
    use crate::registry::GrammarBundle;
    use crate::test_support::grammar_parser_path;

    fn next_test_config_gen() -> u32 {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT_GEN: AtomicU32 = AtomicU32::new(0);
        NEXT_GEN.fetch_add(1, Ordering::Relaxed)
    }

    fn make_bundle(name: &str, symbol: &str) -> Arc<GrammarBundle> {
        let path = grammar_parser_path(name);
        if !path.exists() {
            panic!(
                "grammar fixture missing: {}\nrun scripts/fetch-test-grammars.sh from the repo root",
                path.display()
            );
        }
        let grammar = LoadedGrammar::open(&path, symbol).expect("load grammar");
        let query = Arc::new(tree_sitter::Query::new(grammar.language(), "").expect("empty query"));
        let mut registry = ScopeRegistry::new();
        let highlighter = Arc::new(TreeSitterHighlighter::from_shared_query(
            query,
            &mut registry,
        ));
        Arc::new(GrammarBundle {
            grammar,
            highlighter,
            injections: None,
            config_gen: next_test_config_gen(),
        })
    }

    fn fresh_bid() -> BufferId {
        let mut sm: SlotMap<BufferId, ()> = SlotMap::with_key();
        sm.insert(())
    }

    fn empty_langs() -> Arc<HashMap<String, Arc<GrammarBundle>>> {
        Arc::new(HashMap::new())
    }

    fn json_fixture_available() -> bool {
        grammar_parser_path("json").exists()
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
        if !json_fixture_available() {
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

    // ── frame_tick ────────────────────────────────────────────────────────────

    #[test]
    fn frame_tick_up_to_date_returns_no_request() {
        if !json_fixture_available() {
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
        if !json_fixture_available() {
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
        if !json_fixture_available() {
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
        if !json_fixture_available() {
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
        if !json_fixture_available() {
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
        if !json_fixture_available() {
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

    // ── install ───────────────────────────────────────────────────────────────

    #[test]
    fn install_stale_text_gen_discarded() {
        if !json_fixture_available() {
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
        if !json_fixture_available() {
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
        if !json_fixture_available() {
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
        if !json_fixture_available() {
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
        if !json_fixture_available() {
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
}
