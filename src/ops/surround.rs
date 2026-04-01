//! Surround operations: select the delimiter characters of an enclosing pair.
//!
//! `ms` + char selects the surrounding delimiters as two cursor selections,
//! enabling standard select-then-act composition:
//! - `ms(` → `d`  deletes the parens
//! - `ms(` → `r[` replaces `()` with `[]` (via smart replace)
//! - `ms(` → `c`  enters insert with two cursors on the delimiters

use crate::core::buffer::Buffer;
use crate::core::selection::{Selection, SelectionSet};
use crate::ops::text_object::{find_bracket_pair, find_quote_pair};

// ── Pair lookup ──────────────────────────────────────────────────────────────

/// All recognised delimiter pairs.  Asymmetric first, then symmetric.
const PAIRS: &[(char, char)] = &[
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('<', '>'),
    ('"', '"'),
    ('\'', '\''),
    ('`', '`'),
];

/// Return the `(open, close)` pair that contains `ch` (either side).
pub(crate) fn pair_for_char(ch: char) -> Option<(char, char)> {
    PAIRS.iter().find(|&&(o, c)| o == ch || c == ch).copied()
}

/// True if `ch` is the opening char of an asymmetric pair.
pub(crate) fn is_opening(ch: char) -> bool {
    PAIRS.iter().any(|&(o, c)| o != c && o == ch)
}

/// True if `ch` is the closing char of an asymmetric pair.
pub(crate) fn is_closing(ch: char) -> bool {
    PAIRS.iter().any(|&(o, c)| o != c && c == ch)
}

/// True if `ch` is a symmetric delimiter (same char opens and closes).
pub(crate) fn is_symmetric(ch: char) -> bool {
    PAIRS.iter().any(|&(o, c)| o == c && o == ch)
}

// ── Smart replace resolution ─────────────────────────────────────────────────

/// Resolve the effective replacement character for pair-aware replace.
///
/// When the user types `r[` and the cursor sits on `(`, this returns `[`.
/// When the cursor sits on `)`, this returns `]`.  For symmetric source
/// chars (quotes) the selection index breaks the tie: even = opening,
/// odd = closing.
///
/// Returns `replacement` unchanged when:
/// - `replacement` is not part of any known pair, or
/// - `current` is not a known delimiter character.
pub(crate) fn smart_replace_char(replacement: char, current: char, sel_index: usize) -> char {
    let (open, close) = match pair_for_char(replacement) {
        Some(p) => p,
        None => return replacement,
    };

    if is_opening(current) {
        open
    } else if is_closing(current) {
        close
    } else if is_symmetric(current) {
        // Symmetric source (e.g. `"` → `(`): use selection index as
        // tiebreaker.  After `ms"` the first cursor (even index) sits on
        // the opening quote, the second (odd) on the closing quote.
        if sel_index % 2 == 0 { open } else { close }
    } else {
        replacement
    }
}

// ── Select surrounding delimiters ────────────────────────────────────────────

/// Select the surrounding bracket pair as two cursor selections.
///
/// For each selection in `sels`, finds the enclosing `(open, close)` pair
/// and replaces the selection with two cursors — one on the opening
/// delimiter, one on the closing delimiter.  If no pair is found the
/// selection is preserved unchanged (no-op, matching Helix behaviour).
pub(crate) fn select_surround_bracket(
    buf: &Buffer,
    sels: SelectionSet,
    open: char,
    close: char,
) -> SelectionSet {
    select_surround(buf, sels, |b, pos| find_bracket_pair(b, pos, open, close))
}

/// Select the surrounding quote pair as two cursor selections.
///
/// Same semantics as [`select_surround_bracket`] but uses the
/// line-bounded parity scan for symmetric delimiters.
pub(crate) fn select_surround_quote(
    buf: &Buffer,
    sels: SelectionSet,
    quote: char,
) -> SelectionSet {
    select_surround(buf, sels, |b, pos| find_quote_pair(b, pos, quote))
}

