use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::highlight::layer_highlights_for_line;
use crate::layers::{SyntaxLayer, SyntaxLayers};
use hume_editing::changeset::ChangeSet;
use hume_editing::text::Text;
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
    raw: Vec<(usize, usize, ScopeId, u8)>,
    stack: Vec<(u8, u32, ScopeId)>,
    events: Vec<(usize, bool, u32, ScopeId, u8)>,
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
    /// Scratch for the overlap flattener, reused across `spans_for_line`
    /// calls. Lives here (not per-frame in the engine) because it survives
    /// `install`/`clear_layers` — `SyntaxLayers` is rebuilt wholesale on
    /// every install, `Syntax` is not.
    span_scratch: Mutex<FlattenScratch>,
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
            span_scratch: Mutex::new(FlattenScratch::default()),
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
