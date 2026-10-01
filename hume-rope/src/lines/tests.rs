use super::*;
use crate::column::BufferLineCol;
use crate::test_support::rope;
use unicode_segmentation::UnicodeSegmentation;

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

fn cc(n: usize) -> CharCol {
    CharCol::new(n)
}

fn gc(n: usize) -> GraphemeCol {
    GraphemeCol::new(n)
}

fn bc(n: usize) -> ByteCol {
    ByteCol::new(n)
}

#[test]
fn ropey_line_count_includes_the_phantom_trailing_line() {
    assert_eq!(ropey_line_count(&Rope::from_str("\n")).get(), 2);
    assert_eq!(ropey_line_count(&Rope::from_str("a\nb\nc\n")).get(), 4);
}

#[test]
fn last_ropey_line_is_ropey_line_count_minus_one() {
    assert_eq!(last_ropey_line(&Rope::from_str("\n")).index(), 1);
    assert_eq!(last_ropey_line(&Rope::from_str("a\nb\nc\n")).index(), 3);
}

#[test]
fn content_line_count_excludes_the_phantom_trailing_line() {
    assert_eq!(content_line_count(&Rope::from_str("\n")).get(), 1);
    assert_eq!(content_line_count(&Rope::from_str("a\nb\nc\n")).get(), 3);
}

#[test]
fn last_content_line_is_content_line_count_minus_one() {
    assert_eq!(last_content_line(&Rope::from_str("\n")).index(), 0);
    assert_eq!(last_content_line(&Rope::from_str("a\nb\nc\n")).index(), 2);
}

#[test]
#[should_panic(expected = "trailing-newline invariant violated")]
fn content_line_count_asserts_the_trailing_newline_invariant() {
    // No trailing '\n': violates the invariant every hume_editing::BufferText
    // upholds. Debug builds must catch this loudly rather
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
            "{c:?} must NOT be a ropey line break, LF is the only one"
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
    // None of these terminate a line under this workspace's ropey config.
    // They are ordinary content and must survive untouched.
    assert_eq!(strip_line_break("hello\r"), "hello\r"); // CR
    assert_eq!(strip_line_break("hello\u{0B}"), "hello\u{0B}"); // VT
    assert_eq!(strip_line_break("hello\u{0C}"), "hello\u{0C}"); // FF
    assert_eq!(strip_line_break("hello\u{85}"), "hello\u{85}"); // NEL
    assert_eq!(strip_line_break("hello\u{2028}"), "hello\u{2028}"); // LS
    assert_eq!(strip_line_break("hello\u{2029}"), "hello\u{2029}"); // PS
}

#[test]
fn strip_line_break_keeps_the_cr_of_a_crlf() {
    // A `\r\n` token loses its `\n` and keeps its `\r` as content. The CR
    // was never a terminator, so nothing normalizes it away here. No live
    // buffer holds one (`hume_editing` normalizes every insertion), so this
    // pins the raw-rope contract, not an editor-visible behavior.
    assert_eq!(strip_line_break("hello\r\n"), "hello\r");
}

#[test]
fn strip_line_break_is_a_no_op_without_a_trailing_break() {
    assert_eq!(strip_line_break("hello"), "hello");
}

// ── next_line_start ───────────────────────────────────────────────────────

#[test]
fn next_line_start_first_line_of_two() {
    // "hello\nworld\n": line 0 ends exclusive at char 6 (start of "world")
    let buf = rope("hello\nworld\n");
    assert_eq!(next_line_start(&buf, RopeyLine::new(0)), co(6)); // 'h','e','l','l','o','\n' = 6 chars
}

#[test]
fn next_line_start_last_line() {
    // Last line: returns buf.len_chars()
    let buf = rope("hello\n");
    // single line: len = 6, next_line_start(0) == len_chars() == 6
    assert_eq!(
        next_line_start(&buf, RopeyLine::new(0)),
        co(buf.len_chars())
    );
}

#[test]
fn next_line_start_empty_line_between() {
    // "a\n\nb\n": line 1 is empty ("\n"), its exclusive end is char 3
    let buf = rope("a\n\nb\n");
    // line 0: 'a','\n' = 2 chars → next_line_start(0) = 2
    // line 1: '\n'     = 1 char  → next_line_start(1) = 3
    assert_eq!(next_line_start(&buf, RopeyLine::new(1)), co(3));
}

// ── line_break ────────────────────────────────────────────────────────────
//
// Every expected offset below is hand-counted from the source string's char
// positions, never derived by calling another function under test.

