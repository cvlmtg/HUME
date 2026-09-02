use rustc_hash::FxHashMap;
use std::sync::{Arc, Mutex};

use crate::highlight::layer_highlights_for_line;
use crate::layers::{SyntaxLayer, SyntaxLayers};
use hume_editing::changeset::ChangeSet;
use hume_editing::text::BufferText;
use hume_engine::pipeline::BufferId;
use hume_engine::types::ScopeId;

use crate::edits::input_edits_from_changeset;
use crate::parse_worker::{ParseDone, ParseOutcome, ParseRequest};
use crate::registry::GrammarBundle;

/// Scratch for `layer_highlights_for_line`'s overlap flattener, reused
/// across lines and frames. Behind a `Mutex` because renders reach
/// `spans_for_line` through `&Syntax` (same uncontended single-threaded
/// pattern as `TreeSitterHighlighter::cursor`).
#[derive(Default)]
pub(crate) struct FlattenScratch {
    raw: Vec<(usize, usize, u8, ScopeId)>,
    stack: Vec<(u8, u32, ScopeId)>,
    events: Vec<(usize, bool, u32, u8, ScopeId)>,
}

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
/// One type for the attachment, generations, trees, and in-flight tracking —
/// desync is unrepresentable because there is only one place to look.
pub struct Syntax {
    /// The attached root grammar bundle. Immutable for this attachment's
    /// lifetime — a grammar swap replaces the whole `Syntax` via a fresh
    /// `attach` call, it never mutates this field in place.
    bundle: Arc<GrammarBundle>,
    /// Committed parse layers. `None` until the first `ParseDone` installs.
    layers: Option<SyntaxLayers>,
    /// `text_gen` of the most recently installed (or failed) parse result.
    /// `None` until `install` has run at least once — distinct from
    /// `Some(0)`, which is a genuine installed generation zero (a freshly
    /// opened file's `Buffer::text_gen` starts at 0 and never bumps on
    /// open). Collapsing the two into a bare `u64` would make the very
    /// first parse of every opened file indistinguishable from "already up
    /// to date", discarding it. `Some(g) == Buffer.text_gen` means the
    /// installed tree is up to date.
    parsed_gen: Option<u64>,
    /// BufferText generation whose coordinates the committed `layers` describe.
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
    /// Scratch for the overlap flattener, reused across `spans_for_line`
    /// calls. Lives here (not per-frame in the engine) because it survives
    /// `install`/`clear_layers` — `SyntaxLayers` is rebuilt wholesale on
    /// every install, `Syntax` is not.
    span_scratch: Mutex<FlattenScratch>,
}

impl Syntax {
    /// A fresh, unparsed attachment — no committed layers, no in-flight
    /// request, generations at zero. Shared by `attach` and `attach_sync`,
    /// which differ only in how (or whether) the first parse is requested.
    fn detached(bundle: Arc<GrammarBundle>) -> Self {
        Self {
            bundle,
            layers: None,
            parsed_gen: None,
            tree_gen: 0,
            pending_edits: Vec::new(),
            in_flight: None,
            span_scratch: Mutex::new(FlattenScratch::default()),
        }
    }

