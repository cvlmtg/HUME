//! The `Prompt` layer — a Steel `(prompt! …)` session's own minibuffer mode.
//! Distinct from `Command` despite sharing the `:` look: no history, no
//! completion, no directory-descend special case — exactly one
//! `(callback text-or-#f)` call fires, on Confirm or on any cancel path.

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;

use super::super::minibuf::{self, MiniBuffer, MiniBufferEvent};
use super::super::{Editor, EditorState};
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef};

pub(in crate::editor) struct PromptLayer {
    pub(in crate::editor) minibuf: MiniBuffer,
    pub(in crate::editor) callback: steel::rvals::SteelVal,
}

impl Layer for PromptLayer {
    fn handler(&self) -> LayerHandler {
        prompt_input
    }
    fn mode(&self) -> Option<EditorMode> {
        // The engine has no `Prompt` variant, and today's `(prompt! …)`
        // session already runs as `Command` for every consumer outside this
        // crate.
        Some(EditorMode::Command)
    }
    /// Empty — see `InsertLayer::setup`'s doc.
    fn setup(&mut self, _state: &mut EditorState, _view: &EngineView) {}
    /// Fires the callback with `#f` — unlike every other minibuf-mode
    /// layer's `tear_down`, which never fires a Steel callback (the file
    /// header's "exactly one call fires" contract otherwise has no arm to
    /// rely on when this layer is removed incidentally: buried under a
    /// `Confirm`/`Picker` that a `close-*!`/Rust-internal retirement then
    /// truncates through, or replaced outright by `push_mode_layer`). Same
    /// shape as `PickerLayer::tear_down` — see its own doc. The explicit
    /// accept/cancel path (`finish_steel_prompt`) never runs this: it takes
    /// the layer *by value* via `EditorState::take_layer`, firing its own
    /// callback explicitly instead.
    fn tear_down(&mut self, state: &mut EditorState, _view: &EngineView) {
        state.history.begin_session_all();
        state.queue_steel_call(
            self.callback.clone(),
            vec![steel::rvals::SteelVal::BoolV(false)],
        );
    }
    fn minibuf(&self) -> Option<&MiniBuffer> {
        Some(&self.minibuf)
    }
    fn minibuf_mut(&mut self) -> Option<&mut MiniBuffer> {
        Some(&mut self.minibuf)
    }
}

pub(in crate::editor) fn prompt_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    if let Some(event) = minibuf::minibuf_input(ed, r, ev) {
        handle_steel_prompt_event(ed, r, event);
    }
}

/// Routes a `Prompt` layer's key for a Steel `(prompt! …)` session
/// rather than a `:` command line — no history, no completion, no
/// directory-descend special case. Exactly one `(callback text-or-#f)`
/// call fires, on Confirm or on any of the cancel paths.
fn handle_steel_prompt_event(ed: &mut Editor, r: LayerRef, event: MiniBufferEvent) {
    match event {
        MiniBufferEvent::Cancel
        | MiniBufferEvent::ConfirmEmpty
        | MiniBufferEvent::BackspaceOnEmpty => finish_steel_prompt(ed, r, None),
        MiniBufferEvent::Confirm(text) => finish_steel_prompt(ed, r, Some(text)),
        // Plain editing (char typed/deleted, cursor moved) is already
        // applied by `MiniBuffer::handle_key` — nothing further to do.
        // Tab/Up/Down are no-ops here (no completion, no history for a
        // one-shot prompt).
        MiniBufferEvent::Edited
        | MiniBufferEvent::CursorMoved
        | MiniBufferEvent::EmptiedByBackspace
        | MiniBufferEvent::CompleteRequested { .. }
        | MiniBufferEvent::HistoryPrev
        | MiniBufferEvent::HistoryNext
        | MiniBufferEvent::Ignored => {}
    }
}

/// Queues exactly one `(callback text-or-#f)` call — takes the `Prompt`
/// layer *by value* via `EditorState::take_layer`, which skips its own
/// `tear_down` (unlike `truncate_layers`), so this doesn't double-fire
/// against `PromptLayer::tear_down`'s own `#f` fire for the incidental
/// case. Keeps its own `history.begin_session_all()` call so this
/// explicit path's behavior is unchanged by that split.
fn finish_steel_prompt(ed: &mut Editor, r: LayerRef, text: Option<String>) {
    let prompt = ed.state.take_layer::<PromptLayer>(&ed.view, r);
    ed.state.history.begin_session_all();
    let arg = match text {
        Some(s) => steel::rvals::SteelVal::StringV(s.into()),
        None => steel::rvals::SteelVal::BoolV(false),
    };
    ed.state.queue_steel_call(prompt.callback, vec![arg]);
}
