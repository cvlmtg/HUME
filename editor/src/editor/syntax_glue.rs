use std::sync::Arc;

use engine::builtins::tree_sitter_hl::TreeSitterHighlighter;
use engine::pipeline::BufferId;

use super::Editor;
use super::parse_worker::{ParseDone, ParseOutcome, ParseRequest};
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
        self.buffers.get_mut(bid).syntax = None;
        self.parse_worker.remove_in_flight(bid);

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
        // ParseDone arrives from the backend.
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

        // Write engine state: tree stays None until the backend responds.
        {
            let sbuf = &mut self.engine_view.buffers[bid];
            sbuf.tree = None;
            sbuf.syntax = Some(Arc::new(highlighter));
        }
        self.buffers.get_mut(bid).syntax = Some(BufferSyntax::new(Arc::clone(&lang_config)));

        // Empty buffers need no parse — mark up to date so reparse_stale_buffers
        // skips them until the first edit arrives.
        if self.buffers.get(bid).text().len_bytes() == 0 {
            self.buffers.get_mut(bid).syntax.as_mut()
                .expect("syntax just set above")
                .parsed_gen = text_gen;
            return;
        }

        // Post parse request.  Results arrive via drain_done in reparse_stale_buffers.
        // Tests: InlineParseBackend completes the parse inside post() — a single
        // reparse_stale_buffers call drains and installs the result.
        let source_bytes = self.buffers.get(bid).text().to_string().into_bytes();
        self.parse_worker.post(ParseRequest { bid, text_gen, lang: lang_config, source_bytes });
    }

    /// Install a `ParseDone` result: update `sbuf.tree`, refresh the highlighter
    /// source, and advance `buf.syntax.parsed_gen`.
    ///
    /// Discards the result silently when:
    /// - the buffer no longer has a syntax attachment (language was cleared)
    /// - the grammar identity changed (grammar was swapped while in flight)
    /// - the text advanced past this result (another edit arrived first)
    fn install_parse_done(&mut self, done: ParseDone) {
        let ParseDone { bid, text_gen, lang, outcome, source_bytes } = done;

        // Discard if syntax was detached while the request was in flight.
        let Some(buf_syntax) = self.buffers.get(bid).syntax.as_ref() else {
            self.parse_worker.clear_in_flight_if_matches(bid, text_gen, &lang);
            return;
        };

        // Discard if the grammar was swapped between enqueue and arrival.
        if !Arc::ptr_eq(&lang, &buf_syntax.lang) {
            self.parse_worker.clear_in_flight_if_matches(bid, text_gen, &lang);
            return;
        }

        // Discard if the text moved on since this request was submitted.
        let current_text_gen = self.buffers.get(bid).text_gen;
        if text_gen != current_text_gen {
            self.parse_worker.clear_in_flight_if_matches(bid, text_gen, &lang);
            return;
        }

        match outcome {
            ParseOutcome::Ok(tree) => {
                let sbuf = &mut self.engine_view.buffers[bid];
                sbuf.tree = Some(tree);
                if let Some(hl) = sbuf.syntax.as_deref() {
                    hl.refresh_source(&source_bytes);
                }
            }
            ParseOutcome::AbiRejected => {
                // Grammar rejected by parser ABI — detach syntax permanently.
                let sbuf = &mut self.engine_view.buffers[bid];
                sbuf.tree = None;
                sbuf.syntax = None;
                self.buffers.get_mut(bid).syntax = None;
                self.parse_worker.clear_in_flight_if_matches(bid, text_gen, &lang);
                return;
            }
            ParseOutcome::ParseFailed => {
                // Transient parse failure — leave syntax attached, allow retry next frame.
                self.parse_worker.clear_in_flight_if_matches(bid, text_gen, &lang);
                return;
            }
        }

        self.buffers.get_mut(bid).syntax.as_mut()
            .expect("syntax.is_some() guaranteed: checked above before install")
            .parsed_gen = text_gen;
        self.parse_worker.clear_in_flight_if_matches(bid, text_gen, &lang);
    }

    /// Reparse any visible buffer whose text has changed since the last parse.
    ///
    /// Called from `prepare_frame` before `update_highlight_providers`. Detaches
    /// syntax from a buffer that has grown past `syntax-highlight-max-bytes`.
    ///
    /// Non-blocking: drains any completed backend results, then submits new
    /// requests for stale buffers.  The renderer reads whatever tree is currently
    /// committed — a frame with a slightly stale tree (or no tree) is acceptable.
    pub(super) fn reparse_stale_buffers(&mut self) {
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
            if buf.syntax.is_some() && byte_len > max_bytes {
                let sbuf = &mut self.engine_view.buffers[bid];
                sbuf.syntax = None;
                sbuf.tree = None;
                self.buffers.get_mut(bid).syntax = None;
                self.parse_worker.remove_in_flight(bid);
                continue;
            }

            // Re-attach if no syntax but buffer is under cap and language has a grammar.
            // Covers: buffers that opened over-cap and later shrank, or that had their
            // syntax detached by the growth branch above.
            if buf.syntax.is_none() {
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
            // buf.syntax.is_some() is guaranteed — the is_none branch above continues.
            let parsed_gen = buf.syntax.as_ref().expect("syntax is_none handled above").parsed_gen;
            if parsed_gen == text_gen {
                continue;
            }

            // In-flight check: skip if we already have a pending request for
            // the current text_gen.
            if self.parse_worker.is_in_flight(bid, text_gen) {
                continue;
            }

            // Post a fresh async request.
            let source_bytes = self.buffers.get(bid).text().to_string().into_bytes();
            let lang = Arc::clone(
                &self.buffers.get(bid).syntax
                    .as_ref()
                    .expect("syntax is_some guaranteed above")
                    .lang,
            );
            self.parse_worker.post(ParseRequest { bid, text_gen, lang, source_bytes });
        }
    }

    fn surface_parse_worker_disconnect(&mut self) {
        if !self.parse_worker.is_disconnect_logged() {
            use super::Severity;
            self.message_log.push(
                Severity::Error,
                "parse worker disconnected — syntax highlighting suspended".to_owned(),
            );
            self.parse_worker.mark_disconnect_logged();
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
