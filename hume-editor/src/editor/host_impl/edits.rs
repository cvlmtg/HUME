//! `EditorHostImpl`'s LSP-driven text edits, workspace edits, and
//! go-to-location.

use hume_engine::pipeline::BufferId;
use hume_rope::column::CharCol;

use super::EditorHostImpl;
use crate::editor::commands::resolve_command_pane;
use hume_scripting::PaneHandle;
use hume_scripting::host::EditHost;

impl<'a> EditorHostImpl<'a> {
    fn resolve_edit_pane(
        &self,
        pane: PaneHandle,
    ) -> Result<crate::editor::commands::CommandPane, String> {
        resolve_command_pane(self.state, self.view, pane).map_err(|e| e.to_string())
    }
}

impl<'a> EditHost for EditorHostImpl<'a> {
    // ── Edit + navigation primitives ────────────────────────────────────
    fn apply_text_edits(
        &mut self,
        pane: PaneHandle,
        edits: Vec<hume_scripting::host::WireTextEdit>,
        expect_gen: Option<u64>,
    ) -> Result<(), String> {
        let t = self.resolve_edit_pane(pane)?;
        if self.lsp.is_none() {
            return Err("apply-text-edits!: no LSP state available".to_string());
        }
        // Each entry decoded its own encoding from its own JsonHandle (it
        // may have come from a different response than its batch-mates,
        // e.g. two `additionalTextEdits` merged by a plugin) — a batch that
        // disagrees has no single encoding to convert wire positions with.
        let mut encoding = None;
        let mut typed_edits = Vec::with_capacity(edits.len());
        for edit in edits {
            match encoding {
                None => encoding = Some(edit.encoding),
                Some(enc) if enc != edit.encoding => {
                    return Err(
                        "apply-text-edits!: edits use inconsistent server encodings".to_string()
                    );
                }
                _ => {}
            }
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
        let Some(encoding) = encoding else {
            return Err("apply-text-edits!: no edits given".to_string());
        };
        crate::editor::lsp::edits::apply_text_edits(
            self.state,
            t.pid(),
            t.bid(self.view),
            typed_edits,
            expect_gen,
            encoding,
        )
    }

    fn apply_workspace_edit(
        &mut self,
        pane: PaneHandle,
        edit: &serde_json::Value,
        encoding: hume_rope::position_encoding::PositionEncoding,
    ) -> Result<usize, String> {
        let t = self.resolve_edit_pane(pane)?;
        if self.lsp.is_none() {
            return Err("apply-workspace-edit!: no LSP state available".to_string());
        }
        // Deserializes from the `&Value` reference (serde_json implements
        // `Deserializer` for `&Value` as well as `Value`) rather than
        // `serde_json::from_value`, which needs ownership — the caller's
        // `JsonHandle`/hashmap argument is read here, never consumed.
        let we: lsp_types::WorkspaceEdit = serde::Deserialize::deserialize(edit)
            .map_err(|e: serde_json::Error| format!("malformed WorkspaceEdit: {e}"))?;
        let summary = crate::editor::lsp::edits::apply_workspace_edit(
            self.state,
            self.view,
            t.pid(),
            we,
            encoding,
        )?;
        Ok(summary.buffers_modified)
    }

    fn goto_location_value(
        &mut self,
        pane: PaneHandle,
        loc: &serde_json::Value,
        encoding: hume_rope::position_encoding::PositionEncoding,
    ) -> Result<(), String> {
        let t = self.resolve_edit_pane(pane)?;
        if self.lsp.is_none() {
            return Err("goto-location!: no LSP state available".to_string());
        }
        let wl = hume_lsp::location::decode_location(loc, "goto-location!")?;
        let target = crate::editor::lsp::edits::GotoTarget::Wire {
            uri: wl.uri,
            pos: wl.pos,
            encoding,
        };
        crate::editor::lsp::edits::goto_location(self.state, self.view, t, target)
    }

    fn goto_location_path(
        &mut self,
        pane: PaneHandle,
        path_or_uri: String,
        line: hume_rope::line::RopeyLine,
        char_col: usize,
    ) -> Result<(), String> {
        let t = self.resolve_edit_pane(pane)?;
        if self.lsp.is_none() {
            return Err("goto-location!: no LSP state available".to_string());
        }
        let target = crate::editor::lsp::edits::GotoTarget::Path {
            path_or_uri,
            line,
            char_col: CharCol::new(char_col),
        };
        crate::editor::lsp::edits::goto_location(self.state, self.view, t, target)
    }

    fn goto_location_buffer(
        &mut self,
        pane: PaneHandle,
        target: BufferId,
        line: hume_rope::line::RopeyLine,
        char_col: usize,
    ) -> Result<(), String> {
        let t = self.resolve_edit_pane(pane)?;
        if self.lsp.is_none() {
            return Err("goto-location!: no LSP state available".to_string());
        }
        let goto_target = crate::editor::lsp::edits::GotoTarget::Buffer {
            bid: target,
            line,
            char_col: CharCol::new(char_col),
        };
        crate::editor::lsp::edits::goto_location(self.state, self.view, t, goto_target)
    }
}
