use super::*;
use crate::test_support::rope;

#[test]
fn ropey_line_count_includes_the_phantom_trailing_line() {
    assert_eq!(ropey_line_count(&Rope::from_str("\n")), 2);
    assert_eq!(ropey_line_count(&Rope::from_str("a\nb\nc\n")), 4);
}

#[test]
fn last_ropey_line_is_ropey_line_count_minus_one() {
    assert_eq!(last_ropey_line(&Rope::from_str("\n")), 1);
    assert_eq!(last_ropey_line(&Rope::from_str("a\nb\nc\n")), 3);
}

#[test]
fn ropey_lines_range_is_zero_to_ropey_line_count() {
    assert_eq!(ropey_lines_range(&Rope::from_str("\n")), 0..2);
    assert_eq!(ropey_lines_range(&Rope::from_str("a\nb\nc\n")), 0..4);
}

#[test]
fn content_line_count_excludes_the_phantom_trailing_line() {
    assert_eq!(content_line_count(&Rope::from_str("\n")), 1);
    assert_eq!(content_line_count(&Rope::from_str("a\nb\nc\n")), 3);
}

#[test]
fn last_content_line_is_content_line_count_minus_one() {
    assert_eq!(last_content_line(&Rope::from_str("\n")), 0);
    assert_eq!(last_content_line(&Rope::from_str("a\nb\nc\n")), 2);
}

#[test]
fn content_lines_range_is_zero_to_content_line_count() {
    assert_eq!(content_lines_range(&Rope::from_str("\n")), 0..1);
    assert_eq!(content_lines_range(&Rope::from_str("a\nb\nc\n")), 0..3);
}

#[test]
#[should_panic(expected = "trailing-newline invariant violated")]
fn content_line_count_asserts_the_trailing_newline_invariant() {
    // No trailing '\n' — violates the invariant every hume_editing::Text
    // upholds by construction. Debug builds must catch this loudly rather
    // than silently returning a wrong content line count.
    content_line_count(&Rope::from_str("a\nb\nc"));
}

#[test]
fn strip_line_break_strips_every_unicode_line_break() {
    // Independent oracle: each expected value is a literal, not derived
    // from LINE_BREAKS — a bug that drops one break char from the const
    // still fails this.
    assert_eq!(strip_line_break("hello\n"), "hello");
    assert_eq!(strip_line_break("hello\r"), "hello");
    assert_eq!(strip_line_break("hello\u{0B}"), "hello"); // VT
    assert_eq!(strip_line_break("hello\u{0C}"), "hello"); // FF
    assert_eq!(strip_line_break("hello\u{85}"), "hello"); // NEL
    assert_eq!(strip_line_break("hello\u{2028}"), "hello"); // LS
    assert_eq!(strip_line_break("hello\u{2029}"), "hello"); // PS
}

#[test]
fn strip_line_break_collapses_crlf_in_one_pass() {
    assert_eq!(strip_line_break("hello\r\n"), "hello");
}

#[test]
fn strip_line_break_is_a_no_op_without_a_trailing_break() {
    assert_eq!(strip_line_break("hello"), "hello");
}

// ── line_end_exclusive ────────────────────────────────────────────────────

#[test]
fn line_end_exclusive_first_line_of_two() {
    // "hello\nworld\n" — line 0 ends exclusive at char 6 (start of "world")
    let buf = rope("hello\nworld\n");
    assert_eq!(line_end_exclusive(&buf, 0), 6); // 'h','e','l','l','o','\n' = 6 chars
}

#[test]
fn line_end_exclusive_last_line() {
    // Last line — returns buf.len_chars()
    let buf = rope("hello\n");
    // single line: len = 6, line_end_exclusive(0) == len_chars() == 6
    assert_eq!(line_end_exclusive(&buf, 0), buf.len_chars());
}

#[test]
fn line_end_exclusive_empty_line_between() {
    // "a\n\nb\n" — line 1 is empty ("\n"), its exclusive end is char 3
    let buf = rope("a\n\nb\n");
    // line 0: 'a','\n' = 2 chars → line_end_exclusive(0) = 2
    // line 1: '\n'     = 1 char  → line_end_exclusive(1) = 3
    assert_eq!(line_end_exclusive(&buf, 1), 3);
}

// ── line_break_char ───────────────────────────────────────────────────────
//
// Every expected offset below is hand-counted straight off the source
// string's char positions — never derived by calling `line_end_exclusive`
// (or anything else under test) — so a bug that breaks the `- 1` relationship
// this function replaces can't also fool its own test.

#[test]
fn line_break_char_first_line() {
    // "hello\nworld\n": h=0 e=1 l=2 l=3 o=4 \n=5
    let buf = rope("hello\nworld\n");
    assert_eq!(line_break_char(&buf, 0), 5);
}

#[test]
fn line_break_char_middle_line() {
    // "a\nb\nc\n": a=0 \n=1 b=2 \n=3 c=4 \n=5 — line 1 ("b") breaks at 3.
    let buf = rope("a\nb\nc\n");
    assert_eq!(line_break_char(&buf, 1), 3);
}

