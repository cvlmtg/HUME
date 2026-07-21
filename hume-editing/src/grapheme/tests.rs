use super::*;
use pretty_assertions::assert_eq;

// ── ASCII ─────────────────────────────────────────────────────────────────

#[test]
fn ascii_next_single_step() {
    let buf = Text::from("hello");
    assert_eq!(next_grapheme_boundary(&buf, 0), 1);
    assert_eq!(next_grapheme_boundary(&buf, 1), 2);
    assert_eq!(next_grapheme_boundary(&buf, 4), 5);
}

#[test]
fn ascii_next_walk() {
    // Walk forward through every grapheme in "hello\n" (6 chars).
    // Each char is its own grapheme, so boundaries are 0,1,2,…,6.
    let buf = Text::from("hello");
    let boundaries: Vec<usize> = std::iter::successors(Some(0usize), |&c| {
        let n = next_grapheme_boundary(&buf, c);
        if n > c { Some(n) } else { None }
    })
    .collect();
    assert_eq!(boundaries, vec![0, 1, 2, 3, 4, 5, 6]);
}

#[test]
fn ascii_prev_single_step() {
    let buf = Text::from("hello");
    assert_eq!(prev_grapheme_boundary(&buf, 5), 4);
    assert_eq!(prev_grapheme_boundary(&buf, 1), 0);
}

// ── Combining character (é = U+0065 + U+0301) ─────────────────────────────

#[test]
fn combining_char_next() {
    // "e\u{0301}x\n" is 4 chars, 3 grapheme clusters: ["é", "x", "\n"].
    // next(0) must skip both chars of the combining cluster and return 2.
    let buf = Text::from("e\u{0301}x");
    assert_eq!(buf.len_chars(), 4);
    assert_eq!(next_grapheme_boundary(&buf, 0), 2); // skip the whole é cluster
    assert_eq!(next_grapheme_boundary(&buf, 2), 3); // x → \n boundary
}

#[test]
fn combining_char_next_mid_cluster() {
    // Offset 1 is *inside* the é cluster (between 'e' and U+0301).
    // next() should still find the next boundary at 2, not at 1+1=2
    // by coincidence — it must consult the grapheme algorithm.
    let buf = Text::from("e\u{0301}x");
    assert_eq!(next_grapheme_boundary(&buf, 1), 2);
}

#[test]
fn combining_char_prev_mid_cluster() {
    // prev(1) from inside the é cluster should return 0 (start of cluster),
    // not 1-1=0 by coincidence — test with a prefix to break the coincidence.
    // "ae\u{0301}x\n" — offset 2 is inside the é cluster (between 'e' and U+0301).
    let buf = Text::from("ae\u{0301}x");
    assert_eq!(buf.len_chars(), 5);
    assert_eq!(prev_grapheme_boundary(&buf, 2), 1); // back to start of é, not to 'a'
}

#[test]
fn combining_char_prev() {
    // prev from end of "é" (char offset 2) must jump back to 0, not to 1.
    let buf = Text::from("e\u{0301}x");
    assert_eq!(prev_grapheme_boundary(&buf, 2), 0);
    assert_eq!(prev_grapheme_boundary(&buf, 3), 2);
}

// ── ZWJ emoji (👨‍👩‍👧 = 5 codepoints joined by ZWJ) ──────────────────────────

#[test]
fn zwj_emoji_next() {
    // U+1F468 ZWJ U+1F469 ZWJ U+1F467 — 5 chars, 1 grapheme cluster; + "\n".
    // next(0) must return 5 — the whole family is one cluster.
    let buf = Text::from("👨‍👩‍👧");
    assert_eq!(buf.len_chars(), 6); // 5 emoji chars + \n
    assert_eq!(next_grapheme_boundary(&buf, 0), 5);
}

#[test]
fn zwj_emoji_prev() {
    let buf = Text::from("👨‍👩‍👧");
    assert_eq!(prev_grapheme_boundary(&buf, 5), 0);
}

// ── Mixed string with multiple grapheme types ─────────────────────────────

#[test]
fn mixed_string_boundaries() {
    // "Hello 👨‍👩‍👧!\n" — chars: H(0) e(1) l(2) l(3) o(4) (space)(5)
    //                           👨(6) ZWJ(7) 👩(8) ZWJ(9) 👧(10) !(11) \n(12)
    // Graphemes: H, e, l, l, o, ' ', [👨‍👩‍👧], !, \n
    // Boundaries: 0, 1, 2, 3, 4, 5, 6, 11, 12, 13
    let buf = Text::from("Hello 👨‍👩‍👧!");
    assert_eq!(buf.len_chars(), 13);

    let expected = vec![0usize, 1, 2, 3, 4, 5, 6, 11, 12, 13];
    let got: Vec<usize> = std::iter::successors(Some(0usize), |&c| {
        let n = next_grapheme_boundary(&buf, c);
        if n > c { Some(n) } else { None }
    })
    .collect();
    assert_eq!(got, expected);
}

