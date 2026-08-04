use super::*;
use crate::selection::testing::parse_state;

// ── is_line_start ─────────────────────────────────────────────────────────

#[test]
fn is_line_start_buffer_start() {
    // "hello\n" — char 0 is the buffer start, which is a line start.
    let (buf, _) = parse_state("-[h]>ello\n");
    assert!(is_line_start(&buf, &Selection::collapsed(0)));
}

#[test]
fn is_line_start_mid_line_is_false() {
    // "hello\n" — char 2 ('l') is not at a line start.
    let (buf, _) = parse_state("-[h]>ello\n");
    assert!(!is_line_start(&buf, &Selection::collapsed(2)));
}

#[test]
fn is_line_start_second_line_start() {
    // "hi\nbye\n" — line 1 starts at char 3 ('b').
    // h=0, i=1, \n=2, b=3, y=4, e=5, \n=6
    let (buf, _) = parse_state("-[h]>i\nbye\n");
    assert!(is_line_start(&buf, &Selection::collapsed(3)));
    // Verify a non-boundary on line 1 is false (independent oracle: char 4 = 'y').
    assert!(!is_line_start(&buf, &Selection::collapsed(4)));
}

#[test]
fn is_line_start_newline_itself_is_not_line_start() {
    // "hi\n" — the '\n' is at char 2, which is NOT the start of its line
    // (line 0 starts at char 0). This test verifies the function uses line
    // arithmetic rather than just checking the previous char.
    let (buf, _) = parse_state("-[h]>i\n");
    assert!(!is_line_start(&buf, &Selection::collapsed(2))); // '\n' at end of line 0
}

// ── line_end_exclusive ────────────────────────────────────────────────────

#[test]
fn line_end_exclusive_first_line_of_two() {
    // "hello\nworld\n" — line 0 ends exclusive at char 6 (start of "world")
    let (buf, _) = parse_state("-[h]>ello\nworld\n");
    assert_eq!(line_end_exclusive(&buf, 0), 6); // 'h','e','l','l','o','\n' = 6 chars
}

#[test]
fn line_end_exclusive_last_line() {
    // Last line — returns buf.len_chars()
    let (buf, _) = parse_state("-[h]>ello\n");
    // single line: len = 6, line_end_exclusive(0) == len_chars() == 6
    assert_eq!(line_end_exclusive(&buf, 0), buf.len_chars());
}

#[test]
fn line_end_exclusive_empty_line_between() {
    // "a\n\nb\n" — line 1 is empty ("\n"), its exclusive end is char 3
    let (buf, _) = parse_state("-[a]>\n\nb\n");
    // line 0: 'a','\n' = 2 chars → line_end_exclusive(0) = 2
    // line 1: '\n'     = 1 char  → line_end_exclusive(1) = 3
    assert_eq!(line_end_exclusive(&buf, 1), 3);
}

// ── leading_whitespace_end ────────────────────────────────────────────────

#[test]
fn leading_whitespace_end_none() {
    // "foo\n" — no leading whitespace, end is the line start.
    let (buf, _) = parse_state("-[f]>oo\n");
    assert_eq!(leading_whitespace_end(&buf, 0), 0);
}

#[test]
fn leading_whitespace_end_tabs() {
    // "\t\tfoo\n" — 2 tabs, end is char 2 ('f').
    let (buf, _) = parse_state("\t\t-[f]>oo\n");
    assert_eq!(leading_whitespace_end(&buf, 0), 2);
}

#[test]
fn leading_whitespace_end_mixed() {
    // "\t  x\n" — tab + 2 spaces, end is char 3 ('x').
    let (buf, _) = parse_state("\t  -[x]>\n");
    assert_eq!(leading_whitespace_end(&buf, 0), 3);
}

#[test]
fn leading_whitespace_end_whitespace_only_line() {
    // "   \n" — whole line is whitespace; end is the line's exclusive end
    // (the '\n', offset 3), not the buffer end.
    let (buf, _) = parse_state("-[ ]>  \n");
    let line_start = buf.line_to_char(0);
    assert_eq!(leading_whitespace_end(&buf, 0), line_start + 3);
}

#[test]
fn leading_whitespace_end_empty_line_equals_line_start() {
    // "a\n\nb\n" — line 1 is empty ("\n" only); end equals line_start (no
    // whitespace to skip, not line_start + 1).
    let (buf, _) = parse_state("-[a]>\n\nb\n");
    let line_start = buf.line_to_char(1);
    assert_eq!(leading_whitespace_end(&buf, 1), line_start);
}

// ── line_content_end ──────────────────────────────────────────────────────

#[test]
fn line_content_end_normal_line() {
    // "hello\nworld\n" — line 0: last non-newline char is 'o' at offset 4
    let (buf, _) = parse_state("-[h]>ello\nworld\n");
    assert_eq!(line_content_end(&buf, 0), 4);
}

