use unicode_segmentation::UnicodeSegmentation;

// ── MiniBuffer ────────────────────────────────────────────────────────────────

/// The command-line mini-buffer, active while the user is typing a command
/// or search pattern.
///
/// `prompt` distinguishes the context (`:` for commands, `/` or `?` for search)
/// without needing separate mode variants for each prompt type.
pub(crate) struct MiniBuffer {
    /// The character shown before the input, e.g. `:` for commands, `/` for search.
    pub prompt: char,
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
    /// Esc or Ctrl+C — caller should cancel/close the mini-buffer.
    Cancel,
    /// Enter with non-empty input — `String` is the confirmed text.
    Confirm(String),
    /// Enter with empty input — treat as cancel.
    ConfirmEmpty,
    /// Input changed (char typed or deleted), input is now non-empty.
    Edited,
    /// Backspace made the input empty (search keeps the buffer open but resets position).
    EmptiedByBackspace,
    /// Cursor moved left/right; content unchanged.
    CursorMoved,
    /// Key was not handled (e.g. unrecognised control sequence).
    Ignored,
}

impl MiniBuffer {
    /// Handle a single key event for standard mini-buffer editing.
    ///
    /// Covers: cancel (Esc/Ctrl+C), confirm (Enter), char insertion, grapheme-aware
    /// backspace, and left/right cursor movement. Returns a [`MiniBufferEvent`]
    /// describing the outcome so the caller can apply mode-specific logic.
    pub(super) fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> MiniBufferEvent {
        use crossterm::event::{KeyCode, KeyModifiers};

        match key.code {
            KeyCode::Esc => MiniBufferEvent::Cancel,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
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
                    MiniBufferEvent::EmptiedByBackspace
                } else {
                    let prev = prev_grapheme(&self.input, self.cursor);
                    self.input.drain(prev..self.cursor);
                    self.cursor = prev;
                    if self.input.is_empty() {
                        MiniBufferEvent::EmptiedByBackspace
                    } else {
                        MiniBufferEvent::Edited
                    }
                }
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.insert(self.cursor, ch);
                self.cursor += ch.len_utf8();
                MiniBufferEvent::Edited
            }
            KeyCode::Left => {
                self.cursor = prev_grapheme(&self.input, self.cursor);
                MiniBufferEvent::CursorMoved
            }
            KeyCode::Right => {
                self.cursor = next_grapheme(&self.input, self.cursor);
                MiniBufferEvent::CursorMoved
            }
            _ => MiniBufferEvent::Ignored,
        }
    }
}

// ── Grapheme helpers ─────────────────────────────────────────────────────────

/// Return the byte offset of the grapheme cluster that ends at `cursor`.
///
/// If `cursor` is already at 0 (start of string), returns 0.
fn prev_grapheme(s: &str, cursor: usize) -> usize {
    s[..cursor].grapheme_indices(true).next_back().map(|(i, _)| i).unwrap_or(0)
}

/// Return the byte offset immediately after the grapheme cluster that starts at `cursor`.
///
/// If `cursor` is at or past the end of the string, returns `s.len()`.
fn next_grapheme(s: &str, cursor: usize) -> usize {
    s[cursor..].grapheme_indices(true).next().map(|(_, g)| cursor + g.len()).unwrap_or(s.len())
}
