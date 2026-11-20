//! `EditorHostImpl`'s tree-sitter queries over a buffer's syntax tree.

use std::sync::Arc;

use hume_core::BufferId;
use hume_rope::line::ContentLine;
use hume_rope::offset::ExclusiveRange;
use hume_scripting::host::{CaptureEntry, Captures, SyntaxHost};
use hume_treesitter::captures::{Capture, CaptureQuery};
use hume_treesitter::registry::GrammarBundle;

use super::EditorHostImpl;
use crate::editor::syntax::{GrammarLoads, SyntaxUse, ensure_syntax_current};

impl<'a> SyntaxHost for EditorHostImpl<'a> {
    fn syntax_captures(
        &mut self,
        bid: BufferId,
        language: &str,
        query: &str,
        ranges: &[ExclusiveRange<ContentLine>],
    ) -> Result<Captures, String> {
        // Compiled before the blocking parse, so a bad query raises without one.
        let (bundle, query) = self.capture_query(language, query)?;
        // `args::LiveBuffer` already checked `bid`; `ensure_syntax_current`
        // panics on a buffer the store does not hold.
        if self.state.buffers.try_get(bid).is_none() {
            return Err("unknown buffer".to_owned());
        }
        ensure_syntax_current(
            self.state,
            &mut self.view.registry,
            bid,
            SyntaxUse::Captures(bundle.language),
        );
        let buf = self.state.buffers.get(bid);
        let text = buf.text();
        let mut matches = Vec::new();
        if let Some(layers) = buf.current_layers() {
            query.collect(layers, text.rope(), ranges, &mut matches);
        }
        Ok(captures_of(&query, matches))
    }
}

impl EditorHostImpl<'_> {
    /// `query` compiled against `language`'s grammar, loading it if needed,
    /// with that grammar. The error names the language.
    pub(super) fn capture_query(
        &mut self,
        language: &str,
        query: &str,
    ) -> Result<(Arc<GrammarBundle>, Arc<CaptureQuery>), String> {
        let bundle = GrammarLoads::new(&mut self.state.config.languages, &mut self.view.registry)
            .grammar_named(language)
            .ok_or_else(|| format!("no grammar for language '{language}'"))?;
        let query = bundle
            .capture_query(query)
            .map_err(|e| format!("'{language}' query: {e}"))?;
        Ok((bundle, query))
    }
}

/// `captures`, which `query` collected, as the host reports them.
pub(super) fn captures_of(query: &CaptureQuery, matches: Vec<Vec<Capture>>) -> Captures {
    Captures {
        names: query.names().to_vec(),
        matches: matches
            .into_iter()
            .map(|captures| {
                captures
                    .into_iter()
                    .map(|capture| CaptureEntry {
                        name: capture.name,
                        start_char: capture.range.start.index(),
                        end_char: capture.range.end.index(),
                    })
                    .collect()
            })
            .collect(),
    }
}