#[test]
fn line_break_first_line() {
    // "hello\nworld\n": h=0 e=1 l=2 l=3 o=4 \n=5
    let buf = rope("hello\nworld\n");
    assert_eq!(line_break(&buf, ContentLine::new(0)).offset(), co(5));
}

#[test]
fn line_break_middle_line() {
    // "a\nb\nc\n": a=0 \n=1 b=2 \n=3 c=4 \n=5. Line 1 ("b") breaks at 3.
    let buf = rope("a\nb\nc\n");
    assert_eq!(line_break(&buf, ContentLine::new(1)).offset(), co(3));
}

#[test]
fn line_break_empty_line() {
    // "a\n\nb\n": a=0 \n=1 \n=2 b=3 \n=4. Line 1 is empty, breaks at 2.
    let buf = rope("a\n\nb\n");
    assert_eq!(line_break(&buf, ContentLine::new(1)).offset(), co(2));
}

#[test]
fn line_break_last_content_line() {
    // "a\nb\nc\n": last content line is 2 ("c"), breaks at 5.
    let buf = rope("a\nb\nc\n");
    assert_eq!(line_break(&buf, last_content_line(&buf)).offset(), co(5));
}

#[test]
fn line_break_single_line_buffer() {
    // "hello\n": one content line, breaks at 5.
    let buf = rope("hello\n");
    assert_eq!(line_break(&buf, ContentLine::new(0)).offset(), co(5));
}

#[test]
fn line_break_empty_buffer() {
    // "\n": one empty content line, breaks at 0.
    let buf = rope("\n");
    assert_eq!(line_break(&buf, ContentLine::new(0)).offset(), co(0));
}

#[test]
#[should_panic(expected = "is not a real content line")]
fn line_break_asserts_against_the_phantom_trailing_line() {
    // "a\n" has one content line (0); line 1 is the phantom trailing line, where
    // next_line_start(1) - 1 would silently return line 0's own '\n'
    // instead of failing, so this must be caught instead of mis-answered.
    // `ContentLine::new` doesn't itself validate against a rope: the whole
    // point of this test is that the runtime assert still catches a value
    // minted for a line the type says is content but isn't, for this buffer.
    let buf = rope("a\n");
    line_break(&buf, ContentLine::new(1));
}

// ── leading_whitespace_end ────────────────────────────────────────────────

#[test]
fn leading_whitespace_end_none() {
    // "foo\n": no leading whitespace, end is the line start.
    let buf = rope("foo\n");
    assert_eq!(
        leading_whitespace_end(&buf, ContentLine::new(0)).offset(),
        co(0)
    );
}

#[test]
fn leading_whitespace_end_tabs() {
    // "\t\tfoo\n": 2 tabs, end is char 2 ('f').
    let buf = rope("\t\tfoo\n");
    assert_eq!(
        leading_whitespace_end(&buf, ContentLine::new(0)).offset(),
        co(2)
    );
}

#[test]
fn leading_whitespace_end_mixed() {
    // "\t  x\n": tab + 2 spaces, end is char 3 ('x').
    let buf = rope("\t  x\n");
    assert_eq!(
        leading_whitespace_end(&buf, ContentLine::new(0)).offset(),
        co(3)
    );
}

#[test]
fn leading_whitespace_end_whitespace_only_line() {
    // "   \n": whole line is whitespace; end is the line's exclusive end
    // (the '\n', offset 3), not the buffer end.
    let buf = rope("   \n");
    let line_start = line_start_char(&buf, RopeyLine::new(0)).index();
    assert_eq!(
        leading_whitespace_end(&buf, ContentLine::new(0)).offset(),
        co(line_start + 3)
    );
}

#[test]
fn leading_whitespace_end_empty_line_equals_line_start() {
    // "a\n\nb\n": line 1 is empty ("\n" only); end equals line_start (no
    // whitespace to skip, not line_start + 1).
    let buf = rope("a\n\nb\n");
    let line_start = line_start_char(&buf, RopeyLine::new(1)).index();
    assert_eq!(
        leading_whitespace_end(&buf, ContentLine::new(1)).offset(),
        co(line_start)
    );
}

#[test]
fn leading_whitespace_end_stops_at_the_cluster_after_a_marked_space() {
    // A combining mark on the space makes `" \u{301}"` one cluster, so the
    // run ends at `x`, never between the space and its mark.
    let buf = rope(" \u{301}x\n");
    assert_eq!(
        leading_whitespace_end(&buf, ContentLine::new(0)).offset(),
        co(2)
    );
}

#[test]
fn leading_whitespace_end_skips_marked_and_plain_spaces() {
    let buf = rope(" \u{301}  x\n");
    assert_eq!(
        leading_whitespace_end(&buf, ContentLine::new(0)).offset(),
        co(4)
    );
}

