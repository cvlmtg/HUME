use std::sync::Arc;

use hume_engine::pipeline::BufferId;
use hume_treesitter::parse_worker::ParseDone;
use hume_treesitter::registry::LanguageId;
use hume_treesitter::syntax::{ChainBreak, Syntax};

use crate::editor::{Editor, EditorState, Severity};

impl EditorState {
    /// Whether `bid` is small enough to carry a syntax tree, per
    /// `syntax-highlight-max-bytes`.
    ///
    /// The single spelling of that comparison. Three paths ask it — attach
    /// refusal, the per-frame detach/re-attach sweep, and the on-demand
    /// freshness check below — and before this predicate they asked it
    /// inline, two of them with `>` and one with `<=`, which is how half of
    /// a future cap change would have slipped through.
    pub(in crate::editor) fn syntax_size_ok(&self, bid: BufferId) -> bool {
        self.buffers.get(bid).text().len_bytes() <= self.settings.syntax_highlight_max_bytes
    }
}

/// Trace-log a broken pending-edit chain. Shared by `reparse_stale_buffers`
/// (the per-frame path, `&mut Editor`) and [`ensure_syntax_current`] (the
/// synchronous on-demand path, only `&mut EditorState`) — one message, never
/// a second copy of the string.
fn report_chain_break(state: &mut EditorState, bid: BufferId, brk: &ChainBreak) {
    state.report(
        Severity::Trace,
        format!(
            "syntax: pending-edit chain broken for {bid:?} — \
             tree_gen={}, text_gen={}, first={:?}, last={:?}; \
             full reparse triggered",
            brk.tree_gen, brk.text_gen, brk.first, brk.last,
        ),
    );
}

impl Editor {
    /// Attach the tree-sitter highlighter for `bid`.
    ///
    /// Idempotent: always clears any existing syntax attachment before
    /// attempting setup. No-ops when the buffer has no language, the language
    /// has no grammar, or the buffer exceeds `syntax-highlight-max-bytes`.
    pub(in crate::editor) fn setup_buffer_syntax(&mut self, bid: BufferId) {
        self.state.buffers.get_mut(bid).syntax = None;

        let Some(lang_id) = self.state.buffers.get(bid).language else {
            return;
        };
        let bundle = match self.state.config.languages.grammar(lang_id) {
            Some(b) => Arc::clone(b),
            None => return,
        };

        if !self.state.syntax_size_ok(bid) {
            return;
        }

        let text_gen = self.state.buffers.get(bid).text_gen;
        let text = self.state.buffers.get(bid).text().clone();
        let langs = self.state.config.languages.grammar_snapshot();
        let (syn, req) = Syntax::attach(bundle, bid, text_gen, &text, &langs);
        self.state.buffers.get_mut(bid).syntax = Some(syn);
        if let Some(req) = req {
            self.parse_worker.post(req);
        }
    }

    /// Install a `ParseDone` result into `bid`'s `Syntax`, if it still has one.
    ///
    /// Discards silently (via `Syntax::install`'s own guards) when the grammar
    /// was swapped or the text moved on since the request was submitted.
    /// Discards here, before reaching `Syntax::install`, when the buffer was
    /// closed while the request was in flight (also covers slot reuse:
    /// `BufferId` is a generational slotmap key, so a closed-then-reopened
    /// slot has a bumped version and the stale bid fails `try_get_mut`) or
    /// when syntax was detached (language cleared) while in flight.
    fn install_parse_done(&mut self, done: ParseDone) {
        let Some(buf) = self.state.buffers.try_get_mut(done.bid) else {
            return;
        };
        let text_gen = buf.text_gen;
        if let Some(syn) = buf.syntax.as_mut() {
            syn.install(done, text_gen);
        }
    }

    /// Install every parse result the worker has finished.
    ///
    /// Two callers, both on `Editor` because `parse_worker` is: the per-frame
    /// [`Self::reparse_stale_buffers`] below, and `Editor::run`'s loop
    /// immediately before it dispatches a terminal event. The second exists
    /// because a key already buffered when a parse completes consumes the
    /// worker's wake — `poll` returns that key rather than the `Ok(false)`
    /// interrupt that would have sent the loop back to `settle()` — so
    /// without this the dispatch would run `ensure_syntax_current`'s inline
    /// reparse over a tree the worker already built, paying a full parse of
    /// every injected layer for it.
    ///
    /// Drains even when the worker is disconnected: results buffered before
    /// it exited are still valid and should land.
    pub(in crate::editor) fn install_parse_results(&mut self) {
        let dones = self.parse_worker.drain_done();
        for done in dones {
            self.install_parse_done(done);
        }
        if self.parse_worker.is_disconnected() {
            self.surface_parse_worker_disconnect();
        }
    }

