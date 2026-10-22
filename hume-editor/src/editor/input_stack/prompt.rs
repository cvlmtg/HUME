//! The `Prompt` layer: a Steel `(prompt! …)` session's own minibuffer mode.
//! Distinct from `Command` despite sharing the `:` look: no history, no
//! completion, no directory-descend special case. Exactly one
//! `(callback text-or-#f)` call fires, on Confirm or on any cancel path.

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;

use super::super::error::CommandError;
use super::super::minibuf::{self, MiniBuffer, MiniBufferEvent};
use super::super::replay::{Answer, InputOpener, PendingAnswer, WaitingInput};
use super::super::{Editor, EditorState};
use super::paste::PastedText;
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, PasteHandler, Removal};

/// Built only by [`open_prompt`].
pub(in crate::editor) struct PromptLayer {
    minibuf: MiniBuffer,
    callback: hume_scripting::Rooted,
    /// The check this prompt passed as it opened, holding where its answer
    /// records when it belongs to a recorded change.
    input: WaitingInput,
}

/// `prompt!`'s funnel: opens a prompt labelled `label` over `prefill`,
/// whose `callback` gets the text confirmed, or `#f`.
///
/// During `.` the prompt never opens: the recorded [`Answer::Prompt`]
/// answers it, its callback queued to run after the replayed body, refusing
/// when the next answer is not a prompt's, and running nothing for a prompt
/// that never resolved live.
pub(in crate::editor) fn open_prompt(
    state: &mut EditorState,
    view: &EngineView,
    label: String,
    prefill: String,
    callback: hume_scripting::Rooted,
) -> Result<(), String> {
    const WHAT: &str = "prompt!";
    if let Some(replay) = state.dot_replay.as_mut() {
        return replay.answer_call(
            WHAT,
            |a| match a {
                Answer::Prompt(text) => Some(text.clone()),
                _ => None,
            },
            |text| {
                Ok(PendingAnswer::Call {
                    proc: callback,
                    arg: hume_scripting::Rooted::new(prompt_arg(text)),
                })
            },
        );
    }
    // Not "a Command-mode minibuffer is open": a `prompt!` called from a
    // `:command`'s body runs while that command line's own `Command` layer
    // is still on the stack (it's truncated only after the command
    // returns). Only a `Prompt` layer already on top means a prior
    // `prompt!` call has taken over the session.
    if state.input.is::<PromptLayer>(state.input.mode_layer()) {
        return Err("prompt!: a minibuffer session is already open".to_string());
    }
    let input = state.waiting_input(WHAT, InputOpener::Code)?;
    state.history.begin_session_all();
    state.push_mode_layer(
        view,
        PromptLayer {
            minibuf: MiniBuffer::with_prefill(label, prefill),
            callback,
            input,
        },
    );
    Ok(())
}

/// The argument a prompt's callback gets: the confirmed text, or `#f`.
fn prompt_arg(text: Option<String>) -> steel::rvals::SteelVal {
    match text {
        Some(s) => steel::rvals::SteelVal::StringV(s.into()),
        None => steel::rvals::SteelVal::BoolV(false),
    }
}

impl PromptLayer {
    /// Queue this prompt's callback with `text`, or `#f`, recording it under
    /// its answer ticket, if it has one. Every way a prompt closes ends here.
    fn fire(&mut self, state: &mut EditorState, text: Option<String>) {
        let arg = hume_scripting::Rooted::new(prompt_arg(text.clone()));
        state.queue_answered_call(
            self.input.take_answer(),
            Answer::Prompt(text),
            self.callback.clone(),
            arg,
        );
    }
}

impl Layer for PromptLayer {
    fn snapshot_mut(&mut self) -> Option<&mut super::PaneSnapshot> {
        None
    }

