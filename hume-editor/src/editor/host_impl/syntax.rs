//! `EditorHostImpl`'s tree-sitter queries over a buffer's syntax tree.

use hume_core::BufferId;
use hume_rope::line::{ContentLine, RopeyLine};
use hume_rope::offset::ExclusiveRange;
use hume_scripting::host::{CaptureEntry, Captures, SyntaxHost};

use super::EditorHostImpl;
use crate::editor::syntax::{GrammarLoads, SyntaxUse, ensure_syntax_current};

impl<'a> SyntaxHost for EditorHostImpl<'a> {
    fn syntax_captures(
        &mut self,
        bid: BufferId,
        language: &str,
        query: &str,
        range: Option<ExclusiveRange<ContentLine>>,
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
        let rope = text.rope();
        // The exclusive end may be one past the last line, where
        // `line_to_char` yields the text's end.
        let bytes = range.map(|lines| {
            let char_to_byte = |line: ContentLine| {
                rope.char_to_byte(text.line_to_char(RopeyLine::from(line)).index())
            };
            char_to_byte(lines.start)..char_to_byte(lines.end)
        });
        let mut captures = Vec::new();
        if let Some(layers) = buf.current_layers() {
            query.collect(layers, rope, bytes, &mut captures);
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
