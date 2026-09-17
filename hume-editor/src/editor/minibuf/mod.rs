pub(in crate::editor) mod history;

use hume_ui::width::text_width;

use self::history::{HistoryDir, HistoryKind};
use super::Editor;
use super::input_stack::{InputEvent, LayerRef};

// ── MiniBuffer ────────────────────────────────────────────────────────────────

/// The command-line mini-buffer, active while the user is typing a command
/// or search pattern.
///
/// `prompt` distinguishes the context (`:` for commands, `/` or `?` for search)
/// without needing separate mode variants for each prompt type.
#[derive(Clone)]
pub(crate) struct MiniBuffer {
    /// The text shown before the input: `:` for commands, `/`/`?` for search,
    /// or a Steel prompt's `label` (`(prompt! label …)`) — arbitrary length.
    pub prompt: String,
    /// The text typed so far.
    pub input: String,
    /// Byte offset of the edit cursor within `input`. Always on a UTF-8 char boundary.
    pub cursor: usize,
}

/// Outcome of feeding one key to [`MiniBuffer::handle_key`].
///
/// Callers match on this to perform the mode-specific follow-up action
/// (e.g. search confirmation vs. command execution on `Confirm`).
pub(super) enum MiniBufferEvent {
    /// Esc or Ctrl-c — caller should cancel/close the mini-buffer.
    Cancel,
    /// Enter with non-empty input — `String` is the confirmed text.
    Confirm(String),
    /// Enter with empty input — treat as cancel.
    ConfirmEmpty,
    /// Input changed (char typed or deleted), input is now non-empty.
    Edited,
    /// Backspace deleted the last character, leaving the input empty.
    /// Search/select use this to restore their snapshot but stay open.
    /// Command mode keeps the minibuffer open so a second Backspace is needed.
    EmptiedByBackspace,
    /// Backspace was pressed when the input is empty (nothing to delete).
    /// All modes treat this as a cancel / dismiss request.
    BackspaceOnEmpty,
    /// Cursor moved left/right; content unchanged.
    CursorMoved,
    /// Tab (forward) or Shift-Tab (reverse) pressed — caller should cycle completions.
    CompleteRequested { reverse: bool },
    /// Up pressed — caller should recall the previous history entry for this prompt.
    HistoryPrev,
    /// Down pressed — caller should recall the next history entry (or restore scratch).
    HistoryNext,
    /// Key was not handled (e.g. unrecognised control sequence).
    Ignored,
}

impl MiniBuffer {
    /// A fresh minibuffer for `prompt`, with empty input and the cursor at
    /// its start — the shape every opener but `prompt!` needs.
    pub(in crate::editor) fn new(prompt: impl Into<String>) -> Self {
        Self::with_prefill(prompt, String::new())
    }

    /// A minibuffer for `prompt` pre-populated with `input`, cursor parked
    /// at its end — `(prompt! label prefill …)`'s shape.
    pub(in crate::editor) fn with_prefill(prompt: impl Into<String>, input: String) -> Self {
        let mut mb = Self {
            prompt: prompt.into(),
            input: String::new(),
            cursor: 0,
        };
        mb.set_input(input);
        mb
    }

    /// Installs `text` as `input` and parks the cursor at its end — the
    /// "just replaced the whole input" invariant shared by `with_prefill`
    /// and history recall ([`recall_history`]).
    pub(in crate::editor) fn set_input(&mut self, text: String) {
        self.cursor = text.len();
        self.input = text;
    }

    /// Column offset of the cell at `byte_offset` into `input` within the
    /// rendered statusline: the 1-column `pad_left` space the statusline
    /// renderer prepends, plus the prompt's width, plus the display width of
    /// `input` up to `byte_offset` (clamped to `input`'s length). Add
    /// `area.x` to get the absolute screen column.
    ///
    /// Shared by the edit cursor ([`Self::statusline_cursor_x`], at
    /// `self.cursor`) and the completion-overlay anchor (at a completion
    /// span's start) — both are "where does this byte offset into `input`
    /// land on screen" under the same prompt.
    pub(in crate::editor) fn cursor_x_at(&self, byte_offset: usize) -> u16 {
        let pad: u16 = 1; // pad_left inserts one space before the MiniBuf span
        let prompt_w = text_width(&self.prompt) as u16;
        let safe_offset = byte_offset.min(self.input.len());
        let input_w = text_width(&self.input[..safe_offset]) as u16;
        pad + prompt_w + input_w
    }

