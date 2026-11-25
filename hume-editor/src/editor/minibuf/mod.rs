pub(in crate::editor) mod history;

use hume_grid::Rect;
use hume_rope::width::{chrome_width, fit_prompt};

use hume_engine::pipeline::EngineView;

use self::history::{HistoryKind, HistoryStore, HistoryWalk};
use super::error::CommandError;
use super::input_stack::{InputEvent, LayerRef, PastedText};
use super::keymap::WalkResult;
use super::line_input::{LineEdit, LineInput};
use super::paste_plan::PlannedPaste;
use super::replay::dispatch_command_line_key;
use super::{Editor, EditorState};
use hume_core::BindMode;

/// The one-line input on the statusline, active while the user types a
/// command, a search or sift pattern, a shell command for `|`, or the
/// answer to a Steel prompt.
#[derive(Clone)]
pub(in crate::editor) struct MiniBuffer {
    /// The text shown before the input: `:` for commands, `/`/`?` for search,
    /// `⫽` for sift, `|` for a shell command, or a Steel prompt's `label`
    /// (`(prompt! label …)`), of arbitrary length.
    pub prompt: String,
    /// The text typed so far and its edit cursor.
    pub line: LineInput,
    /// What Tab completes on this line, declared by whoever opened it;
    /// `None` for a line Tab does nothing on.
    pub completion: Option<PromptCompletion>,
    /// The ring Enter records the line in and Up/Down recall from; `None`
    /// for a prompt with no history.
    history: Option<HistoryKind>,
    /// The walk through `history` Up started, until an edit or confirm
    /// ends it.
    walk: Option<HistoryWalk>,
}

/// The completion a minibuffer line offers, which decides the source Tab
/// resolves to (`EditorState::trigger_minibuf_completion`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::editor) enum PromptCompletion {
    /// The `:` line: the command name, then that command's own completer.
    CommandLine,
    /// One registered minibuffer source, which finds its own token in the
    /// whole line.
    Source(&'static str),
}

/// What the statusline row shows of a [`MiniBuffer`] this frame:
/// [`MiniBuffer::statusline_view`]'s result, resolved once in
/// `prepare_frame`. The statusline draws [`text`](Self::text), and the
/// terminal cursor and the completion popup are placed from it, so all
/// three agree on what scrolled off.
pub(crate) struct MinibufView {
    /// The prompt as shown, then the visible part of the input.
    text: String,
    /// Screen column where `text` starts.
    x: u16,
    /// Byte length of the prompt's part of `text`.
    prompt_len: usize,
    prompt_width: usize,
    /// Byte offset in the input where its visible part starts.
    start: usize,
    /// Byte offset of the edit cursor in the input.
    cursor: usize,
}

impl MinibufView {
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Screen column where [`Self::text`] starts.
    pub(crate) fn x(&self) -> u16 {
        self.x
    }

    /// Screen column of the edit cursor.
    pub(in crate::editor) fn cursor_x(&self) -> u16 {
        self.x_at(self.cursor)
    }

    /// Screen column of the cell at `byte_offset` into the input. An offset
    /// before the visible part lands on its first cell and one after the
    /// cursor on the cursor's cell, so the column is always on the row.
    pub(in crate::editor) fn x_at(&self, byte_offset: usize) -> u16 {
        let offset = byte_offset.clamp(self.start, self.cursor) - self.start;
        let input = &self.text[self.prompt_len..];
        self.x + (self.prompt_width + chrome_width(&input[..offset])) as u16
    }
}

/// Outcome of feeding one key to [`MiniBuffer::handle_key`].
///
/// Callers match on this to perform the mode-specific follow-up action
/// (e.g. search confirmation vs. command execution on `Confirm`). History
/// is already handled by then.
pub(super) enum MiniBufferEvent {
    /// Esc or Ctrl-c: caller should cancel/close the mini-buffer.
    Cancel,
    /// Enter with non-empty input: `String` is the confirmed text, already
    /// recorded in the prompt's history.
    Confirm(String),
    /// Enter with empty input: treat as cancel.
    ConfirmEmpty,
    /// Backspace on an empty line, which has nothing to delete.
    BackspaceOnEmpty,
    /// Any other key, and what it did to the line. A history recall that
    /// replaced the text is `Changed`.
    Line(LineEdit),
}

