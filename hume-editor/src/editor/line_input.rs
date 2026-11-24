use std::ops::Range;

use hume_rope::grapheme::{next_str_boundary, prev_str_boundary};
use termina::event::{KeyCode, KeyEvent, Modifiers};

use super::input::is_plain;

/// One line of typed text and its edit cursor: the minibuffer's input and
/// a picker's query. [`edit`](Self::edit) is the one table of text-editing
/// keys both share; each layer handles its own keys (Enter, history,
/// cursor movement, picker actions) before handing a key here.
#[derive(Clone, Default)]
pub(crate) struct LineInput {
    text: String,
    /// Byte offset of the edit cursor within `text`. Always on a UTF-8
    /// char boundary.
    cursor: usize,
}

impl LineInput {
    /// `text` with the cursor parked at its end.
    pub(in crate::editor) fn new(text: String) -> Self {
        Self {
            cursor: text.len(),
            text,
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(in crate::editor) fn cursor(&self) -> usize {
        self.cursor
    }

    /// Replaces the whole text and parks the cursor at its end.
    pub(in crate::editor) fn set(&mut self, text: String) {
        *self = Self::new(text);
    }

    /// Applies one text-editing key: `Backspace` deletes the grapheme
    /// before the cursor, `Ctrl-w` the word before it (see
    /// [`word_boundary_back`]), and a character with no modifier but Shift
    /// ([`is_plain`]) types at the cursor. Whether the text changed: `false`
    /// for any other key, and for a deletion with nothing before the cursor.
    pub(in crate::editor) fn edit(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Backspace => self.delete_back_to(prev_str_boundary(&self.text, self.cursor)),
            KeyCode::Char('w') if key.modifiers.contains(Modifiers::CONTROL) => {
                self.delete_back_to(word_boundary_back(&self.text, self.cursor))
            }
            KeyCode::Char(ch) if is_plain(key.modifiers) => {
                self.text.insert(self.cursor, ch);
                self.cursor += ch.len_utf8();
                true
            }
            _ => false,
        }
    }

    /// Inserts `s` at the cursor, which ends up after it. Whether the text
    /// changed: `false` for an empty `s`.
    pub(in crate::editor) fn insert_str(&mut self, s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
        true
    }

    /// Replaces `text[span]` with `replacement` and parks the cursor at the
    /// end of the inserted text.
    pub(in crate::editor) fn splice(&mut self, span: Range<usize>, replacement: &str) {
        let start = span.start;
        self.text.replace_range(span, replacement);
        self.cursor = start + replacement.len();
    }

    /// Moves the cursor one grapheme left; a no-op at the start.
    pub(in crate::editor) fn move_left(&mut self) {
        self.cursor = prev_str_boundary(&self.text, self.cursor);
    }

    /// Moves the cursor one grapheme right; a no-op at the end.
    pub(in crate::editor) fn move_right(&mut self) {
        self.cursor = next_str_boundary(&self.text, self.cursor);
    }

    /// Deletes `start..cursor` and moves the cursor to `start`. Whether
    /// anything was deleted.
    fn delete_back_to(&mut self, start: usize) -> bool {
        if start == self.cursor {
            return false;
        }
        self.text.drain(start..self.cursor);
        self.cursor = start;
        true
    }
}

/// The readline Ctrl-w "delete word" boundary: walk back from `cursor` over
/// separator graphemes, then over one run of non-separator ones. A
/// separator is whitespace or a path separator (`/`, and `\` on Windows),
/// so a path argument loses one component at a time (`/tmp/alpha/one.txt`
/// → `/tmp/alpha/` → …). Returns the byte offset where the deletion should
/// begin; equals `cursor` when there is nothing to delete.
fn word_boundary_back(s: &str, cursor: usize) -> usize {
    let is_sep = |slice: &str| {
        slice
            .chars()
            .all(|c| c.is_whitespace() || hume_platform::path::is_path_sep(c))
    };
    let back_over = |mut i: usize, sep: bool| {
        while i > 0 {
            let prev = prev_str_boundary(s, i);
            if is_sep(&s[prev..i]) != sep {
                break;
            }
            i = prev;
        }
        i
    };
    back_over(back_over(cursor, true), false)
}

#[cfg(test)]
mod tests;
