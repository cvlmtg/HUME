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
    // No trailing '\n' — violates the invariant every hume_editing::BufferText
    // upholds by construction. Debug builds must catch this loudly rather
    // than silently returning a wrong content line count.
    content_line_count(&Rope::from_str("a\nb\nc"));
}

#[test]
fn line_breaks_matches_the_workspace_ropey_feature_pin() {
    // Pins the workspace's ropey feature set (Cargo.toml: neither `cr_lines`
    // nor `unicode_lines`) to what it actually makes `Rope::lines()` split
    // on. Cargo feature unification is additive: a future dependency pulling
    // in ropey's defaults would silently widen the break set with no diff to
    // Cargo.toml, and only this test would notice.
    assert_eq!(
        Rope::from_str("a\nb").len_lines(),
        2,
        "LF must be a ropey line break"
    );
    for c in ['\r', '\u{0B}', '\u{0C}', '\u{85}', '\u{2028}', '\u{2029}'] {
        assert_eq!(
            Rope::from_str(&format!("a{c}b")).len_lines(),
            1,
            "{c:?} must NOT be a ropey line break — LF is the only one"
        );
    }
    // CRLF is recognized unconditionally by ropey, but only because of its
    // LF: the pair is one break, so this is still two lines, not three.
    assert_eq!(Rope::from_str("a\r\nb").len_lines(), 2);
}

#[test]
fn strip_line_break_strips_lf() {
    assert_eq!(strip_line_break("hello\n"), "hello");
}

#[test]
fn strip_line_break_leaves_non_lf_unicode_breaks_alone() {
    // None of these terminate a line under this workspace's ropey config —
    // ordinary content, must survive untouched.
    assert_eq!(strip_line_break("hello\r"), "hello\r"); // CR
    assert_eq!(strip_line_break("hello\u{0B}"), "hello\u{0B}"); // VT
    assert_eq!(strip_line_break("hello\u{0C}"), "hello\u{0C}"); // FF
    assert_eq!(strip_line_break("hello\u{85}"), "hello\u{85}"); // NEL
    assert_eq!(strip_line_break("hello\u{2028}"), "hello\u{2028}"); // LS
    assert_eq!(strip_line_break("hello\u{2029}"), "hello\u{2029}"); // PS
}

