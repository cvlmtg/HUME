use super::*;
use hume_test_fixtures::assert_state;
use hume_test_fixtures::testing::parse_state;
use pretty_assertions::assert_eq;

// ── cmd_split_selection_on_newlines ────────────────────────────────────

#[test]
fn split_single_line_is_noop() {
    assert_state!(
        "-[hell]>o\n",
        |(buf, sels)| cmd_split_selection_on_newlines(&buf, sels, 0, MotionMode::Move),
        "-[hell]>o\n"
    );
}

#[test]
fn split_two_line_selection() {
    // "foo\nbar\n", selection from 'f'(0) to 'r'(6) (cross-line forward).
    // "#[foo\nba|r]#\n" → anchor=0, head=6 (cursor on 'r').
    // After split: "foo" on line 0, "bar" on line 1.
    let (buf, sels) = parse_state("-[foo\nbar]>\n");
    let sels_out = cmd_split_selection_on_newlines(&buf, sels, 0, MotionMode::Move);
    // Text unchanged (pure op).
    assert_eq!(buf.to_string(), "foo\nbar\n");
    // Two selections.
    assert_eq!(sels_out.len(), 2);
    let s: Vec<_> = sels_out.iter_sorted().copied().collect();
    // First: covers "foo" on line 0 (offsets 0–2).
    assert_eq!(s[0].start(), 0);
    assert_eq!(s[0].end(), 2);
    // Second: covers "bar" on line 1 (offsets 4–6).
    assert_eq!(s[1].start(), 4);
    assert_eq!(s[1].end(), 6);
    // Primary is first piece of original primary (index 0).
    assert_eq!(sels_out.primary_index(), 0);
}

#[test]
fn split_three_line_selection() {
    // "a\nb\nc\n" — forward selection from 'a' to 'c'.
    let (buf, sels) = parse_state("-[a\nb\nc]>\n");
    let sels_out = cmd_split_selection_on_newlines(&buf, sels, 0, MotionMode::Move);
    assert_eq!(sels_out.len(), 3);
    let s: Vec<_> = sels_out.iter_sorted().copied().collect();
    // Line 0: just 'a' at offset 0.
    assert_eq!(s[0].start(), 0);
    assert_eq!(s[0].end(), 0);
    // Line 1: just 'b' at offset 2.
    assert_eq!(s[1].start(), 2);
    assert_eq!(s[1].end(), 2);
    // Line 2: just 'c' at offset 4.
    assert_eq!(s[2].start(), 4);
    assert_eq!(s[2].end(), 4);
}

#[test]
fn split_cursor_at_newline_is_noop() {
    // A cursor sitting on a newline character is a single-line selection
    // (the \n is part of its line).
    let (buf, sels) = parse_state("foo-[\n]>bar\n");
    let sels_out = cmd_split_selection_on_newlines(&buf, sels, 0, MotionMode::Move);
    assert_eq!(sels_out.len(), 1);
    assert_eq!(sels_out.primary().head(), 3); // still on \n
}

#[test]
fn split_empty_line_in_middle() {
    // "foo\n\nbar\n" — selection from 'f'(0) to 'r'(7) spans 3 lines.
    // Line 0: "foo\n", line 1: "\n" (empty), line 2: "bar\n".
    // Middle piece should be a cursor on the lone '\n' at offset 4.
    let (buf, sels) = parse_state("-[foo\n\nbar]>\n");
    let sels_out = cmd_split_selection_on_newlines(&buf, sels, 0, MotionMode::Move);
    assert_eq!(sels_out.len(), 3);
    let s: Vec<_> = sels_out.iter_sorted().copied().collect();
    // Line 0: "foo" → offsets 0–2.
    assert_eq!(s[0].start(), 0);
    assert_eq!(s[0].end(), 2);
    // Line 1: empty → cursor on '\n' at offset 4.
    assert_eq!(s[1].start(), 4);
    assert_eq!(s[1].end(), 4);
    // Line 2: "bar" → offsets 5–7.
    assert_eq!(s[2].start(), 5);
    assert_eq!(s[2].end(), 7);
}

