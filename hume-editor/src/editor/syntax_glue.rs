use std::sync::Arc;

use hume_engine::pipeline::BufferId;
use hume_engine::syntax_layers::{SyntaxLayer, SyntaxLayers};

use super::buffer::Buffer;
use super::parse_worker::{ParseDone, ParseOutcome, ParseRequest};
use super::syntax::BufferSyntax;
use super::{Editor, Severity};
use hume_editing::changeset::{ChangeSet, Operation};

// ── Incremental parse helpers ─────────────────────────────────────────────────

/// Translate a `ChangeSet` into a sequence of `tree_sitter::InputEdit`s.
///
/// `rope` must be the buffer text **before** the edit (the old document).  All
/// char offsets in the changeset are converted to byte offsets and (row, byte-col)
/// positions via the rope's index helpers.
fn input_edits_from_changeset(cs: &ChangeSet, rope: &ropey::Rope) -> Vec<tree_sitter::InputEdit> {
    let mut edits = Vec::new();
    let mut pre_char: usize = 0;
    let mut ops = cs.ops().iter();

    while let Some(op) = ops.next() {
        match op {
            Operation::Retain(n) => {
                pre_char += n;
            }
            Operation::Delete(del_n) => {
                let start_char = pre_char;
                let old_end_char = pre_char + del_n;
                // A following Insert forms a replace — consume it together.
                let inserted = match ops.as_slice().first() {
                    Some(Operation::Insert(s)) => {
                        let s = s.as_str();
                        ops.next();
                        s
                    }
                    _ => "",
                };
                edits.push(make_input_edit(start_char, old_end_char, inserted, rope));
                pre_char = old_end_char;
            }
            Operation::Insert(ins_s) => {
                // Pure insert: old document position doesn't advance.
                edits.push(make_input_edit(pre_char, pre_char, ins_s.as_str(), rope));
            }
        }
    }

    // All edits are computed in pre-edit coordinate space (the old rope).
    // `tree.edit()` mutates coordinates in-place: applying a left edit first shifts
    // every subsequent byte position, so a right edit specified in original coords
    // would land at the wrong place.  Reversing to descending start order means the
    // rightmost edit is applied first — its coordinates are never invalidated by
    // anything to its left, and vice versa, so all edits remain valid in the
    // pre-edit coordinate space at apply time.
    edits.reverse();
    edits
}

/// Build a single `InputEdit` from char-indexed old/new positions and the inserted text.
fn make_input_edit(
    start_char: usize,
    old_end_char: usize,
    inserted: &str,
    rope: &ropey::Rope,
) -> tree_sitter::InputEdit {
    let start_byte = rope.char_to_byte(start_char);
    let old_end_byte = rope.char_to_byte(old_end_char);
    let new_end_byte = start_byte + inserted.len(); // str::len() is byte count

    let start_row = rope.char_to_line(start_char);
    let start_col = start_byte - rope.line_to_byte(start_row);

    let old_end_row = rope.char_to_line(old_end_char);
    let old_end_col = old_end_byte - rope.line_to_byte(old_end_row);

    let (new_end_row, new_end_col) = new_end_point(start_row, start_col, inserted);

    tree_sitter::InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte,
        start_position: tree_sitter::Point {
            row: start_row,
            column: start_col,
        },
        old_end_position: tree_sitter::Point {
            row: old_end_row,
            column: old_end_col,
        },
        new_end_position: tree_sitter::Point {
            row: new_end_row,
            column: new_end_col,
        },
    }
}

/// Compute `new_end_position` for an insertion starting at `(start_row, start_col)`.
fn new_end_point(start_row: usize, start_col: usize, inserted: &str) -> (usize, usize) {
    let newline_count = inserted.bytes().filter(|&b| b == b'\n').count();
    if newline_count == 0 {
        (start_row, start_col + inserted.len())
    } else {
        // Column is the byte count after the last newline in the inserted text.
        let last_nl = inserted.rfind('\n').unwrap();
        (start_row + newline_count, inserted.len() - last_nl - 1)
    }
}