// ── leading_indent ────────────────────────────────────────────────────────

#[test]
fn leading_indent_counts_non_breaking_and_ideographic_spaces() {
    // NBSP is 1 cell, U+3000 is 2, then a tab from column 3 to the stop at 4.
    let buf = rope("\u{a0}\u{3000}\tx\n");
    assert_eq!(
        {
            let (end, width) = leading_indent(&buf, ContentLine::new(0), 4);
            (end.offset(), width)
        },
        (co(3), BufferLineCol::new(4))
    );
}

#[test]
fn leading_whitespace_end_keeps_a_marked_non_breaking_space_whole() {
    let buf = rope("\u{a0}\u{301}x\n");
    assert_eq!(
        leading_whitespace_end(&buf, ContentLine::new(0)).offset(),
        co(2)
    );
}

#[test]
fn leading_indent_agrees_with_leading_whitespace_end() {
    // The `.0` half must match the standalone function on every case above.
    // It's a thin wrapper over this, not an independent implementation.
    let buf = rope("\t  x\n");
    assert_eq!(
        leading_indent(&buf, ContentLine::new(0), 4).0,
        leading_whitespace_end(&buf, ContentLine::new(0))
    );
}

#[test]
fn leading_indent_spaces_width_is_char_count() {
    let buf = rope("   x\n");
    assert_eq!(
        {
            let (end, width) = leading_indent(&buf, ContentLine::new(0), 4);
            (end.offset(), width)
        },
        (co(3), BufferLineCol::new(3))
    );
}

#[test]
fn leading_indent_tab_width_expands_to_next_stop() {
    // One tab at column 0, tab_width 4, advances to column 4, not 1.
    let buf = rope("\tx\n");
    assert_eq!(
        {
            let (end, width) = leading_indent(&buf, ContentLine::new(0), 4);
            (end.offset(), width)
        },
        (co(1), BufferLineCol::new(4))
    );
}

#[test]
fn leading_indent_mixed_tab_then_spaces_is_not_a_whole_multiple() {
    // Tab (0 -> 4) then 2 spaces (4 -> 6): 6 is not a multiple of tab_width,
    // same off-stop shape `>`/`<` must round-trip on.
    let buf = rope("\t  x\n");
    assert_eq!(
        {
            let (end, width) = leading_indent(&buf, ContentLine::new(0), 4);
            (end.offset(), width)
        },
        (co(3), BufferLineCol::new(6))
    );
}

// ── line_content_end ──────────────────────────────────────────────────────

#[test]
fn line_content_end_normal_line() {
    // "hello\nworld\n": line 0's last non-newline char is 'o' at offset 4
    let buf = rope("hello\nworld\n");
    assert_eq!(line_content_end(&buf, ContentLine::new(0)).offset(), co(4));
}

#[test]
fn line_content_end_empty_line_returns_newline_pos() {
    // "hello\n\nworld\n": line 1 is empty; cursor sits on the '\n'
    let buf = rope("hello\n\nworld\n");
    // line 1 starts at char 6, its only char is '\n' → content_end = 6
    assert_eq!(line_content_end(&buf, ContentLine::new(1)).offset(), co(6));
}

#[test]
fn line_content_end_single_char_line() {
    // "a\nb\n": line 0 content end is at 'a' (offset 0)
    let buf = rope("a\nb\n");
    assert_eq!(line_content_end(&buf, ContentLine::new(0)).offset(), co(0));
}

#[test]
fn line_content_end_combining_grapheme_before_newline() {
    // "cafe\u{0301}\n" = c(0) a(1) f(2) e(3) combining_acute(4) \n(5)
    // The grapheme "e\u{0301}" starts at char 3. line_content_end must
    // return 3 (the grapheme cluster start), not 4 (mid-cluster).
    let buf = rope("cafe\u{0301}\n");
    assert_eq!(line_content_end(&buf, ContentLine::new(0)).offset(), co(3));
}

#[test]
fn line_content_end_treats_a_bare_cr_as_content() {
    // "ab\rcd\n" is one line, not two: `\r` is ordinary content here, so the
    // cursor's last landing spot is 'd' (offset 4), not 'b'.
    let buf = rope("ab\rcd\n");
    assert_eq!(line_content_end(&buf, ContentLine::new(0)).offset(), co(4));
}

#[test]
fn line_content_end_stops_on_the_cr_of_a_crlf() {
    // "ab\r\ncd\n": line 0 is "ab\r\n", terminated by the `\n` alone, so
    // the `\r` is the line's own last content char and the cursor lands on
    // it (offset 2), one further than a plain "ab\n" would give.
    let buf = rope("ab\r\ncd\n");
    assert_eq!(line_content_end(&buf, ContentLine::new(0)).offset(), co(2));
}