    /// Column offset of the edit cursor within the rendered statusline.
    pub(in crate::editor) fn statusline_cursor_x(&self) -> u16 {
        self.cursor_x_at(self.cursor)
    }

    /// Handle a single key event for standard mini-buffer editing.
    ///
    /// Covers: cancel (Esc/Ctrl-c), confirm (Enter), char insertion, grapheme-aware
    /// backspace, and left/right cursor movement. Returns a [`MiniBufferEvent`]
    /// describing the outcome so the caller can apply mode-specific logic.
    pub(super) fn handle_key(&mut self, key: termina::event::KeyEvent) -> MiniBufferEvent {
        use termina::event::{KeyCode, Modifiers};

        match key.code {
            KeyCode::Escape => MiniBufferEvent::Cancel,
            KeyCode::Char('c') if key.modifiers.contains(Modifiers::CONTROL) => {
                MiniBufferEvent::Cancel
            }
            KeyCode::Enter => {
                if self.input.is_empty() {
                    MiniBufferEvent::ConfirmEmpty
                } else {
                    MiniBufferEvent::Confirm(self.input.clone())
                }
            }
            KeyCode::Backspace => {
                if self.cursor == 0 {
                    if self.input.is_empty() {
                        MiniBufferEvent::BackspaceOnEmpty
                    } else {
                        MiniBufferEvent::Ignored // cursor at start but input non-empty: no-op
                    }
                } else {
                    let prev = hume_rope::grapheme::prev_str_boundary(&self.input, self.cursor);
                    self.input.drain(prev..self.cursor);
                    self.cursor = prev;
                    if self.input.is_empty() {
                        MiniBufferEvent::EmptiedByBackspace
                    } else {
                        MiniBufferEvent::Edited
                    }
                }
            }
            // Ctrl-w: readline-style delete-word-backward. Skip trailing
            // whitespace first, then remove the word. Never closes the minibuf
            // on empty (unlike Backspace) — emits `Ignored` when there's
            // nothing to the left of the cursor.
            KeyCode::Char('w') if key.modifiers.contains(Modifiers::CONTROL) => {
                let new_cursor = word_boundary_back(&self.input, self.cursor);
                if new_cursor == self.cursor {
                    MiniBufferEvent::Ignored
                } else {
                    self.input.drain(new_cursor..self.cursor);
                    self.cursor = new_cursor;
                    MiniBufferEvent::Edited
                }
            }
            KeyCode::Char(ch) if !key.modifiers.contains(Modifiers::CONTROL) => {
                self.input.insert(self.cursor, ch);
                self.cursor += ch.len_utf8();
                MiniBufferEvent::Edited
            }
            KeyCode::Left => {
                self.cursor = hume_rope::grapheme::prev_str_boundary(&self.input, self.cursor);
                MiniBufferEvent::CursorMoved
            }
            KeyCode::Right => {
                self.cursor = hume_rope::grapheme::next_str_boundary(&self.input, self.cursor);
                MiniBufferEvent::CursorMoved
            }
            KeyCode::Tab => MiniBufferEvent::CompleteRequested { reverse: false },
            KeyCode::BackTab => MiniBufferEvent::CompleteRequested { reverse: true },
            // Up/Down are handled by the caller (mode-specific history ring).
            // Do NOT bind Ctrl-n / Ctrl-p here — those are reserved for
            // future completion-popup navigation.
            KeyCode::Up => MiniBufferEvent::HistoryPrev,
            KeyCode::Down => MiniBufferEvent::HistoryNext,
            _ => MiniBufferEvent::Ignored,
        }
    }

