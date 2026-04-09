//! Surround operations: select the delimiter characters of an enclosing pair.
//!
//! `ms` + char selects the surrounding delimiters as two cursor selections,
//! enabling standard select-then-act composition:
//! - `ms(` → `d`  deletes the parens
//! - `ms(` → `r[` replaces `()` with `[]` (via smart replace)
//! - `ms(` → `c`  enters insert with two cursors on the delimiters

use crate::core::buffer::Buffer;
use crate::core::selection::{Selection, SelectionSet};
use crate::ops::pair::{find_bracket_pair, find_quote_pair};

// ── Pair lookup ──────────────────────────────────────────────────────────────

/// All recognised delimiter pairs.  Asymmetric first, then symmetric.
///
/// Intentionally a superset of the default auto-pair set: angle brackets
/// (`<>`) are useful for surround-select in markup, but shouldn't auto-close
/// in insert mode where `<` is commonly a comparison operator.
const PAIRS: &[(char, char)] = &[
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('<', '>'),
    ('"', '"'),
    ('\'', '\''),
    ('`', '`'),
];

fn pair_for_char(ch: char) -> Option<(char, char)> {
    PAIRS.iter().find(|&&(o, c)| o == ch || c == ch).copied()
}

fn is_opening(ch: char) -> bool {
    PAIRS.iter().any(|&(o, c)| o != c && o == ch)
}

fn is_closing(ch: char) -> bool {
    PAIRS.iter().any(|&(o, c)| o != c && c == ch)
}

fn is_symmetric(ch: char) -> bool {
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
        if sel_index.is_multiple_of(2) { open } else { close }
    } else {
        replacement
    }
}

// ── Select surrounding delimiters ────────────────────────────────────────────

/// Shared implementation: map each selection to two cursors on the pair
/// endpoints, or preserve unchanged on no-match.
fn select_surround(
    buf: &Buffer,
    sels: SelectionSet,
    find_pair: impl Fn(&Buffer, usize) -> Option<(usize, usize)>,
) -> SelectionSet {
    let primary_idx = sels.primary_index();
    let mut new_sels = Vec::with_capacity(sels.len() * 2);
    let mut new_primary = 0;

    for (i, sel) in sels.iter_sorted().enumerate() {
        if i == primary_idx {
            new_primary = new_sels.len();
        }
        if let Some((open_pos, close_pos)) = find_pair(buf, sel.head) {
            new_sels.push(Selection::collapsed(open_pos));
            new_sels.push(Selection::collapsed(close_pos));
        } else {
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

// ── Generated surround commands ──────────────────────────────────────────────

macro_rules! surround_cmd {
    ($name:ident, bracket, $open:literal, $close:literal) => {
        pub(crate) fn $name(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
            select_surround(buf, sels, |b, pos| find_bracket_pair(b, pos, $open, $close))
        }
    };
    ($name:ident, quote, $quote:literal) => {
        pub(crate) fn $name(buf: &Buffer, sels: SelectionSet) -> SelectionSet {
            select_surround(buf, sels, |b, pos| find_quote_pair(b, pos, $quote))
        }
    };
}

surround_cmd!(cmd_surround_paren,        bracket, '(', ')');
surround_cmd!(cmd_surround_bracket,      bracket, '[', ']');
surround_cmd!(cmd_surround_brace,        bracket, '{', '}');
surround_cmd!(cmd_surround_angle,        bracket, '<', '>');
surround_cmd!(cmd_surround_double_quote, quote,   '"');
surround_cmd!(cmd_surround_single_quote, quote,   '\'');
surround_cmd!(cmd_surround_backtick,     quote,   '`');

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
        let sels = SelectionSet::single(Selection::collapsed(cursor_pos));
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
            vec![Selection::collapsed(1), Selection::collapsed(5)],
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
            vec![Selection::collapsed(1), Selection::collapsed(3)],
            0,
        );
        let result = cmd_surround_paren(&buf, sels);
        // Both produce cursors on (0,0) and (6,6) — merge_overlapping deduplicates.
        let pairs: Vec<_> = result.iter_sorted().map(|s| (s.anchor, s.head)).collect();
        assert_eq!(pairs, vec![(0, 0), (6, 6)]);
    }

    #[test]
    fn surround_with_range_selection_uses_head() {
        // (hello) — range selection spanning 'ell' (anchor=2, head=4).
        // find_bracket_pair searches from head (pos 4), finds the enclosing ().
        let buf = Buffer::from("(hello)\n");
        let sels = SelectionSet::single(Selection::new(2, 4));
        let result = cmd_surround_paren(&buf, sels);
        let pairs: Vec<_> = result.iter_sorted().map(|s| (s.anchor, s.head)).collect();
        assert_eq!(pairs, vec![(0, 0), (6, 6)]);
    }

    #[test]
    fn surround_with_backward_range_selection() {
        // (hello) — backward selection (anchor=4, head=2).
        // head is at pos 2, still inside the parens.
        let buf = Buffer::from("(hello)\n");
        let sels = SelectionSet::single(Selection::new(4, 2));
        let result = cmd_surround_paren(&buf, sels);
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
        assert_eq!(smart_replace_char('[', '(', 0), '[');
    }

    #[test]
    fn smart_replace_closing_to_closing() {
        assert_eq!(smart_replace_char('[', ')', 1), ']');
    }

    #[test]
    fn smart_replace_asym_to_sym() {
        assert_eq!(smart_replace_char('"', '(', 0), '"');
        assert_eq!(smart_replace_char('"', ')', 1), '"');
    }

    #[test]
    fn smart_replace_sym_to_asym_uses_index() {
        assert_eq!(smart_replace_char('(', '"', 0), '(');
        assert_eq!(smart_replace_char('(', '"', 1), ')');
    }

    #[test]
    fn smart_replace_sym_to_sym() {
        assert_eq!(smart_replace_char('\'', '"', 0), '\'');
        assert_eq!(smart_replace_char('\'', '"', 1), '\'');
    }

    #[test]
    fn smart_replace_non_delimiter_literal() {
        assert_eq!(smart_replace_char('[', 'x', 0), '[');
    }

    #[test]
    fn smart_replace_non_pair_replacement_literal() {
        assert_eq!(smart_replace_char('x', '(', 0), 'x');
    }
}