#[test]
fn line_content_end_crlf_only_line_is_not_empty() {
    // "a\n\r\nb\n": line 1 is "\r\n", one content char (the `\r`) plus its
    // terminator, so the cursor lands on the `\r` (offset 2) as content, not
    // as the empty-line fallback.
    let buf = rope("a\n\r\nb\n");
    assert_eq!(line_content_end(&buf, ContentLine::new(1)).offset(), co(2));
}

// ── last char of a line ───────────────────────────────────────────────────
//
// No CRLF cases here (unlike line_content_end's own suite above): every
// buffer has `\r` normalized away at
// construction (`normalize_line_endings`, called from `BufferText::from`
// and `ChangeSetBuilder::insert`). A raw `\r` reaches `hume-rope` only
// through this test module's own `rope()` helper, which bypasses that
// normalization and so cannot stand in for a real buffer here.

/// The last codepoint of `line`'s last content cluster (its `\n` when the
/// line is empty).
fn last_char_of_line(buf: &Rope, line: ContentLine) -> CharOffset {
    crate::grapheme::cluster_end(buf.slice(..), line_content_end(buf, line))
        .offset()
        .retreat(1)
}

#[test]
fn last_char_of_line_normal_line() {
    // "hello\nworld\n": every char is its own cluster, so this is the same
    // answer as line_content_end.
    let buf = rope("hello\nworld\n");
    assert_eq!(last_char_of_line(&buf, ContentLine::new(0)), co(4));
}

#[test]
fn last_char_of_line_empty_line_returns_newline_pos() {
    // "hello\n\nworld\n": line 1 is empty; line_content_end already lands
    // on its own '\n', so the cluster round trip is a no-op.
    let buf = rope("hello\n\nworld\n");
    assert_eq!(last_char_of_line(&buf, ContentLine::new(1)), co(6));
}

#[test]
fn last_char_of_line_combining_grapheme_before_newline() {
    // "cafe\u{0301}\n" = c(0) a(1) f(2) e(3) combining_acute(4) \n(5).
    // line_content_end lands on the cluster's start (3); its last char
    // must extend through the combining mark to 4, not stop at 3.
    let buf = rope("cafe\u{0301}\n");
    assert_eq!(last_char_of_line(&buf, ContentLine::new(0)), co(4));
}

#[test]
fn last_char_of_line_last_content_line() {
    // "a\nb\nc\n": last content line is 2 ("c"), last char at 4.
    let buf = rope("a\nb\nc\n");
    assert_eq!(last_char_of_line(&buf, last_content_line(&buf)), co(4));
}

// ── line_token_content ──────────────────────────────────────────────────────

#[test]
fn line_token_content_strips_the_trailing_break() {
    let buf = rope("hello\n");
    assert_eq!(line_token_content(buf.line(0)), "hello");
}

#[test]
fn line_token_content_phantom_trailing_line_has_no_break_to_strip() {
    // The phantom trailing line's token is 0 chars. Nothing to strip, and
    // the result must not be conjured out of thin air.
    let buf = rope("a\n");
    let phantom = last_ropey_line(&buf);
    assert_eq!(line_token_content(buf.line(phantom.index())), "");
}

#[test]
fn line_token_content_across_a_rope_chunk_boundary() {
    // ropey's leaf nodes are a few hundred bytes (`MAX_BYTES`/`MIN_BYTES`);
    // 300 short lines guarantee several chunks, so the last real line's token
    // straddles one rather than sitting wholly inside a single leaf.
    let source: String = (0..300).map(|i| format!("line {i}\n")).collect();
    let buf = rope(source.as_str());
    let last = last_content_line(&buf);
    assert_eq!(line_token_content(buf.line(last.index())), "line 299");
}

// ── is_empty_line_token ──────────────────────────────────────────────────────

#[test]
fn is_empty_line_token_true_for_the_phantom_trailing_line() {
    // The phantom trailing line's token has zero chars. Every real content
    // line has at least the bare `\n`, so this arm is reached only through
    // the phantom line's own (0-char) token.
    let buf = rope("a\n");
    let phantom = last_ropey_line(&buf);
    assert!(is_empty_line_token(buf.line(phantom.index())));
}

#[test]
fn is_empty_line_token_true_for_bare_newline() {
    let buf = rope("a\n\nb\n");
    assert!(is_empty_line_token(buf.line(1)));
}

#[test]
fn is_empty_line_token_false_for_content_line() {
    let buf = rope("hello\n");
    assert!(!is_empty_line_token(buf.line(0)));
}

