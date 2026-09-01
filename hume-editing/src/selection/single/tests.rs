use super::*;
use crate::selection::testing::parse_state;
use pretty_assertions::assert_eq;

// ── ends_on_newline ───────────────────────────────────────────────────────

#[test]
fn ends_on_newline_true_when_end_is_newline() {
    // "ab\n" — select whole first line: anchor=0, head=2 ('\n' at char 2).
    // a=0, b=1, \n=2
    let (text, _) = parse_state("-[ab]>\n");
    let sel = Selection::new(0, 2); // ends on '\n'
    assert!(sel.ends_on_newline(&text));
}

#[test]
fn ends_on_newline_false_when_end_is_content() {
    // "ab\n" — select only 'a': sel.end() = 0, which is 'a', not '\n'.
    let (text, _) = parse_state("-[a]>b\n");
    let sel = Selection::new(0, 0);
    assert!(!sel.ends_on_newline(&text));
}

#[test]
fn ends_on_newline_structural_newline() {
    // "a\n" — the structural trailing '\n' at char 1.
    // Collapsed cursor on it: sel.end() = 1.
    let (text, _) = parse_state("-[a]>\n");
    let sel = Selection::collapsed(1); // on the structural '\n'
    assert!(sel.ends_on_newline(&text));
}

#[test]
fn ends_on_newline_collapsed_on_empty_line() {
    // "a\n\nb\n" — empty line at char 2 (the sole '\n' of that line).
    // a=0, \n=1, \n=2, b=3, \n=4
    let (text, _) = parse_state("-[a]>\n\nb\n");
    let sel = Selection::collapsed(2); // collapsed on the empty line's '\n'
    assert!(sel.ends_on_newline(&text));
}

// ── content_end ───────────────────────────────────────────────────────────

#[test]
fn content_end_normal_selection() {
    // "abc\n" — select 'a','b': start=0, end=1 (both content chars).
    // content_end = end_inclusive.min(last_content_char) = 1.min(2) = 1.
    let (text, _) = parse_state("-[ab]>c\n");
    let sel = Selection::new(0, 1);
    assert_eq!(sel.content_end(&text), 1); // 'b' at char 1
}

#[test]
fn content_end_clamps_at_structural_newline() {
    // "ab\n" — select through the structural '\n' (chars 0-2).
    // end_inclusive = 2, last_content_char = 1 → content_end = 1.
    let (text, _) = parse_state("-[ab]>\n");
    let sel = Selection::new(0, 2); // end on structural '\n'
    // last_content_char for "ab\n" is 1 ('b'). content_end must clamp.
    assert_eq!(sel.content_end(&text), 1);
}

#[test]
fn content_end_combining_grapheme() {
    // "e\u{0301}\n" — 'e'(0) + combining acute(1) + '\n'(2), len_chars = 3.
    // sel collapsed at 0: end_inclusive = next_grapheme_boundary(0) - 1 = 2 - 1 = 1.
    // last_content_char = len_chars() - 2 = 1.
    // content_end = min(1, 1) = 1 — the combiner is still content, not the structural '\n'.
    let (text, _) = parse_state("-[e]>\u{0301}\n");
    let sel = Selection::collapsed(0);
    // Independent oracle: chars 0 and 1 are content; char 2 is the structural '\n'.
    // content_end must equal 1 (includes the combining codepoint, stops before '\n').
    assert_eq!(sel.content_end(&text), 1);
}

// ── is_selection_linewise ─────────────────────────────────────────────────

#[test]
fn is_selection_linewise_whole_single_line() {
    // "hello\nworld\n" — select all of line 0 (chars 0-5 inclusive, ending on '\n').
    // h=0, e=1, l=2, l=3, o=4, \n=5
    let (text, _) = parse_state("-[hello]>\nworld\n");
    let sel = Selection::new(0, 5); // starts at line 0 start, ends on '\n'
    assert!(is_selection_linewise(&text, &sel));
}

#[test]
fn is_selection_linewise_whole_multi_line() {
    // "ab\ncd\n" — select lines 0 and 1: chars 0-5.
    // a=0, b=1, \n=2, c=3, d=4, \n=5
    let (text, _) = parse_state("-[ab\ncd]>\n");
    let sel = Selection::new(0, 5); // spans both lines, ends on '\n'
    assert!(is_selection_linewise(&text, &sel));
}

#[test]
fn is_selection_linewise_false_partial_line_with_newline() {
    // "abc\n" — select 'b','c','\n': start=1 (NOT a line start), end=3 ('\n').
    // This is the key correctness case: a partial line that includes its
    // trailing '\n' must NOT be considered linewise.
    // a=0, b=1, c=2, \n=3
    let (text, _) = parse_state("-[a]>bc\n");
    let sel = Selection::new(1, 3); // starts mid-line, ends on '\n'
    // Flip condition to verify this test catches the bug: if we only checked
    // ends_on_newline, we'd get true — the is_line_start check prevents that.
    assert!(!is_selection_linewise(&text, &sel));
}

