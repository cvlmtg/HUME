use std::sync::Arc;

use engine::builtins::tree_sitter_hl::TreeSitterHighlighter;
use engine::pipeline::BufferId;

use super::Editor;
use super::parse_worker::{ParseDone, ParseRequest};
use super::syntax::BufferSyntax;

impl Editor {
    /// Attach the tree-sitter highlighter for `bid`.
    ///
    /// Idempotent: always clears existing syntax state before attempting setup.
    /// No-ops when the buffer has no language, the language has no grammar, or
    /// the buffer exceeds `syntax-highlight-max-bytes`.
    pub(super) fn setup_buffer_syntax(&mut self, bid: BufferId) {
        // Clear existing syntax state unconditionally.
        {
            let sbuf = &mut self.engine_view.buffers[bid];
            sbuf.syntax = None;
            sbuf.tree = None;
        }
        self.buffers.get_mut(bid).parser = None;
        self.parse_worker.in_flight.remove(&bid);

        // Resolve language → grammar bundle.
        let lang_name = match self.buffers.get(bid).language.clone() {
            Some(n) => n,
            None => return,
        };
        let lang_config = match self.languages.by_name(&lang_name) {
            Some(c) if c.grammar.is_some() => Arc::clone(c),
            _ => return,
        };

        // Size gate.
        if self.buffers.get(bid).text().len_bytes() > self.settings.syntax_highlight_max_bytes {
            return;
        }

        // Build highlighter from shared query (capture names already interned at
        // attach_grammar time — intern_runtime deduplicates).
        // Initial source is empty — refresh_source is called when the first
        // ParseDone arrives from the worker.
        let query = Arc::clone(
            &lang_config
                .grammar
                .as_ref()
                .expect("grammar.is_some() checked at match guard above")
                .query,
        );
        let highlighter =
            TreeSitterHighlighter::from_shared_query(query, &mut self.engine_view.registry, Vec::new());

        let text_gen = self.buffers.get(bid).text_gen;

        // Write engine state: tree stays None until the worker responds.
        {
            let sbuf = &mut self.engine_view.buffers[bid];
            sbuf.tree = None;
            sbuf.syntax = Some(Arc::new(highlighter));
        }
        self.buffers.get_mut(bid).parser = Some(BufferSyntax::new(Arc::clone(&lang_config)));

        // Post parse request asynchronously.  The first frame after attachment
        // will render without highlights; results arrive on a subsequent frame.
        // Tests use `join_pending_parses` to synchronise before asserting state.
        let source_bytes = self.buffers.get(bid).text().to_string().into_bytes();
        self.parse_worker.post(ParseRequest { bid, text_gen, lang: lang_config, source_bytes });
    }

    /// Install a `ParseDone` result: update `sbuf.tree`, refresh the highlighter
    /// source, and advance `buf.parser.parsed_gen`.
    ///
    /// Discards the result silently when:
    /// - the buffer no longer has a syntax attachment (language was cleared)
    /// - the grammar identity changed (grammar was swapped while in flight)
    /// - the text advanced past this result (another edit arrived first)
    fn install_parse_done(&mut self, done: ParseDone) {
        let bid = done.bid;

        // Discard if syntax was detached while the request was in flight.
        let Some(buf_syntax) = self.buffers.get(bid).parser.as_ref() else {
            self.parse_worker.in_flight.remove(&bid);
            return;
        };

        // Discard if the grammar was swapped between enqueue and arrival.
        if !Arc::ptr_eq(&done.lang, &buf_syntax.lang) {
            self.parse_worker.in_flight.remove(&bid);
            return;
        }

        // Discard if the text moved on since this request was submitted.
        let current_text_gen = self.buffers.get(bid).text_gen;
        if done.text_gen != current_text_gen {
            self.parse_worker.in_flight.remove(&bid);
            return;
        }

        match done.tree {
            Some(tree) => {
                let sbuf = &mut self.engine_view.buffers[bid];
                sbuf.tree = Some(tree);
                if let Some(hl) = sbuf.syntax.as_deref() {
                    hl.refresh_source(&done.source_bytes);
                }
            }
            None => {
                // Parser rejected the grammar (ABI mismatch) — detach syntax.
                let sbuf = &mut self.engine_view.buffers[bid];
                sbuf.tree = None;
                sbuf.syntax = None;
                self.buffers.get_mut(bid).parser = None;
                self.parse_worker.in_flight.remove(&bid);
                return;
            }
        }

        self.buffers.get_mut(bid).parser.as_mut()
            .expect("parser.is_some() guaranteed: checked above before install")
            .parsed_gen = done.text_gen;
        self.parse_worker.in_flight.remove(&bid);
    }

