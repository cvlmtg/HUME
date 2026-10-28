//! `EditorHostImpl`'s LSP-driven text edits, workspace edits,
//! go-to-location, `insert-key!`, `paste-text!` and `edit-text!`.

use hume_core::{BufferId, NonEmpty, PaneHandle};
use hume_platform::path::AbsolutePath;
use hume_rope::column::CharCol;
use termina::event::KeyEvent;

use super::EditorHostImpl;
use crate::editor::buffer::offsets::{checked_char_range, widened_to_clusters};
use crate::editor::commands;
use crate::editor::input_stack::PastedText;
use hume_scripting::host::{CharTextEdit, EditHost, FocusedHandle, ShownHandle};

impl<'a> EditHost for EditorHostImpl<'a> {
    // ── Edit + navigation primitives ────────────────────────────────────
    fn apply_text_edits(
        &mut self,
        pane: PaneHandle,
        edits: Vec<hume_scripting::host::WireTextEdit>,
        expect_gen: Option<u64>,
    ) -> Result<(), String> {
        let Some(edits) = NonEmpty::new(edits) else {
            return Ok(());
        };
        // Each entry decoded its own encoding from its own JsonHandle (it
        // may have come from a different response than its batch-mates,
        // e.g. two `additionalTextEdits` merged by a plugin), since a batch that
        // disagrees has no single encoding to convert wire positions with.
        let encoding = edits.first().encoding;
        if edits.iter().any(|edit| edit.encoding != encoding) {
            return Err("edits use inconsistent server encodings".to_string());
        }
        crate::editor::lsp::edits::apply_text_edits(
            self.state,
            &self.view.panes,
            pane,
            &edits.map_ref(|e| (e.range, e.new_text.as_str())),
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
        // Deserializes from the `&Value` reference (serde_json implements
        // `Deserializer` for `&Value` as well as `Value`) rather than
        // `serde_json::from_value`, which needs ownership; the caller's
        // `JsonHandle`/hashmap argument is read here, never consumed.
        let we: lsp_types::WorkspaceEdit = serde::Deserialize::deserialize(edit)
            .map_err(|e: serde_json::Error| format!("malformed WorkspaceEdit: {e}"))?;
        let summary = crate::editor::lsp::edits::apply_workspace_edit(
            self.state, self.view, pane, we, encoding, expect_gen,
        )?;
        Ok(summary.buffers_modified)
    }

    fn goto_location_value(
        &mut self,
        pane: ShownHandle,
        loc: &serde_json::Value,
        encoding: hume_rope::position_encoding::PositionEncoding,
    ) -> Result<Option<PaneHandle>, String> {
        let t = self.shown(pane)?;
        let wl = hume_lsp::location::decode_location(loc)?;
        let target = crate::editor::lsp::edits::GotoTarget::Wire {
            uri: wl.uri,
            pos: wl.pos,
            encoding,
        };
        crate::editor::lsp::edits::goto_location(self.state, self.view, t, target)
    }

    fn goto_location_path(
        &mut self,
        pane: ShownHandle,
        path: AbsolutePath,
        line: usize,
        char_col: usize,
    ) -> Result<Option<PaneHandle>, String> {
        let t = self.shown(pane)?;
        let target = crate::editor::lsp::edits::GotoTarget::Path {
            path,
            line,
            char_col: CharCol::new(char_col),
        };
        crate::editor::lsp::edits::goto_location(self.state, self.view, t, target)
    }

    fn goto_location_buffer(
        &mut self,
        pane: ShownHandle,
        target: BufferId,
        char_offset: usize,
    ) -> Result<PaneHandle, String> {
        let t = self.shown(pane)?;
        crate::editor::lsp::edits::goto_buffer_offset(self.state, self.view, t, target, char_offset)
    }

    fn goto_revision(&mut self, pane: ShownHandle, revision: usize) -> Result<(), String> {
        let t = self.shown(pane)?;
        commands::goto_revision(self.state, self.view, t, revision)
            .map(|_| ())
            .map_err(|e| e.message().to_owned())
    }

    fn insert_key(&mut self, pane: FocusedHandle, key: KeyEvent) -> Result<(), String> {
        let fp = self.focused(pane)?;
        let entry = self
            .state
            .input
            .insert_entry()
            .ok_or("only in Insert mode")?;
        // An Insert key's own keymap binding dispatching is the one context
        // `.` can replay this call in (by re-running that same binding), so
        // this must refuse anywhere else (a hook, a timer): the buffer edit
        // would otherwise land untracked for dot-repeat.
        if !self.state.recording_scope.insert_key() {
            return Err("only from a command bound to an Insert-mode key".to_string());
        }
        // Not recorded here: `.` replays the *binding* that called this
        // (recorded once, at the keymap dispatch that's invoking it;
        // see `handle_insert`'s own doc), which re-runs this same call on
        // replay. Recording it a second time here would double it.
        if !commands::insert_default_key(self.state, self.view, &entry, fp, key) {
            return Err(format!(
                "{:?} has no default Insert-mode behaviour",
                key.code
            ));
        }
        Ok(())
    }

    fn paste_text(&mut self, pane: FocusedHandle, values: Vec<String>) -> Result<(), String> {
        self.focused(pane)?;
        match PastedText::new(values) {
            Some(text) => self
                .state
                .paste(self.view, text)
                .map_err(|e| e.message().to_owned()),
            None => Ok(()),
        }
    }

    fn edit_text(
        &mut self,
        pane: PaneHandle,
        edits: Vec<CharTextEdit>,
        expect_gen: Option<u64>,
    ) -> Result<(), String> {
        if edits.is_empty() {
            return Ok(());
        }
        let buf = crate::editor::lsp::edits::checked_buffer(self.state, pane.buffer(), expect_gen)?;
        let text = buf.text();
        let char_edits = edits
            .iter()
            .map(|e| {
                let range = checked_char_range(text, e.start, e.end)
                    .map_err(|err| format!("range {err}"))?;
                Ok((widened_to_clusters(text, range), e.text.as_str()))
            })
            .collect::<Result<Vec<_>, String>>()?;
        crate::editor::lsp::edits::commit_char_edits(self.state, &self.view.panes, pane, char_edits)
            .map(|_| ())
    }
}