#[test]
fn is_empty_line_token_false_for_a_cr_only_line() {
    // "a\n\r\nb\n": line 1 is "\r\n". The `\r` is content, not part of the
    // terminator, so the line has one char before its `\n` and is not empty.
    let buf = rope("a\n\r\nb\n");
    assert!(!is_empty_line_token(buf.line(1)));
}

#[test]
fn is_empty_line_token_false_for_whitespace_only_line() {
    let buf = rope("   \n");
    assert!(!is_empty_line_token(buf.line(0)));
}

// ── char_col_in_line ─────────────────────────────────────────────────────

#[test]
fn char_col_in_line_at_line_start_is_zero() {
    let buf = rope("ab\ncd\n");
    assert_eq!(char_col_in_line(&buf, ContentLine::new(0), co(0)), cc(0));
}

#[test]
fn char_col_in_line_mid_line() {
    // "ab\ncd\n": line 1 starts at char offset 3; char offset 4 ('d') is
    // column 1.
    let buf = rope("ab\ncd\n");
    assert_eq!(char_col_in_line(&buf, ContentLine::new(1), co(4)), cc(1));
}

#[test]
fn char_col_in_line_on_the_lines_own_newline() {
    // Line 0's own '\n' sits at offset 2: column 2, one past its two
    // content chars.
    let buf = rope("ab\ncd\n");
    assert_eq!(char_col_in_line(&buf, ContentLine::new(0), co(2)), cc(2));
}

#[test]
fn char_col_in_line_is_the_inverse_of_place_char_column() {
    // Round-trip: a char_col that doesn't overshoot the line comes back
    // unchanged through place_char_column -> char_col_in_line.
    let buf = rope("hello\nworld\n");
    let char_col = cc(3);
    let pos = place_char_column(&buf, RopeyLine::new(1), char_col).offset();
    assert_eq!(char_col_in_line(&buf, ContentLine::new(1), pos), char_col);
}

// ── advance_byte_point ───────────────────────────────────────────────────

#[test]
fn advance_byte_point_no_newlines() {
    let (row, byte_col) = advance_byte_point(2, bc(5), "hello");
    assert_eq!(row, 2);
    assert_eq!(byte_col, bc(10)); // 5 + 5
}

#[test]
fn advance_byte_point_with_newlines() {
    let (row, byte_col) = advance_byte_point(1, bc(3), "foo\nbar\nbaz");
    // 2 newlines → row + 2 = 3; byte_col = "baz".len() = 3
    assert_eq!(row, 3);
    assert_eq!(byte_col, bc(3));
}

#[test]
fn advance_byte_point_trailing_newline() {
    // Inserted text ends with '\n', so byte_col must be 0.
    let (row, byte_col) = advance_byte_point(0, bc(0), "foo\n");
    assert_eq!(row, 1);
    assert_eq!(byte_col, bc(0));
}

// ── place_char_column ────────────────────────────────────────────────────

#[test]
fn place_char_column_within_line() {
    // "hello\nworld\n": char col 2 of line 1 lands on 'r' (offset 8).
    let buf = rope("hello\nworld\n");
    assert_eq!(
        place_char_column(&buf, RopeyLine::new(1), cc(2)).offset(),
        co(8)
    );
}

#[test]
fn place_char_column_is_monotonic_across_the_line_end_boundary() {
    // "abc\ndef\n": line 0 holds 'a','b','c' at chars 0,1,2 with its '\n' at
    // char 3. A char col of exactly 3 is one past the last character, i.e.
    // the newline's own offset, and must clamp back to 'c' like every larger
    // column does. Clamping against `next_line_start` (which counts the
    // '\n') instead put col 3 on the newline while col 4 clamped to 'c':
    // moving further right moved the cursor left.
    let buf = rope("abc\ndef\n");
    let placed: Vec<usize> = (0..6)
        .map(|col| {
            place_char_column(&buf, RopeyLine::new(0), cc(col))
                .offset()
                .index()
        })
        .collect();
    assert_eq!(placed, vec![0, 1, 2, 2, 2, 2]);
    assert!(
        placed.windows(2).all(|w| w[0] <= w[1]),
        "placement must never move left as the column grows: {placed:?}"
    );
    // An empty line keeps landing on its own '\n'. There the last content
    // position *is* the newline.
    let empty = rope("a\n\nb\n");
    assert_eq!(
        place_char_column(&empty, RopeyLine::new(1), cc(0)).offset(),
        co(2)
    );
    assert_eq!(
        place_char_column(&empty, RopeyLine::new(1), cc(3)).offset(),
        co(2)
    );
}

