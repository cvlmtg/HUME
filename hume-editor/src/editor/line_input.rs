use std::ops::Range;

use hume_rope::grapheme::{next_str_boundary, prev_str_boundary};
use hume_rope::width::{CursorWindow, window_around_cursor};
use termina::event::{KeyCode, KeyEvent, Modifiers};

use super::input::is_plain;

/// What a key did to a [`LineInput`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum LineEdit {
    /// The text changed.
    Changed,
    /// The cursor moved.
    Moved,
    /// The key changed neither the text nor the cursor.
    Ignored,
}

/// One line of typed text and its edit cursor: the minibuffer's input and
/// a picker's query. [`edit`](Self::edit) is the one table of line keys
/// both share; each layer handles its own keys (Enter, history, picker
/// actions) before handing a key here.
#[derive(Clone, Default)]
pub(crate) struct LineInput {
    text: String,
    /// Byte offset of the edit cursor within `text`. Always on a UTF-8
    /// char boundary.
    cursor: usize,
    /// Byte offset where the last [`scroll_window`](Self::scroll_window)
    /// started. At or before `cursor`, and a position the cursor or an edit
    /// start held, so a boundary whatever keys arrive between two frames.
    scroll: usize,
}

impl LineInput {
    /// `text` with the cursor parked at its end.
    pub(in crate::editor) fn new(text: String) -> Self {
        Self {
            cursor: text.len(),
            scroll: 0,
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

    /// Applies one line key: `Backspace` deletes the grapheme before the
    /// cursor, `Ctrl-w` the word before it (see [`word_boundary_back`]), a
    /// character with no modifier but Shift ([`is_plain`]) types at the
    /// cursor, and `Left`/`Right` move it one grapheme. A deletion with
    /// nothing before the cursor and a move past either end are
    /// [`LineEdit::Ignored`], as is any other key.
    pub(in crate::editor) fn edit(&mut self, key: KeyEvent) -> LineEdit {
        match key.code {
            KeyCode::Backspace => self.delete_back_to(prev_str_boundary(&self.text, self.cursor)),
            KeyCode::Char('w') if key.modifiers.contains(Modifiers::CONTROL) => {
                self.delete_back_to(word_boundary_back(&self.text, self.cursor))
            }
            KeyCode::Char(ch) if is_plain(key.modifiers) => {
                self.text.insert(self.cursor, ch);
                self.cursor += ch.len_utf8();
                LineEdit::Changed
            }
            KeyCode::Left => self.move_to(prev_str_boundary(&self.text, self.cursor)),
            KeyCode::Right => self.move_to(next_str_boundary(&self.text, self.cursor)),
            _ => LineEdit::Ignored,
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
        self.scroll = self.scroll.min(start);
    }

    /// The window of `cells` cells over the text that shows the cursor,
    /// scrolled as little as it can from the last one this returned. The
    /// window's start is kept for the next call.
    pub(in crate::editor) fn scroll_window(&mut self, cells: usize) -> CursorWindow<'_> {
        let window = window_around_cursor(&self.text, self.cursor, cells, self.scroll);
        self.scroll = self.cursor - window.before.len();
        window
    }

    fn move_to(&mut self, to: usize) -> LineEdit {
        if to == self.cursor {
            return LineEdit::Ignored;
        }
        self.cursor = to;
        self.scroll = self.scroll.min(to);
        LineEdit::Moved
    }

    /// Deletes `start..cursor` and moves the cursor to `start`.
    fn delete_back_to(&mut self, start: usize) -> LineEdit {
        if start == self.cursor {
            return LineEdit::Ignored;
        }
        self.text.drain(start..self.cursor);
        self.cursor = start;
        self.scroll = self.scroll.min(start);
        LineEdit::Changed
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