#[test]
fn strip_line_break_keeps_the_cr_of_a_crlf() {
    // A `\r\n` token loses its `\n` and keeps its `\r` as content — the CR
    // was never a terminator, so nothing normalizes it away here. No live
    // buffer holds one (`hume_editing` normalizes every insertion), so this
    // pins the raw-rope contract, not an editor-visible behavior.
    assert_eq!(strip_line_break("hello\r\n"), "hello\r");
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

// ── leading_indent ────────────────────────────────────────────────────────

#[test]
fn leading_indent_agrees_with_leading_whitespace_end() {
    // The `.0` half must match the standalone function on every case above —
    // it's a thin wrapper over this, not an independent implementation.
    let buf = rope("\t  x\n");
    assert_eq!(
        leading_indent(&buf, 0, 4).0,
        leading_whitespace_end(&buf, 0)
    );
}

#[test]
fn leading_indent_spaces_width_is_char_count() {
    let buf = rope("   x\n");
    assert_eq!(leading_indent(&buf, 0, 4), (3, 3));
}

#[test]
fn leading_indent_tab_width_expands_to_next_stop() {
    // One tab at column 0, tab_width 4 — advances to column 4, not 1.
    let buf = rope("\tx\n");
    assert_eq!(leading_indent(&buf, 0, 4), (1, 4));
}

#[test]
fn leading_indent_mixed_tab_then_spaces_is_not_a_whole_multiple() {
    // Tab (0 -> 4) then 2 spaces (4 -> 6): 6 is not a multiple of tab_width,
    // same off-stop shape `>`/`<` must round-trip on.
    let buf = rope("\t  x\n");
    assert_eq!(leading_indent(&buf, 0, 4), (3, 6));
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

#[test]
fn line_content_end_treats_a_bare_cr_as_content() {
    // "ab\rcd\n" is one line, not two: `\r` is ordinary content here, so the
    // cursor's last landing spot is 'd' (offset 4), not 'b'.
    let buf = rope("ab\rcd\n");
    assert_eq!(line_content_end(&buf, 0), 4);
}

#[test]
fn line_content_end_stops_on_the_cr_of_a_crlf() {
    // "ab\r\ncd\n" — line 0 is "ab\r\n", terminated by the `\n` alone, so
    // the `\r` is the line's own last content char and the cursor lands on
    // it (offset 2), one further than a plain "ab\n" would give.
    let buf = rope("ab\r\ncd\n");
    assert_eq!(line_content_end(&buf, 0), 2);
}

#[test]
fn line_content_end_crlf_only_line_is_not_empty() {
    // "a\n\r\nb\n" — line 1 is "\r\n": one content char (the `\r`) plus its
    // terminator, so the cursor lands on the `\r` (offset 2) as content, not
    // as the empty-line fallback.
    let buf = rope("a\n\r\nb\n");
    assert_eq!(line_content_end(&buf, 1), 2);
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

#[test]
fn is_empty_line_false_for_a_cr_only_line() {
    // "a\n\r\nb\n" — line 1 is "\r\n". The `\r` is content, not part of the
    // terminator, so the line has one char and is not empty.
    let buf = rope("a\n\r\nb\n");
    assert!(!is_empty_line(&buf, 1));
}

// ── char_col_in_line ─────────────────────────────────────────────────────

#[test]
fn char_col_in_line_at_line_start_is_zero() {
    let buf = rope("ab\ncd\n");
    assert_eq!(char_col_in_line(&buf, 0, 0), 0);
}

#[test]
fn char_col_in_line_mid_line() {
    // "ab\ncd\n" — line 1 starts at char offset 3; char offset 4 ('d') is
    // column 1.
    let buf = rope("ab\ncd\n");
    assert_eq!(char_col_in_line(&buf, 1, 4), 1);
}

#[test]
fn char_col_in_line_on_the_lines_own_newline() {
    // Line 0's own '\n' sits at offset 2 — column 2, one past its two
    // content chars.
    let buf = rope("ab\ncd\n");
    assert_eq!(char_col_in_line(&buf, 0, 2), 2);
}

#[test]
fn char_col_in_line_is_the_inverse_of_place_char_column() {
    // Round-trip: a char_col that doesn't overshoot the line comes back
    // unchanged through place_char_column -> char_col_in_line.
    let buf = rope("hello\nworld\n");
    let char_col = 3;
    let pos = place_char_column(&buf, 1, char_col);
    assert_eq!(char_col_in_line(&buf, 1, pos), char_col);
}

// ── advance_byte_point ───────────────────────────────────────────────────

#[test]
fn advance_byte_point_no_newlines() {
    let (row, byte_col) = advance_byte_point(2, 5, "hello");
    assert_eq!(row, 2);
    assert_eq!(byte_col, 10); // 5 + 5
}

#[test]
fn advance_byte_point_with_newlines() {
    let (row, byte_col) = advance_byte_point(1, 3, "foo\nbar\nbaz");
    // 2 newlines → row + 2 = 3; byte_col = "baz".len() = 3
    assert_eq!(row, 3);
    assert_eq!(byte_col, 3);
}

#[test]
fn advance_byte_point_trailing_newline() {
    // Inserted text ends with '\n' — byte_col must be 0.
    let (row, byte_col) = advance_byte_point(0, 0, "foo\n");
    assert_eq!(row, 1);
    assert_eq!(byte_col, 0);
}

// ── place_char_column ────────────────────────────────────────────────────

#[test]
fn place_char_column_within_line() {
    // "hello\nworld\n" — char col 2 of line 1 lands on 'r' (offset 8).
    let buf = rope("hello\nworld\n");
    assert_eq!(place_char_column(&buf, 1, 2), 8);
}

#[test]
fn place_char_column_is_monotonic_across_the_line_end_boundary() {
    // "abc\ndef\n" — line 0 holds 'a','b','c' at chars 0,1,2 with its '\n' at
    // char 3. A char col of exactly 3 is one past the last character, i.e.
    // the newline's own offset, and must clamp back to 'c' like every larger
    // column does. Clamping against `line_end_exclusive` (which counts the
    // '\n') instead put col 3 on the newline while col 4 clamped to 'c' —
    // moving further right moved the cursor left.
    let buf = rope("abc\ndef\n");
    let placed: Vec<usize> = (0..6).map(|col| place_char_column(&buf, 0, col)).collect();
    assert_eq!(placed, vec![0, 1, 2, 2, 2, 2]);
    assert!(
        placed.windows(2).all(|w| w[0] <= w[1]),
        "placement must never move left as the column grows: {placed:?}"
    );
    // An empty line keeps landing on its own '\n' — there the last content
    // position *is* the newline.
    let empty = rope("a\n\nb\n");
    assert_eq!(place_char_column(&empty, 1, 0), 2);
    assert_eq!(place_char_column(&empty, 1, 3), 2);
}

#[test]
fn place_char_column_overshoot_clamps_to_line_content_end() {
    // "hi\nhello\n" — line 0 only has 2 real chars; char col 10 clamps to
    // 'i' (offset 1).
    let buf = rope("hi\nhello\n");
    assert_eq!(place_char_column(&buf, 0, 10), 1);
}

#[test]
fn place_char_column_on_empty_line_lands_on_newline() {
    // "a\n\nb\n" — line 1 is empty; any char column lands on its '\n'
    // (offset 2).
    let buf = rope("a\n\nb\n");
    assert_eq!(place_char_column(&buf, 1, 3), 2);
}

// ── place_grapheme_column ────────────────────────────────────────────────

#[test]
fn place_grapheme_column_within_line() {
    // "hello\nworld\n" — grapheme col 2 of line 1 lands on 'r' (offset 8),
    // same as the char-column case here since every grapheme is one char.
    let buf = rope("hello\nworld\n");
    assert_eq!(place_grapheme_column(&buf, 1, 2), 8);
}

#[test]
fn place_grapheme_column_counts_combining_marks_as_one_column() {
    // "e\u{0301}x\n" — 'e' + combining acute (one grapheme cluster) then 'x'.
    // Column 0 is the cluster start; column 1 is 'x'; char_col would have
    // landed column 1 on the combining mark itself instead.
    let buf = rope("e\u{0301}x\n");
    assert_eq!(place_grapheme_column(&buf, 0, 0), 0);
    assert_eq!(place_grapheme_column(&buf, 0, 1), 2);
}

#[test]
fn place_grapheme_column_overshoot_clamps_to_line_content_end() {
    // "hi\nhello\n" — line 0 only has 2 grapheme clusters; column 10 clamps
    // to 'i' (offset 1).
    let buf = rope("hi\nhello\n");
    assert_eq!(place_grapheme_column(&buf, 0, 10), 1);
}

#[test]
fn place_grapheme_column_zero_is_line_start() {
    let buf = rope("hello\nworld\n");
    assert_eq!(place_grapheme_column(&buf, 1, 0), 6);
}

#[test]
fn place_grapheme_column_on_empty_line_lands_on_newline() {
    // "a\n\nb\n" — line 1 is empty; any grapheme column lands on its '\n'
    // (offset 2).
    let buf = rope("a\n\nb\n");
    assert_eq!(place_grapheme_column(&buf, 1, 3), 2);
}

#[test]
fn line_segments_yields_one_triple_per_line_covered() {
    // "abc\ndef\nghi\n" — a range spanning all of line 0's "abc" through
    // line 2's "gh" covers content on three lines.
    let buf = rope("abc\ndef\nghi\n");
    let start = buf.line_to_char(0);
    let end = buf.line_to_char(2) + 2; // through "gh" on line 2
    let segs: Vec<_> = line_segments(&buf, start, end).collect();
    assert_eq!(segs, vec![(0, 0, 3), (1, 0, 3), (2, 0, 2)]);
}

#[test]
fn line_segments_skips_a_line_the_range_only_touches_at_its_own_newline() {
    // "abc\ndef\n" — a range starting exactly on line 0's own '\n' (char 3,
    // one past 'c') and continuing onto line 1 covers zero chars of line
    // 0's content: an LSP diagnostic anchored at end-of-line looks exactly
    // like this. Only line 1's segment should be yielded — a zero-width
    // (3, 3) triple for line 0 would sort its end before its own start once
    // downstream flattening builds start/end events from it.
    let buf = rope("abc\ndef\n");
    let start = buf.line_to_char(0) + 3; // line 0's own '\n'
    let end = buf.line_to_char(1) + 2; // through "de" on line 1
    let segs: Vec<_> = line_segments(&buf, start, end).collect();
    assert_eq!(segs, vec![(1, 0, 2)]);
}
