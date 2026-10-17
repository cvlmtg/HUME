//! The `Insert` layer and its key and paste handlers.
//! [`EditorState::apply_insert_mode_paste`] is a method rather than part of
//! the paste handler because it has a second caller: `replay.rs`'s
//! dot-repeat replay of a recorded paste.
//!
//! `handle_insert` itself only walks the Insert keymap and dispatches a
//! match; a key with no match runs its *default* behaviour via
//! `commands::insert_default_key`. See that function's own doc (`commands/
//! insert_keys.rs`) for why it's a free function rather than inlined here.

use termina::event::KeyEvent;

use hume_engine::pipeline::{BufferId, EngineView, PaneId};
use hume_engine::types::EditorMode;

use super::super::edit_session::InsertSession;
use super::super::keymap::WalkResult;
use super::super::replay::{Input, dispatch_insert_key};
use super::super::{Editor, EditorState, commands, doc_ops};
use super::paste::PastedText;
use super::popup::PopupLayer;
use super::stack::{InputEvent, InputStack, Layer, LayerHandler, LayerRef, PasteHandler, Removal};

pub(in crate::editor) struct InsertLayer {
    pub(in crate::editor) sticky_popup: Option<PopupLayer>,
    /// The open Insert session, which the layer's teardown commits: the
    /// layer and its session exist together.
    pub(super) session: InsertSession,
}

/// Proof that an Insert session is open, from its opening
/// ([`EditorState::push_insert`]) or from the stack
/// ([`InputStack::insert_entry`]): what each step of an Insert entry, and
/// each Insert key, takes, so none runs without a session to act on. Holds
/// no borrow, so the editor state stays usable between steps;
/// [`InputStack::entered`] reads the session back.
#[must_use]
pub(in crate::editor) struct InsertEntry(());

const ENTERED: &str = "an InsertEntry is minted only while its Insert session is open";

/// Named sugar over the generic lookup; `stack.rs` stays agnostic.
impl InputStack {
    /// The open Insert session, if any.
    pub(in crate::editor) fn insert_session(&self) -> Option<&InsertSession> {
        self.find::<InsertLayer>().map(|l| &l.session)
    }

    /// `true` if the open Insert session belongs to `(pane, buffer)`.
    pub(in crate::editor) fn inserting_into(&self, pane: PaneId, buffer: BufferId) -> bool {
        self.insert_session()
            .is_some_and(|s| s.owned_by(pane, buffer))
    }

    /// [`Self::insert_session`], mutably.
    pub(in crate::editor) fn insert_session_mut(&mut self) -> Option<&mut InsertSession> {
        self.find_mut::<InsertLayer>().map(|l| &mut l.session)
    }

    /// An [`InsertEntry`] for the open Insert session, if any.
    pub(in crate::editor) fn insert_entry(&self) -> Option<InsertEntry> {
        self.insert_session().map(|_| InsertEntry(()))
    }

    /// The Insert session `entry` proves open.
    pub(in crate::editor) fn entered(&self, _entry: &InsertEntry) -> &InsertSession {
        self.insert_session().expect(ENTERED)
    }

    /// [`Self::entered`], mutably.
    pub(in crate::editor) fn entered_mut(&mut self, _entry: &InsertEntry) -> &mut InsertSession {
        self.insert_session_mut().expect(ENTERED)
    }
}

impl EditorState {
    /// Push the Insert layer owning the just-opened `session`.
    pub(in crate::editor) fn push_insert(
        &mut self,
        view: &EngineView,
        session: InsertSession,
    ) -> InsertEntry {
        self.push_mode_layer(
            view,
            InsertLayer {
                sticky_popup: None,
                session,
            },
        );
        InsertEntry(())
    }
}

impl Layer for InsertLayer {
    fn snapshot_mut(&mut self) -> Option<&mut super::PaneSnapshot> {
        None
    }

    fn handler(&self) -> LayerHandler {
        insert_input
    }
    fn paste_handler(&self) -> PasteHandler {
        insert_paste
    }
    fn mode(&self) -> Option<EditorMode> {
        Some(EditorMode::Insert)
    }
    fn tear_down(self: Box<Self>, state: &mut EditorState, _view: &EngineView, _why: Removal) {
        commands::tear_down_insert(state, self.session);
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
        // A click or a wheel notch is Base's own action to run. Most
        // visibly, a click's `focus_pane` ends this very Insert session
        // before resolving the click (see `focus::focus_pane`'s doc).
        InputEvent::Mouse(mouse) => ed.fall_through(r, InputEvent::Mouse(mouse)),
    }
}

fn insert_paste(state: &mut EditorState, view: &EngineView, _r: LayerRef, text: PastedText) {
    state.apply_insert_mode_paste(view, text.as_str());
    state.record_paste(text.into_string());
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
            let (binding, scope) = ed
                .state
                .record_binding(cmd.name, reg_cmd.meta().repeatable)
                .unzip();
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
    let fp = commands::FocusedPane::current(&ed.view);
    let entry = ed
        .state
        .input
        .insert_entry()
        .expect("the Insert layer handling this key holds its session");
    if commands::insert_default_key(&mut ed.state, &ed.view, &entry, fp, key) {
        ed.state.record_input(Input::Key(key));
    }
}

impl EditorState {
    // ── Insert mode ───────────────────────────────────────────────────────────

    /// Bulk-insert `text` into the focused buffer as one grouped edit: the
    /// Insert-mode paste path. Also used by dot-repeat replay so a replayed
    /// paste re-runs as one edit rather than as synthesized per-char keys
    /// (which would wrongly re-trigger auto-indent on an embedded newline).
    ///
    /// Bypasses auto-pairs, trigger-char hooks, and per-char LSP
    /// refiltering: auto-pairing pasted brackets would corrupt already-balanced
    /// text, and refiltering a completion against a pasted blob is meaningless.
    pub(in crate::editor) fn apply_insert_mode_paste(&mut self, view: &EngineView, text: &str) {
        let focused = view.focused_pane();
        let buf = view.panes[focused].buffer_id();
        let (buffers, mut stores) = self.edit_parts();
        doc_ops::apply_doc_edit_grouped(buffers, &mut stores, focused, buf, |s| {
            hume_ops::edit::insert_str(s, text)
        });
        self.dismiss_completion(view);
    }
}