    /// Create a fresh attachment. Empty text short-circuits: `parsed_gen` is
    /// set to `text_gen` immediately, no request is built, `in_flight` stays
    /// `None`. Otherwise returns the initial full-parse request — the caller
    /// MUST post it to the parse backend.
    pub fn attach(
        bundle: Arc<GrammarBundle>,
        bid: BufferId,
        text_gen: u64,
        text: &BufferText,
        langs: &Arc<FxHashMap<String, Arc<GrammarBundle>>>,
    ) -> (Self, Option<ParseRequest>) {
        let mut syn = Self::detached(Arc::clone(&bundle));

        if text.len_bytes() == 0 {
            syn.parsed_gen = Some(text_gen);
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

    /// Parse `text` once, synchronously, into a ready-to-query attachment —
    /// no async worker round-trip, no `frame_tick`/`install` dance. For
    /// small, static, one-shot content that isn't a real editor buffer (a
    /// hover popup's markdown): the content never changes after this call,
    /// so there is nothing to incrementally reparse, and the popup already
    /// persists across frames — a one-frame async delay would buy nothing.
    ///
    /// `bid` in the underlying parse request is never read back (this
    /// bypasses the normal `bid`-keyed `ParseDone` routing entirely, calling
    /// `install` directly), so `BufferId::default()` is fine. `langs` still
    /// resolves any fenced-code injections the content contains.
    pub fn attach_sync(
        bundle: Arc<GrammarBundle>,
        text: &BufferText,
        langs: &Arc<FxHashMap<String, Arc<GrammarBundle>>>,
    ) -> Self {
        let mut syn = Self::detached(Arc::clone(&bundle));

        if text.len_bytes() == 0 {
            return syn;
        }

        syn.ensure_current(BufferId::default(), 1, text, langs);
        syn
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
        text: &BufferText,
        langs: &Arc<FxHashMap<String, Arc<GrammarBundle>>>,
    ) -> FrameTickOutcome {
        if self.parsed_gen == Some(text_gen) {
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

        let req = self.build_request(bid, text_gen, text, langs);
        self.in_flight = Some(text_gen);
        FrameTickOutcome {
            request: Some(req),
            chain_break,
        }
    }

    /// Build the next incremental (or, absent a baked tree at `text_gen`,
    /// full) parse request — the shared tail of `frame_tick` and
    /// `ensure_current`, which differ only in how the result reaches
    /// `install` (posted to the async worker vs. run inline).
    fn build_request(
        &self,
        bid: BufferId,
        text_gen: u64,
        text: &BufferText,
        langs: &Arc<FxHashMap<String, Arc<GrammarBundle>>>,
    ) -> ParseRequest {
        let old_tree = if self.tree_gen == text_gen {
            self.layers
                .as_ref()
                .and_then(SyntaxLayers::root_tree)
                .cloned()
        } else {
            None
        };

        ParseRequest {
            bid,
            text_gen,
            bundle: Arc::clone(&self.bundle),
            text: text.clone(),
            old_tree,
            langs: Arc::clone(langs),
        }
    }

    /// Bring the committed tree up to date with `text_gen` *synchronously*,
    /// bypassing the async worker entirely. A structural command (text
    /// object, navigation) reads the tree after `frame_tick` has already run
    /// for the frame, and a macro or dot-repeat batch replays several edits
    /// with no settle in between — either way the committed tree can be a
    /// generation behind by the time a query needs it, which would return
    /// wrong spans (or panic on a byte offset past the pre-edit tree's end).
    /// This closes that window at the query site instead of relying on the
    /// next frame's `frame_tick`.
    ///
    /// Bakes pending edits first, same as `frame_tick`; when the chain is
    /// intact this reparse is incremental and sub-frame. A full parse only
    /// happens before the worker has delivered the buffer's first tree, or
    /// after a broken edit chain — both already bounded by
    /// `syntax-highlight-max-bytes` refusing to attach syntax at all above
    /// that size. Inside a macro or dot-repeat batch, every step after the
    /// first sees an intact chain and reparses incrementally.
    ///
    /// Deliberately leaves `in_flight` untouched: an asynchronous request
    /// already posted for an earlier generation is left to arrive and be
    /// discarded by `install`'s own generation guard, rather than cancelled
    /// or raced here.
    pub fn ensure_current(
        &mut self,
        bid: BufferId,
        text_gen: u64,
        text: &BufferText,
        langs: &Arc<FxHashMap<String, Arc<GrammarBundle>>>,
    ) -> Option<ChainBreak> {
        if self.parsed_gen == Some(text_gen) {
            return None;
        }

        let chain_break = self.bake(text_gen);
        let req = self.build_request(bid, text_gen, text, langs);
        let mut parser = tree_sitter::Parser::new();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let done = crate::parse_worker::do_parse(&mut parser, req, &cancel);
        self.install(done, text_gen);
        chain_break
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
            && self.pending_edits.windows(2).all(|w| w[1].0 <= w[0].0 + 1);

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
    /// in flight), a stale `text_gen` (text moved on since submission), or a
    /// `text_gen` already installed (a synchronous `ensure_current` beat an
    /// asynchronous request to the same generation — the late arrival is
    /// redundant, not stale, so it must not re-run the `ParseOutcome::Ok`
    /// arm a second time). The already-installed check runs after the
    /// `in_flight` clear above: an async result superseded this way still
    /// answered its own posted request and must still clear it, or a later
    /// `frame_tick` would dedup against a request that will never resolve.
    /// One edge this creates: a `ParseFailed` install still advances
    /// `parsed_gen`, so a later successful async result for that generation
    /// is discarded too — the next edit retries the parse.
    pub fn install(&mut self, done: ParseDone, current_text_gen: u64) {
        let ParseDone {
            text_gen,
            bundle,
            outcome,
            ..
        } = done;

        if bundle.config_gen != self.bundle.config_gen {
            return; // superseded attachment — must not clear the new one's in_flight
        }
        if self.in_flight == Some(text_gen) {
            self.in_flight = None;
        }
        if text_gen != current_text_gen {
            return;
        }
        if self.parsed_gen == Some(text_gen) {
            return;
        }

        match outcome {
            ParseOutcome::Ok(parsed) => {
                let mut layers = Vec::with_capacity(1 + parsed.injected.len());
                layers.push(SyntaxLayer {
                    tree: parsed.root,
                    bundle: Arc::clone(&bundle),
                    ranges: Vec::new(),
                    depth: 0,
                });
                for injected in parsed.injected {
                    layers.push(SyntaxLayer {
                        tree: injected.tree,
                        bundle: injected.bundle,
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

        self.parsed_gen = Some(text_gen);
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

    pub fn parsed_gen(&self) -> Option<u64> {
        self.parsed_gen
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn tree_gen(&self) -> u64 {
        self.tree_gen
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn pending_edits(&self) -> &[(u64, tree_sitter::InputEdit)] {
        &self.pending_edits
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn is_in_flight(&self) -> bool {
        self.in_flight.is_some()
    }
}

impl hume_engine::providers::SyntaxSpans for Syntax {
    fn spans_for_line(
        &self,
        line_idx: usize,
        rope: &ropey::Rope,
        out: &mut Vec<(usize, usize, ScopeId)>,
    ) {
        let Some(layers) = self.layers.as_ref() else {
            return;
        };
        let mut scratch = self
            .span_scratch
            .lock()
            .expect("span scratch lock poisoned");
        let FlattenScratch { raw, stack, events } = &mut *scratch;
        layer_highlights_for_line(layers, line_idx, rope, raw, stack, events, out);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