    /// Insert a whole string at the edit cursor — the terminal-paste
    /// counterpart of the single-char branch in [`handle_key`](Self::handle_key),
    /// returning the same `Edited` event so a paste runs each mode's real
    /// `Edited` follow-up instead of a hand-mirrored copy of it. `Ignored`
    /// on an empty string, matching `handle_key`'s own unbound-key result.
    pub(super) fn insert_str(&mut self, s: &str) -> MiniBufferEvent {
        if s.is_empty() {
            return MiniBufferEvent::Ignored;
        }
        self.input.insert_str(self.cursor, s);
        self.cursor += s.len();
        MiniBufferEvent::Edited
    }
}

/// Walk back from `cursor` over trailing whitespace, then over one run of
/// non-whitespace graphemes — the readline Ctrl-w "delete word" boundary.
/// Returns the byte offset where the deletion should begin; equals `cursor`
/// when there is nothing to delete.
///
/// `/` (and `\` on Windows) is treated as a separator so that path arguments
/// delete one component at a time (`/tmp/alpha/one.txt` → `/tmp/alpha/` → …).
fn word_boundary_back(s: &str, cursor: usize) -> usize {
    let is_sep = |slice: &str| {
        slice
            .chars()
            .all(|c| c.is_whitespace() || hume_platform::path::is_path_sep(c))
    };
    let mut i = cursor;
    // Phase 1: skip trailing separators (whitespace or '/').
    while i > 0 {
        let prev = hume_rope::grapheme::prev_str_boundary(s, i);
        if is_sep(&s[prev..i]) {
            i = prev;
        } else {
            break;
        }
    }
    // Phase 2: consume the run of non-separator graphemes.
    while i > 0 {
        let prev = hume_rope::grapheme::prev_str_boundary(s, i);
        if !is_sep(&s[prev..i]) {
            i = prev;
        } else {
            break;
        }
    }
    i
}

// ── Shared by every minibuf-backed mode layer ───────────────────────────────
//
// `Command`/`Search`/`Sift`/`Prompt` (`input_stack/{command,search,sift,
// prompt}.rs`) all call these — homed here rather than in any one of those
// files so none of the four has to own a helper the other three also need.

/// Converts one `InputEvent` into a `MiniBufferEvent` for whichever
/// minibuf-mode layer (`Command`/`Search`/`Sift`/`Prompt`) is dispatching
/// — the single place a key or a paste becomes an edit to the
/// minibuffer, shared by all four so a paste runs the same `Edited`
/// follow-up a typed character would (see each mode's own event match).
/// A mouse event has no minibuffer edit to become — it falls through
/// (the cursor still moves under an open `:`/`/`/`s` prompt) and
/// returns `None` either way. `None` also covers
/// the case where no minibuffer is live (dispatch reached this layer
/// kind but its payload was already torn down mid-call — matches every
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
        InputEvent::Key(key) => Some(ed.state.input.minibuf_mut()?.handle_key(key)),
        InputEvent::Paste(text) => Some(
            ed.state
                .input
                .minibuf_mut()?
                .insert_str(&flatten_single_line(&text)),
        ),
    }
}

/// Recall the previous (`Prev`) or next (`Next`) entry from `kind`'s history
/// ring and install it in the minibuffer. No-op when there is no active
/// minibuffer or when the ring has nowhere to go.
pub(in crate::editor) fn recall_history(ed: &mut Editor, kind: HistoryKind, dir: HistoryDir) {
    let current = ed
        .state
        .input
        .minibuf()
        .map(|m| m.input.as_str())
        .unwrap_or("");
    let text = match dir {
        HistoryDir::Prev => ed.state.history.get_mut(kind).prev(current),
        HistoryDir::Next => ed.state.history.get_mut(kind).next(),
    };
    if let Some(text) = text
        && let Some(mb) = ed.state.input.minibuf_mut()
    {
        mb.set_input(text);
    }
}

/// Flatten already-newline-normalized text for a single-line input field
/// (the minibuffer, a picker query): drop trailing newlines, turn any
/// interior newline into a space.
pub(in crate::editor) fn flatten_single_line(text: &str) -> String {
    text.trim_end_matches('\n').replace('\n', " ")
}

#[cfg(test)]
mod tests;
