use std::sync::Arc;

use engine::builtins::tree_sitter_hl::TreeSitterHighlighter;
use engine::pipeline::BufferId;

use super::Editor;
use super::syntax::BufferParser;

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

        // Build BufferParser — fails only if set_language rejects the ABI.
        let mut buf_parser = match BufferParser::new(Arc::clone(&lang_config)) {
            Some(bp) => bp,
            None => return,
        };

        // Initial parse.
        let source_bytes = self.buffers.get(bid).text().to_string().into_bytes();
        let tree = buf_parser.parser.parse(&source_bytes, None);

        // Build highlighter from shared query (capture names already interned at
        // attach_grammar time — intern_runtime deduplicates).
        let query = Arc::clone(&lang_config.grammar.as_ref().unwrap().query);
        let highlighter =
            TreeSitterHighlighter::from_shared_query(query, &mut self.engine_view.registry, source_bytes);

        let text_gen = self.buffers.get(bid).text_gen;
        buf_parser.parsed_gen = text_gen;

        // Write both the engine SharedBuffer and the editor Buffer atomically.
        {
            let sbuf = &mut self.engine_view.buffers[bid];
            sbuf.tree = tree;
            sbuf.syntax = Some(Arc::new(highlighter));
        }
        self.buffers.get_mut(bid).parser = Some(buf_parser);
    }

    /// Reparse any visible buffer whose text has changed since the last parse.
    ///
    /// Called from `prepare_frame` before `update_highlight_providers`. Detaches
    /// syntax from a buffer that has grown past `syntax-highlight-max-bytes`.
    pub(super) fn reparse_stale_buffers(&mut self) {
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

            // Full reparse — owned source avoids borrow conflict with get_mut below.
            let source_bytes = self.buffers.get(bid).text().to_string().into_bytes();
            let tree = {
                let bp = self.buffers.get_mut(bid).parser.as_mut().unwrap();
                bp.parser.parse(&source_bytes, None)
            };
            {
                let sbuf = &mut self.engine_view.buffers[bid];
                sbuf.tree = tree;
                if let Some(hl) = sbuf.syntax.as_deref() {
                    hl.refresh_source(&source_bytes);
                }
            }
            self.buffers.get_mut(bid).parser.as_mut().unwrap().parsed_gen = text_gen;
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
