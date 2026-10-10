//! The `Insert` layer and its key handler. [`Editor::apply_insert_mode_paste`]
//! is kept as an `impl Editor` method rather than a free function like this
//! crate's other layer handlers because it has a second caller outside
//! dispatch: `replay.rs`'s dot-repeat replay of a recorded paste.
//!
//! `handle_insert` itself only walks the Insert keymap and dispatches a
//! match; a key with no match runs its *default* behaviour via
//! `commands::insert_default_key`. See that function's own doc (`commands/
//! insert_keys.rs`) for why it's a free function rather than inlined here.

use termina::event::KeyEvent;

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;

use super::super::keymap::WalkResult;
use super::super::replay::{Input, RecordingScope, dispatch_insert_key};
use super::super::{Editor, EditorState, commands, doc_ops};
use super::popup::PopupLayer;
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, Removal};

pub(in crate::editor) struct InsertLayer {
    pub(in crate::editor) sticky_popup: Option<PopupLayer>,
}

impl Layer for InsertLayer {
    fn snapshot_mut(&mut self) -> Option<&mut super::PaneSnapshot> {
        None
    }

    fn handler(&self) -> LayerHandler {
        insert_input
    }
    fn mode(&self) -> Option<EditorMode> {
        Some(EditorMode::Insert)
    }
    fn tear_down(&mut self, state: &mut EditorState, _view: &EngineView, _why: Removal) {
        commands::tear_down_insert(state);
    }
    fn sticky_popup_slot(&self) -> Option<&Option<PopupLayer>> {
        Some(&self.sticky_popup)
    }
    fn sticky_popup_slot_mut(&mut self) -> Option<&mut Option<PopupLayer>> {
        Some(&mut self.sticky_popup)
    }
}

pub(in crate::editor) fn insert_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    match ev {
        InputEvent::Key(key) => handle_insert(ed, key),
        InputEvent::Paste(text) => {
            ed.apply_insert_mode_paste(&text);
            ed.state.record_input(Input::Paste(text));
        }
        // A click or a wheel notch is Base's own action to run. Most
        // visibly, a click's `focus_pane` ends this very Insert session
        // before resolving the click (see `focus::focus_pane`'s doc).
        InputEvent::Mouse(mouse) => ed.fall_through(r, InputEvent::Mouse(mouse)),
    }
}

/// Walks the Insert keymap for `key`: a bound key re-dispatches its
/// command, an unbound one runs its default behaviour
/// (`commands::insert_default_key`). Either way, records the key as the
/// input it was: a binding by its name, so `.` re-runs the binding, an
/// unbound key as itself, so `.` re-runs its default behaviour.
fn handle_insert(ed: &mut Editor, key: KeyEvent) {
    // Only Esc, Ctrl-c, arrows, and user bindings live in the insert trie;
    // plain chars, Tab, Enter, Backspace, and Delete fall through to their
    // default behaviour below.
    let trie_result = ed.state.config.keymap.insert.walk(&[key]);
    match trie_result {
        WalkResult::Leaf(cmd) => {
            // A bound command edits through `commands::run_body`, never
            // `insert_default_key`, so it hands no `ChangeSet` back to an
            // open completion session. Dismissing that session is
            // `completion_input`'s job: it peeks this same trie walk before
            // falling through here, and retires its layer once this call
            // returns (see its own doc).
            let Some(reg_cmd) = ed.resolve_mappable(cmd.name.as_ref()) else {
                return;
            };
            // Recorded before it runs: replay re-runs this binding, so it
            // decides again at the new cursor. A repeatable command's inputs
            // record their answers in that entry; an input a
            // non-repeatable one opens ends the change and drops the entry
            // (`EditorState::waiting_input`). Wrapped in a dot-capture, so a
            // completion accept it makes replaces that entry with its own net
            // edit.
            // Through the full pipeline like any keypress: an edit composes
            // into the open insert-session group (`run_body` routes through
            // `apply_doc_edit_grouped`), a motion clears a pinned typed run
            // (`with_jump`), and `repeat_slot_owned` keeps a repeatable
            // command it dispatches from starting a change of its own.
            let scope = ed.state.record_binding(cmd.name, reg_cmd.meta().repeatable);
            let binding = scope.as_ref().and_then(RecordingScope::binding);
            ed.with_dot_capture(binding, |ed| {
                dispatch_insert_key(ed, scope, reg_cmd);
            });
            return;
        }
        WalkResult::NoMatch => {}
        // Interior can't arise in the insert trie (no multi-key sequences).
        WalkResult::Interior => {}
    }

    // Recorded only when the key has default behaviour: an unbound key with
    // none (an unbound Ctrl-chord, an F-key) did nothing, so replaying it
    // would be a silent no-op entry.
    let fp = commands::FocusedPane::current(&ed.state);
    if commands::insert_default_key(&mut ed.state, &ed.view, fp, key) {
        ed.state.record_input(Input::Key(key));
    }
}

impl Editor {
    // ── Insert mode ───────────────────────────────────────────────────────────

    /// Bulk-insert `text` into the focused buffer as one grouped edit: the
    /// Insert-mode paste path. Also used by dot-repeat replay so a replayed
    /// paste re-runs as one edit rather than as synthesized per-char keys
    /// (which would wrongly re-trigger auto-indent on an embedded newline).
    ///
    /// Bypasses auto-pairs, trigger-char hooks, and per-char LSP
    /// refiltering: auto-pairing pasted brackets would corrupt already-balanced
    /// text, and refiltering a completion against a pasted blob is meaningless.
    pub(in crate::editor) fn apply_insert_mode_paste(&mut self, text: &str) {
        let focused = self.state.focus.id();
        let buf = self.focused_buffer_id();
        doc_ops::apply_doc_edit_grouped(
            &mut self.state.buffers,
            &mut crate::editor::position_stores::PositionStores::new(
                &mut self.state.panes,
                &mut self.state.input,
                &mut self.state.buffer_positions,
                &mut self.state.config.decorations,
            ),
            &mut self.state.active_session,
            focused,
            buf,
            |s| hume_ops::edit::insert_str(s, text),
        );
        self.state.dismiss_completion(&self.view);
    }
}
