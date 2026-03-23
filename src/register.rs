use std::collections::HashMap;

use crate::buffer::Buffer;
use crate::selection::SelectionSet;

// ── Register name constants ────────────────────────────────────────────────────
//
// HUME uses mnemonic single-char register names rather than the cryptic Vim/
// Helix convention (`"`, `+`, `_`). The key insight: 10 named registers (0-9)
// are enough for real workflows, freeing letters for intuitive special names.
//
// User-facing register names:
//   '0'–'9'  Named storage — text or macros (last write wins).
//   'q'      Default macro register. `qq` records, `Q` replays.
//            `q3` records into register '3', `Q3` replays from it.
//   'c'      System clipboard. (Deferred to M3 — needs OS integration.)
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

/// The system clipboard register (`c`). Deferred to M3 (requires OS integration).
/// Reserved here so the editor layer can reference it by name.
pub(crate) const CLIPBOARD_REGISTER: char = 'c';

/// The search register (`s`) — holds the last search pattern.
/// Written by the search command; readable for paste into the command line.
pub(crate) const SEARCH_REGISTER: char = 's';

/// The default macro register (`q`).
/// `qq` starts/stops recording into this register; `Q` replays from it.
/// Can also hold yanked text if the user explicitly writes to it.
pub(crate) const MACRO_REGISTER: char = 'q';

/// One named register.
///
/// Stores a `Vec<String>` — one string per selection that was active at yank
/// time, in document order. A single-cursor yank produces a `Vec` of length 1.
///
/// The linewise-vs-charwise distinction is not tracked explicitly. At paste
/// time, content that ends with `\n` is treated as linewise. This heuristic
/// covers the common cases and can be promoted to an explicit flag later.
///
/// In the future, a `RegisterContent` enum will distinguish `Text(Vec<String>)`
/// from `Keystrokes(Vec<KeyEvent>)` so that macro registers and text registers
/// can share the same storage cleanly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Register {
    values: Vec<String>,
}

impl Register {
    /// Create a register from a `Vec<String>`.
    pub(crate) fn new(values: Vec<String>) -> Self {
        Self { values }
    }

    /// Borrow the stored strings.
    pub(crate) fn values(&self) -> &[String] {
        &self.values
    }
}

/// The full collection of named registers.
///
/// Each register holds a `Vec<String>` — one entry per selection at yank time.
///
/// Special registers (enforced here):
/// - `DEFAULT_REGISTER` (`'"'`): internal default; all yanks/deletes go here
///   when no register is explicitly named.
/// - `BLACK_HOLE_REGISTER` (`'b'`): writes discarded silently; reads return `None`.
///
/// Named registers `'0'`–`'9'` are user storage. Special registers `'c'`
/// (clipboard), `'s'` (search), and `'q'` (macro) are reserved by constants
/// above; their behaviour is wired in the editor layer (M3).
///
/// A register picker UI (like Helix's) will be added in M3 so users can
/// discover register names and contents without memorising them.
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

    /// Write to a register, replacing its previous contents.
    ///
    /// Writes to the black-hole register (`'b'`) are silently discarded.
    pub(crate) fn write(&mut self, name: char, values: Vec<String>) {
        if name == BLACK_HOLE_REGISTER {
            return;
        }
        self.registers.insert(name, Register::new(values));
    }
}

/// Extract the text of each selection from the buffer, in document order.
///
/// Returns one `String` per selection. This is the content that gets stored in
/// a register on yank or captured before a delete:
///
/// ```ignore
/// let yanked = yank_selections(&buf, &sels);
/// let (new_buf, new_sels) = delete_selection(buf, sels);
/// registers.write(DEFAULT_REGISTER, yanked);
/// ```
///
/// Selections are always inclusive, so the text spans `start()..=end()` —
/// internally `buf.slice(start..end+1)`.
pub(crate) fn yank_selections(buf: &Buffer, sels: &SelectionSet) -> Vec<String> {
    sels.iter_sorted()
        .map(|sel| {
            // end_inclusive() gives the last codepoint of the final grapheme
            // (handles multi-codepoint clusters like e + \u{0301}); +1 converts
            // to an exclusive upper bound for the slice.
            buf.slice(sel.start()..sel.end_inclusive(buf) + 1).to_string()
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
        regs.write(DEFAULT_REGISTER, vec!["hello".to_string()]);
        assert_eq!(regs.read(DEFAULT_REGISTER).unwrap().values(), &["hello"]);
    }

    #[test]
    fn overwrite_replaces_previous() {
        let mut regs = RegisterSet::new();
        regs.write('0', vec!["first".to_string()]);
        regs.write('0', vec!["second".to_string()]);
        assert_eq!(regs.read('0').unwrap().values(), &["second"]);
    }

    #[test]
    fn read_empty_register_returns_none() {
        let regs = RegisterSet::new();
        assert!(regs.read('0').is_none());
    }

    #[test]
    fn black_hole_write_is_discarded() {
        let mut regs = RegisterSet::new();
        regs.write(BLACK_HOLE_REGISTER, vec!["ignored".to_string()]);
        assert!(regs.read(BLACK_HOLE_REGISTER).is_none());
    }

    #[test]
    fn black_hole_read_always_none() {
        let regs = RegisterSet::new();
        assert!(regs.read(BLACK_HOLE_REGISTER).is_none());
    }

    #[test]
    fn named_registers_are_independent() {
        let mut regs = RegisterSet::new();
        regs.write('1', vec!["one".to_string()]);
        regs.write('2', vec!["two".to_string()]);
        assert_eq!(regs.read('1').unwrap().values(), &["one"]);
        assert_eq!(regs.read('2').unwrap().values(), &["two"]);
    }

    #[test]
    fn constants_have_expected_values() {
        // Document the register name choices so a future reader sees them tested.
        assert_eq!(BLACK_HOLE_REGISTER, 'b');
        assert_eq!(CLIPBOARD_REGISTER, 'c');
        assert_eq!(SEARCH_REGISTER,    's');
        assert_eq!(MACRO_REGISTER,     'q');
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