// ── Edge cases ────────────────────────────────────────────────────────────

#[test]
fn next_at_end_returns_len() {
    // "hi\n" is 3 chars. next(2) steps past '\n' to len_chars=3.
    let buf = Text::from("hi");
    assert_eq!(next_grapheme_boundary(&buf, 2), 3); // '\n' → one past it = len_chars
    assert_eq!(next_grapheme_boundary(&buf, 99), 3); // past end — clamped to len_chars
}

#[test]
fn prev_at_start_returns_zero() {
    let buf = Text::from("hi");
    assert_eq!(prev_grapheme_boundary(&buf, 0), 0);
}

#[test]
fn empty_buffer_next() {
    // Text::empty() = "\n" (1 char). next(0) steps past '\n' to len_chars=1.
    let buf = Text::empty();
    assert_eq!(next_grapheme_boundary(&buf, 0), 1);
}

#[test]
fn empty_buffer_prev() {
    let buf = Text::empty();
    assert_eq!(prev_grapheme_boundary(&buf, 0), 0);
}

// ── Complex Unicode grapheme clusters ─────────────────────────────────────

#[test]
fn regional_indicator_flag_emoji() {
    // 🇺🇸 is U+1F1FA (regional indicator U) + U+1F1F8 (regional indicator S).
    // Both codepoints form a single grapheme cluster. next from 0 must skip
    // both to land at 2.
    let buf = Text::from("\u{1F1FA}\u{1F1F8}");
    // buf: U+1F1FA(0) U+1F1F8(1) '\n'(2) = 3 chars
    assert_eq!(next_grapheme_boundary(&buf, 0), 2);
    assert_eq!(prev_grapheme_boundary(&buf, 2), 0);
}

#[test]
fn devanagari_vowel_sign() {
    // "क" (U+0915) + "ा" (U+093E vowel sign aa) form one grapheme cluster.
    let buf = Text::from("\u{0915}\u{093E}");
    // buf: U+0915(0) U+093E(1) '\n'(2) = 3 chars
    assert_eq!(next_grapheme_boundary(&buf, 0), 2);
    assert_eq!(prev_grapheme_boundary(&buf, 2), 0);
}

// ── grapheme_count ────────────────────────────────────────────────────────

#[test]
fn grapheme_count_ascii() {
    let buf = Text::from("hello\n");
    // "hello" = 5 graphemes; line starts at 0
    assert_eq!(grapheme_count(&buf, 0, 5), 5);
}

#[test]
fn grapheme_count_zero_range() {
    let buf = Text::from("hello\n");
    assert_eq!(grapheme_count(&buf, 2, 2), 0);
}

#[test]
fn grapheme_count_combining_char() {
    // "e\u{0301}x" = 3 chars but 2 grapheme clusters ("é", "x") + structural \n
    let buf = Text::from("e\u{0301}x\n");
    // from char 0 to char 2 (past the combining cluster): 1 grapheme
    assert_eq!(grapheme_count(&buf, 0, 2), 1);
    // from char 0 to char 3 (past "x"): 2 graphemes
    assert_eq!(grapheme_count(&buf, 0, 3), 2);
}

#[test]
fn grapheme_count_zwj_emoji() {
    // 👨‍👩‍👧 = 5 codepoints, 1 grapheme cluster.
    // Text::from("👨‍👩‍👧\n"): the string already ends with \n so no extra is
    // added — total 6 chars (5 emoji codepoints + \n).
    let buf = Text::from("👨‍👩‍👧\n");
    assert_eq!(buf.len_chars(), 6); // 5 emoji chars + \n
    // from 0 to 5 (past the whole emoji): 1 grapheme
    assert_eq!(grapheme_count(&buf, 0, 5), 1);
}

#[test]
fn grapheme_count_multiline_offset() {
    // "ab\ncd\n" — "cd" starts at char 3
    let buf = Text::from("ab\ncd\n");
    // from line 1 start (char 3) to char 5 (past "cd"): 2 graphemes
    assert_eq!(grapheme_count(&buf, 3, 5), 2);
    // from 3 to 3: 0
    assert_eq!(grapheme_count(&buf, 3, 3), 0);
}

#[test]
fn grapheme_count_reversed_range_returns_zero() {
    // to_char < from_char is clamped to an empty range.
    let buf = Text::from("hello\n");
    assert_eq!(grapheme_count(&buf, 3, 1), 0);
}

#[test]
fn grapheme_count_to_buffer_end() {
    // to_char == len_chars (the structural \n is the last char).
    // "hi\n" has len_chars = 3; counting from 0 to 3 covers h, i, \n = 3 graphemes.
    let buf = Text::from("hi\n");
    assert_eq!(buf.len_chars(), 3);
    assert_eq!(grapheme_count(&buf, 0, 3), 3);
}