#[test]
fn line_content_end_empty_line_returns_newline_pos() {
    // "hello\n\nworld\n" — line 1 is empty; cursor sits on the '\n'
    let (buf, _) = parse_state("-[h]>ello\n\nworld\n");
    // line 1 starts at char 6, its only char is '\n' → content_end = 6
    assert_eq!(line_content_end(&buf, 1), 6);
}

#[test]
fn line_content_end_single_char_line() {
    // "a\nb\n" — line 0 content end is at 'a' (offset 0)
    let (buf, _) = parse_state("-[a]>\nb\n");
    assert_eq!(line_content_end(&buf, 0), 0);
}

#[test]
fn line_content_end_combining_grapheme_before_newline() {
    // "cafe\u{0301}\n" = c(0) a(1) f(2) e(3) combining_acute(4) \n(5)
    // The grapheme "e\u{0301}" starts at char 3. line_content_end must
    // return 3 (the grapheme cluster start), not 4 (mid-cluster).
    let (buf, _) = parse_state("-[c]>afe\u{0301}\n");
    assert_eq!(line_content_end(&buf, 0), 3);
}

// ── snap_to_grapheme_boundary ─────────────────────────────────────────────

#[test]
fn snap_to_grapheme_boundary_ascii_lands_exactly() {
    let (buf, _) = parse_state("-[h]>ello\n");
    // Target 3 in ASCII — all single-char graphemes, so snap returns 3
    assert_eq!(snap_to_grapheme_boundary(&buf, 0, 3), 3);
}

#[test]
fn snap_to_grapheme_boundary_target_at_line_start() {
    let (buf, _) = parse_state("-[h]>ello\n");
    assert_eq!(snap_to_grapheme_boundary(&buf, 0, 0), 0);
}

#[test]
fn snap_to_grapheme_boundary_target_beyond_line_returns_len_chars() {
    // snap walks forward until `next > target || next == pos`. When target
    // is past all graphemes, the loop walks all the way to len_chars (where
    // next_grapheme_boundary clamps and returns the same position, triggering
    // the `next == pos` stop). The result is len_chars, not the last char.
    // Callers (vertical motion) apply their own clamping to len_chars - 1.
    let (buf, _) = parse_state("-[h]>i\n");
    // "hi\n": h=0, i=1, \n=2; len_chars=3
    assert_eq!(snap_to_grapheme_boundary(&buf, 0, 100), buf.len_chars());
}

#[test]
fn snap_to_grapheme_boundary_mid_cluster_snaps_back() {
    // "e\u{0301}\n" — 'e' + combining acute = one grapheme cluster (2 chars).
    // snap with target=1 (inside the cluster) should return 0 (start of cluster).
    let (buf, _) = parse_state("-[e]>\u{0301}\n");
    // The combining char is at char index 1. target=1 is inside the cluster.
    assert_eq!(snap_to_grapheme_boundary(&buf, 0, 1), 0);
}

// ── is_empty_line ─────────────────────────────────────────────────────────

#[test]
fn is_empty_line_true_for_bare_newline() {
    // "a\n\nb\n" — line 1 is just "\n".
    let (buf, _) = parse_state("-[a]>\n\nb\n");
    assert!(is_empty_line(&buf, 1));
}

#[test]
fn is_empty_line_false_for_content_line() {
    let (buf, _) = parse_state("-[h]>ello\n");
    assert!(!is_empty_line(&buf, 0));
}

#[test]
fn is_empty_line_false_for_whitespace_only_line() {
    // "   \n" — whitespace-only is NOT empty (Helix semantics).
    let (buf, _) = parse_state("-[ ]>  \n");
    assert!(!is_empty_line(&buf, 0));
}

// ── place_column ──────────────────────────────────────────────────────────

#[test]
fn place_column_within_line() {
    // "hello\nworld\n" — col 2 of line 1 lands on 'r' (offset 8).
    let (buf, _) = parse_state("-[h]>ello\nworld\n");
    assert_eq!(place_column(&buf, 1, 2), 8);
}

#[test]
fn place_column_overshoot_clamps_to_line_content_end() {
    // "hi\nhello\n" — line 0 only has 2 real chars; col 10 clamps to 'i' (offset 1).
    let (buf, _) = parse_state("-[h]>i\nhello\n");
    assert_eq!(place_column(&buf, 0, 10), 1);
}

#[test]
fn place_column_on_empty_line_lands_on_newline() {
    // "a\n\nb\n" — line 1 is empty; any column lands on its '\n' (offset 2).
    let (buf, _) = parse_state("-[a]>\n\nb\n");
    assert_eq!(place_column(&buf, 1, 3), 2);
}
