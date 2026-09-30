use rustc_hash::FxHashMap;
use std::sync::{Arc, Mutex};

use crate::highlight::layer_highlights_for_line;
use crate::layers::{SyntaxLayer, SyntaxLayers};
use hume_editing::edit::TextChange;
use hume_editing::text::BufferText;
use hume_engine::pipeline::BufferId;
use hume_engine::types::ScopeId;
use hume_rope::column::ByteCol;

use crate::edits::input_edits_from_changeset;
use crate::parse_worker::{ParseDone, ParseOutcome, ParseRequest};
use crate::registry::GrammarBundle;

/// Scratch for `layer_highlights_for_line`'s overlap flattener, reused
/// across lines and frames. Behind a `Mutex` because renders reach
/// `spans_for_line` through `&Syntax` (same uncontended single-threaded
/// pattern as `TreeSitterHighlighter::cursor`).
#[derive(Default)]
pub(crate) struct FlattenScratch {
    raw: Vec<(ByteCol, ByteCol, u8, ScopeId)>,
    stack: Vec<(u8, u32, ScopeId)>,
    events: Vec<(ByteCol, bool, u32, u8, ScopeId)>,
}

/// Diagnostic info for a broken pending-edit chain: the recorded edits do not
/// lead from the tree's text to the current one, so a text mutation went
/// unrecorded. `first` is the first pending edit's starting generation and
/// `last` the last one's ending generation. The editor logs this at
/// `Severity::Trace`; the state machine itself has no message-log access.
#[derive(Debug)]
pub struct ChainBreak {
    pub tree_gen: u64,
    pub generation: u64,
    pub first: u64,
    pub last: u64,
}

/// The `InputEdit`s of one text mutation and the two text generations it
/// leads between.
#[derive(Debug)]
pub struct PendingEdit {
    from: u64,
    to: u64,
    edits: Vec<tree_sitter::InputEdit>,
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
/// One type for the attachment, generations, trees, and in-flight tracking:
/// desync is unrepresentable because there is only one place to look.
pub struct Syntax {
    /// The attached root grammar bundle. Immutable for this attachment's
    /// lifetime: a grammar swap replaces the whole `Syntax` via a fresh
    /// `attach` call, it never mutates this field in place.
    bundle: Arc<GrammarBundle>,
    /// Committed parse layers. `None` until the first `ParseDone` installs.
    layers: Option<SyntaxLayers>,
    /// `generation` of the most recently installed (or failed) parse result.
    /// `None` until `install` has run at least once. Distinct from
    /// `Some(0)`, which is a genuine installed generation zero (a freshly
    /// opened file's text generation starts at 0 and never bumps on
    /// open). Collapsing the two into a bare `u64` would make the very
    /// first parse of every opened file indistinguishable from "already up
    /// to date", discarding it. `Some(g) == the buffer text's generation` means the
    /// installed tree is up to date.
    parsed_gen: Option<u64>,
    /// BufferText generation whose coordinates the committed `layers` describe.
    /// Advances on every successful bake and on every precise parse install.
    /// Distinct from `parsed_gen`: edits can outpace the worker, so
    /// `tree_gen` advances every frame (via bake) while `parsed_gen` only
    /// advances when the worker delivers a result.
    tree_gen: u64,
    /// Mutations recorded since the last bake or install, in order. A chain
    /// that starts at `tree_gen`, ends at the current `generation`, and has
    /// each mutation start where the previous one ended enables in-place baking; a
    /// broken link forces a full reparse. Generations identify texts rather
    /// than count edits, so a link may skip numbers.
    pending_edits: Vec<PendingEdit>,
    /// `generation` of the posted-but-unanswered parse request, if any. No
    /// `config_gen` slot is needed: `bundle` never changes within one
    /// attachment, so the posted config is always `bundle.config_gen`.
    in_flight: Option<u64>,
    /// Scratch for the overlap flattener, reused across `spans_for_line`
    /// calls. Lives here (not per-frame in the engine) because it survives
    /// `install`: `SyntaxLayers` is rebuilt wholesale on every install,
    /// `Syntax` is not.
    span_scratch: Mutex<FlattenScratch>,
}

impl Syntax {
    /// A fresh, unparsed attachment: no committed layers, no in-flight
    /// request, generations at zero.
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

