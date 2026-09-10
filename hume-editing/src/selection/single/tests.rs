use super::*;
use crate::selection::testing::parse_state;
use hume_rope::offset::CharOffset;
use pretty_assertions::assert_eq;

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

// ── ends_on_newline ───────────────────────────────────────────────────────

#[test]
fn ends_on_newline_true_when_end_is_newline() {
    // "ab\n" — select whole first line: anchor=0, head=2 ('\n' at char 2).
    // a=0, b=1, \n=2
    let (text, _) = parse_state("-[ab]>\n");
    let sel = Selection::new(co(0), co(2)); // ends on '\n'
    assert!(sel.ends_on_newline(&text));
}

#[test]
fn ends_on_newline_false_when_end_is_content() {
    // "ab\n" — select only 'a': sel.end() = 0, which is 'a', not '\n'.
    let (text, _) = parse_state("-[a]>b\n");
    let sel = Selection::new(co(0), co(0));
    assert!(!sel.ends_on_newline(&text));
}

#[test]
fn ends_on_newline_structural_newline() {
    // "a\n" — the structural trailing '\n' at char 1.
    // Collapsed cursor on it: sel.end() = 1.
    let (text, _) = parse_state("-[a]>\n");
    let sel = Selection::collapsed(co(1)); // on the structural '\n'
    assert!(sel.ends_on_newline(&text));
}

#[test]
fn ends_on_newline_collapsed_on_empty_line() {
    // "a\n\nb\n" — empty line at char 2 (the sole '\n' of that line).
    // a=0, \n=1, \n=2, b=3, \n=4
    let (text, _) = parse_state("-[a]>\n\nb\n");
    let sel = Selection::collapsed(co(2)); // collapsed on the empty line's '\n'
    assert!(sel.ends_on_newline(&text));
}

// ── content_end ───────────────────────────────────────────────────────────

#[test]
fn content_end_normal_selection() {
    // "abc\n" — select 'a','b': start=0, end=1 (both content chars).
    // content_end = end_inclusive.min(last_content_char) = 1.min(2) = 1.
    let (text, _) = parse_state("-[ab]>c\n");
    let sel = Selection::new(co(0), co(1));
    assert_eq!(sel.content_end(&text), co(1)); // 'b' at char 1
}

#[test]
fn content_end_clamps_at_structural_newline() {
    // "ab\n" — select through the structural '\n' (chars 0-2).
    // end_inclusive = 2, last_content_char = 1 → content_end = 1.
    let (text, _) = parse_state("-[ab]>\n");
    let sel = Selection::new(co(0), co(2)); // end on structural '\n'
    // last_content_char for "ab\n" is 1 ('b'). content_end must clamp.
    assert_eq!(sel.content_end(&text), co(1));
}

#[test]
fn content_end_combining_grapheme() {
    // "e\u{0301}\n" — 'e'(0) + combining acute(1) + '\n'(2), len_chars = 3.
    // sel collapsed at 0: end_inclusive = next_grapheme_boundary(0) - 1 = 2 - 1 = 1.
    // last_content_char = len_chars() - 2 = 1.
    // content_end = min(1, 1) = 1 — the combiner is still content, not the structural '\n'.
    let (text, _) = parse_state("-[e]>\u{0301}\n");
    let sel = Selection::collapsed(co(0));
    // Independent oracle: chars 0 and 1 are content; char 2 is the structural '\n'.
    // content_end must equal 1 (includes the combining codepoint, stops before '\n').
    assert_eq!(sel.content_end(&text), co(1));
}

// ── end_exclusive ────────────────────────────────────────────────────────

#[test]
fn end_exclusive_normal_selection() {
    // "abc\n" — select 'a','b': end() = 1, a single-codepoint grapheme, so
    // end_exclusive is one past it.
    let (text, _) = parse_state("-[ab]>c\n");
    let sel = Selection::new(co(0), co(1));
    assert_eq!(sel.end_exclusive(&text), co(2));
}