    /// Reparse any visible buffer whose text has changed since the last parse.
    ///
    /// Called from `prepare_frame` before `update_highlight_providers`. Detaches
    /// syntax from a buffer that has grown past `syntax-highlight-max-bytes`.
    ///
    /// Non-blocking: drains any completed worker results, then submits new
    /// requests for stale buffers.  The renderer reads whatever tree is currently
    /// committed — a frame with a slightly stale tree (or no tree) is acceptable.
    pub(super) fn reparse_stale_buffers(&mut self) {
        // Surface a one-shot warning if the worker exited unexpectedly.
        // `disconnected` stays true for the session lifetime; `disconnect_logged`
        // ensures the message fires exactly once.
        if self.parse_worker.disconnected {
            if !self.parse_worker.disconnect_logged {
                use super::Severity;
                self.message_log.push(
                    Severity::Error,
                    "parse worker disconnected — syntax highlighting suspended".to_owned(),
                );
                self.parse_worker.disconnect_logged = true;
            }
            return;
        }

        // Drain phase: install any parse results the worker has completed since
        // the previous frame.
        while let Ok(done) = self.parse_worker.rx_done.try_recv() {
            self.install_parse_done(done);
        }

        // Deduplicated set of visible BufferIds.
        let mut seen = std::collections::HashSet::new();
        let visible: Vec<BufferId> = self
            .engine_view
            .panes
            .values()
            .map(|p| p.buffer_id)
            .filter(|bid| seen.insert(*bid))
            .collect();

        let max_bytes = self.settings.syntax_highlight_max_bytes;

        for bid in visible {
            let buf = self.buffers.get(bid);
            let text_gen = buf.text_gen;
            let byte_len = buf.text().len_bytes();

            // Detach if grown past cap.
            if buf.parser.is_some() && byte_len > max_bytes {
                let sbuf = &mut self.engine_view.buffers[bid];
                sbuf.syntax = None;
                sbuf.tree = None;
                self.buffers.get_mut(bid).parser = None;
                self.parse_worker.in_flight.remove(&bid);
                continue;
            }

            // Re-attach if no parser but buffer is under cap and language has a grammar.
            // Covers: buffers that opened over-cap and later shrank, or that had their
            // parser detached by the growth branch above.
            if buf.parser.is_none() {
                if byte_len <= max_bytes
                    && self.buffers.get(bid).language.as_deref()
                        .and_then(|l| self.languages.by_name(l))
                        .is_some_and(|c| c.grammar.is_some())
                {
                    self.setup_buffer_syntax(bid);
                }
                continue;
            }

            // Gen-gate: skip if already up to date.
            // buf.parser.is_some() is guaranteed — the is_none branch above continues.
            let parsed_gen = buf.parser.as_ref().expect("parser is_none handled above").parsed_gen;
            if parsed_gen == text_gen {
                continue;
            }

            // In-flight check: skip if we already have a pending request for
            // the current text_gen.
            if self.parse_worker.in_flight.get(&bid)
                .is_some_and(|inf| inf.text_gen == text_gen)
            {
                continue;
            }

            // Post a fresh async request.
            let source_bytes = self.buffers.get(bid).text().to_string().into_bytes();
            let lang = Arc::clone(
                &self.buffers.get(bid).parser
                    .as_ref()
                    .expect("parser is_some guaranteed above")
                    .lang,
            );
            self.parse_worker.post(ParseRequest { bid, text_gen, lang, source_bytes });
        }
    }

    /// Block until all in-flight parse requests have produced results, then
    /// drain and install them.  Used in tests and wherever a sync checkpoint
    /// is needed (e.g. a future `:write` guarantee).
    pub(crate) fn join_pending_parses(&mut self) {
        while !self.parse_worker.in_flight.is_empty() {
            match self.parse_worker.rx_done.recv() {
                Ok(done) => self.install_parse_done(done),
                Err(_) => {
                    self.parse_worker.disconnected = true;
                    self.parse_worker.in_flight.clear();
                    return;
                }
            }
        }
    }

    /// Called when one or more grammars are attached. Re-runs `setup_buffer_syntax`
    /// on every open buffer whose language is in `names`.
    pub(super) fn sweep_buffers_for_grammars(&mut self, names: Vec<String>) {
        if names.is_empty() {
            return;
        }
        let bids: Vec<BufferId> = self
            .buffers
            .iter()
            .filter(|(_, buf)| {
                buf.language
                    .as_deref()
                    .is_some_and(|lang| names.iter().any(|n| n == lang))
            })
            .map(|(bid, _)| bid)
            .collect();
        for bid in bids {
            self.setup_buffer_syntax(bid);
        }
    }
}
