use std::collections::HashMap;

use crossterm::event::KeyEvent;

use crate::core::selection::SelectionSet;
use crate::core::text::Text;

// ── Register name constants ────────────────────────────────────────────────────
//
// HUME uses mnemonic single-char register names rather than the cryptic Vim/
// Helix convention (`"`, `+`, `_`). The key insight: 10 named registers (0-9)
// are enough for real workflows, freeing letters for intuitive special names.
//
// User-facing register names:
//   '0'–'9'  Named storage — text or macros (last write wins).
//   'q'      Default macro register. `QQ` records, `qq` replays.
//            `Q3` records into register '3', `q3` replays from it.
//   'c'      System clipboard (requires OS integration).
//   'b'      Black hole — writes discarded, reads return None.
//   's'      Search register — last search pattern.
//
// DEFAULT_REGISTER is an internal sentinel used when the user does not name a
// register explicitly. It is never typed — the editor layer writes to it
// automatically on every yank/delete.

/// The default register — receives all yanks and deletes when the user does not
/// name an explicit register. This is an internal sentinel; users never type it.
pub(crate) const DEFAULT_REGISTER: char = '"';

/// The black-hole register (`b`) — writes are silently discarded, reads return `None`.
/// Use `"by` to yank without touching the default register.
pub(crate) const BLACK_HOLE_REGISTER: char = 'b';

/// The search register (`s`) — holds the last search pattern.
/// Written by the search command; readable for paste into the command line.
pub(crate) const SEARCH_REGISTER: char = 's';

/// The default macro register (`q`).
/// `QQ` starts/stops recording into this register; `qq` replays from it.
/// Can also hold yanked text if the user explicitly writes to it.
pub(crate) const MACRO_REGISTER: char = 'q';

/// The system clipboard register (`c`).
/// Reads and writes the OS clipboard via `arboard`. Falls back to in-memory
/// storage with a warning when the clipboard is unavailable (headless CI/SSH).
pub(crate) const CLIPBOARD_REGISTER: char = 'c';

/// The content of a register — either yanked text or a recorded macro.
///
/// Registers are single-slot: the last write wins. Writing a macro to a register
/// that previously held text replaces it (and vice-versa).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RegisterContent {
    /// Yanked text — one `String` per selection that was active at yank time,
    /// in document order. A single-cursor yank produces a `Vec` of length 1.
    ///
    /// The linewise-vs-charwise distinction is not tracked explicitly; at paste
    /// time, content that ends with `\n` is treated as linewise.
    Text(Vec<String>),
    /// A recorded macro — the raw sequence of key events captured during recording.
    Macro(Vec<KeyEvent>),
}

/// One named register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Register {
    content: RegisterContent,
}

impl Register {
    fn new(content: RegisterContent) -> Self {
        Self { content }
    }

    /// If this register holds text, borrow the string slice. Returns `None` for macro registers.
    ///
    /// Callers that try to paste a macro register get `None` and treat it as a no-op.
    pub(crate) fn as_text(&self) -> Option<&[String]> {
        match &self.content {
            RegisterContent::Text(v) => Some(v),
            RegisterContent::Macro(_) => None,
        }
    }

    /// If this register holds a recorded macro, borrow the key slice. Returns `None` for text registers.
    pub(crate) fn as_macro(&self) -> Option<&[KeyEvent]> {
        match &self.content {
            RegisterContent::Macro(keys) => Some(keys),
            RegisterContent::Text(_) => None,
        }
    }
}

/// The full collection of named registers.
///
/// Each register holds a [`RegisterContent`] — either yanked text or a recorded macro.
///
/// Special registers (enforced here):
/// - `DEFAULT_REGISTER` (`'"'`): internal default; all yanks/deletes go here
///   when no register is explicitly named.
/// - `BLACK_HOLE_REGISTER` (`'b'`): writes discarded silently; reads return `None`.
///
/// Named registers `'0'`–`'9'` are user storage. Special registers `'c'`
/// (clipboard), `'s'` (search), and `'q'` (macro) are reserved by constants
/// above; their behaviour is wired in the editor layer.
#[derive(Debug, Clone, Default)]
pub(crate) struct RegisterSet {
    registers: HashMap<char, Register>,
}