#[test]
fn end_exclusive_combining_grapheme() {
    // "e\u{0301}\n" — 'e'(0) + combining acute(1) + '\n'(2). Collapsed at 0
    // sits on a 2-codepoint cluster, so the exclusive bound is one past the
    // whole cluster (char 2), not one past 'e' alone (char 1) — the case
    // the raw `end() + 1` idiom this method replaces would get wrong.
    let (text, _) = parse_state("-[e]>\u{0301}\n");
    let sel = Selection::collapsed(co(0));
    assert_eq!(sel.end_exclusive(&text), co(2));
}

// ── content_end_exclusive ────────────────────────────────────────────────

#[test]
fn content_end_exclusive_normal_selection() {
    // "abc\n" — select 'a','b': content_end_exclusive = end_exclusive(2)
    // clamped to last_char (3, the structural '\n') = 2, unaffected by the
    // clamp here since it's already short of it.
    let (text, _) = parse_state("-[ab]>c\n");
    let sel = Selection::new(co(0), co(1));
    assert_eq!(sel.content_end_exclusive(&text), co(2));
}

#[test]
fn content_end_exclusive_clamps_at_the_structural_newline() {
    // "ab\n" — end on the structural '\n' (char 2): end_exclusive = 3
    // (len_chars), clamped to last_char = 2 so the bound never reaches past
    // the buffer's own end.
    let (text, _) = parse_state("-[ab]>\n");
    let sel = Selection::new(co(0), co(2));
    assert_eq!(sel.content_end_exclusive(&text), co(2));
}

#[test]
fn content_end_exclusive_on_the_minimal_buffer_is_zero_not_one() {
    // "\n" alone (len_chars == 1) — the one case where this genuinely
    // diverges from the `content_end(text) + 1` idiom it replaces.
    // end() = 0, end_inclusive = 0 (the '\n' is its own one-char cluster),
    // end_exclusive = next_grapheme_boundary(0) = 1 = len_chars().
    // last_char() = len_chars() - 1 = 0, so content_end_exclusive = min(1, 0) = 0.
    //
    // The old idiom instead computed content_end(text) + 1: content_end =
    // end_inclusive.min(last_content_char) = 0.min(len_chars().saturating_sub(2))
    // = 0.min(0) = 0, so `content_end + 1` = 1 — one past the structural
    // '\n', which would delete it and trip `ChangeSet::apply`'s
    // `TrailingNewlineMissing`. `content_end_exclusive` correctly refuses
    // that char instead.
    let text = BufferText::from("\n");
    let sel = Selection::collapsed(co(0));
    assert_eq!(sel.content_end_exclusive(&text), co(0));
}

// ── is_selection_linewise ─────────────────────────────────────────────────

#[test]
fn is_selection_linewise_whole_single_line() {
    // "hello\nworld\n" — select all of line 0 (chars 0-5 inclusive, ending on '\n').
    // h=0, e=1, l=2, l=3, o=4, \n=5
    let (text, _) = parse_state("-[hello]>\nworld\n");
    let sel = Selection::new(co(0), co(5)); // starts at line 0 start, ends on '\n'
    assert!(is_selection_linewise(&text, &sel));
}

#[test]
fn is_selection_linewise_whole_multi_line() {
    // "ab\ncd\n" — select lines 0 and 1: chars 0-5.
    // a=0, b=1, \n=2, c=3, d=4, \n=5
    let (text, _) = parse_state("-[ab\ncd]>\n");
    let sel = Selection::new(co(0), co(5)); // spans both lines, ends on '\n'
    assert!(is_selection_linewise(&text, &sel));
}

#[test]
fn is_selection_linewise_false_partial_line_with_newline() {
    // "abc\n" — select 'b','c','\n': start=1 (NOT a line start), end=3 ('\n').
    // This is the key correctness case: a partial line that includes its
    // trailing '\n' must NOT be considered linewise.
    // a=0, b=1, c=2, \n=3
    let (text, _) = parse_state("-[a]>bc\n");
    let sel = Selection::new(co(1), co(3)); // starts mid-line, ends on '\n'
    // Flip condition to verify this test catches the bug: if we only checked
    // ends_on_newline, we'd get true — the is_line_start check prevents that.
    assert!(!is_selection_linewise(&text, &sel));
}

#[test]
fn is_selection_linewise_false_mid_line_selection() {
    // "hello\n" — select 'e','l': start=1, end=2, neither on '\n'.
    let (text, _) = parse_state("-[h]>ello\n");
    let sel = Selection::new(co(1), co(2));
    assert!(!is_selection_linewise(&text, &sel));
}

