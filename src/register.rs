use std::collections::HashMap;

use crate::buffer::Buffer;
use crate::selection::SelectionSet;

/// The default register — receives yanks and deletes when no register is named.
pub(crate) const DEFAULT_REGISTER: char = '"';

/// The black-hole register — writes are silently discarded, reads always return `None`.
pub(crate) const BLACK_HOLE_REGISTER: char = '_';

/// One named register.
///
/// Stores a `Vec<String>` — one string per selection that was active at yank
/// time, in document order. A single-cursor yank produces a `Vec` of length 1.
///
/// The linewise-vs-charwise distinction is not tracked explicitly. At paste
/// time, content that ends with `\n` is treated as linewise. This heuristic
/// covers the common cases and can be promoted to an explicit flag later.
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

    /// Consume the register and return the strings.
    pub(crate) fn into_values(self) -> Vec<String> {
        self.values
    }
}

/// The full collection of named registers.
///
/// Each register holds a `Vec<String>` — one entry per selection at yank time.
///
/// Special registers:
/// - `'"'` (`DEFAULT_REGISTER`): unnamed/default register; all yanks and deletes
///   go here unless an explicit register is named.
/// - `'_'` (`BLACK_HOLE_REGISTER`): writes are discarded silently; reads always
///   return `None`.
///
/// Registers `'a'`–`'z'` are user-named storage. `'+'` (clipboard) is reserved
/// but not yet wired to the OS clipboard — for now it behaves like a regular
/// named register.
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
    /// Writes to the black-hole register (`'_'`) are silently discarded.
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
            // end() is inclusive — add 1 for the exclusive upper bound.
            buf.slice(sel.start()..sel.end() + 1).to_string()
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
        regs.write('"', vec!["hello".to_string()]);
        assert_eq!(regs.read('"').unwrap().values(), &["hello"]);
    }

    #[test]
    fn overwrite_replaces_previous() {
        let mut regs = RegisterSet::new();
        regs.write('a', vec!["first".to_string()]);
        regs.write('a', vec!["second".to_string()]);
        assert_eq!(regs.read('a').unwrap().values(), &["second"]);
    }

    #[test]
    fn read_empty_register_returns_none() {
        let regs = RegisterSet::new();
        assert!(regs.read('a').is_none());
    }

    #[test]
    fn black_hole_write_is_discarded() {
        let mut regs = RegisterSet::new();
        regs.write('_', vec!["ignored".to_string()]);
        assert!(regs.read('_').is_none());
    }

    #[test]
    fn black_hole_read_always_none() {
        let regs = RegisterSet::new();
        assert!(regs.read('_').is_none());
    }

    #[test]
    fn named_registers_are_independent() {
        let mut regs = RegisterSet::new();
        regs.write('a', vec!["alpha".to_string()]);
        regs.write('b', vec!["beta".to_string()]);
        assert_eq!(regs.read('a').unwrap().values(), &["alpha"]);
        assert_eq!(regs.read('b').unwrap().values(), &["beta"]);
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
        // A cursor on 'e' (pos 0) is a 1-char selection — yanks only 'e'.
        let (buf, sels) = parse_state("-[e]>\u{0301}x\n");
        assert_eq!(yank_selections(&buf, &sels), vec!["e"]);
    }

    #[test]
    fn yank_on_structural_newline() {
        // Cursor on the trailing '\n' — captures the newline itself.
        let (buf, sels) = parse_state("hello-[\n]>");
        assert_eq!(yank_selections(&buf, &sels), vec!["\n"]);
    }
}