#[test]
fn line_break_char_empty_line() {
    // "a\n\nb\n": a=0 \n=1 \n=2 b=3 \n=4 — line 1 is empty, breaks at 2.
    let buf = rope("a\n\nb\n");
    assert_eq!(line_break_char(&buf, 1), 2);
}

#[test]
fn line_break_char_last_content_line() {
    // "a\nb\nc\n": last content line is 2 ("c"), breaks at 5.
    let buf = rope("a\nb\nc\n");
    assert_eq!(line_break_char(&buf, last_content_line(&buf)), 5);
}

#[test]
fn line_break_char_single_line_buffer() {
    // "hello\n": one content line, breaks at 5.
    let buf = rope("hello\n");
    assert_eq!(line_break_char(&buf, 0), 5);
}

#[test]
fn line_break_char_empty_buffer() {
    // "\n": one empty content line, breaks at 0.
    let buf = rope("\n");
    assert_eq!(line_break_char(&buf, 0), 0);
}

#[test]
#[should_panic(expected = "is not a real content line")]
fn line_break_char_asserts_against_the_phantom_trailing_line() {
    // "a\n" has one content line (0); line 1 is the phantom trailing line —
    // line_end_exclusive(1) - 1 would silently return line 0's own '\n'
    // instead of failing, so this must be caught instead of mis-answered.
    let buf = rope("a\n");
    line_break_char(&buf, 1);
}

// ── leading_whitespace_end ────────────────────────────────────────────────

#[test]
fn leading_whitespace_end_none() {
    // "foo\n" — no leading whitespace, end is the line start.
    let buf = rope("foo\n");
    assert_eq!(leading_whitespace_end(&buf, 0), 0);
}

#[test]
fn leading_whitespace_end_tabs() {
    // "\t\tfoo\n" — 2 tabs, end is char 2 ('f').
    let buf = rope("\t\tfoo\n");
    assert_eq!(leading_whitespace_end(&buf, 0), 2);
}

#[test]
fn leading_whitespace_end_mixed() {
    // "\t  x\n" — tab + 2 spaces, end is char 3 ('x').
    let buf = rope("\t  x\n");
    assert_eq!(leading_whitespace_end(&buf, 0), 3);
}

#[test]
fn leading_whitespace_end_whitespace_only_line() {
    // "   \n" — whole line is whitespace; end is the line's exclusive end
    // (the '\n', offset 3), not the buffer end.
    let buf = rope("   \n");
    let line_start = buf.line_to_char(0);
    assert_eq!(leading_whitespace_end(&buf, 0), line_start + 3);
}

#[test]
fn leading_whitespace_end_empty_line_equals_line_start() {
    // "a\n\nb\n" — line 1 is empty ("\n" only); end equals line_start (no
    // whitespace to skip, not line_start + 1).
    let buf = rope("a\n\nb\n");
    let line_start = buf.line_to_char(1);
    assert_eq!(leading_whitespace_end(&buf, 1), line_start);
}

// ── line_content_end ──────────────────────────────────────────────────────

#[test]
fn line_content_end_normal_line() {
    // "hello\nworld\n" — line 0: last non-newline char is 'o' at offset 4
    let buf = rope("hello\nworld\n");
    assert_eq!(line_content_end(&buf, 0), 4);
}

#[test]
fn line_content_end_empty_line_returns_newline_pos() {
    // "hello\n\nworld\n" — line 1 is empty; cursor sits on the '\n'
    let buf = rope("hello\n\nworld\n");
    // line 1 starts at char 6, its only char is '\n' → content_end = 6
    assert_eq!(line_content_end(&buf, 1), 6);
}

#[test]
fn line_content_end_single_char_line() {
    // "a\nb\n" — line 0 content end is at 'a' (offset 0)
    let buf = rope("a\nb\n");
    assert_eq!(line_content_end(&buf, 0), 0);
}

#[test]
fn line_content_end_combining_grapheme_before_newline() {
    // "cafe\u{0301}\n" = c(0) a(1) f(2) e(3) combining_acute(4) \n(5)
    // The grapheme "e\u{0301}" starts at char 3. line_content_end must
    // return 3 (the grapheme cluster start), not 4 (mid-cluster).
    let buf = rope("cafe\u{0301}\n");
    assert_eq!(line_content_end(&buf, 0), 3);
}

// ── snap_to_grapheme_boundary ─────────────────────────────────────────────

#[test]
fn snap_to_grapheme_boundary_ascii_lands_exactly() {
    let buf = rope("hello\n");
    // Target 3 in ASCII — all single-char graphemes, so snap returns 3
    assert_eq!(snap_to_grapheme_boundary(&buf, 0, 3), 3);
}

#[test]
fn snap_to_grapheme_boundary_target_at_line_start() {
    let buf = rope("hello\n");
    assert_eq!(snap_to_grapheme_boundary(&buf, 0, 0), 0);
}