impl MiniBuffer {
    /// A fresh minibuffer for `prompt`, with empty input and the cursor at
    /// its start, recording in and recalling from `history`: the shape
    /// every opener but `prompt!` needs.
    pub(in crate::editor) fn new(
        prompt: impl Into<String>,
        completion: Option<PromptCompletion>,
        history: HistoryKind,
    ) -> Self {
        Self {
            prompt: prompt.into(),
            line: LineInput::default(),
            completion,
            history: Some(history),
            walk: None,
        }
    }

    /// A minibuffer for `prompt` pre-populated with `input`, cursor parked
    /// at its end: `(prompt! label prefill …)`'s shape, which completes
    /// nothing and has no history.
    pub(in crate::editor) fn with_prefill(prompt: impl Into<String>, input: String) -> Self {
        Self {
            prompt: prompt.into(),
            line: LineInput::new(input),
            completion: None,
            history: None,
            walk: None,
        }
    }

    /// What `area`, the statusline's cells for the minibuffer, shows of
    /// this line: the prompt, cut from its end so the cursor keeps a cell,
    /// then the part of the input around the edit cursor that fits after
    /// it, scrolled as little as it can from the last frame's.
    pub(in crate::editor) fn statusline_view(&mut self, area: Rect) -> MinibufView {
        let cells = usize::from(area.width);
        let (prompt, prompt_width) = fit_prompt(&self.prompt, cells);
        let cursor = self.line.cursor();
        let window = self.line.scroll_window(cells - prompt_width);
        MinibufView {
            text: [prompt, window.before, window.at, window.after].concat(),
            x: area.x,
            prompt_len: prompt.len(),
            prompt_width,
            start: window.start,
            cursor,
        }
    }

    /// Handle a single key event for standard mini-buffer editing.
    ///
    /// Covers the prompt's own keys: cancel (Esc/Ctrl-c), confirm (Enter),
    /// Backspace on an empty line and history (Up/Down). Every
    /// other key goes to [`LineInput::edit`]. Enter records the line in
    /// the prompt's history ring, and an edit ends a walk through it; a
    /// cursor move does not, as the walk filters by text. Returns a
    /// [`MiniBufferEvent`] describing the outcome so the caller can apply
    /// mode-specific logic.
    pub(super) fn handle_key(
        &mut self,
        key: termina::event::KeyEvent,
        history: &mut HistoryStore,
    ) -> MiniBufferEvent {
        use termina::event::{KeyCode, Modifiers};

        match key.code {
            KeyCode::Escape => MiniBufferEvent::Cancel,
            KeyCode::Char('c') if key.modifiers.contains(Modifiers::CONTROL) => {
                MiniBufferEvent::Cancel
            }
            KeyCode::Enter => {
                let text = self.line.text();
                if text.is_empty() {
                    return MiniBufferEvent::ConfirmEmpty;
                }
                if let Some(kind) = self.history {
                    history.get_mut(kind).push(text.to_owned());
                }
                self.walk = None;
                MiniBufferEvent::Confirm(text.to_owned())
            }
            KeyCode::Backspace if self.line.text().is_empty() => MiniBufferEvent::BackspaceOnEmpty,
            KeyCode::Up | KeyCode::Down => {
                let recalled = self.history.and_then(|kind| {
                    let ring = history.get(kind);
                    if key.code == KeyCode::Up {
                        ring.prev(&mut self.walk, self.line.text())
                    } else {
                        ring.next(&mut self.walk)
                    }
                });
                match recalled {
                    Some(text) if text != self.line.text() => {
                        self.line.set(text);
                        MiniBufferEvent::Line(LineEdit::Changed)
                    }
                    _ => MiniBufferEvent::Line(LineEdit::Ignored),
                }
            }
            _ => {
                let edit = self.line.edit(key);
                if edit == LineEdit::Changed {
                    self.walk = None;
                }
                MiniBufferEvent::Line(edit)
            }
        }
    }
}

