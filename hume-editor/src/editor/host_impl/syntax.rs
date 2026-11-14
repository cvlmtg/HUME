//! `EditorHostImpl`'s tree-sitter queries over a buffer's syntax tree.

use hume_core::BufferId;
use hume_rope::line::ContentLine;
use hume_rope::offset::{ExclusiveRange, InclusiveRange};
use hume_scripting::host::{CaptureEntry, Captures, SyntaxHost};

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
        let bundle = GrammarLoads::new(&mut self.state.config.languages, &mut self.view.registry)
            .grammar_named(language)
            .ok_or_else(|| format!("no grammar for language '{language}'"))?;
        // Compiled before the blocking parse, so a bad query raises without one.
        let query = bundle
            .capture_query(query)
            .map_err(|e| format!("'{language}' query: {e}"))?;
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
        let mut captures = Vec::new();
        // An empty range captures nothing. A non-empty one ends past a real
        // line, so its last line is one before its end.
        let lines: Vec<_> = ranges
            .iter()
            .filter(|range| !range.is_empty())
            .map(|range| InclusiveRange::new(range.start, range.end.retreat_saturating(1)))
            .collect();
        if let Some(layers) = buf.current_layers() {
            query.collect(layers, text.rope(), &lines, &mut captures);
        }
        Ok(Captures {
            names: query.names().to_vec(),
            entries: captures
                .into_iter()
                .map(|capture| CaptureEntry {
                    name: capture.name,
                    start_char: capture.range.start.index(),
                    end_char: capture.range.end.index(),
                })
                .collect(),
        })
    }
}