/// Record one batch of `InputEdit`s on `buf.syntax.pending_edits`.
///
/// No-op when no grammar is attached (`buf.syntax` is `None`).
/// Called from `doc_ops` immediately after every text mutation.
pub(super) fn record_pending_edits(
    buf: &mut Buffer,
    text_gen: u64,
    cs: &ChangeSet,
    rope_pre: &ropey::Rope,
) {
    if let Some(syn) = buf.syntax.as_mut() {
        for edit in input_edits_from_changeset(cs, rope_pre) {
            syn.pending_edits.push((text_gen, edit));
        }
    }
}

impl Editor {
    /// Attach the tree-sitter highlighter for `bid`.
    ///
    /// Idempotent: always clears existing syntax state before attempting setup.
    /// No-ops when the buffer has no language, the language has no grammar, or
    /// the buffer exceeds `syntax-highlight-max-bytes`.
    pub(super) fn setup_buffer_syntax(&mut self, bid: BufferId) {
        // Clear existing syntax state unconditionally.
        self.view.buffers[bid].syntax = None;
        self.state.buffers.get_mut(bid).syntax = None;
        // Must clear before posting the fresh request: is_in_flight() matches on
        // text_gen alone, so a stale entry (different Arc<LanguageConfig> after a
        // grammar swap via sweep_buffers_for_grammars) would short-circuit the
        // reparse_stale_buffers request-phase even though the language changed.
        self.parse_worker.remove_in_flight(bid);

        // Resolve language → grammar bundle.
        let lang_name = match self.state.buffers.get(bid).language.clone() {
            Some(n) => n,
            None => return,
        };
        let lang_config = match self.state.languages.by_name(&lang_name) {
            Some(c) if c.grammar.is_some() => Arc::clone(c),
            _ => return,
        };

        // Size gate.
        if self.state.buffers.get(bid).text().len_bytes()
            > self.state.settings.syntax_highlight_max_bytes
        {
            return;
        }

        let text_gen = self.state.buffers.get(bid).text_gen;

        // The engine-side layers stay None until the backend responds; the
        // renderer reads them straight from `SharedBuffer.syntax`.
        self.state.buffers.get_mut(bid).syntax = Some(BufferSyntax::new(Arc::clone(&lang_config)));

        // Empty buffers need no parse — mark up to date so reparse_stale_buffers
        // skips them until the first edit arrives.
        if self.state.buffers.get(bid).text().len_bytes() == 0 {
            self.state
                .buffers
                .get_mut(bid)
                .syntax
                .as_mut()
                .expect("syntax just set above")
                .parsed_gen = text_gen;
            return;
        }

        // Post parse request.  tree stays None until the backend responds, so the
        // first painted frame after open is uncoloured.  Colour arrives on a later
        // frame once drain_done in reparse_stale_buffers installs the result.
        // Accepted tradeoff for uniform-path simplicity; the 8 ms in-flight poll in
        // the run loop (lifecycle.rs) makes the flash imperceptible interactively.
        // Tests: InlineParseBackend completes the parse inside post() — a single
        // reparse_stale_buffers call drains and installs the result.
        let text = self.state.buffers.get(bid).text().clone();
        self.parse_worker.post(ParseRequest {
            bid,
            text_gen,
            lang: lang_config,
            text,
            old_tree: None,
            langs: self.state.languages.grammar_snapshot(),
        });
    }

    /// Install a `ParseDone` result: update `sbuf.tree` and advance
    /// `buf.syntax.parsed_gen`.
    ///
    /// Discards the result silently when:
    /// - the buffer was closed while the request was in flight
    /// - the buffer no longer has a syntax attachment (language was cleared)
    /// - the grammar identity changed (grammar was swapped while in flight)
    /// - the text advanced past this result (another edit arrived first)
    fn install_parse_done(&mut self, done: ParseDone) {
        let ParseDone {
            bid,
            text_gen,
            lang,
            outcome,
        } = done;
        self.apply_parse_outcome(bid, text_gen, &lang, outcome);
        self.parse_worker
            .clear_in_flight_if_matches(bid, text_gen, &lang);
    }

