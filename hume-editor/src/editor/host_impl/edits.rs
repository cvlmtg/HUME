//! `EditorHostImpl`'s LSP-driven text edits, workspace edits,
//! go-to-location, and `insert-key!`.

use hume_engine::pipeline::BufferId;
use hume_rope::column::CharCol;
use termina::event::KeyEvent;

use super::EditorHostImpl;
use crate::editor::Mode;
use crate::editor::commands::{self, FocusedPane};
use hume_scripting::PaneHandle;
use hume_scripting::host::EditHost;

impl<'a> EditHost for EditorHostImpl<'a> {
    // ── Edit + navigation primitives ────────────────────────────────────
    fn apply_text_edits(
        &mut self,
        pane: PaneHandle,
        edits: Vec<hume_scripting::host::WireTextEdit>,
        expect_gen: Option<u64>,
    ) -> Result<(), String> {
        let t = self.command_pane(pane)?;
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
            &self.view.panes,
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
        expect_gen: Option<u64>,
    ) -> Result<usize, String> {
        let t = self.command_pane(pane)?;
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
            expect_gen,
        )?;
        Ok(summary.buffers_modified)
    }

    fn goto_location_value(
        &mut self,
        pane: PaneHandle,
        loc: &serde_json::Value,
        encoding: hume_rope::position_encoding::PositionEncoding,
    ) -> Result<(), String> {
        let t = self.command_pane(pane)?;
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
        let t = self.command_pane(pane)?;
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
        let t = self.command_pane(pane)?;
        let goto_target = crate::editor::lsp::edits::GotoTarget::Buffer {
            bid: target,
            line,
            char_col: CharCol::new(char_col),
        };
        crate::editor::lsp::edits::goto_location(self.state, self.view, t, goto_target)
    }

    fn insert_key(&mut self, pane: PaneHandle, key: KeyEvent) -> Result<(), String> {
        let fp = FocusedPane::resolve(self.state, self.view, pane).map_err(|e| e.to_string())?;
        if self.state.mode() != Mode::Insert {
            return Err("insert-key!: only in Insert mode".to_string());
        }
        // `in_insert_key_dispatch` (see its own doc) is exactly "an Insert
        // key's own keymap binding is dispatching right now" — the one
        // context `.` can replay this call in (by re-running that same
        // binding, per `handle_insert`'s own doc), so this must refuse
        // anywhere else (a hook, a timer): the buffer edit would otherwise
        // land untracked for dot-repeat.
        if !self.state.in_insert_key_dispatch {
            return Err("insert-key!: only from a command bound to an Insert-mode key".to_string());
        }
        // Not recorded here: `.` replays the *binding* that called this
        // (recorded once, at the keymap dispatch that's invoking it —
        // `handle_insert`'s own doc), which re-runs this same call on
        // replay. Recording it a second time here would double it.
        if !commands::insert_default_key(self.state, self.view, fp, key) {
            return Err(format!(
                "insert-key!: {:?} has no default Insert-mode behaviour",
                key.code
            ));
        }
        Ok(())
    }
}
