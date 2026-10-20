//! `EditorHostImpl`'s LSP-driven text edits, workspace edits,
//! go-to-location, `insert-key!`, `paste-text!` and `edit-text!`.

use hume_editing::text::BufferText;
use hume_engine::pipeline::BufferId;
use hume_rope::column::CharCol;
use hume_rope::offset::{CharOffset, ExclusiveRange};
use termina::event::KeyEvent;

use super::{EditorHostImpl, checked_char_range};
use crate::editor::commands::{self, FocusedPane};
use crate::editor::doc_ops::HistoryWalk;
use crate::editor::input_stack::PastedText;
use hume_scripting::PaneHandle;
use hume_scripting::host::{CharTextEdit, EditHost};

impl<'a> EditHost for EditorHostImpl<'a> {
    // ── Edit + navigation primitives ────────────────────────────────────
    fn apply_text_edits(
        &mut self,
        pane: PaneHandle,
        edits: Vec<hume_scripting::host::WireTextEdit>,
        expect_gen: Option<u64>,
    ) -> Result<(), String> {
        // Each entry decoded its own encoding from its own JsonHandle (it
        // may have come from a different response than its batch-mates,
        // e.g. two `additionalTextEdits` merged by a plugin), since a batch that
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
                // Untrusted plugin input, not an internal invariant: a
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
            pane.pane(),
            pane.buffer(),
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
        // Deserializes from the `&Value` reference (serde_json implements
        // `Deserializer` for `&Value` as well as `Value`) rather than
        // `serde_json::from_value`, which needs ownership; the caller's
        // `JsonHandle`/hashmap argument is read here, never consumed.
        let we: lsp_types::WorkspaceEdit = serde::Deserialize::deserialize(edit)
            .map_err(|e: serde_json::Error| format!("malformed WorkspaceEdit: {e}"))?;
        let summary = crate::editor::lsp::edits::apply_workspace_edit(
            self.state,
            self.view,
            pane.pane(),
            we,
            encoding,
            expect_gen.map(|g| (pane.buffer(), g)),
        )?;
        Ok(summary.buffers_modified)
    }

    fn goto_location_value(
        &mut self,
        pane: PaneHandle,
        loc: &serde_json::Value,
        encoding: hume_rope::position_encoding::PositionEncoding,
    ) -> Result<bool, String> {
        let t = self.command_pane(pane)?;
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
        pane: PaneHandle,
        path: String,
        line: usize,
        char_col: usize,
    ) -> Result<bool, String> {
        let t = self.command_pane(pane)?;
        let target = crate::editor::lsp::edits::GotoTarget::Path {
            path,
            line,
            char_col: CharCol::new(char_col),
        };
        crate::editor::lsp::edits::goto_location(self.state, self.view, t, target)
    }

    fn goto_location_buffer(
        &mut self,
        pane: PaneHandle,
        target: BufferId,
        char_offset: usize,
    ) -> Result<(), String> {
        let t = self.command_pane(pane)?;
        let goto_target = crate::editor::lsp::edits::GotoTarget::Buffer {
            bid: target,
            char_offset,
        };
        crate::editor::lsp::edits::goto_location(self.state, self.view, t, goto_target).map(|_| ())
    }

    fn goto_revision(&mut self, pane: PaneHandle, revision: usize) -> Result<(), String> {
        let t = self.command_pane(pane)?;
        match commands::goto_revision(self.state, self.view, t, revision)
            .map_err(|e| e.message().to_string())?
        {
            HistoryWalk::Took(_) => Ok(()),
            HistoryWalk::RefusedReadOnly => Err("goto-revision!: buffer is read-only".to_string()),
        }
    }

    fn insert_key(&mut self, pane: PaneHandle, key: KeyEvent) -> Result<(), String> {
        let fp = FocusedPane::resolve(self.state, self.view, pane).map_err(|e| e.to_string())?;
        let entry = self
            .state
            .input
            .insert_entry()
            .ok_or("insert-key!: only in Insert mode")?;
        // An Insert key's own keymap binding dispatching is the one context
        // `.` can replay this call in (by re-running that same binding), so
        // this must refuse anywhere else (a hook, a timer): the buffer edit
        // would otherwise land untracked for dot-repeat.
        if !self.state.recording_scope.insert_key() {
            return Err("insert-key!: only from a command bound to an Insert-mode key".to_string());
        }
        // Not recorded here: `.` replays the *binding* that called this
        // (recorded once, at the keymap dispatch that's invoking it;
        // see `handle_insert`'s own doc), which re-runs this same call on
        // replay. Recording it a second time here would double it.
        if !commands::insert_default_key(self.state, self.view, &entry, fp, key) {
            return Err(format!(
                "insert-key!: {:?} has no default Insert-mode behaviour",
                key.code
            ));
        }
        Ok(())
    }

    fn paste_text(&mut self, pane: PaneHandle, values: Vec<String>) -> Result<(), String> {
        FocusedPane::resolve(self.state, self.view, pane).map_err(|e| e.to_string())?;
        if let Some(text) = PastedText::new(values) {
            self.state.paste(self.view, text);
        }
        Ok(())
    }

    fn edit_text(&mut self, pane: PaneHandle, edits: Vec<CharTextEdit>) -> Result<(), String> {
        let fail = |e: String| format!("edit-text!: {e}");
        let target = self.command_pane(pane).map_err(fail)?;
        let (pid, bid) = (target.pid(), target.bid(self.view));
        let text = self.state.buffers.get(bid).text();
        let char_edits = edits
            .iter()
            .map(|e| Ok((char_range(text, e.start, e.end)?, e.text.as_str())))
            .collect::<Result<Vec<_>, String>>()
            .map_err(fail)?;
        crate::editor::lsp::edits::commit_char_edits(
            self.state,
            &self.view.panes,
            pid,
            bid,
            char_edits,
        )
        .map(|_| ())
        .map_err(fail)
    }
}

/// The chars `start..end` of `text` name, checked against it. An edge inside
/// a grapheme cluster widens the range to the whole cluster, and an empty
/// range inside one moves to the cluster's start, so no edit splits one.
fn char_range(
    text: &BufferText,
    start: usize,
    end: usize,
) -> Result<ExclusiveRange<CharOffset>, String> {
    let range = checked_char_range(text, start, end).map_err(|e| format!("range {e}"))?;
    if range.is_empty() {
        let at = if range.start == text.end() {
            range.start
        } else {
            text.snap(range.start).offset()
        };
        return Ok(ExclusiveRange::new(at, at));
    }
    Ok(text
        .covering(range)
        .expect("a non-empty range within the text covers a cluster")
        .chars())
}