    /// Create a fresh attachment for `text`. Empty text short-circuits:
    /// `parsed_gen` is set to its generation immediately, no request is
    /// built, `in_flight` stays `None`. Otherwise returns the initial
    /// full-parse request; the caller MUST post it to the parse backend.
    pub fn attach(
        bundle: Arc<GrammarBundle>,
        bid: BufferId,
        text: &BufferText,
        langs: &Arc<FxHashMap<String, Arc<GrammarBundle>>>,
    ) -> (Self, Option<ParseRequest>) {
        let generation = text.generation();
        let mut syn = Self::detached(Arc::clone(&bundle));

        if text.len_bytes() == 0 {
            syn.parsed_gen = Some(generation);
            return (syn, None);
        }

        let req = ParseRequest {
            bid,
            generation,
            bundle,
            text: text.clone(),
            old_tree: None,
            langs: Arc::clone(langs),
        };
        syn.in_flight = Some(generation);
        (syn, Some(req))
    }

    /// Parse `text` once, synchronously, into a ready-to-query attachment,
    /// with no async worker round-trip, no `frame_tick`/`install` dance. For
    /// small, static, one-shot content that isn't a real editor buffer (a
    /// hover popup's markdown): the content never changes after this call,
    /// so there is nothing to incrementally reparse, and the popup already
    /// persists across frames, so a one-frame async delay would buy nothing.
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

        syn.ensure_current(BufferId::default(), text, langs);
        syn
    }

    /// Record the `InputEdit`s of `change`, translated against its old text.
    /// Must be recorded after every text mutation.
    pub fn record_edit(&mut self, change: &TextChange<'_>) {
        self.pending_edits.push(PendingEdit {
            from: change.before().generation(),
            to: change.after().generation(),
            edits: input_edits_from_changeset(change.changes(), change.before().rope()),
        });
    }

    /// Per-frame driver. In order: gen-gate (already up to date → no
    /// request), bake pending edits into the committed layers, in-flight
    /// dedup (a request for this exact `generation` is already posted → no
    /// request), then build the next incremental request and record it as
    /// in-flight. The caller MUST post a returned request.
    pub fn frame_tick(
        &mut self,
        bid: BufferId,
        text: &BufferText,
        langs: &Arc<FxHashMap<String, Arc<GrammarBundle>>>,
    ) -> FrameTickOutcome {
        let generation = text.generation();
        if self.parsed_gen == Some(generation) {
            return FrameTickOutcome {
                request: None,
                chain_break: None,
            };
        }

        let chain_break = self.bake(generation);

        if self.in_flight == Some(generation) {
            return FrameTickOutcome {
                request: None,
                chain_break,
            };
        }

        let req = self.build_request(bid, generation, text, langs);
        self.in_flight = Some(generation);
        FrameTickOutcome {
            request: Some(req),
            chain_break,
        }
    }

    /// Build the next incremental (or, absent a baked tree at `generation`,
    /// full) parse request: the shared tail of `frame_tick` and
    /// `ensure_current`, which differ only in how the result reaches
    /// `install` (posted to the async worker vs. run inline).
    fn build_request(
        &self,
        bid: BufferId,
        generation: u64,
        text: &BufferText,
        langs: &Arc<FxHashMap<String, Arc<GrammarBundle>>>,
    ) -> ParseRequest {
        let old_tree = if self.tree_gen == generation {
            self.layers
                .as_ref()
                .and_then(SyntaxLayers::root_tree)
                .cloned()
        } else {
            None
        };

        ParseRequest {
            bid,
            generation,
            bundle: Arc::clone(&self.bundle),
            text: text.clone(),
            old_tree,
            langs: Arc::clone(langs),
        }
    }

