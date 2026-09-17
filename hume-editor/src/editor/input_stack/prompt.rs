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
    fn tear_down(&mut self, state: &mut EditorState, _view: &EngineView) {
        state.history.begin_session_all();
    }
    fn minibuf(&self) -> Option<&MiniBuffer> {
        Some(&self.minibuf)
    }
    fn minibuf_mut(&mut self) -> Option<&mut MiniBuffer> {
        Some(&mut self.minibuf)
    }
}

impl super::stack::InputStack {
    /// The open `(prompt! …)` session's callback, if `Prompt` is on the
    /// stack. Read-only — a caller finishing the prompt clones this out
    /// (cheap: `SteelVal` is reference-counted) before truncating the
    /// `Prompt` layer away, rather than taking ownership through this
    /// lookup.
    pub(in crate::editor) fn prompt_callback(&self) -> Option<&steel::rvals::SteelVal> {
        self.find::<PromptLayer>().map(|l| &l.callback)
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

/// Queues exactly one `(callback text-or-#f)` call and truncates the
/// `Prompt` layer (`r`) — the callback is cloned out (cheap: `SteelVal`
/// is reference-counted) before truncating, since teardown never fires
/// a Steel callback itself.
fn finish_steel_prompt(ed: &mut Editor, r: LayerRef, text: Option<String>) {
    let Some(callback) = ed.state.input.prompt_callback().cloned() else {
        return;
    };
    let arg = match text {
        Some(s) => steel::rvals::SteelVal::StringV(s.into()),
        None => steel::rvals::SteelVal::BoolV(false),
    };
    ed.state.queue_steel_call(callback, vec![arg]);
    ed.state.truncate_layers(&ed.view, r);
}