#[test]
fn is_selection_linewise_collapsed_on_empty_line() {
    // "a\n\nb\n" — collapsed cursor on the empty line (char 2, the '\n').
    // a=0, \n=1, \n=2, b=3, \n=4
    // The empty line's only char IS its '\n', and char 2 is the line start.
    let (text, _) = parse_state("-[a]>\n\nb\n");
    let sel = Selection::collapsed(co(2));
    assert!(is_selection_linewise(&text, &sel));
}

#[test]
fn is_selection_linewise_whole_last_line() {
    // "ab\ncd\n" — select line 1 (chars 3-5: 'c','d','\n').
    // a=0, b=1, \n=2, c=3, d=4, \n=5
    let (text, _) = parse_state("-[ab]>\ncd\n");
    let sel = Selection::new(co(3), co(5)); // starts at line 1 start, ends on structural '\n'
    assert!(is_selection_linewise(&text, &sel));
}

// ── linewise_classification ──────────────────────────────────────────────

#[test]
fn linewise_classification_none_for_a_collapsed_selection_on_an_empty_line() {
    // "a\n\nb\n" — collapsed cursor on the empty line (char 2, the '\n').
    let (text, _) = parse_state("-[a]>\n\nb\n");
    let sel = Selection::collapsed(co(2));
    assert_eq!(linewise_classification(&text, &sel), None);
}

#[test]
fn linewise_classification_some_true_for_a_full_line_selection() {
    let (text, _) = parse_state("-[hello]>\nworld\n");
    let sel = Selection::new(co(0), co(5)); // same fixture as is_selection_linewise_whole_single_line
    assert_eq!(linewise_classification(&text, &sel), Some(true));
}

#[test]
fn linewise_classification_some_false_for_a_collapsed_mid_line_selection() {
    // Collapsed (unlike a plain mid-line selection) but not on an empty
    // line — the guard must only fire when the selection is *also* linewise.
    let (text, _) = parse_state("h-[e]>llo\n");
    let sel = Selection::collapsed(co(1));
    assert_eq!(linewise_classification(&text, &sel), Some(false));
}

// ── Selection ─────────────────────────────────────────────────────────────

#[test]
fn cursor_is_collapsed() {
    let s = Selection::collapsed(co(5));
    assert_eq!(s.anchor, co(5));
    assert_eq!(s.head, co(5));
    assert!(s.is_collapsed());
}

#[test]
fn forward_selection_start_end() {
    let s = Selection::new(co(2), co(7)); // anchor < head → forward
    assert_eq!(s.start(), co(2));
    assert_eq!(s.end(), co(7));
    assert!(!s.is_collapsed());
}

#[test]
fn backward_selection_start_end() {
    let s = Selection::new(co(7), co(2)); // anchor > head → backward
    assert_eq!(s.start(), co(2));
    assert_eq!(s.end(), co(7));
}

#[test]
fn flip_reverses_direction() {
    let fwd = Selection::new(co(2), co(7));
    let bwd = fwd.flip();
    assert_eq!(bwd.anchor, co(7));
    assert_eq!(bwd.head, co(2));
    assert_eq!(fwd.flip().flip(), fwd); // double-flip is identity
}

// ── Selection::directed ───────────────────────────────────────────────────

#[test]
fn directed_forward_places_anchor_at_start() {
    let sel = Selection::directed(co(3), co(7), true);
    assert_eq!(sel.anchor, co(3));
    assert_eq!(sel.head, co(7));
    assert!(!sel.is_collapsed());
}

#[test]
fn directed_backward_places_anchor_at_end() {
    let sel = Selection::directed(co(3), co(7), false);
    assert_eq!(sel.anchor, co(7));
    assert_eq!(sel.head, co(3));
    assert!(!sel.is_collapsed());
}

#[test]
fn directed_cursor_is_same_regardless_of_direction() {
    let fwd = Selection::directed(co(5), co(5), true);
    let bwd = Selection::directed(co(5), co(5), false);
    assert!(fwd.is_collapsed());
    assert!(bwd.is_collapsed());
    assert_eq!(fwd, bwd);
}
