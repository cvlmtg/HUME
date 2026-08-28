use std::sync::Arc;

use hume_engine::pipeline::BufferId;
use hume_treesitter::parse_worker::ParseDone;
use hume_treesitter::registry::LanguageId;
use hume_treesitter::syntax::Syntax;

use crate::editor::{Editor, Severity};

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

        if self.state.buffers.get(bid).text().len_bytes()
            > self.state.settings.syntax_highlight_max_bytes
        {
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
        // Drain phase: runs even when disconnected — buffered results produced
        // before the worker exited are still valid and should land.
        let dones = self.parse_worker.drain_done();
        for done in dones {
            self.install_parse_done(done);
        }

        // Surface a one-shot warning if the worker exited unexpectedly and
        // suspend further request submission for this session.
        if self.parse_worker.is_disconnected() {
            self.surface_parse_worker_disconnect();
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

        let max_bytes = self.state.settings.syntax_highlight_max_bytes;

        for bid in visible {
            let buf = self.state.buffers.get(bid);
            let text_gen = buf.text_gen;
            let byte_len = buf.text().len_bytes();

            // Detach if grown past cap.
            if buf.syntax.is_some() && byte_len > max_bytes {
                self.state.buffers.get_mut(bid).syntax = None;
                continue;
            }

            // Re-attach if no syntax but buffer is under cap and language has a grammar.
            // Covers: buffers that opened over-cap and later shrank, or that had their
            // syntax detached by the growth branch above.
            if buf.syntax.is_none() {
                if byte_len <= max_bytes
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
            if buf
                .syntax
                .as_ref()
                .is_some_and(|s| s.parsed_gen() == text_gen)
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
                self.report(
                    Severity::Trace,
                    format!(
                        "syntax: pending-edit chain broken for {bid:?} — \
                         tree_gen={}, text_gen={}, first={:?}, last={:?}; \
                         full reparse triggered",
                        brk.tree_gen, brk.text_gen, brk.first, brk.last,
                    ),
                );
            }
            if let Some(req) = outcome.request {
                self.parse_worker.post(req);
            }
        }
    }

    fn surface_parse_worker_disconnect(&mut self) {
        if !self.parse_worker_disconnect_logged {
            use crate::editor::Severity;
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