#[test]
fn place_char_column_overshoot_clamps_to_line_content_end() {
    // "hi\nhello\n": line 0 only has 2 real chars; char col 10 clamps to
    // 'i' (offset 1).
    let buf = rope("hi\nhello\n");
    assert_eq!(
        place_char_column(&buf, RopeyLine::new(0), cc(10)).offset(),
        co(1)
    );
}

#[test]
fn place_char_column_on_empty_line_lands_on_newline() {
    // "a\n\nb\n": line 1 is empty; any char column lands on its '\n'
    // (offset 2).
    let buf = rope("a\n\nb\n");
    assert_eq!(
        place_char_column(&buf, RopeyLine::new(1), cc(3)).offset(),
        co(2)
    );
}

#[test]
fn place_char_column_on_the_phantom_line_places_on_the_structural_newline() {
    // "a\nb\n": the phantom trailing line (ropey index 2) holds no cluster,
    // so it places on the text's last cluster, the structural `\n`,
    // regardless of `char_col`.
    let buf = rope("a\nb\n");
    for col in [cc(0), cc(5)] {
        assert_eq!(
            place_char_column(&buf, last_ropey_line(&buf), col).offset(),
            co(buf.len_chars() - 1)
        );
    }
}

// ── place_grapheme_column ────────────────────────────────────────────────

#[test]
fn place_grapheme_column_within_line() {
    // "hello\nworld\n": grapheme col 2 of line 1 lands on 'r' (offset 8),
    // same as the char-column case here since every grapheme is one char.
    let buf = rope("hello\nworld\n");
    assert_eq!(
        place_grapheme_column(&buf, RopeyLine::new(1), gc(2)).offset(),
        co(8)
    );
}

#[test]
fn place_grapheme_column_counts_combining_marks_as_one_column() {
    // "e\u{0301}x\n": 'e' + combining acute (one grapheme cluster) then 'x'.
    // Column 0 is the cluster start; column 1 is 'x'; char_col would have
    // landed column 1 on the combining mark itself instead.
    let buf = rope("e\u{0301}x\n");
    assert_eq!(
        place_grapheme_column(&buf, RopeyLine::new(0), gc(0)).offset(),
        co(0)
    );
    assert_eq!(
        place_grapheme_column(&buf, RopeyLine::new(0), gc(1)).offset(),
        co(2)
    );
}

#[test]
fn place_grapheme_column_overshoot_clamps_to_line_content_end() {
    // "hi\nhello\n": line 0 only has 2 grapheme clusters; column 10 clamps
    // to 'i' (offset 1).
    let buf = rope("hi\nhello\n");
    assert_eq!(
        place_grapheme_column(&buf, RopeyLine::new(0), gc(10)).offset(),
        co(1)
    );
}

#[test]
fn place_grapheme_column_zero_is_line_start() {
    let buf = rope("hello\nworld\n");
    assert_eq!(
        place_grapheme_column(&buf, RopeyLine::new(1), gc(0)).offset(),
        co(6)
    );
}

#[test]
fn place_grapheme_column_on_empty_line_lands_on_newline() {
    // "a\n\nb\n": line 1 is empty; any grapheme column lands on its '\n'
    // (offset 2).
    let buf = rope("a\n\nb\n");
    assert_eq!(
        place_grapheme_column(&buf, RopeyLine::new(1), gc(3)).offset(),
        co(2)
    );
}

#[test]
fn grapheme_col_in_line_is_the_inverse_of_place_grapheme_column() {
    // Round-trip over a line with a combining sequence, the pairing the
    // CLI `path:line:col` feature and the statusline's `line:col` both rest
    // on: the statusline displays `grapheme_col_in_line(head) + 1`, and the
    // CLI feeds a number read off that display straight back through
    // `place_grapheme_column`. `char_col_in_line`/`place_char_column` have
    // exactly this round-trip test already
    // (`char_col_in_line_is_the_inverse_of_place_char_column`, above). This
    // is its grapheme-unit sibling, on a line where the two units disagree.
    //
    // "e\u{0301}x\n": 'e' + combining acute (one cluster, grapheme col 0),
    // then 'x' (grapheme col 1). char_col 1 would land mid-cluster (on the
    // combining mark itself), so this line is required to actually exercise
    // the grapheme/char distinction, not just restate the char-column test.
    let buf = rope("e\u{0301}x\n");
    let grapheme_col = gc(1);
    let pos = place_grapheme_column(&buf, RopeyLine::new(0), grapheme_col).offset();
    assert_eq!(
        crate::grapheme::grapheme_col_in_line(buf.slice(..), ContentLine::new(0), pos),
        grapheme_col
    );
}

