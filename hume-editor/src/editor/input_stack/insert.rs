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

use super::super::dispatch::CmdCtx;
use super::super::keymap::WalkResult;
use super::super::replay::InsertInput;
use super::super::{Editor, EditorState, commands, doc_ops};
use super::popup::PopupLayer;
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, Removal};

/// The fixed dispatch context for a key that resolved to an Insert keymap
/// leaf: always an explicit count of 1, never extending. Shared by
/// `handle_insert` and `replay.rs`'s replay of a recorded
/// `InsertInput::Binding` entry, so a binding re-run on `.` sees exactly
/// the context it ran with the first time.
pub(in crate::editor) const INSERT_KEY_CTX: CmdCtx = CmdCtx {
    count: Some(1),
    extend: false,
};

pub(in crate::editor) struct InsertLayer {
    pub(in crate::editor) sticky_popup: Option<PopupLayer>,
}

impl Layer for InsertLayer {
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
            ed.state.record_insert_input(InsertInput::Paste(text));
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
/// input it was; see `InsertInput`'s own doc.
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
            // Wrapped in a dot-capture: replay re-runs this binding, so it
            // decides again at the new cursor, unless it turns out
            // interactive, in which case the capture's own net edit is
            // recorded instead (see `edit_session::DotCapture`'s own doc).
            // Through the full pipeline like any keypress: an edit composes
            // into the open insert-session group (`run_body` routes through
            // `apply_doc_edit_grouped`), a motion clears a pinned typed run
            // (`step_clear_typed_run`), and `repeat_slot_owned` keeps
            // a repeatable command from stamping over the session's owner.
            let fallback = InsertInput::Binding { name: cmd.name };
            ed.with_dot_capture(Some(fallback), |ed| {
                ed.with_insert_key_dispatch(|ed| ed.dispatch(reg_cmd, INSERT_KEY_CTX));
            });
            return;
        }
        WalkResult::NoMatch => {}
        // Interior / WaitChar can't arise in the insert trie (no multi-key
        // sequences, no wait-char bindings).
        WalkResult::Interior | WalkResult::WaitChar(_) => {}
    }

    // Recorded only when the key has default behaviour: an unbound key with
    // none (an unbound Ctrl-chord, an F-key) did nothing, so replaying it
    // would be a silent no-op entry.
    let fp = commands::FocusedPane::current(&ed.state);
    if commands::insert_default_key(&mut ed.state, &ed.view, fp, key) {
        ed.state.record_insert_input(InsertInput::Key(key));
    }
}

impl Editor {
    // ── Insert mode ───────────────────────────────────────────────────────────

    /// Runs `f` with `EditorState::in_insert_key_dispatch` set, restoring
    /// its prior value afterward. Shared by `handle_insert`'s trie-leaf
    /// dispatch and `replay.rs`'s replay of a recorded `InsertInput::
    /// Binding` entry, so a binding re-run on `.` sees the same flag it saw
    /// live. Set for the whole call regardless of what `f` does internally
    /// (mode switches included: dispatch is synchronous all the way
    /// through a Steel `call!`, so this never outlives the call it wraps).
    /// See `commands::repeat_slot_owned`'s own doc for what the flag
    /// itself gates.
    pub(in crate::editor) fn with_insert_key_dispatch<R>(
        &mut self,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let prev = std::mem::replace(&mut self.state.in_insert_key_dispatch, true);
        let r = f(self);
        self.state.in_insert_key_dispatch = prev;
        r
    }

    /// Bulk-insert `text` into the focused buffer as one grouped edit: the
    /// Insert-mode paste path. Also used by dot-repeat replay so a replayed
    /// paste re-runs as one edit rather than as synthesized per-char keys
    /// (which would wrongly re-trigger auto-indent on an embedded newline).
    ///
    /// Deliberately bypasses auto-pairs, trigger-char hooks, and per-char LSP
    /// refiltering: auto-pairing pasted brackets would corrupt already-balanced
    /// text, and refiltering a completion against a pasted blob is meaningless.
    pub(in crate::editor) fn apply_insert_mode_paste(&mut self, text: &str) {
        let focused = self.state.focus.id();
        let buf = self.focused_buffer_id();
        doc_ops::apply_doc_edit_grouped(
            &mut self.state.buffers,
            &self.state.config.decorations,
            &mut crate::editor::position_stores::PositionStores::new(
                &mut self.state.panes,
                &mut self.state.input,
            ),
            &mut self.state.active_session,
            focused,
            buf,
            |s| hume_ops::edit::insert_str(s, text),
        );
        self.state.dismiss_completion(&self.view);
    }
}