    /// Reparse any visible buffer whose text has changed since the last parse.
    ///
    /// Called from `Editor::settle` (via `drain_async_sources`), which runs
    /// before `prepare_frame` and thus before `update_highlight_providers`. Detaches
    /// syntax from a buffer that has grown past `syntax-highlight-max-bytes`.
    ///
    /// Non-blocking: drains any completed backend results, then for each
    /// visible buffer drives `Syntax::frame_tick` (bake + gen-gate + in-flight
    /// dedup, all internal) and submits any returned reparse request.
    pub(in crate::editor) fn reparse_stale_buffers(&mut self) {
        self.install_parse_results();

        // Suspend further request submission for this session once the
        // worker has exited; the warning itself is one-shot, raised by the
        // drain above.
        if self.parse_worker.is_disconnected() {
            return;
        }

        // Deduplicated set of visible BufferIds.
        let mut seen = rustc_hash::FxHashSet::default();
        let visible: Vec<BufferId> = self
            .view
            .panes
            .values()
            .map(|p| p.buffer_id)
            .filter(|bid| seen.insert(*bid))
            .collect();

        for bid in visible {
            let size_ok = self.state.syntax_size_ok(bid);
            let buf = self.state.buffers.get(bid);
            let text_gen = buf.text_gen;

            // Detach if grown past cap.
            if buf.syntax.is_some() && !size_ok {
                self.state.buffers.get_mut(bid).syntax = None;
                continue;
            }

            // Re-attach if no syntax but buffer is under cap and language has a grammar.
            // Covers: buffers that opened over-cap and later shrank, or that had their
            // syntax detached by the growth branch above.
            if buf.syntax.is_none() {
                if size_ok
                    && self
                        .state
                        .buffers
                        .get(bid)
                        .language
                        .is_some_and(|l| self.state.config.languages.grammar(l).is_some())
                {
                    self.setup_buffer_syntax(bid);
                }
                continue;
            }

            // frame_tick is a no-op once parsed_gen == text_gen — check that
            // before paying for the text clone and grammar-snapshot Arc bump.
            //
            // Deliberately `parsed_gen`, not `Syntax::is_current`: this asks
            // "should another request be posted for this generation?", and a
            // generation whose parse failed has already been attempted. The
            // stronger `is_current` here would re-post it every frame forever.
            // The on-demand path (`commands::structural::ensure_syntax_current`)
            // asks the other question — "are the layers safe to read?" — and
            // must use `is_current`.
            if buf
                .syntax
                .as_ref()
                .is_some_and(|s| s.parsed_gen() == Some(text_gen))
            {
                continue;
            }

            let text = self.state.buffers.get(bid).text().clone();
            let langs = self.state.config.languages.grammar_snapshot();
            let syn = self
                .state
                .buffers
                .get_mut(bid)
                .syntax
                .as_mut()
                .expect("syntax is_some checked above");
            let outcome = syn.frame_tick(bid, text_gen, &text, &langs);

            if let Some(brk) = outcome.chain_break {
                report_chain_break(&mut self.state, bid, &brk);
            }
            if let Some(req) = outcome.request {
                self.parse_worker.post(req);
            }
        }
    }
}

