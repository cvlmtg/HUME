use hume_engine::pipeline::BufferId;
use hume_engine::theme::ScopeRegistry;
use hume_treesitter::parse_worker::ParseDone;
use hume_treesitter::registry::LanguageId;
use hume_treesitter::syntax::{ChainBreak, Syntax};

use super::GrammarLoads;
use crate::editor::{Editor, EditorState, Severity};

impl EditorState {
    /// Whether `bid` is small enough to carry a syntax tree, per
    /// `syntax-highlight-max-bytes`.
    ///
    /// The single spelling of that comparison. Three paths ask it: attach
    /// refusal, the per-frame detach/re-attach sweep, and the on-demand
    /// freshness check below. Asking it inline at each path would let the
    /// copies drift apart (one `>`, another `<=`), so a cap change could
    /// reach only some of them.
    pub(in crate::editor::syntax::parse) fn syntax_size_ok(&self, bid: BufferId) -> bool {
        self.buffers.get(bid).text().len_bytes() <= self.settings.syntax_highlight_max_bytes
    }
}

/// Trace-log a broken pending-edit chain.
fn report_chain_break(state: &mut EditorState, bid: BufferId, brk: &ChainBreak) {
    state.report(
        Severity::Trace,
        format!(
            "syntax: pending-edit chain broken for {bid:?}: \
             tree_gen={}, generation={}, first={}, last={}; \
             full reparse triggered",
            brk.tree_gen, brk.generation, brk.first, brk.last,
        ),
    );
}