#[test]
fn split_backward_multi_line_with_empty_line_preserves_direction() {
    // "foo\n\nbar\n" — backward selection spanning 3 lines including an
    // empty one. All 3 pieces must be backward, and the empty-line piece
    // must be a cursor on the '\n'.
    let (buf, sels) = parse_state("<[foo\n\nbar]-\n");
    let sels_out = cmd_split_selection_on_newlines(&buf, sels, 0, MotionMode::Move);
    assert_eq!(sels_out.len(), 3);
    let s: Vec<_> = sels_out.iter_sorted().copied().collect();
    // All pieces must be backward (anchor >= head; cursor is anchor == head).
    assert!(s[0].anchor() >= s[0].head(), "line 0 should be backward");
    assert!(
        s[1].anchor() >= s[1].head(),
        "empty line should be cursor/backward"
    );
    assert!(s[2].anchor() >= s[2].head(), "line 2 should be backward");
    // Empty line: cursor on the lone '\n' at offset 4.
    assert_eq!(s[1].head(), 4);
}

#[test]
fn split_backward_multi_line_preserves_direction() {
    // "foo\nbar\n" — backward selection: anchor=6('r'), head=0('f').
    // Each piece should be backward (anchor > head).
    let (buf, sels) = parse_state("<[foo\nbar]-\n");
    let sels_out = cmd_split_selection_on_newlines(&buf, sels, 0, MotionMode::Move);
    assert_eq!(sels_out.len(), 2);
    let s: Vec<_> = sels_out.iter_sorted().copied().collect();
    // Both pieces should be backward selections.
    assert!(
        s[0].anchor() > s[0].head(),
        "line 0 piece should be backward"
    );
    assert!(
        s[1].anchor() > s[1].head(),
        "line 1 piece should be backward"
    );
}

#[test]
fn split_selection_on_newlines_empty_buffer_is_noop() {
    // Empty buffer: cursor on the single structural '\n'. The cursor's
    // start_line == end_line → single-line branch → kept as-is.
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_split_selection_on_newlines(&buf, sels, 0, MotionMode::Move),
        "-[\n]>"
    );
}

// ── cmd_trim_selection_whitespace ──────────────────────────────────────

#[test]
fn trim_leading_spaces() {
    // "  hello\n", forward selection covering the whole word + leading spaces.
    // "#[  hell|o]#\n" → anchor=0, head=6 (cursor on 'o', offsets:  (0) (1) h(2) e(3) l(4) l(5) o(6)).
    // After trim: start advances past the 2 spaces → start=2, end=6.
    let (buf, sels) = parse_state("-[  hello]>\n");
    let sels_out = cmd_trim_selection_whitespace(&buf, sels, 0, MotionMode::Move);
    assert_eq!(sels_out.primary().start(), 2); // after the two spaces
    assert_eq!(sels_out.primary().end(), 6); // 'o' at offset 6
}

#[test]
fn trim_trailing_spaces() {
    // "hello  \n", forward selection covering "hello  " (with trailing spaces).
    // "#[hello | ]#\n" → anchor=0, head=6 (cursor on second space).
    // After trim: end walks back past 2 spaces → end=4 ('o').
    let (buf, sels) = parse_state("-[hello  ]>\n");
    let sels_out = cmd_trim_selection_whitespace(&buf, sels, 0, MotionMode::Move);
    assert_eq!(sels_out.primary().start(), 0);
    assert_eq!(sels_out.primary().end(), 4); // 'o' at offset 4
}

#[test]
fn trim_all_whitespace_collapses_to_cursor_at_head() {
    // Selection covering only spaces — should collapse to cursor at head.
    let (buf, sels) = parse_state("-[    ]>\n");
    let sels_out = cmd_trim_selection_whitespace(&buf, sels, 0, MotionMode::Move);
    assert!(sels_out.primary().is_collapsed());
    // Head was at offset 3 (the `|` position in DSL).
    assert_eq!(sels_out.primary().head(), 3);
}

#[test]
fn trim_no_whitespace_is_noop() {
    assert_state!(
        "-[hell]>o\n",
        |(buf, sels)| cmd_trim_selection_whitespace(&buf, sels, 0, MotionMode::Move),
        "-[hell]>o\n"
    );
}

#[test]
fn trim_tab_characters() {
    // "\thello\t\n" — selection from tab(0) to tab(6) inclusive.
    // After trim: start=1 ('h'), end=5 ('o').
    // "\thello\t\n": \t(0),h(1),e(2),l(3),l(4),o(5),\t(6),\n(7).
    let (buf, sels) = parse_state("-[\thello]>\t\n");
    let sels_out = cmd_trim_selection_whitespace(&buf, sels, 0, MotionMode::Move);
    assert_eq!(sels_out.primary().start(), 1); // past leading tab
    assert_eq!(sels_out.primary().end(), 5); // 'o'
}

