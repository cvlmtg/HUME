use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

// ── MiniBuffer ────────────────────────────────────────────────────────────────

/// The command-line mini-buffer, active while the user is typing a command
/// or search pattern.
///
/// `prompt` distinguishes the context (`:` for commands, `/` or `?` for search)
/// without needing separate mode variants for each prompt type.
#[derive(Clone)]
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
    /// Column offset of the edit cursor within the rendered statusline.
    ///
    /// Accounts for the 1-column `pad_left` space prepended by the statusline
    /// renderer, the prompt character, and the input text before the cursor.
    /// Add `area.x` to get the absolute screen column.
    pub(crate) fn statusline_cursor_col(&self) -> u16 {
        let pad: u16 = 1; // pad_left inserts one space before the MiniBuf span
        let prompt_w = self.prompt.width().unwrap_or(1) as u16;
        let input_w = UnicodeWidthStr::width(&self.input[..self.cursor]) as u16;
        pad + prompt_w + input_w
    }

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
            // Ctrl-W: readline-style delete-word-backward. Skip trailing
            // whitespace first, then remove the word. Never closes the minibuf
            // on empty (unlike Backspace) — emits `Ignored` when there's
            // nothing to the left of the cursor.
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let new_cursor = word_boundary_back(&self.input, self.cursor);
                if new_cursor == self.cursor {
                    MiniBufferEvent::Ignored
                } else {
                    self.input.drain(new_cursor..self.cursor);
                    self.cursor = new_cursor;
                    MiniBufferEvent::Edited
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
            KeyCode::Tab => MiniBufferEvent::CompleteRequested { reverse: false },
            KeyCode::BackTab => MiniBufferEvent::CompleteRequested { reverse: true },
            // Up/Down are handled by the caller (mode-specific history ring).
            // Do NOT bind Ctrl+N / Ctrl+P here — those are reserved for
            // future completion-popup navigation.
            KeyCode::Up => MiniBufferEvent::HistoryPrev,
            KeyCode::Down => MiniBufferEvent::HistoryNext,
            _ => MiniBufferEvent::Ignored,
        }
    }
}

// ── Grapheme helpers ─────────────────────────────────────────────────────────

/// Return the byte offset of the grapheme cluster that ends at `cursor`.
///
/// If `cursor` is already at 0 (start of string), returns 0.
fn prev_grapheme(s: &str, cursor: usize) -> usize {
    s[..cursor]
        .grapheme_indices(true)
        .next_back()
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Return the byte offset immediately after the grapheme cluster that starts at `cursor`.
///
/// If `cursor` is at or past the end of the string, returns `s.len()`.
fn next_grapheme(s: &str, cursor: usize) -> usize {
    s[cursor..]
        .grapheme_indices(true)
        .next()
        .map(|(_, g)| cursor + g.len())
        .unwrap_or(s.len())
}

/// Walk back from `cursor` over trailing whitespace, then over one run of
/// non-whitespace graphemes — the readline Ctrl-W "delete word" boundary.
/// Returns the byte offset where the deletion should begin; equals `cursor`
/// when there is nothing to delete.
///
/// `/` (and `\` on Windows) is treated as a separator so that path arguments
/// delete one component at a time (`/tmp/alpha/one.txt` → `/tmp/alpha/` → …).
fn word_boundary_back(s: &str, cursor: usize) -> usize {
    let is_sep = |slice: &str| {
        slice
            .chars()
            .all(|c| c.is_whitespace() || crate::os::path::is_path_sep(c))
    };
    let mut i = cursor;
    // Phase 1: skip trailing separators (whitespace or '/').
    while i > 0 {
        let prev = prev_grapheme(s, i);
        if is_sep(&s[prev..i]) {
            i = prev;
        } else {
            break;
        }
    }
    // Phase 2: consume the run of non-separator graphemes.
    while i > 0 {
        let prev = prev_grapheme(s, i);
        if !is_sep(&s[prev..i]) {
            i = prev;
        } else {
            break;
        }
    }
    i
}