impl Editor {
    /// Attach the tree-sitter highlighter for `bid`, loading its language's
    /// grammar first if it is registered but not loaded.
    ///
    /// Idempotent: always clears any existing syntax attachment before
    /// attempting setup. No-ops when the buffer has no language, the buffer
    /// exceeds `syntax-highlight-max-bytes`, or the language has no grammar
    /// or its grammar fails to load. The size is checked first, so a buffer
    /// over the cap loads no grammar.
    pub(in crate::editor::syntax) fn setup_buffer_syntax(&mut self, bid: BufferId) {
        self.state.buffers.get_mut(bid).syntax = None;

        let Some(lang_id) = self.state.buffers.get(bid).language else {
            return;
        };
        if !self.state.syntax_size_ok(bid) {
            return;
        }
        let Some(bundle) = self
            .state
            .config
            .languages
            .load_grammar(lang_id, &mut self.view.registry)
        else {
            return;
        };

        let text = self.state.buffers.get(bid).text().clone();
        let langs = self.state.config.languages.grammar_snapshot();
        let (syn, req) = Syntax::attach(bundle, bid, &text, &langs);
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
    ///
    /// Loads the grammars of the injections the parse could not resolve, and
    /// has the buffer resolve them again when any loads: the worker has no
    /// access to the registry it would load them into.
    fn install_parse_done(&mut self, done: ParseDone) {
        let Some(buf) = self.state.buffers.try_get_mut(done.bid) else {
            return;
        };
        let generation = buf.text().generation();
        let Some(syn) = buf.syntax.as_mut() else {
            return;
        };
        syn.install(
            done,
            generation,
            GrammarLoads::new(&mut self.state.config.languages, &mut self.view.registry),
        );
    }

    /// Install every parse result the worker has finished.
    ///
    /// Two callers, both on `Editor` because `parse_worker` is: the per-frame
    /// [`Self::reparse_stale_buffers`] below, and `Editor::run`'s loop
    /// immediately before it dispatches a terminal event. The second exists
    /// because a key already buffered when a parse completes consumes the
    /// worker's wake: `poll` returns that key rather than the `Ok(false)`
    /// interrupt that would have sent the loop back to `settle()`, so
    /// without this the dispatch would run `ensure_syntax_current`'s inline
    /// reparse over a tree the worker already built, paying a full parse of
    /// every injected layer for it.
    ///
    /// Drains even when the worker is disconnected: results buffered before
    /// it exited are still valid and should land.
    ///
    /// See [`Self::reparse_stale_buffers`]'s doc for the other caller's own
    /// reason to drain first. The two cover different wake sources (a key
    /// racing a completion vs. a completion with no key pending) and neither
    /// makes the other's call here redundant.
    pub(in crate::editor) fn install_parse_results(&mut self) {
        let dones = self.parse_worker.drain_done();
        for done in dones {
            self.install_parse_done(done);
        }
        if self.parse_worker.is_disconnected() {
            self.surface_parse_worker_disconnect();
        }
        self.load_grammar_languages();
        // Before the key `Editor::run` dispatches next, so a grammar these
        // results failed to load is not reported as that key's message.
        self.state.report_failed_grammar_loads();
    }

    /// Read the bundled data file of every language given a grammar since the
    /// last call. [`Self::install_parse_results`] runs it before each key's
    /// dispatch and before each frame's parse requests. The read needs the
    /// Steel engine, which a command does not have, and a command's own parse
    /// (`ensure_syntax_current`) can build an injected layer: a language read
    /// only once its layer appeared would reach that command without its data.
    /// So every registered grammar's file is read, at startup for every
    /// installed one.
    fn load_grammar_languages(&mut self) {
        for id in self.state.config.languages.take_bundled_pending() {
            self.load_bundled_language(id);
        }
    }

    /// Reparse any visible buffer whose text has changed since the last parse.
    ///
    /// Called last in `Editor::settle`, after the pending-work drain, so a
    /// text change a queued call made is carried into the tree before
    /// `prepare_frame` and `update_highlight_providers` read it. Detaches
    /// syntax from a buffer that has grown past `syntax-highlight-max-bytes`.
    ///
    /// Non-blocking: drains any completed backend results, then for each
    /// visible buffer drives `Syntax::frame_tick` (bake + gen-gate + in-flight
    /// dedup, all internal) and submits any returned reparse request.
    ///
    /// The drain here is not redundant with [`Self::install_parse_results`]'s
    /// other call site in `Editor::run` (before dispatching a terminal
    /// event): that one only runs on the `Ok(true)` branch of `poll`, i.e.
    /// when an actual event is about to be read. A parse-worker wake with
    /// *no* event pending takes `poll`'s `Ok(false)` branch straight back to
    /// `settle()`. This function, via `settle`, is the only
    /// place that result ever gets installed. Skip the drain here and a
    /// buffer whose async parse finishes while the user is idle (a large
    /// file just opened, a slow injected-layer grammar, an LSP edit with no
    /// follow-up keystroke) never highlights until the next actual keypress
    /// happens to trigger the other call site, or never, if none comes.
    pub(in crate::editor) fn reparse_stale_buffers(&mut self) {
        self.install_parse_results();
        // Suspend further request submission for this session once the
        // worker has exited; the warning itself is one-shot, raised by the
        // drain above.
        if !self.parse_worker.is_disconnected() {
            self.post_stale_requests();
        }
        self.state.report_failed_grammar_loads();
    }

    /// The per-buffer pass of [`Self::reparse_stale_buffers`]: detach,
    /// re-attach or post a reparse request for each visible buffer.
    fn post_stale_requests(&mut self) {
        // Deduplicated set of every buffer *any* pane is showing, not just
        // the active tab's. Built from the full `view.panes` pool, not
        // `Editor::active_pane_ids`: a buffer open in a background tab
        // should keep its grammar attached and stay reparsed, so switching
        // back to that tab doesn't land on stale highlighting.
        let mut seen = rustc_hash::FxHashSet::default();
        let visible: Vec<BufferId> = self
            .view
            .panes
            .every_pane_across_all_tabs()
            .map(|(_, p)| p.buffer_id())
            .filter(|bid| seen.insert(*bid))
            .collect();

        for bid in visible {
            let size_ok = self.state.syntax_size_ok(bid);
            let buf = self.state.buffers.get(bid);
            let generation = buf.text().generation();

            // Detach if grown past cap.
            if buf.syntax.is_some() && !size_ok {
                self.state.buffers.get_mut(bid).syntax = None;
                continue;
            }

            // Re-attach if no syntax but buffer is under cap and language has a
            // grammar that is loaded or yet to load. Covers: buffers that opened
            // over-cap and later shrank, or that had their syntax detached by the
            // growth branch above. A grammar that failed to load is unavailable,
            // so its failure is reported once rather than every frame.
            if buf.syntax.is_none() {
                if size_ok
                    && self
                        .state
                        .buffers
                        .get(bid)
                        .language
                        .is_some_and(|l| self.state.config.languages.grammar_available(l))
                {
                    self.setup_buffer_syntax(bid);
                }
                continue;
            }

            // frame_tick is a no-op once parsed_gen == generation, so check that
            // before paying for the text clone and grammar-snapshot Arc bump.
            //
            // Checks `parsed_gen`, not `Syntax::is_current`: this asks
            // "should another request be posted for this generation?", and a
            // generation whose parse failed has already been attempted. The
            // stronger `is_current` here would re-post it every frame forever.
            // The on-demand path (`ensure_syntax_current`, below) asks the
            // other question ("are the layers safe to read?") and must
            // use `is_current`.
            if buf
                .syntax
                .as_ref()
                .is_some_and(|s| s.parsed_gen() == Some(generation))
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
            let outcome = syn.frame_tick(bid, &text, &langs);

            if let Some(brk) = outcome.chain_break {
                report_chain_break(&mut self.state, bid, &brk);
            }
            if let Some(req) = outcome.request {
                self.parse_worker.post(req);
            }
        }
    }
}

/// What a caller of [`ensure_syntax_current`] reads from the layers, which
/// decides whether a blocking parse can change its answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum SyntaxUse {
    /// Structural text objects and navigation.
    TextObjects,
    /// The language of each injected layer.
    InjectedLanguages,
    /// The indent of a new line.
    Indent,
}

