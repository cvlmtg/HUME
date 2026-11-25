//! The shell prompt: the `|` line `pipe` reads its command from. An overlay,
//! not a mode layer, so an Insert session under it stays open and the
//! command's output lands in it.

use hume_engine::pipeline::EngineView;
use hume_engine::types::EditorMode;

use super::super::minibuf::history::HistoryKind;
use super::super::minibuf::{self, MiniBuffer, MiniBufferEvent, PromptCompletion};
use super::super::replay::WaitingInput;
use super::super::{Editor, EditorState, commands};
use super::stack::{InputEvent, Layer, LayerHandler, LayerRef, PasteHandler};

/// The completion source `runtime/scheme/shells.scm` registers for the
/// `|` line.
const SHELL_SOURCE: &str = "shell";

pub(in crate::editor) struct ShellPromptLayer {
    minibuf: MiniBuffer,
    /// The `pipe` this prompt reads the command for, run with it on Enter.
    then: commands::PendingDispatch,
    /// The check this prompt passed as it opened.
    input: WaitingInput,
}

/// Open the shell prompt for `then`, a `pipe` dispatched without its
/// command.
pub(in crate::editor) fn open_shell_prompt(
    state: &mut EditorState,
    view: &EngineView,
    then: commands::PendingDispatch,
    input: WaitingInput,
) {
    state.push_layer(
        view,
        ShellPromptLayer {
            minibuf: MiniBuffer::new(
                "|",
                Some(PromptCompletion::Source(SHELL_SOURCE)),
                HistoryKind::Pipe,
            ),
            then,
            input,
        },
    );
}

impl Layer for ShellPromptLayer {
    fn handler(&self) -> LayerHandler {
        shell_prompt_input
    }
    fn paste_handler(&self) -> PasteHandler {
        minibuf::paste_line
    }
    fn mode(&self) -> Option<EditorMode> {
        None
    }
    fn snapshot_mut(&mut self) -> Option<&mut super::PaneSnapshot> {
        None
    }
    fn minibuf(&self) -> Option<&MiniBuffer> {
        Some(&self.minibuf)
    }
    fn minibuf_mut(&mut self) -> Option<&mut MiniBuffer> {
        Some(&mut self.minibuf)
    }
}

fn shell_prompt_input(ed: &mut Editor, r: LayerRef, ev: InputEvent) {
    if let Some(event) = minibuf::command_line_input(ed, r, ev) {
        handle_shell_prompt_event(ed, r, event);
    }
}

fn handle_shell_prompt_event(ed: &mut Editor, r: LayerRef, event: MiniBufferEvent) {
    match event {
        MiniBufferEvent::Cancel
        | MiniBufferEvent::ConfirmEmpty
        | MiniBufferEvent::BackspaceOnEmpty => finish_shell_prompt(ed, r, None),
        MiniBufferEvent::Confirm(command) => finish_shell_prompt(ed, r, Some(command)),
        MiniBufferEvent::Line(_) => {}
    }
}

/// Close the prompt, then hand `command` (`None` when it was cancelled) to
/// the `pipe` it was opened for.
fn finish_shell_prompt(ed: &mut Editor, r: LayerRef, command: Option<String>) {
    let ShellPromptLayer {
        then, mut input, ..
    } = *ed.state.take_layer::<ShellPromptLayer>(&ed.view, r);
    commands::finish_text_operand(ed, then, input.take_answer(), command);
}