    /// Bring the committed tree up to date with `generation` *synchronously*,
    /// bypassing the async worker entirely. A structural command (text
    /// object, navigation) reads the tree after `frame_tick` has already run
    /// for the frame, but `frame_tick` only *posts* a reparse request. The
    /// worker may still be parsing it on another thread when the query runs,
    /// most reliably during macro replay, which settles between keys but
    /// dispatches the next one faster than tree-sitter finishes. Either way
    /// the committed tree can be a generation behind by the time a query
    /// needs it, which would return wrong spans (or panic on a byte offset
    /// past the pre-edit tree's end). This closes that window at the query
    /// site instead of relying on the next frame's `frame_tick`.
    ///
    /// Bakes pending edits first, same as `frame_tick`; when the chain is
    /// intact the *root* tree's reparse is incremental and sub-frame. A full
    /// parse only happens before the worker has delivered the buffer's first
    /// tree, or after a broken edit chain, both already bounded by
    /// `syntax-highlight-max-bytes` refusing to attach syntax at all above
    /// that size. Inside a macro or dot-repeat batch, every step after the
    /// first sees an intact chain and reparses the root incrementally. Every
    /// *injected* layer (a fenced code block, `markdown.inline`) is always a
    /// full parse regardless, since incremental parsing is root-only by design
    /// (`parse_worker::run_parse`'s doc), so on a buffer with many injected
    /// layers this call's cost scales with their combined size, not just the
    /// edit.
    ///
    /// Leaves `in_flight` untouched: an asynchronous request
    /// already posted for an earlier generation is left to arrive and be
    /// discarded by `install`'s own generation guard, rather than cancelled
    /// or raced here.
    pub fn ensure_current(
        &mut self,
        bid: BufferId,
        text: &BufferText,
        langs: &Arc<FxHashMap<String, Arc<GrammarBundle>>>,
    ) -> Option<ChainBreak> {
        let generation = text.generation();
        // See `is_current` for why `parsed_gen` alone is the wrong gate here.
        if self.is_current(generation) {
            return None;
        }

        let chain_break = self.bake(generation);
        let req = self.build_request(bid, generation, text, langs);
        let mut parser = tree_sitter::Parser::new();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let done = crate::parse_worker::do_parse(&mut parser, req, &cancel);
        self.install(done, generation);
        chain_break
    }