#[test]
fn is_selection_linewise_false_mid_line_selection() {
    // "hello\n" — select 'e','l': start=1, end=2, neither on '\n'.
    let (text, _) = parse_state("-[h]>ello\n");
    let sel = Selection::new(1, 2);
    assert!(!is_selection_linewise(&text, &sel));
}

#[test]
fn is_selection_linewise_collapsed_on_empty_line() {
    // "a\n\nb\n" — collapsed cursor on the empty line (char 2, the '\n').
    // a=0, \n=1, \n=2, b=3, \n=4
    // The empty line's only char IS its '\n', and char 2 is the line start.
    let (text, _) = parse_state("-[a]>\n\nb\n");
    let sel = Selection::collapsed(2);
    assert!(is_selection_linewise(&text, &sel));
}

#[test]
fn is_selection_linewise_whole_last_line() {
    // "ab\ncd\n" — select line 1 (chars 3-5: 'c','d','\n').
    // a=0, b=1, \n=2, c=3, d=4, \n=5
    let (text, _) = parse_state("-[ab]>\ncd\n");
    let sel = Selection::new(3, 5); // starts at line 1 start, ends on structural '\n'
    assert!(is_selection_linewise(&text, &sel));
}

// ── linewise_classification ──────────────────────────────────────────────

#[test]
fn linewise_classification_none_for_a_collapsed_selection_on_an_empty_line() {
    // "a\n\nb\n" — collapsed cursor on the empty line (char 2, the '\n').
    // Ambiguous: satisfies is_selection_linewise by construction (the
    // line's one char is both its own start and its own '\n'), not because
    // it was deliberately extended across a whole line.
    let (text, _) = parse_state("-[a]>\n\nb\n");
    let sel = Selection::collapsed(2);
    assert_eq!(linewise_classification(&text, &sel), None);
}

#[test]
fn linewise_classification_some_true_for_a_full_line_selection() {
    let (text, _) = parse_state("-[hello]>\nworld\n");
    let sel = Selection::new(0, 5); // same fixture as is_selection_linewise_whole_single_line
    assert_eq!(linewise_classification(&text, &sel), Some(true));
}

#[test]
fn linewise_classification_some_false_for_a_mid_line_selection() {
    let (text, _) = parse_state("-[h]>ello\n");
    let sel = Selection::new(1, 2); // same fixture as is_selection_linewise_false_mid_line_selection
    assert_eq!(linewise_classification(&text, &sel), Some(false));
}

// ── Selection ─────────────────────────────────────────────────────────────

#[test]
fn cursor_is_collapsed() {
    let s = Selection::collapsed(5);
    assert_eq!(s.anchor, 5);
    assert_eq!(s.head, 5);
    assert!(s.is_collapsed());
}

#[test]
fn forward_selection_start_end() {
    let s = Selection::new(2, 7); // anchor < head → forward
    assert_eq!(s.start(), 2);
    assert_eq!(s.end(), 7);
    assert!(!s.is_collapsed());
}

#[test]
fn backward_selection_start_end() {
    let s = Selection::new(7, 2); // anchor > head → backward
    assert_eq!(s.start(), 2);
    assert_eq!(s.end(), 7);
}

#[test]
fn flip_reverses_direction() {
    let fwd = Selection::new(2, 7);
    let bwd = fwd.flip();
    assert_eq!(bwd.anchor, 7);
    assert_eq!(bwd.head, 2);
    assert_eq!(fwd.flip().flip(), fwd); // double-flip is identity
}

// ── Selection::directed ───────────────────────────────────────────────────

#[test]
fn directed_forward_places_anchor_at_start() {
    let sel = Selection::directed(3, 7, true);
    assert_eq!(sel.anchor, 3);
    assert_eq!(sel.head, 7);
    assert!(!sel.is_collapsed());
}

#[test]
fn directed_backward_places_anchor_at_end() {
    let sel = Selection::directed(3, 7, false);
    assert_eq!(sel.anchor, 7);
    assert_eq!(sel.head, 3);
    assert!(!sel.is_collapsed());
}

#[test]
fn directed_cursor_is_same_regardless_of_direction() {
    let fwd = Selection::directed(5, 5, true);
    let bwd = Selection::directed(5, 5, false);
    assert!(fwd.is_collapsed());
    assert!(bwd.is_collapsed());
    assert_eq!(fwd, bwd);
}