#[test]
fn line_segments_yields_one_triple_per_line_covered() {
    // "abc\ndef\nghi\n": a range spanning all of line 0's "abc" through
    // line 2's "gh" covers content on three lines.
    let buf = rope("abc\ndef\nghi\n");
    let start = line_start_char(&buf, RopeyLine::new(0));
    let end = co(line_start_char(&buf, RopeyLine::new(2)).index() + 2); // through "gh" on line 2
    let segs: Vec<_> = line_segments(&buf, ExclusiveRange::new(start, end))
        .map(|(l, s, e)| (l.index(), s.index(), e.index()))
        .collect();
    assert_eq!(segs, vec![(0, 0, 3), (1, 0, 3), (2, 0, 2)]);
}

#[test]
fn line_segments_skips_a_line_the_range_only_touches_at_its_own_newline() {
    // "abc\ndef\n": a range starting exactly on line 0's own '\n' (char 3,
    // one past 'c') and continuing onto line 1 covers zero chars of line
    // 0's content: an LSP diagnostic anchored at end-of-line looks exactly
    // like this. Only line 1's segment should be yielded: a zero-width
    // (3, 3) triple for line 0 would sort its end before its own start once
    // downstream flattening builds start/end events from it.
    let buf = rope("abc\ndef\n");
    let start = co(line_start_char(&buf, RopeyLine::new(0)).index() + 3); // line 0's own '\n'
    let end = co(line_start_char(&buf, RopeyLine::new(1)).index() + 2); // through "de" on line 1
    let segs: Vec<_> = line_segments(&buf, ExclusiveRange::new(start, end))
        .map(|(l, s, e)| (l.index(), s.index(), e.index()))
        .collect();
    assert_eq!(segs, vec![(1, 0, 2)]);
}

// ── next_line_start_byte ──────────────────────────────────────────────────

#[test]
fn next_line_start_byte_counts_utf8_bytes_of_multi_byte_lines() {
    // "é\n" is 3 bytes, "漢\n" is 4, "😀\n" is 5.
    let buf = rope("\u{e9}\n\u{6f22}\n\u{1f600}\n");
    assert_eq!(next_line_start_byte(&buf, RopeyLine::new(0)), 3);
    assert_eq!(next_line_start_byte(&buf, RopeyLine::new(1)), 7);
    assert_eq!(next_line_start_byte(&buf, RopeyLine::new(2)), 12);
}

// ── Multi-byte and cluster coverage ───────────────────────────────────────

#[test]
fn place_char_column_snaps_a_column_inside_a_cluster_back_to_its_start() {
    let buf = rope("ae\u{301}b\n");
    let at = |col| place_char_column(&buf, RopeyLine::new(0), CharCol::new(col)).offset();
    assert_eq!((at(1), at(2), at(3)), (co(1), co(1), co(3)));
}

#[test]
fn place_char_column_counts_chars_not_bytes_over_multi_byte_text() {
    let buf = rope("\u{e9}\u{6f22}\u{1f600}x\n");
    for col in 0..4 {
        assert_eq!(
            place_char_column(&buf, RopeyLine::new(0), CharCol::new(col)).offset(),
            co(col)
        );
    }
}

#[test]
fn advance_byte_point_counts_utf8_bytes() {
    assert_eq!(
        advance_byte_point(0, ByteCol::new(1), "\u{e9}\u{6f22}"),
        (0, ByteCol::new(6))
    );
    assert_eq!(
        advance_byte_point(2, ByteCol::new(9), "\u{e9}\n\u{6f22}\u{1f600}"),
        (3, ByteCol::new(7))
    );
}

#[test]
fn char_to_line_byte_gives_a_line_relative_utf8_offset() {
    let buf = rope("\u{e9}\n\u{6f22}x\n");
    assert_eq!(
        char_to_line_byte(&buf, co(3)),
        (RopeyLine::new(1), ByteCol::new(3))
    );
}

#[test]
fn line_segments_report_utf8_byte_columns_per_line() {
    let buf = rope("\u{e9}\u{6f22}\n\u{1f600}x\n");
    let got: Vec<_> = line_segments(&buf, ExclusiveRange::new(co(0), co(5))).collect();
    assert_eq!(
        got,
        vec![
            (ContentLine::new(0), ByteCol::new(0), ByteCol::new(5)),
            (ContentLine::new(1), ByteCol::new(0), ByteCol::new(5)),
        ]
    );
}