// ── Shared by every minibuf-backed layer ────────────────────────────────────
//
// `Command`/`Search`/`Sift`/`Prompt`/`ShellPrompt` (`input_stack/{command,
// search,sift,prompt,shell_prompt}.rs`) all call these. They are homed here
// rather than in any one of those files so none of the five has to own a
// helper the others also need.

/// Converts one `InputEvent` into a `MiniBufferEvent` for whichever
/// minibuf-backed layer (`Command`/`Search`/`Sift`/`Prompt`/`ShellPrompt`)
/// is dispatching: the single place a key becomes an edit to the
/// minibuffer, shared by all five. Tab starts the line's
/// [`MiniBuffer::completion`], if it declares one, and returns `None`; once
/// a popup opens, the completion layer above takes Tab and Shift-Tab
/// before they reach here. A mouse event has no minibuffer edit to become: it falls through
/// (the cursor still moves under an open `:`/`/`/`s` prompt) and
/// returns `None` either way. `None` also covers
/// the case where no minibuffer is live (dispatch reached this layer
/// kind but its payload was already torn down mid-call, which matches every
/// other handler's own liveness discipline).
pub(in crate::editor) fn minibuf_input(
    ed: &mut Editor,
    r: LayerRef,
    ev: InputEvent,
) -> Option<MiniBufferEvent> {
    match ev {
        InputEvent::Mouse(mouse) => {
            ed.fall_through(r, InputEvent::Mouse(mouse));
            None
        }
        InputEvent::Key(key) if key.code == termina::event::KeyCode::Tab => {
            ed.start_minibuf_completion();
            None
        }
        InputEvent::Key(key) => Some(
            ed.state
                .input
                .minibuf_mut()?
                .handle_key(key, &mut ed.state.history),
        ),
    }
}

/// [`minibuf_input`] for the command-line prompts (`Command`/`Prompt`/
/// `ShellPrompt`): a key
/// bound in the `'command` keymap runs its command instead of the prompt's
/// own handling. The key is consumed even when its command is unknown, as
/// an Insert-mode binding's is.
pub(in crate::editor) fn command_line_input(
    ed: &mut Editor,
    r: LayerRef,
    ev: InputEvent,
) -> Option<MiniBufferEvent> {
    if let InputEvent::Key(key) = ev
        && let WalkResult::Leaf(cmd) = ed.resolve_keys(BindMode::Command, &[key]).walk
    {
        if let Some(cmd) = ed.resolve_mappable(cmd.name.as_ref()) {
            dispatch_command_line_key(ed, cmd);
        }
        return None;
    }
    minibuf_input(ed, r, ev)
}

/// Insert `text` as one line ([`PastedText::single_line`]) at the
/// minibuffer's cursor, ending any history walk as a typed edit does: the
/// paste half every minibuf-mode layer's paste handler shares. Whether the
/// input changed, for a layer that refreshes a live preview on a changed
/// line: `false` when no minibuffer is live or `text` flattens to nothing.
pub(in crate::editor) fn paste_into_minibuf(state: &mut EditorState, text: &PastedText) -> bool {
    let Some(mb) = state.input.minibuf_mut() else {
        return false;
    };
    let changed = mb.line.insert_str(&text.single_line());
    if changed {
        mb.walk = None;
    }
    changed
}

/// The paste handler of a minibuf-mode layer with no follow-up to a
/// changed line (`:`, `|`, `prompt!`).
pub(in crate::editor) fn paste_line(
    state: &mut EditorState,
    _view: &EngineView,
    _r: LayerRef,
    text: PastedText,
) -> Result<PlannedPaste, CommandError> {
    paste_into_minibuf(state, &text);
    Ok(PlannedPaste::NONE)
}

/// Flatten already-newline-normalized text for a single-line input field
/// (the minibuffer, a picker query): drop trailing newlines, turn any
/// interior newline into a space.
pub(in crate::editor) fn flatten_single_line(text: &str) -> String {
    text.trim_end_matches('\n').replace('\n', " ")
}

#[cfg(test)]
mod tests;