/// Bring `bid`'s committed tree up to date with its current text,
/// synchronously, before a structural command reads it.
///
/// The on-demand twin of [`Editor::reparse_stale_buffers`] above, and it
/// lives beside it for that reason: both decide when a buffer's tree may be
/// reparsed, and they share the byte cap ([`EditorState::syntax_size_ok`])
/// and the chain-break report. A **free function on `&mut EditorState`**, not
/// an `Editor` method, because the command dispatch funnel that calls it
/// (`commands::pipeline::run_native_body`) never holds an `Editor` — which is
/// also why it must parse inline rather than post to `Editor`'s worker.
///
/// A structural command runs after `Editor::settle` has already ticked the
/// frame's async reparse for the *previous* edit, but that tick only posts a
/// request — the worker may still be parsing it when this query runs, most
/// reliably during macro replay, which settles between keys but dispatches
/// the next one faster than tree-sitter finishes. Either way the committed
/// tree can be a generation behind by the time this query needs it.
/// `Syntax::ensure_current` closes that window; this wrapper resolves
/// the borrows it needs (an `Arc` grammar snapshot, an O(1) rope clone) and
/// routes any `ChainBreak` through [`report_chain_break`].
///
/// No-op when the buffer has no syntax attached at all (no grammar, or over
/// `syntax-highlight-max-bytes`) — the caller's `object_spans` then collects
/// nothing, which is the same "no grammar" no-op every structural command
/// already has. Also a no-op — rather than a blocking parse — in three cases
/// a fresh reparse here cannot help:
///
/// - **No committed tree yet.** Before the worker's first parse lands,
///   `build_request` has no `old_tree` to diff against, so this would run a
///   full parse of the whole buffer (up to `syntax-highlight-max-bytes`) on
///   the UI thread while the worker parses the identical bytes in the
///   background. `object_spans` already returns `ObjectSpans::default()`
///   when `layers` is `None`, so the command reads as the same "no grammar"
///   no-op until the next frame installs the worker's result.
/// - **Over the byte cap.** `reparse_stale_buffers` detaches syntax from an
///   over-cap buffer, but only once a frame — a paste that grows a buffer
///   past the cap is not yet detached if a structural keypress lands in the
///   same input batch. Checked here too rather than parsing the whole buffer
///   once before the next frame catches up.
/// - **No layer defines a textobjects query.** A grammar with no
///   `textobjects.scm` (most of them — PLUM's fetch is best-effort) can
///   never make `object_spans` return anything either way, so reparsing to
///   answer it is wasted work, worst on a `.`-repeat or macro batch that
///   pays it once per step. Misses one case: an edit that introduces a
///   *new* injected layer carrying a textobjects query is missed for this
///   one keypress — the next command call sees it.
pub(in crate::editor) fn ensure_syntax_current(state: &mut EditorState, bid: BufferId) {
    let size_ok = state.syntax_size_ok(bid);
    let buf = state.buffers.get(bid);
    let text_gen = buf.text_gen;
    let Some(syn) = buf.syntax.as_ref() else {
        return;
    };
    // Must be `is_current`, not `parsed_gen() == Some(text_gen)`: the weaker
    // form returns early on a generation whose parse failed, leaving the
    // stale-layer window `Syntax::ensure_current` exists to close wide open.
    if syn.is_current(text_gen) {
        return;
    }
    let Some(layers) = syn.layers() else {
        return;
    };
    if !size_ok {
        return;
    }
    let has_textobjects = layers
        .layers
        .iter()
        .any(|layer| layer.bundle.textobjects.is_some());
    if !has_textobjects {
        return;
    }

    let text = buf.text().clone();
    let langs = state.config.languages.grammar_snapshot();
    let syn = state
        .buffers
        .get_mut(bid)
        .syntax
        .as_mut()
        .expect("syntax is_some checked above");
    if let Some(brk) = syn.ensure_current(bid, text_gen, &text, &langs) {
        report_chain_break(state, bid, &brk);
    }
}

impl Editor {
    fn surface_parse_worker_disconnect(&mut self) {
        if !self.parse_worker_disconnect_logged {
            self.state.message_log.push(
                Severity::Error,
                "parse worker disconnected — syntax highlighting suspended".to_owned(),
            );
            self.parse_worker_disconnect_logged = true;
        }
    }

    /// Called when one or more grammars are attached. Re-runs
    /// `setup_buffer_syntax` on every open buffer whose language is in
    /// `names`, **or** whose currently-attached root grammar has an
    /// injections query — a newly attached grammar (e.g. rust) may complete
    /// injection sites in an already-open buffer of a different language
    /// (e.g. markdown fenced code blocks) without that buffer's own language
    /// ever appearing in `names`.
    pub(in crate::editor) fn sweep_buffers_for_grammars(&mut self, ids: Vec<LanguageId>) {
        if ids.is_empty() {
            return;
        }
        let bids: Vec<BufferId> = self
            .state
            .buffers
            .iter()
            .filter(|(_, buf)| {
                let matches_id = buf.language.is_some_and(|lang| ids.contains(&lang));
                let root_has_injections = buf
                    .syntax
                    .as_ref()
                    .is_some_and(|syn| syn.bundle().injections.is_some());
                matches_id || root_has_injections
            })
            .map(|(bid, _)| bid)
            .collect();
        for bid in bids {
            self.setup_buffer_syntax(bid);
        }
    }
}