    /// Bake `pending_edits` into the committed `layers`. No-op (and no
    /// `ChainBreak`) when there is no committed tree yet or nothing pending,
    /// checked *before* the chain-contiguity test so edits recorded before
    /// the first parse lands never trace-log or clear pending here.
    ///
    /// On a complete chain (from `tree_gen` to `generation`, every mutation
    /// starting where the previous one ended): applies
    /// every recorded `InputEdit` to every layer's tree, refreshes injected
    /// layers' cached `ranges`, advances `tree_gen`, clears `pending_edits`.
    ///
    /// On a broken chain: clears `pending_edits` (so the caller's `old_tree ==
    /// None` path posts a full reparse) and leaves `tree_gen` untouched,
    /// returning the break info for the caller to log.
    fn bake(&mut self, generation: u64) -> Option<ChainBreak> {
        if self.pending_edits.is_empty() || self.layers.is_none() {
            return None;
        }

        let tree_gen = self.tree_gen;
        let chain_ok = self.pending_edits[0].from == tree_gen
            && self
                .pending_edits
                .last()
                .expect("checked non-empty above")
                .to
                == generation
            && self.pending_edits.windows(2).all(|w| w[1].from == w[0].to);

        if chain_ok {
            let installed = self.layers.as_mut().expect("checked above");
            // Every span the text-object memo holds was collected from these
            // trees at their pre-edit positions. This is the one path that
            // mutates layers without replacing them, so it is the one path
            // that has to say so.
            installed.clear_textobject_memo();
            for layer in installed.layers.iter_mut() {
                for edit in self.pending_edits.iter().flat_map(|e| &e.edits) {
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
            self.tree_gen = generation;
            self.pending_edits.clear();
            None
        } else {
            let break_info = ChainBreak {
                tree_gen,
                generation,
                first: self.pending_edits[0].from,
                last: self
                    .pending_edits
                    .last()
                    .expect("checked non-empty above")
                    .to,
            };
            self.pending_edits.clear();
            Some(break_info)
        }
    }

    /// Install a `ParseDone` result.
    ///
    /// Clears `in_flight` when `done` matches the posted request (`generation`
    /// equal, and `config_gen` equal; a done from a *previous* attachment
    /// fails the config match and must not clear a newer attachment's
    /// in-flight record). Discards the parse outcome itself (without
    /// touching `parsed_gen`) on a config-gen mismatch (grammar swapped
    /// in flight), a stale `generation` (text moved on since submission), or a
    /// `generation` whose layers are already installed (a synchronous
    /// `ensure_current` beat an asynchronous request to the same generation:
    /// the late arrival is redundant, not stale, so it must not re-run the
    /// `ParseOutcome::Ok` arm a second time). The already-installed check
    /// runs after the `in_flight` clear above: an async result superseded
    /// this way still answered its own posted request and must still clear
    /// it, or a later `frame_tick` would dedup against a request that will
    /// never resolve.
    ///
    /// Requires all three of `parsed_gen == Some(generation)`, `tree_gen ==
    /// generation`, *and* `layers.is_some()`. No single field distinguishes
    /// "already installed" from every other state alone:
    /// - `tree_gen` alone is not enough: `bake` also advances it, on the
    ///   *mainline* path, before this very call: an intact edit chain bakes
    ///   `tree_gen` up to `generation` and only then calls `install` with the
    ///   freshly reparsed replacement, which is not redundant and must run.
    /// - `parsed_gen` alone is not enough: `ParseFailed` advances it too, so
    ///   a later result for that same generation (a retried `ensure_current`
    ///   call, or a slow async request that finally lands) would hit this
    ///   guard and be discarded even though it succeeded, leaving
    ///   `layers`/`tree_gen` stuck on stale data until an unrelated edit
    ///   bumps `generation` past this generation entirely.
    /// - `layers.is_some()` resolves the generation-`0` ambiguity `tree_gen`
    ///   would otherwise have on its own: it starts at plain `0`, coinciding
    ///   with a buffer's first parse (a new text lineage starts at
    ///   generation `0`), the same ambiguity
    ///   `parsed_gen` is `Option` to avoid.
    ///
    /// Together: `parsed_gen == Some(generation)` means an `install` call has
    /// already *run* for this generation (either arm); `tree_gen ==
    /// generation && layers.is_some()` means the layers it left behind
    /// reflect that generation, not just a bake pending a
    /// replacement. Only when both hold was this generation's `Ok` result
    /// already committed.
    pub fn install(&mut self, done: ParseDone, current_generation: u64) {
        let ParseDone {
            generation,
            bundle,
            outcome,
            ..
        } = done;

        if bundle.config_gen != self.bundle.config_gen {
            return; // superseded attachment: must not clear the new one's in_flight
        }
        if self.in_flight == Some(generation) {
            self.in_flight = None;
        }
        if generation != current_generation {
            return;
        }
        if self.parsed_gen == Some(generation)
            && self.tree_gen == generation
            && self.layers.is_some()
        {
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
                self.layers = Some(SyntaxLayers::new(layers));
                // Every recorded edit ends at or before the current
                // generation, which this result is for.
                self.pending_edits.clear();
                self.tree_gen = generation;
            }
            ParseOutcome::ParseFailed => {
                // Advance parsed_gen so this generation is not retried every
                // frame; tree_gen/layers stay as-is (next edit bumps
                // generation and triggers a fresh attempt).
            }
        }

        self.parsed_gen = Some(generation);
    }

    /// Committed layers for the renderer. `None` until the first install.
    pub fn layers(&self) -> Option<&SyntaxLayers> {
        self.layers.as_ref()
    }

    /// The attached root grammar bundle, read by `sweep_buffers_for_grammars`
    /// to check whether it has an injections query.
    pub fn bundle(&self) -> &Arc<GrammarBundle> {
        &self.bundle
    }

    pub fn parsed_gen(&self) -> Option<u64> {
        self.parsed_gen
    }

    /// Whether the committed layers describe `generation` itself: the
    /// freshness question every caller actually means, and the gate
    /// [`Self::ensure_current`] skips its reparse on.
    ///
    /// Both halves are required, and `parsed_gen` alone is the tempting
    /// wrong answer: `install`'s `ParseFailed` arm advances `parsed_gen` to
    /// the failed generation while leaving `layers`/`tree_gen` exactly where
    /// they were. A caller gating on `parsed_gen` alone therefore reports
    /// "current" over a tree that predates the edit, and the next reader (a
    /// structural text-object query) hands `byte_to_char` an offset past the
    /// buffer's own length. Requiring `tree_gen == generation` too closes that:
    /// a `ParseFailed` generation never satisfies it, so the caller reparses
    /// instead of trusting stale layers.
    ///
    /// Not the same question as `install`'s "already installed" guard, which
    /// additionally requires `layers.is_some()`: that one asks whether this
    /// generation's `Ok` result was already committed, not whether the layers
    /// are current.
    pub fn is_current(&self, generation: u64) -> bool {
        self.parsed_gen == Some(generation) && self.tree_gen == generation
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn tree_gen(&self) -> u64 {
        self.tree_gen
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn pending_edits(&self) -> &[PendingEdit] {
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
        line_idx: hume_rope::line::ContentLine,
        rope: &ropey::Rope,
        out: &mut Vec<(ByteCol, ByteCol, ScopeId)>,
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

#[cfg(test)]
mod tests;