impl RegisterSet {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Look up a register by name.
    ///
    /// Returns `None` for the black-hole register and for any register that has
    /// not been written yet.
    pub(crate) fn read(&self, name: char) -> Option<&Register> {
        if name == BLACK_HOLE_REGISTER {
            return None;
        }
        self.registers.get(&name)
    }

    /// Write text to a register, replacing its previous contents.
    ///
    /// Writes to the black-hole register (`'b'`) are silently discarded.
    pub(crate) fn write_text(&mut self, name: char, values: Vec<String>) {
        self.write(name, RegisterContent::Text(values));
    }

    /// Write a recorded macro to a register, replacing its previous contents.
    ///
    /// Writes to the black-hole register (`'b'`) are silently discarded.
    pub(crate) fn write_macro(&mut self, name: char, keys: Vec<KeyEvent>) {
        self.write(name, RegisterContent::Macro(keys));
    }

    fn write(&mut self, name: char, content: RegisterContent) {
        if name == BLACK_HOLE_REGISTER {
            return;
        }
        self.registers.insert(name, Register::new(content));
    }
}

/// Extract the text of each selection from the buffer, in document order.
///
/// Returns one `String` per selection. This is the content that gets stored in
/// a register on yank or captured before a delete:
///
/// ```text
/// let yanked = yank_selections(&buf, &sels);
/// let (new_buf, new_sels, _cs) = delete_selection(buf, sels);
/// registers.write_text(DEFAULT_REGISTER, yanked);
/// ```
///
/// Selections are always inclusive, so the text spans `start()..=end()` —
/// internally `buf.slice(start..end+1)`.
pub(crate) fn yank_selections(buf: &Text, sels: &SelectionSet) -> Vec<String> {
    sels.iter_sorted()
        .map(|sel| {
            // end_inclusive() gives the last codepoint of the final grapheme
            // (handles multi-codepoint clusters like e + \u{0301}); +1 converts
            // to an exclusive upper bound for the slice.
            buf.slice(sel.start()..sel.end_inclusive(buf) + 1)
                .to_string()
        })
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::parse_state;

    // ── RegisterSet ───────────────────────────────────────────────────────────

    #[test]
    fn write_and_read() {
        let mut regs = RegisterSet::new();
        regs.write_text(DEFAULT_REGISTER, vec!["hello".to_string()]);
        assert_eq!(
            regs.read(DEFAULT_REGISTER).unwrap().as_text(),
            Some(vec!["hello".to_string()].as_slice())
        );
    }

    #[test]
    fn overwrite_replaces_previous() {
        let mut regs = RegisterSet::new();
        regs.write_text('0', vec!["first".to_string()]);
        regs.write_text('0', vec!["second".to_string()]);
        assert_eq!(
            regs.read('0').unwrap().as_text(),
            Some(vec!["second".to_string()].as_slice())
        );
    }

    #[test]
    fn read_empty_register_returns_none() {
        let regs = RegisterSet::new();
        assert!(regs.read('0').is_none());
    }

    #[test]
    fn black_hole_write_text_is_discarded() {
        let mut regs = RegisterSet::new();
        regs.write_text(BLACK_HOLE_REGISTER, vec!["ignored".to_string()]);
        assert!(regs.read(BLACK_HOLE_REGISTER).is_none());
    }

    #[test]
    fn black_hole_write_macro_is_discarded() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mut regs = RegisterSet::new();
        let keys = vec![KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)];
        regs.write_macro(BLACK_HOLE_REGISTER, keys);
        // The black-hole guard must apply to macro writes too.
        assert!(regs.read(BLACK_HOLE_REGISTER).is_none());
    }

    #[test]
    fn named_registers_are_independent() {
        let mut regs = RegisterSet::new();
        regs.write_text('1', vec!["one".to_string()]);
        regs.write_text('2', vec!["two".to_string()]);
        assert_eq!(
            regs.read('1').unwrap().as_text(),
            Some(vec!["one".to_string()].as_slice())
        );
        assert_eq!(
            regs.read('2').unwrap().as_text(),
            Some(vec!["two".to_string()].as_slice())
        );
    }

    #[test]
    fn write_macro_and_read_back() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mut regs = RegisterSet::new();
        let keys = vec![KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)];
        regs.write_macro('q', keys.clone());
        assert_eq!(regs.read('q').unwrap().as_macro(), Some(keys.as_slice()));
        // as_text() returns None for a macro register
        assert!(regs.read('q').unwrap().as_text().is_none());
    }

    #[test]
    fn macro_overwrites_text_last_write_wins() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mut regs = RegisterSet::new();
        regs.write_text('0', vec!["hello".to_string()]);
        let keys = vec![KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)];
        regs.write_macro('0', keys.clone());
        // now holds a macro, not text
        assert!(regs.read('0').unwrap().as_text().is_none());
        assert_eq!(regs.read('0').unwrap().as_macro(), Some(keys.as_slice()));
    }

    #[test]
    fn text_overwrites_macro_last_write_wins() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mut regs = RegisterSet::new();
        let keys = vec![KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)];
        regs.write_macro('0', keys);
        regs.write_text('0', vec!["text".to_string()]);
        // now holds text, not a macro
        assert!(regs.read('0').unwrap().as_macro().is_none());
        assert_eq!(
            regs.read('0').unwrap().as_text(),
            Some(vec!["text".to_string()].as_slice())
        );
    }

    #[test]
    fn constants_have_expected_values() {
        // Document the register name choices so a future reader sees them tested.
        assert_eq!(BLACK_HOLE_REGISTER, 'b');
        assert_eq!(SEARCH_REGISTER, 's');
        assert_eq!(MACRO_REGISTER, 'q');
        assert_eq!(CLIPBOARD_REGISTER, 'c');
    }

    // ── yank_selections ───────────────────────────────────────────────────────

    #[test]
    fn yank_single_cursor() {
        // Cursor on 'h' — yank captures just 'h'.
        let (buf, sels) = parse_state("-[h]>ello\n");
        assert_eq!(yank_selections(&buf, &sels), vec!["h"]);
    }

    #[test]
    fn yank_multi_char_selection() {
        // Selection covers "hell".
        let (buf, sels) = parse_state("-[hell]>o\n");
        assert_eq!(yank_selections(&buf, &sels), vec!["hell"]);
    }

    #[test]
    fn yank_backward_selection_same_text() {
        // Direction doesn't change the yanked text — it's always start()..=end().
        let (buf, sels) = parse_state("<[hell]-o\n");
        assert_eq!(yank_selections(&buf, &sels), vec!["hell"]);
    }

    #[test]
    fn yank_multi_cursor_document_order() {
        // Two cursors — one on 'h', one on 'o'. Returned in document order.
        let (buf, sels) = parse_state("-[h]>ell-[o]>\n");
        let yanked = yank_selections(&buf, &sels);
        assert_eq!(yanked, vec!["h", "o"]);
    }

    #[test]
    fn yank_full_line_including_newline() {
        // Selection covers "hello\n" — result ends with '\n' (linewise heuristic).
        let (buf, sels) = parse_state("-[hello\n]>");
        assert_eq!(yank_selections(&buf, &sels), vec!["hello\n"]);
    }

    #[test]
    fn yank_grapheme_cluster() {
        // "e\u{0301}" is two chars (e + combining acute) but one grapheme cluster.
        // A cursor on 'e' (pos 0) covers that grapheme — yank must include the
        // combining mark so the yanked text is the complete grapheme "é".
        let (buf, sels) = parse_state("-[e]>\u{0301}x\n");
        assert_eq!(yank_selections(&buf, &sels), vec!["e\u{0301}"]);
    }

    #[test]
    fn yank_on_structural_newline() {
        // Cursor on the trailing '\n' — captures the newline itself.
        let (buf, sels) = parse_state("hello-[\n]>");
        assert_eq!(yank_selections(&buf, &sels), vec!["\n"]);
    }

    #[test]
    fn yank_empty_buffer() {
        // Empty buffer is just "\n"; cursor on it — yank captures the newline.
        let (buf, sels) = parse_state("-[\n]>");
        assert_eq!(yank_selections(&buf, &sels), vec!["\n"]);
    }
}