/// Bring `bid`'s committed tree up to date with its current text,
/// synchronously, before a command reads it.
///
/// The on-demand twin of [`Editor::reparse_stale_buffers`]: it shares that
/// function's byte cap ([`EditorState::syntax_size_ok`]) and chain-break
/// report. It takes `&mut EditorState` and parses inline because its caller,
/// the command dispatch funnel, never holds an `Editor` or its worker.
///
/// The async reparse posted for the previous edit may still be running (most
/// often during macro replay), so the committed tree can be a generation
/// behind. `Syntax::ensure_current` closes that window.
///
/// A site whose language's grammar is registered but not loaded loads it
/// here and gets its layer, so the command reading the layers sees it.
///
/// No-op when the buffer has no syntax, and also when a blocking parse here
/// cannot help:
/// - no committed tree yet: it would be a full parse on the UI thread racing
///   the worker's identical one;
/// - over the byte cap: a paste can outgrow it before the next frame detaches
///   syntax;
/// - for [`SyntaxUse::TextObjects`], no layer has a textobjects query:
///   `object_spans` would return nothing anyway. An edit that adds a new
///   injected layer with one is missed for that one keypress;
/// - for [`SyntaxUse::Indent`], no layer has an indents query;
/// - for [`SyntaxUse::InjectedLanguages`], the root grammar has no
///   injections query, so there is no layer but the root.
///
/// The last three do not apply while injections await resolving again with
/// a grammar loaded since: the layer that grammar injects can have the
/// query.
pub(in crate::editor) fn ensure_syntax_current(
    state: &mut EditorState,
    scopes: &mut ScopeRegistry,
    bid: BufferId,
    use_: SyntaxUse,
) {
    let size_ok = state.syntax_size_ok(bid);
    let buf = state.buffers.get(bid);
    let generation = buf.text().generation();
    let Some(syn) = buf.syntax.as_ref() else {
        return;
    };
    // Must be `is_current`, not `parsed_gen() == Some(generation)`: the weaker
    // form returns early on a generation whose parse failed, leaving the
    // stale-layer window `Syntax::ensure_current` exists to close wide open.
    if syn.is_current(generation) {
        return;
    }
    let Some(layers) = syn.layers() else {
        return;
    };
    if !size_ok {
        return;
    }
    let can_change = syn.reresolve_pending()
        || match use_ {
            SyntaxUse::TextObjects => layers
                .layers
                .iter()
                .any(|layer| layer.bundle.textobjects.is_some()),
            SyntaxUse::Indent => layers
                .layers
                .iter()
                .any(|layer| layer.bundle.indents.is_some()),
            SyntaxUse::InjectedLanguages => syn.bundle().injections.is_some(),
        };
    if !can_change {
        return;
    }

    let text = buf.text().clone();
    let loads = GrammarLoads::new(&mut state.config.languages, scopes);
    let syn = state
        .buffers
        .get_mut(bid)
        .syntax
        .as_mut()
        .expect("syntax is_some checked above");
    let chain_break = syn.ensure_current(&text, loads);
    if let Some(brk) = chain_break {
        report_chain_break(state, bid, &brk);
    }
}

impl Editor {
    fn surface_parse_worker_disconnect(&mut self) {
        if !self.parse_worker_disconnect_logged {
            self.state.message_log.push(
                Severity::Error,
                "parse worker disconnected: syntax highlighting suspended".to_owned(),
            );
            self.parse_worker_disconnect_logged = true;
        }
    }

    /// Called when one or more grammars are registered or attached. Re-runs
    /// `setup_buffer_syntax` on every open buffer whose language is in
    /// `ids`, as its own grammar changed. Every other open buffer whose root
    /// grammar has an injections query resolves its injections again, keeping
    /// its layers: the new grammar (e.g. rust) may complete injection sites
    /// in a buffer of a different language (e.g. markdown fenced code blocks)
    /// without that buffer's own language ever appearing in `ids`.
    pub(in crate::editor) fn sweep_buffers_for_grammars(&mut self, ids: Vec<LanguageId>) {
        if ids.is_empty() {
            return;
        }
        let mut rebuilt = Vec::new();
        let mut reresolved = Vec::new();
        for (bid, buf) in self.state.buffers.iter() {
            if buf.language.is_some_and(|lang| ids.contains(&lang)) {
                rebuilt.push(bid);
            } else if buf
                .syntax
                .as_ref()
                .is_some_and(|syn| syn.bundle().injections.is_some())
            {
                reresolved.push(bid);
            }
        }
        let langs = self.state.config.languages.grammar_snapshot();
        for bid in reresolved {
            self.state
                .buffers
                .get_mut(bid)
                .syntax
                .as_mut()
                .expect("collected for its syntax above")
                .reresolve_injections(&langs);
        }
        for bid in rebuilt {
            self.setup_buffer_syntax(bid);
        }
    }
}