    fn apply_parse_outcome(
        &mut self,
        bid: hume_engine::pipeline::BufferId,
        text_gen: u64,
        lang: &Arc<super::syntax::LanguageConfig>,
        outcome: ParseOutcome,
    ) {
        // Discard if the buffer was closed while the request was in flight.
        // Also covers slot reuse: BufferId is a generational slotmap key, so a
        // closed-then-reopened slot has a bumped version and the stale bid fails
        // here — it can never reach, let alone clobber, the new buffer.
        if self.state.buffers.try_get(bid).is_none() {
            return;
        }

        // Discard if syntax was detached while the request was in flight.
        let Some(buf_syntax) = self.state.buffers.get(bid).syntax.as_ref() else {
            return;
        };

        // Discard if the grammar was swapped between enqueue and arrival.
        if !Arc::ptr_eq(lang, &buf_syntax.lang) {
            return;
        }

        // Discard if the text moved on since this request was submitted.
        if text_gen != self.state.buffers.get(bid).text_gen {
            return;
        }

        match outcome {
            ParseOutcome::Ok(parsed) => {
                let root_highlighter = Arc::clone(
                    &lang
                        .grammar
                        .as_ref()
                        .expect("grammar.is_some() verified at setup_buffer_syntax")
                        .highlighter,
                );
                let mut layer_langs = Vec::with_capacity(parsed.injected.len());
                let mut layers = Vec::with_capacity(1 + parsed.injected.len());
                layers.push(SyntaxLayer {
                    tree: parsed.root,
                    highlighter: root_highlighter,
                    ranges: Vec::new(),
                    depth: 0,
                });
                for injected in parsed.injected {
                    // Defensive: the bundle backing an injected layer's
                    // language could vanish between the worker resolving it
                    // and this install (e.g. a concurrent grammar removal).
                    let Some(bundle) = injected.lang.grammar.as_ref() else {
                        continue;
                    };
                    layer_langs.push(Arc::clone(&injected.lang));
                    layers.push(SyntaxLayer {
                        tree: injected.tree,
                        highlighter: Arc::clone(&bundle.highlighter),
                        ranges: injected.ranges,
                        depth: injected.depth,
                    });
                }
                self.view.buffers[bid].syntax = Some(SyntaxLayers { layers });
                // Drain pending edits baked into the installed tree, and
                // advance tree_gen to match the newly installed precise tree.
                if let Some(syn) = self.state.buffers.get_mut(bid).syntax.as_mut() {
                    syn.pending_edits.retain(|(g, _)| *g > text_gen);
                    syn.tree_gen = text_gen;
                    syn.layer_langs = layer_langs;
                }
            }
            ParseOutcome::ParseFailed => {
                // Advance parsed_gen so this generation is not retried every
                // frame.  The next edit will bump text_gen and trigger a fresh
                // attempt.  tree_gen is NOT advanced — the committed tree (if
                // any) still describes whatever generation it was baked to.
            }
        }

        self.state
            .buffers
            .get_mut(bid)
            .syntax
            .as_mut()
            .expect("syntax.is_some() guaranteed: checked above before install")
            .parsed_gen = text_gen;
    }