#[test]
fn leading_indent_measures_a_marked_no_break_space_as_one_cell() {
    let buf = rope("\u{a0}\u{301}x\n");
    assert_eq!(
        {
            let (end, width) = leading_indent(&buf, ContentLine::new(0), 4);
            (end.offset(), width)
        },
        (co(2), BufferLineCol::new(1))
    );
}

// ── Typed line positions ──────────────────────────────────────────────────

fn cl(n: usize) -> ContentLine {
    ContentLine::new(n)
}

#[test]
fn line_start_and_line_break_are_the_line_ends() {
    let r = rope("ab\n\ne\u{301}f");
    assert_eq!(line_start(&r, cl(0)).offset(), co(0));
    assert_eq!(line_break(&r, cl(0)).offset(), co(2));
    assert_eq!(line_start(&r, cl(1)).offset(), co(3));
    assert_eq!(line_break(&r, cl(1)).offset(), co(3));
    assert_eq!(line_start(&r, cl(2)).offset(), co(4));
    assert_eq!(line_break(&r, cl(2)).offset(), co(7));
}

#[test]
fn ropey_line_start_reaches_the_phantom_line() {
    let r = rope("ab");
    assert_eq!(ropey_line_start(&r, RopeyLine::new(0)).offset(), co(0));
    assert_eq!(ropey_line_start(&r, RopeyLine::new(1)).offset(), co(3));
}

#[test]
fn line_range_covers_the_line_and_its_break() {
    let r = rope("ab\ncd");
    let range = line_range(&r, cl(1));
    assert_eq!(range.chars(), ExclusiveRange::new(co(3), co(6)));
    assert_eq!(range.last(), line_break(&r, cl(1)));
}

#[test]
fn lines_range_spans_first_through_last_in_either_order() {
    let r = rope("ab\ncd\nef");
    let forward = lines_range(&r, cl(0), cl(1));
    assert_eq!(forward.chars(), ExclusiveRange::new(co(0), co(6)));
    assert_eq!(lines_range(&r, cl(1), cl(0)), forward);
    assert_eq!(lines_range(&r, cl(2), cl(2)), line_range(&r, cl(2)));
}

#[test]
fn line_content_range_excludes_the_break_and_is_none_on_an_empty_line() {
    let r = rope("ae\u{301}\n\n");
    let content = line_content_range(&r, cl(0)).expect("line 0 has content");
    assert_eq!(content.chars(), ExclusiveRange::new(co(0), co(3)));
    assert_eq!(content.last().offset(), co(1));
    assert_eq!(line_content_range(&r, cl(1)), None);
}

// ── LineText ──────────────────────────────────────────────────────────────

#[test]
fn line_text_strips_the_break_and_reports_it() {
    let r = rope("ab\ncd");
    let mut line = LineText::new();
    line.load(&r, RopeyLine::new(0));
    assert_eq!(line.as_str(), "ab");
    assert_eq!(line.break_pos(), Some(line_break(&r, cl(0))));
    line.load(&r, RopeyLine::new(2));
    assert_eq!(line.as_str(), "");
    assert_eq!(line.break_pos(), None);
}

#[test]
fn line_text_clusters_match_rope_segmentation_over_the_corpus() {
    for sample in test_fixtures::unicode::ALL {
        let text = format!("{sample}a{sample}\n{sample}\u{301}\n\n{sample}");
        let r = rope(&text);
        let mut line = LineText::new();
        let mut from_lines = Vec::new();
        for idx in 0..ropey_line_count(&r).get() {
            line.load(&r, RopeyLine::new(idx));
            for cluster in line.clusters() {
                let bytes = &line.as_str()[cluster.bytes.start.index()..cluster.bytes.end.index()];
                assert_eq!(cluster.text, bytes);
                from_lines.push((cluster.start.offset(), cluster.text.to_owned()));
            }
            if let Some(pos) = line.break_pos() {
                from_lines.push((pos.offset(), "\n".to_owned()));
            }
        }
        let mut whole = Vec::new();
        let mut chars = 0;
        for g in r.to_string().graphemes(true) {
            whole.push((co(chars), g.to_owned()));
            chars += g.chars().count();
        }
        assert_eq!(from_lines, whole, "{text:?}");
    }
}

// What `truncate_line_break` adds over `strip_line_break`: in-place
// truncation and reporting whether a break was actually removed.

#[test]
fn truncate_line_break_removes_newline_and_reports_true() {
    let mut buf = "hello\n".to_string();
    assert!(truncate_line_break(&mut buf));
    assert_eq!(buf, "hello");
}

#[test]
fn truncate_line_break_no_newline_unchanged_and_reports_false() {
    let mut buf = "hello".to_string();
    assert!(!truncate_line_break(&mut buf));
    assert_eq!(buf, "hello");
}