/// Shared implementation: map each selection to two cursors on the pair
/// endpoints, or preserve unchanged on no-match.
fn select_surround(
    buf: &Buffer,
    sels: SelectionSet,
    find_pair: impl Fn(&Buffer, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    let primary_idx = sels.primary_index();
    let mut new_sels = Vec::new();
    let mut new_primary = 0;

    for (i, sel) in sels.iter_sorted().enumerate() {
        if let Some((open_pos, close_pos)) = find_pair(buf, sel.head) {
            if i == primary_idx {
                // Primary tracks the opening delimiter of the matched pair.
                new_primary = new_sels.len();
            }
            new_sels.push(Selection::cursor(open_pos));
            new_sels.push(Selection::cursor(close_pos));
        } else {
            if i == primary_idx {
                new_primary = new_sels.len();
            }
            new_sels.push(*sel);
        }
    }

    // Clamp in case the primary fell off (shouldn't happen, but be safe).
    if new_primary >= new_sels.len() {
        new_primary = 0;
    }

    let result = SelectionSet::from_vec(new_sels, new_primary).merge_overlapping();
    result.debug_assert_valid(buf);
    result
}

// ── Macro for generating bracket surround commands ───────────────────────────

macro_rules! surround_bracket_cmds {
    ($name:ident, $open:literal, $close:literal) => {
        pub(crate) fn $name(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
            select_surround_bracket(buf, sels, $open, $close)
        }
    };
}

surround_bracket_cmds!(cmd_surround_paren,   '(', ')');
surround_bracket_cmds!(cmd_surround_bracket, '[', ']');
surround_bracket_cmds!(cmd_surround_brace,   '{', '}');
surround_bracket_cmds!(cmd_surround_angle,   '<', '>');

macro_rules! surround_quote_cmds {
    ($name:ident, $quote:literal) => {
        pub(crate) fn $name(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
            select_surround_quote(buf, sels, $quote)
        }
    };
}

surround_quote_cmds!(cmd_surround_double_quote, '"');
surround_quote_cmds!(cmd_surround_single_quote, '\'');
surround_quote_cmds!(cmd_surround_backtick,     '`');

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::buffer::Buffer;
    use crate::core::selection::{Selection, SelectionSet};

    /// Helper: make a buffer + single-cursor SelectionSet and run a surround
    /// command, returning the resulting selections as `(anchor, head)` pairs.
    fn run_surround(
        text: &str,
        cursor_pos: usize,
        f: impl Fn(&Buffer, SelectionSet) -> SelectionSet,
    ) -> Vec<(usize, usize)> {
        let buf = Buffer::from(text);
        let sels = SelectionSet::single(Selection::cursor(cursor_pos));
        let result = f(&buf, sels);
        result.iter_sorted().map(|s| (s.anchor, s.head)).collect()
    }

    // ── Bracket surround ─────────────────────────────────────────────────────

    #[test]
    fn surround_paren_from_inside() {
        // (hello) — cursor on 'h' (pos 1)
        let sels = run_surround("(hello)\n", 1, cmd_surround_paren);
        assert_eq!(sels, vec![(0, 0), (6, 6)]);
    }

    #[test]
    fn surround_bracket_from_inside() {
        let sels = run_surround("[hello]\n", 3, cmd_surround_bracket);
        assert_eq!(sels, vec![(0, 0), (6, 6)]);
    }

    #[test]
    fn surround_brace_from_on_open() {
        // Cursor ON the opening `{` — should still find the pair.
        let sels = run_surround("{hello}\n", 0, cmd_surround_brace);
        assert_eq!(sels, vec![(0, 0), (6, 6)]);
    }

    #[test]
    fn surround_angle_from_on_close() {
        // Cursor ON the closing `>`.
        let sels = run_surround("<hello>\n", 6, cmd_surround_angle);
        assert_eq!(sels, vec![(0, 0), (6, 6)]);
    }

    #[test]
    fn surround_paren_nested_selects_innermost() {
        // ((hello)) — cursor on 'e' (pos 4), innermost pair is positions 1..7.
        let sels = run_surround("((hello))\n", 4, cmd_surround_paren);
        assert_eq!(sels, vec![(1, 1), (7, 7)]);
    }

    #[test]
    fn surround_no_match_preserves_selection() {
        // No parens at all — cursor stays put.
        let sels = run_surround("hello\n", 2, cmd_surround_paren);
        assert_eq!(sels, vec![(2, 2)]);
    }

    // ── Quote surround ───────────────────────────────────────────────────────

    #[test]
    fn surround_double_quote() {
        let sels = run_surround("\"hello\"\n", 3, cmd_surround_double_quote);
        assert_eq!(sels, vec![(0, 0), (6, 6)]);
    }

    #[test]
    fn surround_single_quote() {
        let sels = run_surround("'hello'\n", 3, cmd_surround_single_quote);
        assert_eq!(sels, vec![(0, 0), (6, 6)]);
    }

    #[test]
    fn surround_backtick() {
        let sels = run_surround("`hello`\n", 3, cmd_surround_backtick);
        assert_eq!(sels, vec![(0, 0), (6, 6)]);
    }

    #[test]
    fn surround_quote_no_match() {
        let sels = run_surround("hello\n", 2, cmd_surround_double_quote);
        assert_eq!(sels, vec![(2, 2)]);
    }

    // ── Multi-cursor ─────────────────────────────────────────────────────────

    #[test]
    fn surround_multi_cursor_different_pairs() {
        // (a) [b] — cursor on 'a' (pos 1) and 'b' (pos 5).
        let buf = Buffer::from("(a) [b]\n");
        let sels = SelectionSet::from_vec(
            vec![Selection::cursor(1), Selection::cursor(5)],
            0,
        );
        let result = cmd_surround_paren(&buf, sels);
        // Only the first cursor is inside parens; second is not.
        // First → cursors on ( and ), second preserved.
        let pairs: Vec<_> = result.iter_sorted().map(|s| (s.anchor, s.head)).collect();
        assert_eq!(pairs, vec![(0, 0), (2, 2), (5, 5)]);
    }

    #[test]
    fn surround_multi_cursor_same_pair_merges() {
        // (hello) — two cursors both inside the same parens (pos 1 and 3).
        let buf = Buffer::from("(hello)\n");
        let sels = SelectionSet::from_vec(
            vec![Selection::cursor(1), Selection::cursor(3)],
            0,
        );
        let result = cmd_surround_paren(&buf, sels);
        // Both produce cursors on (0,0) and (6,6) — merge_overlapping deduplicates.
        let pairs: Vec<_> = result.iter_sorted().map(|s| (s.anchor, s.head)).collect();
        assert_eq!(pairs, vec![(0, 0), (6, 6)]);
    }

    // ── Pair lookup helpers ──────────────────────────────────────────────────

    #[test]
    fn pair_for_char_finds_brackets() {
        assert_eq!(pair_for_char('('), Some(('(', ')')));
        assert_eq!(pair_for_char(')'), Some(('(', ')')));
        assert_eq!(pair_for_char('['), Some(('[', ']')));
        assert_eq!(pair_for_char('"'), Some(('"', '"')));
        assert_eq!(pair_for_char('x'), None);
    }

    #[test]
    fn opening_closing_symmetric_classification() {
        assert!(is_opening('('));
        assert!(is_opening('['));
        assert!(!is_opening(')'));
        assert!(!is_opening('"'));

        assert!(is_closing(')'));
        assert!(is_closing(']'));
        assert!(!is_closing('('));
        assert!(!is_closing('"'));

        assert!(is_symmetric('"'));
        assert!(is_symmetric('\''));
        assert!(is_symmetric('`'));
        assert!(!is_symmetric('('));
        assert!(!is_symmetric(')'));
    }

    // ── Smart replace ────────────────────────────────────────────────────────

    #[test]
    fn smart_replace_opening_to_opening() {
        // ( → [ (current is opening, replacement is opening)
        assert_eq!(smart_replace_char('[', '(', 0), '[');
    }

    #[test]
    fn smart_replace_closing_to_closing() {
        // ) → ] (current is closing, replacement resolves to closing)
        assert_eq!(smart_replace_char('[', ')', 1), ']');
    }

    #[test]
    fn smart_replace_asym_to_sym() {
        // ( → " and ) → "  (asymmetric to symmetric — both become the same char)
        assert_eq!(smart_replace_char('"', '(', 0), '"');
        assert_eq!(smart_replace_char('"', ')', 1), '"');
    }

    #[test]
    fn smart_replace_sym_to_asym_uses_index() {
        // " → ( for even index (opening), " → ) for odd index (closing)
        assert_eq!(smart_replace_char('(', '"', 0), '(');
        assert_eq!(smart_replace_char('(', '"', 1), ')');
    }

    #[test]
    fn smart_replace_sym_to_sym() {
        // " → ' — symmetric to symmetric, both become '
        assert_eq!(smart_replace_char('\'', '"', 0), '\'');
        assert_eq!(smart_replace_char('\'', '"', 1), '\'');
    }

    #[test]
    fn smart_replace_non_delimiter_literal() {
        // Current char is not a delimiter — replacement is literal.
        assert_eq!(smart_replace_char('[', 'x', 0), '[');
    }

    #[test]
    fn smart_replace_non_pair_replacement_literal() {
        // Replacement char is not part of any pair — always literal.
        assert_eq!(smart_replace_char('x', '(', 0), 'x');
    }
}