    /// Reparse any visible buffer whose text has changed since the last parse.
    ///
    /// Called from `prepare_frame` before `update_highlight_providers`. Detaches
    /// syntax from a buffer that has grown past `syntax-highlight-max-bytes`.
    ///
    /// Non-blocking: drains any completed backend results, then for each stale
    /// buffer bakes pending edits into the committed tree (keeping the renderer
    /// coordinate-aligned every frame) and submits an incremental reparse request.
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
                self.view.buffers[bid].syntax = None;
                self.state.buffers.get_mut(bid).syntax = None;
                self.parse_worker.remove_in_flight(bid);
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
                        .as_deref()
                        .and_then(|l| self.state.languages.by_name(l))
                        .is_some_and(|c| c.grammar.is_some())
                {
                    self.setup_buffer_syntax(bid);
                }
                continue;
            }

            // Gen-gate: skip if already up to date.
            // buf.syntax.is_some() is guaranteed — the is_none branch above continues.
            let parsed_gen = buf
                .syntax
                .as_ref()
                .expect("syntax is_none handled above")
                .parsed_gen;
            if parsed_gen == text_gen {
                continue;
            }

            // Bake any pending edits into the committed tree so the renderer
            // stays coordinate-aligned with the live text every frame, even
            // while a background reparse is in flight.
            self.bake_pending_edits(bid, text_gen);

            // In-flight check: skip if we already have a pending request for
            // the current text_gen.
            if self.parse_worker.is_in_flight(bid, text_gen) {
                continue;
            }

            // Build old_tree for incremental re-parsing.  The committed tree has
            // been baked to text_gen (when the edit chain was intact and a tree
            // existed), so clone it directly.  Falls back to None (full reparse)
            // when no tree exists yet or the chain was broken.
            let old_tree: Option<tree_sitter::Tree> = {
                let tree_gen = self
                    .state
                    .buffers
                    .get(bid)
                    .syntax
                    .as_ref()
                    .expect("syntax is_some guaranteed above")
                    .tree_gen;
                if tree_gen == text_gen {
                    self.view.buffers[bid]
                        .syntax
                        .as_ref()
                        .and_then(SyntaxLayers::root_tree)
                        .cloned()
                } else {
                    None
                }
            };

            let text = self.state.buffers.get(bid).text().clone();
            let lang = Arc::clone(
                &self
                    .state
                    .buffers
                    .get(bid)
                    .syntax
                    .as_ref()
                    .expect("syntax is_some guaranteed above")
                    .lang,
            );
            self.parse_worker.post(ParseRequest {
                bid,
                text_gen,
                lang,
                text,
                old_tree,
                langs: self.state.languages.grammar_snapshot(),
            });
        }
    }

    /// Bake `pending_edits` from `buf.syntax` into the committed `sbuf.tree`.
    ///
    /// Applies each recorded `InputEdit` in order so the tree's byte coordinates
    /// match the live rope before render.  No-op when there is no syntax
    /// attachment, no committed tree, or no pending edits.
    ///
    /// On a complete chain (edits contiguous from `tree_gen + 1` to `text_gen`):
    /// edits are applied in-place, `tree_gen` is advanced, and `pending_edits` cleared.
    ///
    /// On a chain break (a text mutation bypassed `doc_ops`): `pending_edits` are
    /// cleared and `tree_gen` is left unchanged; the caller then posts a full
    /// reparse (`old_tree = None`).
    fn bake_pending_edits(&mut self, bid: BufferId, text_gen: u64) {
        let Some(buf_syntax) = self.state.buffers.get(bid).syntax.as_ref() else {
            return;
        };
        let tree_gen = buf_syntax.tree_gen;
        let has_pending = !buf_syntax.pending_edits.is_empty();

        if !has_pending || self.view.buffers[bid].syntax.is_none() {
            return;
        }

        let pending = &self
            .state
            .buffers
            .get(bid)
            .syntax
            .as_ref()
            .expect("syntax checked above")
            .pending_edits;
        // A gap of 2+ between consecutive gens means a mutation bypassed
        // `record_pending_edits` (bumped text_gen without recording an
        // InputEdit) between two recorded edits — the endpoints alone would
        // pass, baking an incomplete edit set into the tree.
        let chain_ok = pending[0].0 == tree_gen + 1
            && pending.last().unwrap().0 == text_gen
            && pending.windows(2).all(|w| w[1].0 - w[0].0 <= 1);

        if chain_ok {
            let edits: Vec<_> = pending.iter().map(|(_, e)| *e).collect();
            // Edits apply to every layer's tree — `InputEdit` coordinates are
            // absolute buffer bytes, valid for the root layer and any
            // injected layer alike.
            let layers = &mut self.view.buffers[bid]
                .syntax
                .as_mut()
                .expect("syntax checked above")
                .layers;
            for layer in layers.iter_mut() {
                for edit in &edits {
                    layer.tree.edit(edit);
                }
                // The tree's own included ranges shift in-place on `edit()`,
                // but `layer.ranges` is a separate cached copy (consulted by
                // `layer_covers_line` for line-intersection tests) that must
                // be refreshed to match. The root layer's ranges are always
                // empty (whole-buffer) and need no refresh.
                if layer.depth > 0 {
                    layer.ranges = layer.tree.included_ranges();
                }
            }
            let syn = self
                .state
                .buffers
                .get_mut(bid)
                .syntax
                .as_mut()
                .expect("syntax checked above");
            syn.tree_gen = text_gen;
            syn.pending_edits.clear();
        } else {
            // pending_edits chain is broken: a text mutation bumped text_gen
            // without recording an InputEdit (e.g. set_view_content on a buffer
            // that unexpectedly had a syntax attachment).  Clear pending edits so
            // the caller's old_tree == None path posts a full reparse.
            self.report(
                Severity::Trace,
                format!(
                    "bake_pending_edits: chain broken for {bid:?} — \
                     tree_gen={tree_gen}, text_gen={text_gen}, \
                     first={:?}, last={:?}; full reparse triggered",
                    pending.first().map(|(g, _)| *g),
                    pending.last().map(|(g, _)| *g),
                ),
            );
            self.state
                .buffers
                .get_mut(bid)
                .syntax
                .as_mut()
                .expect("syntax checked above")
                .pending_edits
                .clear();
        }
    }

    fn surface_parse_worker_disconnect(&mut self) {
        if !self.parse_worker_disconnect_logged {
            use super::Severity;
            self.state.message_log.push(
                Severity::Error,
                "parse worker disconnected — syntax highlighting suspended".to_owned(),
            );
            self.parse_worker_disconnect_logged = true;
        }
    }

    /// Called when one or more grammars are attached. Re-runs `setup_buffer_syntax`
    /// on every open buffer whose language is in `names`.
    /// Re-runs `setup_buffer_syntax` on every open buffer whose language is
    /// in `names`, **or** whose currently-attached root grammar has an
    /// injections query — a newly attached grammar (e.g. rust) may complete
    /// injection sites in an already-open buffer of a different language
    /// (e.g. markdown fenced code blocks) without that buffer's own language
    /// ever appearing in `names`.
    pub(super) fn sweep_buffers_for_grammars(&mut self, names: Vec<String>) {
        if names.is_empty() {
            return;
        }
        let bids: Vec<BufferId> = self
            .state
            .buffers
            .iter()
            .filter(|(_, buf)| {
                let matches_name = buf
                    .language
                    .as_deref()
                    .is_some_and(|lang| names.iter().any(|n| n == lang));
                let root_has_injections = buf.syntax.as_ref().is_some_and(|syn| {
                    syn.lang
                        .grammar
                        .as_ref()
                        .is_some_and(|b| b.injections.is_some())
                });
                matches_name || root_has_injections
            })
            .map(|(bid, _)| bid)
            .collect();
        for bid in bids {
            self.setup_buffer_syntax(bid);
        }
    }
}

