use super::*;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::text::BufferText;
use hume_test_fixtures::assert_state;

/// Helper: make a buffer + single-cursor SelectionSet and run a surround
/// command, returning the resulting selections as `(anchor, head)` pairs.
fn run_surround(
    text: &str,
    cursor_pos: usize,
    f: impl Fn(&BufferText, SelectionSet, usize, MotionMode) -> SelectionSet,
) -> Vec<(usize, usize)> {
    let text = BufferText::from(text);
    let sels = SelectionSet::single(Selection::collapsed(cursor_pos));
    let result = f(&text, sels, 0, MotionMode::Move);
    result
        .iter_sorted()
        .map(|s| (s.anchor(), s.head()))
        .collect()
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
    let text = BufferText::from("(a) [b]\n");
    let sels = SelectionSet::from_vec(vec![Selection::collapsed(1), Selection::collapsed(5)], 0);
    let result = cmd_surround_paren(&text, sels, 0, MotionMode::Move);
    // Only the first cursor is inside parens; second is not.
    // First → cursors on ( and ), second preserved.
    let pairs: Vec<_> = result
        .iter_sorted()
        .map(|s| (s.anchor(), s.head()))
        .collect();
    assert_eq!(pairs, vec![(0, 0), (2, 2), (5, 5)]);
}

#[test]
fn surround_multi_cursor_same_pair_merges() {
    // (hello) — two cursors both inside the same parens (pos 1 and 3).
    let text = BufferText::from("(hello)\n");
    let sels = SelectionSet::from_vec(vec![Selection::collapsed(1), Selection::collapsed(3)], 0);
    let result = cmd_surround_paren(&text, sels, 0, MotionMode::Move);
    // Both produce cursors on (0,0) and (6,6) — merge_overlapping deduplicates.
    let pairs: Vec<_> = result
        .iter_sorted()
        .map(|s| (s.anchor(), s.head()))
        .collect();
    assert_eq!(pairs, vec![(0, 0), (6, 6)]);
}

#[test]
fn surround_with_range_selection_uses_head() {
    // (hello) — range selection spanning 'ell' (anchor=2, head=4).
    // find_bracket_pair searches from head (pos 4), finds the enclosing ().
    let text = BufferText::from("(hello)\n");
    let sels = SelectionSet::single(Selection::new(2, 4));
    let result = cmd_surround_paren(&text, sels, 0, MotionMode::Move);
    let pairs: Vec<_> = result
        .iter_sorted()
        .map(|s| (s.anchor(), s.head()))
        .collect();
    assert_eq!(pairs, vec![(0, 0), (6, 6)]);
}

#[test]
fn surround_with_backward_range_selection() {
    // (hello) — backward selection (anchor=4, head=2).
    // head is at pos 2, still inside the parens.
    let text = BufferText::from("(hello)\n");
    let sels = SelectionSet::single(Selection::new(4, 2));
    let result = cmd_surround_paren(&text, sels, 0, MotionMode::Move);
    let pairs: Vec<_> = result
        .iter_sorted()
        .map(|s| (s.anchor(), s.head()))
        .collect();
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

// ── wrap_each_selection ──────────────────────────────────────────────────

#[test]
fn wrap_cursor_selection() {
    assert_state!(
        "-[h]>ello\n",
        |(text, sels)| wrap_each_selection(text, sels, '[', ']'),
        "[h-[]]>ello\n"
    );
}

#[test]
fn wrap_forward_selection() {
    assert_state!(
        "-[hello]>\n",
        |(text, sels)| wrap_each_selection(text, sels, '(', ')'),
        "(hello-[)]>\n"
    );
}

#[test]
fn wrap_backward_selection() {
    assert_state!(
        "<[hello]-\n",
        |(text, sels)| wrap_each_selection(text, sels, '(', ')'),
        "(hello-[)]>\n"
    );
}

#[test]
fn wrap_partial_word() {
    assert_state!(
        "foo -[bar]> baz\n",
        |(text, sels)| wrap_each_selection(text, sels, '[', ']'),
        "foo [bar-[]]> baz\n"
    );
}

#[test]
fn wrap_multi_cursor_selections() {
    assert_state!(
        "-[ab]>c-[de]>f\n",
        |(text, sels)| wrap_each_selection(text, sels, '(', ')'),
        "(ab-[)]>c(de-[)]>f\n"
    );
}

#[test]
fn wrap_multi_line_selection() {
    // Selection spans a newline; the structural trailing `\n` must not be
    // included in the wrap — end_inclusive clamping to len_chars()-2 guards this.
    assert_state!(
        "-[foo\nbar]> baz\n",
        |(text, sels)| wrap_each_selection(text, sels, '"', '"'),
        "\"foo\nbar-[\"]> baz\n"
    );
}