#[test]
fn trim_backward_selection_preserves_direction() {
    // Backward selection covering "  hello\n": anchor=7('\n'), head=0.
    // After trim: spans 'h'(2) to 'o'(6), still backward.
    assert_state!(
        "<[  hello\n]-",
        |(buf, sels)| cmd_trim_selection_whitespace(&buf, sels, 0, MotionMode::Move),
        "  <[hello]-\n"
    );
}

#[test]
fn trim_empty_buffer_collapses() {
    // Only char is '\n' (whitespace) — all-whitespace selection collapses.
    assert_state!(
        "-[\n]>",
        |(buf, sels)| cmd_trim_selection_whitespace(&buf, sels, 0, MotionMode::Move),
        "-[\n]>"
    );
}

// ── select_matches_within ─────────────────────────────────────────────

#[test]
fn select_matches_basic() {
    // Select "ab" within a selection that spans "aababab".
    let (buf, sels) = parse_state("-[aababab]>\n");
    let regex = regex_cursor::engines::meta::Regex::new("ab").unwrap();
    let result = select_matches_within(&buf, &sels, &regex).unwrap();
    // Expect 3 selections: (1,2), (3,4), (5,6)
    assert_eq!(result.len(), 3);
    assert_eq!((result.primary().anchor(), result.primary().head()), (1, 2));
}

#[test]
fn select_matches_no_hits_returns_none() {
    let (buf, sels) = parse_state("-[hello]>\n");
    let regex = regex_cursor::engines::meta::Regex::new("xyz").unwrap();
    assert!(select_matches_within(&buf, &sels, &regex).is_none());
}

#[test]
fn select_matches_bounded_to_selection() {
    // Only matches within the selection range should be found.
    // "ab" appears at (0,1) and (4,5) in "abcdab\n", but selection
    // covers only chars 2..3 ("cd") — no matches.
    let buf = BufferText::from("abcdab\n");
    let sels = SelectionSet::single(Selection::new(2, 3));
    let regex = regex_cursor::engines::meta::Regex::new("ab").unwrap();
    assert!(select_matches_within(&buf, &sels, &regex).is_none());
}

#[test]
fn select_matches_multiple_selections() {
    // Two selections, each containing one "ab".
    let buf = BufferText::from("ab cd ab\n");
    let sel0 = Selection::new(0, 1); // "ab"
    let sel1 = Selection::new(6, 7); // "ab"
    let sels = SelectionSet::from_vec(vec![sel0, sel1], 0);
    let regex = regex_cursor::engines::meta::Regex::new("ab").unwrap();
    let result = select_matches_within(&buf, &sels, &regex).unwrap();
    assert_eq!(result.len(), 2);
}

#[test]
fn select_matches_backward_selection() {
    // Backward selection (anchor > head) should work identically.
    let buf = BufferText::from("aababab\n");
    let sels = SelectionSet::single(Selection::new(6, 0)); // backward
    let regex = regex_cursor::engines::meta::Regex::new("ab").unwrap();
    let result = select_matches_within(&buf, &sels, &regex).unwrap();
    assert_eq!(result.len(), 3);
    assert_eq!((result.primary().anchor(), result.primary().head()), (1, 2));
}

#[test]
fn select_matches_single_char_match() {
    // Single-char regex matches produce cursor-sized selections.
    let (buf, sels) = parse_state("-[abc]>\n");
    let regex = regex_cursor::engines::meta::Regex::new("b").unwrap();
    let result = select_matches_within(&buf, &sels, &regex).unwrap();
    assert_eq!(result.len(), 1);
    let sel = result.primary();
    assert_eq!(sel.anchor(), 1);
    assert_eq!(sel.head(), 1);
    assert!(sel.is_collapsed());
}

#[test]
fn select_matches_combining_grapheme() {
    // "café\n" where 'é' is e + U+0301 (2 codepoints at chars 3,4).
    // Selection covers the whole word. Matching "é" should produce a
    // selection spanning both codepoints (3,4).
    let buf = BufferText::from("caf\u{0065}\u{0301}\n");
    let sels = SelectionSet::single(Selection::new(0, 4));
    let regex = regex_cursor::engines::meta::Regex::new("\u{0065}\u{0301}").unwrap();
    let result = select_matches_within(&buf, &sels, &regex).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!((result.primary().anchor(), result.primary().head()), (3, 4));
}
