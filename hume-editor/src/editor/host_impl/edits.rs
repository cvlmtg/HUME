//! `EditorHostImpl`'s LSP-driven text edits, workspace edits, and
//! go-to-location.

use hume_engine::pipeline::BufferId;
use hume_rope::column::CharCol;

use super::EditorHostImpl;
use hume_scripting::host::EditHost;

impl<'a> EditHost for EditorHostImpl<'a> {
    // ── Edit + navigation primitives ────────────────────────────────────
    fn apply_text_edits(
        &mut self,
        bid: BufferId,
        edits: Vec<hume_scripting::host::WireTextEdit>,
        expect_gen: Option<u64>,
    ) -> Result<(), String> {
        let Some(lsp) = self.lsp.as_deref() else {
            return Err("apply-text-edits!: no LSP state available".to_string());
        };
        let mut typed_edits = Vec::with_capacity(edits.len());
        for edit in edits {
            typed_edits.push(lsp_types::TextEdit {
                // Untrusted plugin input, not an internal invariant — a
                // position that doesn't fit `u32` is a malformed edit,
                // reported as an error, never a panic.
                range: hume_lsp::position::to_lsp_range(edit.range).ok_or_else(|| {
                    "apply-text-edits!: position exceeds u32 (malformed edit)".to_string()
                })?,
                new_text: edit.new_text,
            });
        }
        crate::editor::lsp::edits::apply_text_edits(self.state, lsp, bid, typed_edits, expect_gen)
    }

    fn apply_workspace_edit(&mut self, edit: serde_json::Value) -> Result<usize, String> {
        let Some(lsp) = self.lsp.as_deref() else {
            return Err("apply-workspace-edit!: no LSP state available".to_string());
        };
        let we: lsp_types::WorkspaceEdit =
            serde_json::from_value(edit).map_err(|e| format!("malformed WorkspaceEdit: {e}"))?;
        let summary =
            crate::editor::lsp::edits::apply_workspace_edit(self.state, self.view, lsp, we)?;
        Ok(summary.buffers_modified)
    }

    fn goto_location_value(&mut self, loc: serde_json::Value) -> Result<(), String> {
        let Some(lsp) = self.lsp.as_deref() else {
            return Err("goto-location!: no LSP state available".to_string());
        };
        let wl = hume_lsp::location::decode_location(&loc, "goto-location!")?;
        let target = crate::editor::lsp::edits::GotoTarget::Wire {
            uri: wl.uri,
            pos: wl.pos,
        };
        crate::editor::lsp::edits::goto_location(self.state, self.view, lsp, target)
    }

    fn goto_location_path(
        &mut self,
        path_or_uri: String,
        line: hume_rope::line::RopeyLine,
        char_col: usize,
    ) -> Result<(), String> {
        let Some(lsp) = self.lsp.as_deref() else {
            return Err("goto-location!: no LSP state available".to_string());
        };
        let target = crate::editor::lsp::edits::GotoTarget::Path {
            path_or_uri,
            line,
            char_col: CharCol::new(char_col),
        };
        crate::editor::lsp::edits::goto_location(self.state, self.view, lsp, target)
    }

    fn goto_location_buffer(
        &mut self,
        bid: BufferId,
        line: hume_rope::line::RopeyLine,
        char_col: usize,
    ) -> Result<(), String> {
        let Some(lsp) = self.lsp.as_deref() else {
            return Err("goto-location!: no LSP state available".to_string());
        };
        let target = crate::editor::lsp::edits::GotoTarget::Buffer {
            bid,
            line,
            char_col: CharCol::new(char_col),
        };
        crate::editor::lsp::edits::goto_location(self.state, self.view, lsp, target)
    }
}