#[test]
fn snap_to_grapheme_boundary_target_beyond_line_returns_len_chars() {
    // snap walks forward until `next > target || next == pos`. When target
    // is past all graphemes, the loop walks all the way to len_chars (where
    // next_grapheme_boundary clamps and returns the same position, triggering
    // the `next == pos` stop). The result is len_chars, not the last char.
    // Callers (vertical motion) apply their own clamping to len_chars - 1.
    let buf = rope("hi\n");
    // "hi\n": h=0, i=1, \n=2; len_chars=3
    assert_eq!(snap_to_grapheme_boundary(&buf, 0, 100), buf.len_chars());
}

#[test]
fn snap_to_grapheme_boundary_mid_cluster_snaps_back() {
    // "e\u{0301}\n" — 'e' + combining acute = one grapheme cluster (2 chars).
    // snap with target=1 (inside the cluster) should return 0 (start of cluster).
    let buf = rope("e\u{0301}\n");
    // The combining char is at char index 1. target=1 is inside the cluster.
    assert_eq!(snap_to_grapheme_boundary(&buf, 0, 1), 0);
}

// ── is_empty_line ─────────────────────────────────────────────────────────

#[test]
fn is_empty_line_true_for_bare_newline() {
    // "a\n\nb\n" — line 1 is just "\n".
    let buf = rope("a\n\nb\n");
    assert!(is_empty_line(&buf, 1));
}

#[test]
fn is_empty_line_false_for_content_line() {
    let buf = rope("hello\n");
    assert!(!is_empty_line(&buf, 0));
}

#[test]
fn is_empty_line_false_for_whitespace_only_line() {
    // "   \n" — whitespace-only is NOT empty (Helix semantics).
    let buf = rope("   \n");
    assert!(!is_empty_line(&buf, 0));
}

// ── place_char_column ────────────────────────────────────────────────────

#[test]
fn place_char_column_within_line() {
    // "hello\nworld\n" — col 2 of line 1 lands on 'r' (offset 8).
    let buf = rope("hello\nworld\n");
    assert_eq!(place_char_column(&buf, 1, 2), 8);
}

#[test]
fn place_char_column_overshoot_clamps_to_line_content_end() {
    // "hi\nhello\n" — line 0 only has 2 real chars; col 10 clamps to 'i' (offset 1).
    let buf = rope("hi\nhello\n");
    assert_eq!(place_char_column(&buf, 0, 10), 1);
}

#[test]
fn place_char_column_on_empty_line_lands_on_newline() {
    // "a\n\nb\n" — line 1 is empty; any column lands on its '\n' (offset 2).
    let buf = rope("a\n\nb\n");
    assert_eq!(place_char_column(&buf, 1, 3), 2);
}

// ── place_column (display-column-aware) ─────────────────────────────────

#[test]
fn place_column_within_line() {
    // "hello\nworld\n" — display col 2 of line 1 lands on 'r' (offset 8).
    let buf = rope("hello\nworld\n");
    assert_eq!(place_column(&buf, 1, 2, 4), 8);
}

#[test]
fn place_column_overshoot_clamps_to_line_content_end() {
    // "hi\nhello\n" — line 0 is 2 columns wide; col 10 clamps to 'i' (offset 1).
    let buf = rope("hi\nhello\n");
    assert_eq!(place_column(&buf, 0, 10, 4), 1);
}

#[test]
fn place_column_on_empty_line_lands_on_newline() {
    // "a\n\nb\n" — line 1 is empty; any column lands on its '\n' (offset 2).
    let buf = rope("a\n\nb\n");
    assert_eq!(place_column(&buf, 1, 3, 4), 2);
}

#[test]
fn place_column_tab_before_target_lands_display_correct() {
    // "\tworld\nhi\n" — tab_width 4: tab occupies cols 0-3, 'w' starts at
    // display col 4. Landing at display col 4 must land on 'w' (offset 1),
    // not char-offset 4 (which would be 'o').
    let buf = rope("\tworld\nhi\n");
    assert_eq!(place_column(&buf, 0, 4, 4), 1);
}

#[test]
fn place_column_wide_cjk_before_target_lands_display_correct() {
    // "\u{6F22}bc\nhi\n" — 漢 (East Asian Wide) occupies display cols 0-1,
    // so 'b' is at display col 2. A char-offset walk would put 'b' at col 1
    // instead — this is the exact bug the display-column split fixes.
    let buf = rope("\u{6F22}bc\nhi\n");
    assert_eq!(place_column(&buf, 0, 2, 4), 1); // lands on 'b'
    assert_eq!(place_column(&buf, 0, 3, 4), 2); // lands on 'c'
}

#[test]
fn place_column_overshoot_past_wide_line_clamps_to_last_char() {
    // "\u{6F22}b\nhi\n" — line 0 is 3 display columns wide (漢=2, b=1). A
    // target col past that clamps to the last real char ('b', offset 1), not
    // char_pos_at_display_col's own "always land on \n" overshoot behavior.
    let buf = rope("\u{6F22}b\nhi\n");
    assert_eq!(place_column(&buf, 0, 10, 4), 1);
}