// ── display_col_in_line ───────────────────────────────────────────────────

#[test]
fn display_col_no_tabs_matches_grapheme_col() {
    // No tabs → display col == grapheme col.
    let buf = Text::from("hello\n");
    assert_eq!(display_col_in_line(&buf, 0, 0, 4), 0);
    assert_eq!(display_col_in_line(&buf, 0, 2, 4), 2);
    assert_eq!(display_col_in_line(&buf, 0, 5, 4), 5);
}

#[test]
fn display_col_tab_advances_to_next_stop() {
    // "\tx\n": tab at col 0 → col 4; 'x' at col 4 → col 5.
    let buf = Text::from("\tx\n");
    assert_eq!(display_col_in_line(&buf, 0, 0, 4), 0); // at the tab itself
    assert_eq!(display_col_in_line(&buf, 0, 1, 4), 4); // past the tab
    assert_eq!(display_col_in_line(&buf, 0, 2, 4), 5); // past 'x'
}

#[test]
fn display_col_tab_mid_line_uses_current_col() {
    // "ab\tcd\n" with tw=4: 'a'(1) 'b'(2) '\t' → next stop of 2 is 4; then 'c'(5).
    let buf = Text::from("ab\tcd\n");
    assert_eq!(display_col_in_line(&buf, 0, 2, 4), 2); // before the tab
    assert_eq!(display_col_in_line(&buf, 0, 3, 4), 4); // past the tab
    assert_eq!(display_col_in_line(&buf, 0, 4, 4), 5); // past 'c'
}

#[test]
fn display_col_tab_width_8() {
    // "\t\n" with tw=8: tab → col 8.
    let buf = Text::from("\t\n");
    assert_eq!(display_col_in_line(&buf, 0, 1, 8), 8);
}

#[test]
fn display_col_at_line_start_is_zero() {
    let buf = Text::from("ab\ncd\n");
    // char 3 is the start of line 1.
    assert_eq!(display_col_in_line(&buf, 1, 3, 4), 0);
    assert_eq!(display_col_in_line(&buf, 1, 4, 4), 1);
}

// ── char_pos_at_display_col ───────────────────────────────────────────────

#[test]
fn char_pos_at_col_zero_is_line_start() {
    let buf = Text::from("\tfoo\n");
    assert_eq!(char_pos_at_display_col(&buf, 0, 0, 4), 0);
}

#[test]
fn char_pos_at_tab_stop_after_tab() {
    // "\tx\n": tab takes col 0→4. char at col 4 is past the tab (char 1).
    let buf = Text::from("\tx\n");
    assert_eq!(char_pos_at_display_col(&buf, 0, 4, 4), 1);
}

#[test]
fn char_pos_at_col_two_in_spaces() {
    // "    \n": 4 spaces. char at col 2 is char 2 (third space).
    let buf = Text::from("    \n");
    assert_eq!(char_pos_at_display_col(&buf, 0, 2, 4), 2);
}

#[test]
fn char_pos_at_col_eight_two_tabs() {
    // "\t\t\n": tab→col4, tab→col8. char at col 8 is past second tab (char 2).
    let buf = Text::from("\t\t\n");
    assert_eq!(char_pos_at_display_col(&buf, 0, 8, 4), 2);
    // Mid stop: col 4 is past first tab (char 1).
    assert_eq!(char_pos_at_display_col(&buf, 0, 4, 4), 1);
}

#[test]
fn char_pos_mixed_spaces_and_tab() {
    // "  \t\n": 2 spaces (col 0,1) + tab (col 2→4). char at col 4 is char 3.
    let buf = Text::from("  \t\n");
    assert_eq!(char_pos_at_display_col(&buf, 0, 4, 4), 3);
    assert_eq!(char_pos_at_display_col(&buf, 0, 2, 4), 2);
}

#[test]
fn char_pos_overshoot_stops_short() {
    // "\t\n" with tw=4, target col 2: the tab would jump col 0→4,
    // overshooting 2. Walk stops at line_start (col 0).
    let buf = Text::from("\t\n");
    assert_eq!(char_pos_at_display_col(&buf, 0, 2, 4), 0);
}

#[test]
fn char_pos_target_beyond_line_width_stops_at_newline() {
    // "ab\ncd\n" — line 0 is 2 columns wide. A target past that must stop
    // on line 0's '\n' (char 2), never walk onto line 1.
    let buf = Text::from("ab\ncd\n");
    assert_eq!(char_pos_at_display_col(&buf, 0, 4, 4), 2);
    // Same guard on the last content line: stops at its structural '\n'.
    assert_eq!(char_pos_at_display_col(&buf, 1, 99, 4), 5);
}