// ── Unit tests for input_edits_from_changeset ─────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{input_edits_from_changeset, new_end_point};
    use hume_editing::changeset::ChangeSetBuilder;

    // ── bake_pending_edits chain-contiguity ──────────────────────────────────

    fn zero_edit() -> tree_sitter::InputEdit {
        tree_sitter::InputEdit {
            start_byte: 0,
            old_end_byte: 0,
            new_end_byte: 0,
            start_position: tree_sitter::Point { row: 0, column: 0 },
            old_end_position: tree_sitter::Point { row: 0, column: 0 },
            new_end_position: tree_sitter::Point { row: 0, column: 0 },
        }
    }

    /// Set up a JSON-attached editor with the initial parse installed and
    /// `pending_edits` empty. Returns the editor, buffer id, and the
    /// post-initial-parse generation. Skips (returns `None`) if the JSON
    /// grammar fixture has not been fetched.
    fn baked_json_editor() -> Option<(crate::editor::Editor, hume_engine::pipeline::BufferId, u64)>
    {
        use crate::editor::Editor;
        use crate::editor::buffer::Buffer;
        use crate::editor::tests::{grammar_parser_path, grammar_query_path};
        use hume_editing::selection::SelectionSet;
        use hume_editing::text::Text;

        let parser_path = grammar_parser_path("json");
        if !parser_path.exists() {
            return None;
        }
        let hl_path = grammar_query_path("json");
        let buf = Buffer::new(Text::from("{}\n"), SelectionSet::default());
        let mut ed = Editor::for_testing(buf);
        let bid = ed.focused_buffer_id();
        ed.state
            .languages
            .attach_grammar(
                "json",
                &parser_path,
                "tree_sitter_json",
                &hl_path,
                None,
                &mut ed.view.registry,
            )
            .expect("attach json grammar");
        ed.set_buffer_language(bid, Some("json".to_owned()));
        ed.reparse_stale_buffers(); // drains the initial full parse
        let gen0 = ed.state.buffers.get(bid).text_gen;
        Some((ed, bid, gen0))
    }

    /// A mid-chain gap (a mutation bumped `text_gen` without recording an
    /// `InputEdit` between two recorded edits) must be rejected even though
    /// the chain's endpoints match `tree_gen + 1 ..= text_gen`. Pre-fix, the
    /// endpoint-only check would bake an incomplete edit set into the tree
    /// and advance `tree_gen` as if the tree were correct.
    #[test]
    fn bake_pending_edits_rejects_mid_chain_gap() {
        let Some((mut ed, bid, gen0)) = baked_json_editor() else {
            return; // fixture not fetched — scripts/fetch-test-grammars.sh
        };

        // Fabricate a broken chain: recorded gens gen0+1 and gen0+3 (a gap at
        // gen0+2), with matching endpoints against tree_gen(=gen0)+1..=text_gen.
        {
            let syn = ed.state.buffers.get_mut(bid).syntax.as_mut().unwrap();
            syn.pending_edits = vec![(gen0 + 1, zero_edit()), (gen0 + 3, zero_edit())];
        }
        ed.state.buffers.get_mut(bid).text_gen = gen0 + 3;

        ed.bake_pending_edits(bid, gen0 + 3);

        let syn = ed.state.buffers.get(bid).syntax.as_ref().unwrap();
        assert_eq!(
            syn.tree_gen, gen0,
            "gapped chain must be rejected — tree_gen must NOT advance"
        );
        assert!(
            syn.pending_edits.is_empty(),
            "broken chain must still clear pending_edits so the caller falls back to a full reparse"
        );
    }

    /// Flip of the above: a genuinely contiguous chain (no gap) must still
    /// bake and advance `tree_gen` as before.
    #[test]
    fn bake_pending_edits_accepts_contiguous_chain() {
        let Some((mut ed, bid, gen0)) = baked_json_editor() else {
            return; // fixture not fetched
        };

        {
            let syn = ed.state.buffers.get_mut(bid).syntax.as_mut().unwrap();
            syn.pending_edits = vec![(gen0 + 1, zero_edit()), (gen0 + 2, zero_edit())];
        }
        ed.state.buffers.get_mut(bid).text_gen = gen0 + 2;

        ed.bake_pending_edits(bid, gen0 + 2);

        let syn = ed.state.buffers.get(bid).syntax.as_ref().unwrap();
        assert_eq!(
            syn.tree_gen,
            gen0 + 2,
            "contiguous chain must still bake and advance tree_gen"
        );
        assert!(syn.pending_edits.is_empty());
    }

    #[test]
    fn pure_insert_at_start() {
        let rope = ropey::Rope::from_str("hello\n");
        let mut b = ChangeSetBuilder::new(rope.len_chars());
        b.insert("AB");
        b.retain_rest();
        let cs = b.finish();

        let edits = input_edits_from_changeset(&cs, &rope);
        assert_eq!(edits.len(), 1);
        let e = &edits[0];
        assert_eq!(e.start_byte, 0);
        assert_eq!(e.old_end_byte, 0);
        assert_eq!(e.new_end_byte, 2);
        assert_eq!(e.start_position, tree_sitter::Point { row: 0, column: 0 });
        assert_eq!(e.old_end_position, tree_sitter::Point { row: 0, column: 0 });
        assert_eq!(e.new_end_position, tree_sitter::Point { row: 0, column: 2 });
    }

    #[test]
    fn pure_insert_middle() {
        let rope = ropey::Rope::from_str("hello\n");
        let mut b = ChangeSetBuilder::new(rope.len_chars());
        b.retain(3);
        b.insert("XY");
        b.retain_rest();
        let cs = b.finish();

        let edits = input_edits_from_changeset(&cs, &rope);
        assert_eq!(edits.len(), 1);
        let e = &edits[0];
        assert_eq!(e.start_byte, 3);
        assert_eq!(e.old_end_byte, 3);
        assert_eq!(e.new_end_byte, 5);
        assert_eq!(e.new_end_position.column, 5);
    }

    #[test]
    fn pure_delete_single_char() {
        let rope = ropey::Rope::from_str("abc\n");
        let mut b = ChangeSetBuilder::new(rope.len_chars());
        b.retain(1);
        b.delete(1);
        b.retain_rest();
        let cs = b.finish();

        let edits = input_edits_from_changeset(&cs, &rope);
        assert_eq!(edits.len(), 1);
        let e = &edits[0];
        assert_eq!(e.start_byte, 1);
        assert_eq!(e.old_end_byte, 2);
        assert_eq!(e.new_end_byte, 1);
    }

    #[test]
    fn delete_crosses_line_boundary() {
        let rope = ropey::Rope::from_str("foo\nbar\n");
        let mut b = ChangeSetBuilder::new(rope.len_chars());
        b.retain(3);
        b.delete(3); // deletes "\nba"
        b.retain_rest();
        let cs = b.finish();

        let edits = input_edits_from_changeset(&cs, &rope);
        assert_eq!(edits.len(), 1);
        let e = &edits[0];
        assert_eq!(e.start_byte, 3);
        assert_eq!(e.old_end_byte, 6);
        assert_eq!(e.new_end_byte, 3);
        assert_eq!(e.old_end_position, tree_sitter::Point { row: 1, column: 2 });
    }

    #[test]
    fn replace_within_one_line() {
        let rope = ropey::Rope::from_str("hello world\n");
        let mut b = ChangeSetBuilder::new(rope.len_chars());
        b.retain(6);
        b.delete(5);
        b.insert("Rust");
        b.retain_rest();
        let cs = b.finish();

        let edits = input_edits_from_changeset(&cs, &rope);
        assert_eq!(edits.len(), 1);
        let e = &edits[0];
        assert_eq!(e.start_byte, 6);
        assert_eq!(e.old_end_byte, 11);
        assert_eq!(e.new_end_byte, 10);
        assert_eq!(e.new_end_position.column, 10);
    }

    #[test]
    fn multiline_insert_new_end_position() {
        let rope = ropey::Rope::from_str("ab\n");
        let mut b = ChangeSetBuilder::new(rope.len_chars());
        b.retain(1);
        b.insert("foo\nbar\n");
        b.retain_rest();
        let cs = b.finish();

        let edits = input_edits_from_changeset(&cs, &rope);
        assert_eq!(edits.len(), 1);
        let e = &edits[0];
        // "foo\nbar\n" — 2 newlines; last '\n' at byte 7; col = 8 - 7 - 1 = 0
        assert_eq!(e.new_end_position.row, 2);
        assert_eq!(e.new_end_position.column, 0);
    }

    #[test]
    fn two_separate_edit_sites_emit_two_edits() {
        let rope = ropey::Rope::from_str("abc\n");
        let mut b = ChangeSetBuilder::new(rope.len_chars());
        b.delete(1); // delete 'a'
        b.retain(1); // keep 'b'
        b.delete(1); // delete 'c'
        b.retain_rest();
        let cs = b.finish();

        let edits = input_edits_from_changeset(&cs, &rope);
        assert_eq!(edits.len(), 2);
        // Edits are returned in DESCENDING start-byte order so callers that apply
        // them via `tree.edit()` (which mutates coordinates in-place) apply the
        // rightmost edit first — keeping all original-coordinate offsets valid.
        assert!(
            edits[0].start_byte > edits[1].start_byte,
            "edits must be in descending start-byte order for correct tree.edit() baking"
        );
        assert_eq!(edits[0].start_byte, 2);
        assert_eq!(edits[0].old_end_byte, 3);
        assert_eq!(edits[1].start_byte, 0);
        assert_eq!(edits[1].old_end_byte, 1);
    }

    #[test]
    fn multibyte_utf8_byte_offsets() {
        // "é" = U+00E9 (precomposed) = 2 bytes in UTF-8, 1 char.
        // "漢" = U+6F22 = 3 bytes, 1 char.
        let rope = ropey::Rope::from_str("é漢\n");
        let mut b = ChangeSetBuilder::new(rope.len_chars());
        b.delete(1); // delete "é" (1 char, but 2 bytes)
        b.retain_rest();
        let cs = b.finish();

        let edits = input_edits_from_changeset(&cs, &rope);
        assert_eq!(edits.len(), 1);
        let e = &edits[0];
        assert_eq!(e.start_byte, 0);
        // "é" (U+00E9) = 2 bytes — different from char count of 1.
        assert_eq!(e.old_end_byte, 2, "byte offset must count bytes not chars");
        assert_eq!(e.new_end_byte, 0);
    }

    #[test]
    fn new_end_point_no_newlines() {
        let (row, col) = new_end_point(2, 5, "hello");
        assert_eq!(row, 2);
        assert_eq!(col, 10); // 5 + 5
    }

    #[test]
    fn new_end_point_with_newlines() {
        let (row, col) = new_end_point(1, 3, "foo\nbar\nbaz");
        // 2 newlines → row + 2 = 3; col = "baz".len() = 3
        assert_eq!(row, 3);
        assert_eq!(col, 3);
    }

    #[test]
    fn new_end_point_trailing_newline() {
        // Inserted text ends with '\n' — col must be 0.
        let (row, col) = new_end_point(0, 0, "foo\n");
        assert_eq!(row, 1);
        assert_eq!(col, 0);
    }

    /// Regression: a single changeset with edits at two non-adjacent positions must
    /// produce an incremental parse tree identical to a full reparse of the same bytes.
    ///
    /// The fix: `input_edits_from_changeset` returns edits in DESCENDING start-byte
    /// order.  `tree.edit()` mutates coordinates in-place, so the rightmost edit must
    /// be applied first — its original-coordinate bytes stay valid because nothing to
    /// its left has been touched yet.  Before the fix (ascending order), a left edit's
    /// byte-delta corrupted the right edit's coordinates, misaligning nodes and causing
    /// highlight queries to return wrong results after multi-cursor edits.
    ///
    /// Uses the JSON grammar from `tests/fixtures/grammars/` (requires
    /// `scripts/fetch-test-grammars.sh`).
    #[test]
    fn multi_edit_changeset_incremental_tree_matches_full_reparse() {
        use hume_engine::grammar::LoadedGrammar;

        let parser_path = crate::editor::tests::grammar_parser_path("json");
        if !parser_path.exists() {
            // Grammar fixture not fetched — skip rather than fail CI unexpectedly.
            return;
        }

        let grammar =
            LoadedGrammar::open(&parser_path, "tree_sitter_json").expect("load json grammar");
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(grammar.language())
            .expect("set language");

        // Old text: JSON array.  Two edits: replace "abc" (chars 2-4) with "X" and
        // replace "def" (chars 8-10) with "YY".  Different byte deltas at each site
        // so the left edit's shift would corrupt the right edit's coordinates if
        // applied in the wrong order.
        //
        // Chars: [ " a b c " , " d e f " ]  \n
        //         0  1 2 3 4 5 6  7 8 9 10 11 12 13
        let old_text = "[\"abc\",\"def\"]\n";
        let rope = ropey::Rope::from_str(old_text);

        let old_bytes: Vec<u8> = old_text.bytes().collect();
        let old_tree = parser.parse(&old_bytes, None).expect("initial parse");

        // Changeset: retain 2, delete 3 + insert "X", retain 3, delete 3 + insert "YY", retain rest.
        // Edit 1: chars [2,5) → "X"   (byte delta: 1 - 3 = -2)
        // Edit 2: chars [8,11) → "YY" (byte delta: 2 - 3 = -1)
        let mut b = ChangeSetBuilder::new(rope.len_chars());
        b.retain(2);
        b.delete(3);
        b.insert("X");
        b.retain(3);
        b.delete(3);
        b.insert("YY");
        b.retain_rest();
        let cs = b.finish();

        // Verify edits come out in descending order (right before left).
        let edits = input_edits_from_changeset(&cs, &rope);
        assert_eq!(edits.len(), 2, "expected two edits from the changeset");
        assert!(
            edits[0].start_byte > edits[1].start_byte,
            "edits must be descending: right edit first, then left"
        );

        // Apply edits and do an incremental reparse.
        let mut baked_tree = old_tree;
        for edit in &edits {
            baked_tree.edit(edit);
        }
        let new_text = "[\"X\",\"YY\"]\n";
        let new_bytes: Vec<u8> = new_text.bytes().collect();
        let incremental_tree = parser
            .parse(&new_bytes, Some(&baked_tree))
            .expect("incremental parse");
        let full_tree = parser.parse(&new_bytes, None).expect("full parse");

        assert_eq!(
            incremental_tree.root_node().to_sexp(),
            full_tree.root_node().to_sexp(),
            "incremental tree from multi-edit changeset must match full parse"
        );
    }
}