    fn handler(&self) -> LayerHandler {
        prompt_input
    }
    fn paste_handler(&self) -> PasteHandler {
        prompt_paste
    }
    fn mode(&self) -> Option<EditorMode> {
        // The engine has no `Prompt` variant, and today's `(prompt! …)`
        // session already runs as `Command` for every consumer outside this
        // crate.
        Some(EditorMode::Command)
    }
    /// Fires the callback with `#f` unconditionally, ignoring `why`,
    /// unlike every other minibuf-mode layer's `tear_down`, which never
    /// fires a Steel callback (the file header's "exactly one call fires"
    /// contract otherwise has no arm to rely on when this layer is removed
    /// incidentally: buried under a `Confirm`/`Picker` that a
    /// `close-*!`/Rust-internal retirement then truncates through). Unlike
    /// `PickerLayer::tear_down` (see its own doc), this one *is* also
    /// reached with `why == Removal::Explicit`: `push_mode_layer` replaces
    /// the current mode layer outright when a `:`/`/`/sift/`i` key (or the
    /// Steel/hook/timer path behind any of those commands) lands while a
    /// `prompt!` session is open, naming this layer as `truncate_layers`'s
    /// own target. `#f` is the right answer either way, so `why` doesn't
    /// change what fires, only how it was reached. The explicit accept/
    /// cancel path (`finish_steel_prompt`) never runs this: it takes the
    /// layer *by value* via `EditorState::take_layer`, firing its own
    /// callback explicitly instead.
    fn tear_down(mut self: Box<Self>, state: &mut EditorState, _view: &EngineView, _why: Removal) {
        state.history.begin_session_all();
        self.fire(state, None);
    }
    fn minibuf(&self) -> Option<&MiniBuffer> {
        Some(&self.minibuf)
    }
    fn minibuf_mut(&mut self) -> Option<&mut MiniBuffer> {
        Some(&mut self.minibuf)
    }
}

pub(in crate::editor) fn prompt_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    if let Some(event) = minibuf::command_line_input(ed, r, ev) {
        handle_steel_prompt_event(ed, r, event);
    }
}

/// An edit has no follow-up here, as `handle_steel_prompt_event`'s `Edited`
/// arm shows.
fn prompt_paste(
    state: &mut EditorState,
    _view: &EngineView,
    _r: LayerRef,
    text: PastedText,
) -> Result<(), CommandError> {
    minibuf::paste_into_minibuf(state, &text);
    Ok(())
}

/// Routes a `Prompt` layer's key for a Steel `(prompt! …)` session
/// rather than a `:` command line: no history, no completion, no
/// directory-descend special case. Exactly one `(callback text-or-#f)`
/// call fires, on Confirm or on any of the cancel paths.
fn handle_steel_prompt_event(ed: &mut Editor, r: LayerRef, event: MiniBufferEvent) {
    match event {
        MiniBufferEvent::Cancel
        | MiniBufferEvent::ConfirmEmpty
        | MiniBufferEvent::BackspaceOnEmpty => finish_steel_prompt(ed, r, None),
        MiniBufferEvent::Confirm(text) => finish_steel_prompt(ed, r, Some(text)),
        // Plain editing (char typed/deleted, cursor moved) is already
        // applied by `MiniBuffer::handle_key`; nothing further to do.
        // Up/Down are no-ops here (no history for a one-shot prompt).
        MiniBufferEvent::Edited
        | MiniBufferEvent::CursorMoved
        | MiniBufferEvent::EmptiedByBackspace
        | MiniBufferEvent::HistoryPrev
        | MiniBufferEvent::HistoryNext
        | MiniBufferEvent::Ignored => {}
    }
}

/// Queues exactly one `(callback text-or-#f)` call. Takes the `Prompt`
/// layer *by value* via `EditorState::take_layer`, which skips its own
/// `tear_down` (unlike `truncate_layers`), so this doesn't double-fire
/// against `PromptLayer::tear_down`'s own `#f` fire for the incidental
/// case. Keeps its own `history.begin_session_all()` call so this
/// explicit path's behavior is unchanged by that split.
fn finish_steel_prompt(ed: &mut Editor, r: LayerRef, text: Option<String>) {
    let mut prompt = ed.state.take_layer::<PromptLayer>(&ed.view, r);
    ed.state.history.begin_session_all();
    prompt.fire(&mut ed.state, text);
}
