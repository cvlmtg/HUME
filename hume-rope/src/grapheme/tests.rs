use super::*;
use crate::test_support::{chunk_straddling_text, rope, segmentation_boundaries};
use pretty_assertions::assert_eq;
use ropey::Rope;

fn co(n: usize) -> CharOffset {
    CharOffset::new(n)
}

fn gc(n: usize) -> GraphemeCol {
    GraphemeCol::new(n)
}

fn dc(n: u32) -> BufferLineCol {
    BufferLineCol::new(n)
}

// ── ASCII ─────────────────────────────────────────────────────────────────

#[test]
fn ascii_next_single_step() {
    let buf = rope("hello");
    assert_eq!(next_grapheme_boundary(buf.slice(..), co(0)), co(1));
    assert_eq!(next_grapheme_boundary(buf.slice(..), co(1)), co(2));
    assert_eq!(next_grapheme_boundary(buf.slice(..), co(4)), co(5));
}

#[test]
fn ascii_next_walk() {
    // Walk forward through every grapheme in "hello\n" (6 chars).
    // Each char is its own grapheme, so boundaries are 0,1,2,…,6.
    let buf = rope("hello");
    let boundaries: Vec<CharOffset> = std::iter::successors(Some(co(0)), |&c| {
        let n = next_grapheme_boundary(buf.slice(..), c);
        if n > c { Some(n) } else { None }
    })
    .collect();
    assert_eq!(
        boundaries,
        vec![0, 1, 2, 3, 4, 5, 6]
            .into_iter()
            .map(co)
            .collect::<Vec<_>>()
    );
}

#[test]
fn ascii_prev_single_step() {
    let buf = rope("hello");
    assert_eq!(prev_grapheme_boundary(buf.slice(..), co(5)), co(4));
    assert_eq!(prev_grapheme_boundary(buf.slice(..), co(1)), co(0));
}

// ── Combining character (é = U+0065 + U+0301) ─────────────────────────────

#[test]
fn combining_char_next() {
    // "e\u{0301}x\n" is 4 chars, 3 grapheme clusters: ["é", "x", "\n"].
    // next(0) must skip both chars of the combining cluster and return 2.
    let buf = rope("e\u{0301}x");
    assert_eq!(buf.len_chars(), 4);
    assert_eq!(next_grapheme_boundary(buf.slice(..), co(0)), co(2)); // skip the whole é cluster
    assert_eq!(next_grapheme_boundary(buf.slice(..), co(2)), co(3)); // x → \n boundary
}

#[test]
fn combining_char_next_mid_cluster() {
    // Offset 1 is *inside* the é cluster (between 'e' and U+0301).
    // next() should still find the next boundary at 2, not at 1+1=2
    // by coincidence: it must consult the grapheme algorithm.
    let buf = rope("e\u{0301}x");
    assert_eq!(next_grapheme_boundary(buf.slice(..), co(1)), co(2));
}

#[test]
fn combining_char_prev_mid_cluster() {
    // prev(1) from inside the é cluster should return 0 (start of cluster),
    // not 1-1=0 by coincidence. Test with a prefix to break the coincidence.
    // "ae\u{0301}x\n": offset 2 is inside the é cluster (between 'e' and U+0301).
    let buf = rope("ae\u{0301}x");
    assert_eq!(buf.len_chars(), 5);
    assert_eq!(prev_grapheme_boundary(buf.slice(..), co(2)), co(1)); // back to start of é, not to 'a'
}

#[test]
fn combining_char_prev() {
    // prev from end of "é" (char offset 2) must jump back to 0, not to 1.
    let buf = rope("e\u{0301}x");
    assert_eq!(prev_grapheme_boundary(buf.slice(..), co(2)), co(0));
    assert_eq!(prev_grapheme_boundary(buf.slice(..), co(3)), co(2));
}

// ── ZWJ emoji (👨‍👩‍👧 = 5 codepoints joined by ZWJ) ──────────────────────────

#[test]
fn zwj_emoji_next() {
    // U+1F468 ZWJ U+1F469 ZWJ U+1F467: 5 chars, 1 grapheme cluster; + "\n".
    // next(0) must return 5, since the whole family is one cluster.
    let buf = rope("👨‍👩‍👧");
    assert_eq!(buf.len_chars(), 6); // 5 emoji chars + \n
    assert_eq!(next_grapheme_boundary(buf.slice(..), co(0)), co(5));
}

#[test]
fn zwj_emoji_prev() {
    let buf = rope("👨‍👩‍👧");
    assert_eq!(prev_grapheme_boundary(buf.slice(..), co(5)), co(0));
}

// ── Mixed string with multiple grapheme types ─────────────────────────────

#[test]
fn mixed_string_boundaries() {
    // "Hello 👨‍👩‍👧!\n", chars: H(0) e(1) l(2) l(3) o(4) (space)(5)
    //                          👨(6) ZWJ(7) 👩(8) ZWJ(9) 👧(10) !(11) \n(12)
    // Graphemes: H, e, l, l, o, ' ', [👨‍👩‍👧], !, \n
    // Boundaries: 0, 1, 2, 3, 4, 5, 6, 11, 12, 13
    let buf = rope("Hello 👨‍👩‍👧!");
    assert_eq!(buf.len_chars(), 13);

    let expected: Vec<CharOffset> = vec![0usize, 1, 2, 3, 4, 5, 6, 11, 12, 13]
        .into_iter()
        .map(co)
        .collect();
    let got: Vec<CharOffset> = std::iter::successors(Some(co(0)), |&c| {
        let n = next_grapheme_boundary(buf.slice(..), c);
        if n > c { Some(n) } else { None }
    })
    .collect();
    assert_eq!(got, expected);
}

// ── Edge cases ────────────────────────────────────────────────────────────

#[test]
fn next_at_end_returns_len() {
    // "hi\n" is 3 chars. next(2) steps past '\n' to len_chars=3.
    let buf = rope("hi");
    assert_eq!(next_grapheme_boundary(buf.slice(..), co(2)), co(3)); // '\n' → one past it = len_chars
    assert_eq!(next_grapheme_boundary(buf.slice(..), co(3)), co(3));
}

#[test]
#[should_panic(expected = "past the text end")]
fn next_past_the_end_panics() {
    let buf = rope("hi");
    next_grapheme_boundary(buf.slice(..), co(4));
}

#[test]
#[should_panic(expected = "past the text end")]
fn prev_past_the_end_panics() {
    let buf = rope("hi");
    prev_grapheme_boundary(buf.slice(..), co(4));
}

#[test]
fn prev_at_start_returns_zero() {
    let buf = rope("hi");
    assert_eq!(prev_grapheme_boundary(buf.slice(..), co(0)), co(0));
}

#[test]
fn empty_buffer_next() {
    // rope("") = "\n" (1 char). next(0) steps past '\n' to len_chars=1.
    let buf = rope("");
    assert_eq!(next_grapheme_boundary(buf.slice(..), co(0)), co(1));
}

#[test]
fn empty_buffer_prev() {
    let buf = rope("");
    assert_eq!(prev_grapheme_boundary(buf.slice(..), co(0)), co(0));
}

// ── Complex Unicode grapheme clusters ─────────────────────────────────────

#[test]
fn regional_indicator_flag_emoji() {
    // 🇺🇸 is U+1F1FA (regional indicator U) + U+1F1F8 (regional indicator S).
    // Both codepoints form a single grapheme cluster. next from 0 must skip
    // both to land at 2.
    let buf = rope("\u{1F1FA}\u{1F1F8}");
    // buf: U+1F1FA(0) U+1F1F8(1) '\n'(2) = 3 chars
    assert_eq!(next_grapheme_boundary(buf.slice(..), co(0)), co(2));
    assert_eq!(prev_grapheme_boundary(buf.slice(..), co(2)), co(0));
}

#[test]
fn devanagari_vowel_sign() {
    // "क" (U+0915) + "ा" (U+093E vowel sign aa) form one grapheme cluster.
    let buf = rope("\u{0915}\u{093E}");
    // buf: U+0915(0) U+093E(1) '\n'(2) = 3 chars
    assert_eq!(next_grapheme_boundary(buf.slice(..), co(0)), co(2));
    assert_eq!(prev_grapheme_boundary(buf.slice(..), co(2)), co(0));
}

// ── A cluster's last char ───────────────────────────────────────────────────

/// The last codepoint of the cluster holding `pos`: one before its end.
fn last_char_of(buf: &Rope, pos: usize) -> CharOffset {
    snap_to_cluster(buf.slice(..), co(pos))
        .expect("a non-empty text")
        .end
        .retreat(1)
}

#[test]
fn a_single_codepoint_cluster_ends_on_its_own_char() {
    // Every char in "hello" is its own 1-codepoint cluster.
    let buf = rope("hello");
    assert_eq!(last_char_of(&buf, 0), co(0));
    assert_eq!(last_char_of(&buf, 4), co(4));
}

#[test]
fn a_combining_cluster_ends_on_its_combining_mark() {
    // "e\u{0301}x\n" (é = e + combining acute): the cluster starting at 0
    // spans chars 0-1, so its last codepoint is 1 (the mark), not 0 (the
    // base letter).
    let buf = rope("e\u{0301}x");
    assert_eq!(last_char_of(&buf, 0), co(1));
}

#[test]
fn the_last_cluster_of_a_one_char_text_ends_on_it() {
    // Single-char buffer: the structural '\n' is its own cluster, ending at
    // len_chars() (1).
    let buf = rope("");
    assert_eq!(last_char_of(&buf, 0), co(0));
}

// ── grapheme_count ────────────────────────────────────────────────────────

#[test]
fn grapheme_count_ascii() {
    let buf = rope("hello\n");
    // "hello" = 5 graphemes; line starts at 0
    assert_eq!(grapheme_count(buf.slice(..), co(0), co(5)), 5);
}

#[test]
fn grapheme_count_zero_range() {
    let buf = rope("hello\n");
    assert_eq!(grapheme_count(buf.slice(..), co(2), co(2)), 0);
}

#[test]
fn grapheme_count_combining_char() {
    // "e\u{0301}x" = 3 chars but 2 grapheme clusters ("é", "x") + structural \n
    let buf = rope("e\u{0301}x\n");
    // from char 0 to char 2 (past the combining cluster): 1 grapheme
    assert_eq!(grapheme_count(buf.slice(..), co(0), co(2)), 1);
    // from char 0 to char 3 (past "x"): 2 graphemes
    assert_eq!(grapheme_count(buf.slice(..), co(0), co(3)), 2);
}

#[test]
fn grapheme_count_zwj_emoji() {
    // 👨‍👩‍👧 = 5 codepoints, 1 grapheme cluster.
    // rope("👨‍👩‍👧\n"): the string already ends with \n so no extra is
    // added: total 6 chars (5 emoji codepoints + \n).
    let buf = rope("👨‍👩‍👧\n");
    assert_eq!(buf.len_chars(), 6); // 5 emoji chars + \n
    // from 0 to 5 (past the whole emoji): 1 grapheme
    assert_eq!(grapheme_count(buf.slice(..), co(0), co(5)), 1);
}

#[test]
fn grapheme_count_multiline_offset() {
    // "ab\ncd\n": "cd" starts at char 3
    let buf = rope("ab\ncd\n");
    // from line 1 start (char 3) to char 5 (past "cd"): 2 graphemes
    assert_eq!(grapheme_count(buf.slice(..), co(3), co(5)), 2);
    // from 3 to 3: 0
    assert_eq!(grapheme_count(buf.slice(..), co(3), co(3)), 0);
}

#[test]
fn grapheme_count_reversed_range_returns_zero() {
    // to_char < from_char is clamped to an empty range.
    let buf = rope("hello\n");
    assert_eq!(grapheme_count(buf.slice(..), co(3), co(1)), 0);
}

#[test]
fn grapheme_count_to_buffer_end() {
    // to_char == len_chars (the structural \n is the last char).
    // "hi\n" has len_chars = 3; counting from 0 to 3 covers h, i, \n = 3 graphemes.
    let buf = rope("hi\n");
    assert_eq!(buf.len_chars(), 3);
    assert_eq!(grapheme_count(buf.slice(..), co(0), co(3)), 3);
}

#[test]
fn grapheme_col_and_display_col_diverge_after_a_tab() {
    // The confusion the whole column taxonomy exists to prevent, pinned at
    // the two functions that define it. On "\tx", 'x' is preceded by exactly
    // one grapheme cluster, so its *grapheme* column is 1: the unit the
    // editing model counts in, and the one HUME shows a user (1-based: 2).
    // Its *display* column is 4, because the tab expands to the next stop.
    // Rendering the display column as "the column" would report 5 for a
    // cursor the user reached with a single press of →.
    let buf = rope("\tx\n");
    assert_eq!(
        grapheme_col_in_line(buf.slice(..), ContentLine::new(0), co(1)),
        gc(1)
    );
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(0), co(1), 4),
        dc(4)
    );
}

// ── display_col_in_line ───────────────────────────────────────────────────

#[test]
fn display_col_no_tabs_matches_grapheme_col() {
    // No tabs → display col == grapheme col.
    let buf = rope("hello\n");
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(0), co(0), 4),
        dc(0)
    );
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(0), co(2), 4),
        dc(2)
    );
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(0), co(5), 4),
        dc(5)
    );
}

#[test]
fn display_col_tab_advances_to_next_stop() {
    // "\tx\n": tab at display col 0 → display col 4; 'x' at display col 4 →
    // display col 5.
    let buf = rope("\tx\n");
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(0), co(0), 4),
        dc(0)
    ); // at the tab itself
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(0), co(1), 4),
        dc(4)
    ); // past the tab
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(0), co(2), 4),
        dc(5)
    ); // past 'x'
}

#[test]
fn display_col_tab_mid_line_uses_current_display_col() {
    // "ab\tcd\n" with tw=4: 'a'(1) 'b'(2) '\t' → next stop of 2 is 4; then 'c'(5).
    let buf = rope("ab\tcd\n");
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(0), co(2), 4),
        dc(2)
    ); // before the tab
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(0), co(3), 4),
        dc(4)
    ); // past the tab
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(0), co(4), 4),
        dc(5)
    ); // past 'c'
}

#[test]
fn display_col_tab_width_8() {
    // "\t\n" with tw=8: tab → display col 8.
    let buf = rope("\t\n");
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(0), co(1), 8),
        dc(8)
    );
}

#[test]
fn display_col_at_line_start_is_zero() {
    let buf = rope("ab\ncd\n");
    // char 3 is the start of line 1.
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(1), co(3), 4),
        dc(0)
    );
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(1), co(4), 4),
        dc(1)
    );
}

#[test]
fn display_col_wide_cjk_before_tab_shifts_the_stop() {
    // "\u{6F22}\tx\n" (漢 is East Asian Wide, 2 display columns) with tw=4:
    // 漢 takes display col 0→2; tab from display col 2 advances to the next
    // stop, display col 4; 'x' lands at display col 4. A char-counting (not
    // display-column-counting) walk would have put the tab's stop at
    // display col 3 instead.
    let buf = rope("\u{6F22}\tx\n");
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(0), co(1), 4),
        dc(2)
    ); // past 漢
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(0), co(2), 4),
        dc(4)
    ); // past the tab
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(0), co(3), 4),
        dc(5)
    ); // past 'x'
}

#[test]
fn display_col_decomposed_e_acute_before_tab_counts_as_one_display_column() {
    // "e\u{0301}\tx\n": the decomposed é is one grapheme cluster occupying 1
    // display column (base 'e' + zero-width combining mark), so the tab
    // after it behaves exactly as it would after a plain 'e'.
    let buf = rope("e\u{0301}\tx\n");
    assert_eq!(buf.len_chars(), 5); // e, U+0301, \t, x, \n
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(0), co(2), 4),
        dc(1)
    ); // past the é cluster
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(0), co(3), 4),
        dc(4)
    ); // past the tab
    assert_eq!(
        display_col_in_line(buf.slice(..), ContentLine::new(0), co(4), 4),
        dc(5)
    ); // past 'x'
}

// ── char_pos_at_display_col ───────────────────────────────────────────────

#[test]
fn char_pos_at_display_col_zero_is_line_start() {
    let buf = rope("\tfoo\n");
    assert_eq!(
        char_pos_at_display_col(buf.slice(..), ContentLine::new(0), dc(0), 4).offset(),
        co(0)
    );
}

#[test]
fn char_pos_at_tab_stop_after_tab() {
    // "\tx\n": tab takes display col 0→4. char at display col 4 is past the
    // tab (char 1).
    let buf = rope("\tx\n");
    assert_eq!(
        char_pos_at_display_col(buf.slice(..), ContentLine::new(0), dc(4), 4).offset(),
        co(1)
    );
}

#[test]
fn char_pos_at_display_col_inside_a_wide_cluster_stays_on_its_start() {
    // "漢bc\n": 漢 occupies display cols 0 AND 1, so col 1 falls *inside*
    // the cluster rather than on any cluster start. The walk must stop
    // before the grapheme that would overshoot and answer 0 (漢's own
    // position), never 1, which is 'b', a full column to the right of
    // where the caller pointed. Every other test here targets a cluster
    // start, where the overshoot branch never fires.
    let buf = rope("\u{6F22}bc\n");
    assert_eq!(
        char_pos_at_display_col(buf.slice(..), ContentLine::new(0), dc(0), 4).offset(),
        co(0)
    );
    assert_eq!(
        char_pos_at_display_col(buf.slice(..), ContentLine::new(0), dc(1), 4).offset(),
        co(0)
    );
    assert_eq!(
        char_pos_at_display_col(buf.slice(..), ContentLine::new(0), dc(2), 4).offset(),
        co(1)
    ); // 'b'
}

#[test]
fn char_pos_at_display_col_two_in_spaces() {
    // "    \n": 4 spaces. char at display col 2 is char 2 (third space).
    let buf = rope("    \n");
    assert_eq!(
        char_pos_at_display_col(buf.slice(..), ContentLine::new(0), dc(2), 4).offset(),
        co(2)
    );
}

#[test]
fn char_pos_at_display_col_eight_two_tabs() {
    // "\t\t\n": tab→col4, tab→col8. char at display col 8 is past second tab
    // (char 2).
    let buf = rope("\t\t\n");
    assert_eq!(
        char_pos_at_display_col(buf.slice(..), ContentLine::new(0), dc(8), 4).offset(),
        co(2)
    );
    // Mid stop: display col 4 is past first tab (char 1).
    assert_eq!(
        char_pos_at_display_col(buf.slice(..), ContentLine::new(0), dc(4), 4).offset(),
        co(1)
    );
}

#[test]
fn char_pos_mixed_spaces_and_tab() {
    // "  \t\n": 2 spaces (display col 0,1) + tab (display col 2→4). char at
    // display col 4 is char 3.
    let buf = rope("  \t\n");
    assert_eq!(
        char_pos_at_display_col(buf.slice(..), ContentLine::new(0), dc(4), 4).offset(),
        co(3)
    );
    assert_eq!(
        char_pos_at_display_col(buf.slice(..), ContentLine::new(0), dc(2), 4).offset(),
        co(2)
    );
}

#[test]
fn char_pos_overshoot_stops_short() {
    // "\t\n" with tw=4, target display col 2: the tab would jump display col
    // 0→4, overshooting 2. Walk stops at line_start (display col 0).
    let buf = rope("\t\n");
    assert_eq!(
        char_pos_at_display_col(buf.slice(..), ContentLine::new(0), dc(2), 4).offset(),
        co(0)
    );
}

#[test]
fn char_pos_at_display_col_after_wide_cjk_and_tab() {
    // "\u{6F22}\tx\n" with tw=4: 漢 spans display col 0→2, tab spans display
    // col 2→4, so the char at display col 4 is 'x' (char index 2).
    let buf = rope("\u{6F22}\tx\n");
    assert_eq!(
        char_pos_at_display_col(buf.slice(..), ContentLine::new(0), dc(4), 4).offset(),
        co(2)
    );
}

#[test]
fn char_pos_target_beyond_line_width_stops_at_newline() {
    // "ab\ncd\n": line 0 is 2 display columns wide. A target past that must
    // stop on line 0's '\n' (char 2), never walk onto line 1.
    let buf = rope("ab\ncd\n");
    assert_eq!(
        char_pos_at_display_col(buf.slice(..), ContentLine::new(0), dc(4), 4).offset(),
        co(2)
    );
    // Same guard on the last content line: stops at its structural '\n'.
    assert_eq!(
        char_pos_at_display_col(buf.slice(..), ContentLine::new(1), dc(99), 4).offset(),
        co(5)
    );
}

// ── graphemes_at ──────────────────────────────────────────────────────────

/// The clusters `walk_from(text, pos)` must yield, derived from
/// `unicode-segmentation` over the whole text as one `&str`, not from the
/// rope walker under test. A cluster that starts before `pos` and ends after
/// it is cut at `pos`, matching a cursor dropped inside a cluster.
fn segmentation_clusters(text: &Rope, pos: usize) -> Vec<Cluster> {
    let s = text.to_string();
    let mut out = Vec::new();
    let mut start = 0;
    for g in s.graphemes(true) {
        let end = start + g.chars().count();
        if end > pos {
            let from = start.max(pos);
            out.push(Cluster {
                start: co(from),
                end: co(end),
                first: g.chars().nth(from - start).expect("from < end"),
            });
        }
        start = end;
    }
    out
}

fn walk(text: &Rope, pos: usize) -> Vec<Cluster> {
    walk_from(text.slice(..), co(pos)).collect()
}

#[test]
fn graphemes_at_yields_every_cluster_segmentation_finds() {
    for text in [
        "hello world",
        "cafe\u{301} au lait",
        "family: \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467} ok",
        "flags \u{1F1EE}\u{1F1F9}\u{1F1EB}\u{1F1F7} end",
        "漢字とかな、句読点。",
        "two\nlines\n",
    ] {
        let r = rope(text);
        assert_eq!(walk(&r, 0), segmentation_clusters(&r, 0), "text: {text:?}");
    }
}

#[test]
fn graphemes_at_from_inside_a_cluster_starts_at_that_position() {
    // "e" + U+0301 is one cluster at chars 0..2; starting at 1 yields the
    // combining mark alone as the first item, like `next_grapheme_boundary`
    // from inside a cluster.
    let r = rope("e\u{301}x");
    let clusters = walk(&r, 1);
    assert_eq!(
        clusters[0],
        Cluster {
            start: co(1),
            end: co(2),
            first: '\u{301}'
        }
    );
    assert_eq!(clusters, segmentation_clusters(&r, 1));
}

#[test]
fn graphemes_at_the_end_yields_nothing() {
    let r = rope("ab");
    assert_eq!(walk(&r, r.len_chars()), Vec::new());
}

#[test]
fn graphemes_at_crosses_chunk_boundaries_including_mid_cluster() {
    let unit = "abc e\u{301} \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467} \u{1F1EE}\u{1F1F9} 字\n";
    let r = rope(&unit.repeat(2000));
    let s = r.to_string();

    let mut chunk_ends = Vec::new();
    let mut byte = 0;
    for chunk in r.chunks() {
        byte += chunk.len();
        chunk_ends.push(byte);
    }
    assert!(chunk_ends.len() > 1, "the text must span several chunks");
    assert!(
        s.grapheme_indices(true)
            .any(|(i, g)| chunk_ends.iter().any(|&b| i < b && b < i + g.len())),
        "at least one cluster must straddle a chunk boundary for this test to cover that path"
    );

    assert_eq!(walk(&r, 0), segmentation_clusters(&r, 0));
}

#[test]
fn graphemes_at_keeps_a_flag_pair_together_across_a_chunk_boundary() {
    let flags = "\u{1F1EE}\u{1F1F9}\u{1F1EB}\u{1F1F7}".repeat(4000);
    let mut straddled_a_pair = false;
    for prefix in 0..4 {
        let r = rope(&format!("{}{flags}", "a".repeat(prefix)));
        let s = r.to_string();
        let mut chunk_ends = Vec::new();
        let mut byte = 0;
        for chunk in r.chunks() {
            byte += chunk.len();
            chunk_ends.push(byte);
        }
        straddled_a_pair |= s.grapheme_indices(true).any(|(i, g)| {
            g.chars().count() == 2 && chunk_ends.iter().any(|&b| i < b && b < i + g.len())
        });
        assert_eq!(walk(&r, 0), segmentation_clusters(&r, 0), "prefix {prefix}");
    }
    assert!(
        straddled_a_pair,
        "some flag pair must straddle a chunk boundary for this test to cover that path"
    );
}

// ── Cluster boundaries and snapping ───────────────────────────────────────

fn corpus_texts(sample: &str) -> [String; 4] {
    [
        format!("a{sample}b"),
        format!("{sample}{sample}"),
        format!("\n{sample}\n"),
        format!("{sample}\u{301}x"),
    ]
}

#[test]
fn cluster_walk_boundaries_match_segmentation_over_the_corpus() {
    for sample in test_fixtures::unicode::ALL {
        for text in corpus_texts(sample) {
            let r = rope(&text);
            let boundaries = segmentation_boundaries(&r.to_string());
            let walked: Vec<usize> = graphemes_at(r.slice(..), ClusterBound::TEXT_START)
                .map(|c| c.start.index())
                .chain(std::iter::once(text_end(r.slice(..)).offset().index()))
                .collect();
            assert_eq!(walked, boundaries, "{:?}", r.to_string());
        }
    }
}

#[test]
fn snap_to_cluster_floors_to_the_segmentation_boundary_over_the_corpus() {
    for sample in test_fixtures::unicode::ALL {
        for text in corpus_texts(sample) {
            let r = rope(&text);
            let boundaries = segmentation_boundaries(&r.to_string());
            for pos in 0..r.len_chars() {
                let floor = boundaries
                    .iter()
                    .rev()
                    .find(|&&b| b <= pos)
                    .expect("0 is a boundary");
                assert_eq!(
                    snap_to_cluster(r.slice(..), co(pos)).map(|c| c.start),
                    Some(co(*floor)),
                    "{:?} at {pos}",
                    r.to_string()
                );
            }
        }
    }
}

/// The start of the cluster holding `pos`.
fn cluster_start_of(r: &Rope, pos: usize) -> CharOffset {
    snap_to_cluster(r.slice(..), co(pos))
        .expect("a non-empty text")
        .start
}

#[test]
fn snap_to_cluster_keeps_crlf_together() {
    let r = Rope::from_str("a\r\nb");
    assert_eq!(cluster_start_of(&r, 1), co(1));
    assert_eq!(cluster_start_of(&r, 2), co(1));
    assert_eq!(cluster_start_of(&r, 3), co(3));
}

#[test]
fn snap_to_cluster_joins_a_prepend_char_and_its_base() {
    let r = rope("\u{600}x");
    assert_eq!(cluster_start_of(&r, 1), co(0));
}

// ── LF-normalized text ────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "LF-normalized")]
fn char_pos_at_display_col_rejects_a_carriage_return() {
    let r = Rope::from_str("ab\r\ncd\n");
    char_pos_at_display_col(r.slice(..), ContentLine::new(0), dc(10), 4);
}

// ── Corpus coverage ───────────────────────────────────────────────────────

#[test]
fn str_boundaries_walk_every_corpus_cluster_both_ways() {
    for sample in test_fixtures::unicode::ALL {
        let text = format!("a{sample}b");
        let expected: Vec<usize> = text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain([text.len()])
            .collect();

        let mut forward = vec![0];
        while *forward.last().unwrap() < text.len() {
            forward.push(next_str_boundary(&text, *forward.last().unwrap()));
        }
        assert_eq!(forward, expected, "forward over {text:?}");

        let mut backward = vec![text.len()];
        while *backward.last().unwrap() > 0 {
            backward.push(prev_str_boundary(&text, *backward.last().unwrap()));
        }
        backward.reverse();
        assert_eq!(backward, expected, "backward over {text:?}");
    }
}

#[test]
fn prev_and_next_boundary_agree_with_segmentation_across_chunk_boundaries() {
    let (r, probes) = chunk_straddling_text();
    let boundaries = segmentation_boundaries(&r.to_string());
    for pos in probes {
        let expected_prev = boundaries.iter().rev().find(|&&b| b < pos).copied();
        let expected_next = boundaries.iter().find(|&&b| b > pos).copied();
        assert_eq!(
            prev_grapheme_boundary(r.slice(..), co(pos)),
            co(expected_prev.unwrap_or(0)),
            "prev at {pos}"
        );
        if let Some(next) = expected_next {
            assert_eq!(
                next_grapheme_boundary(r.slice(..), co(pos)),
                co(next),
                "next at {pos}"
            );
        }
    }
}

/// Asserts the cluster walk over `slice`, and `snap_to_cluster` and both
/// boundary steppers at each of `probes`, against segmentation of the
/// slice's own text.
fn assert_queries_match_segmentation(
    slice: RopeSlice<'_>,
    probes: impl IntoIterator<Item = usize>,
    label: &str,
) {
    let b = segmentation_boundaries(&slice.to_string());
    let len = slice.len_chars();
    let walked: Vec<usize> = graphemes_at(slice, ClusterBound::TEXT_START)
        .map(|c| c.start.index())
        .chain([len])
        .collect();
    assert_eq!(walked, b, "walk over {label}");
    for pos in probes {
        let floor = *b
            .iter()
            .rev()
            .find(|&&x| x <= pos)
            .expect("0 is a boundary");
        let prev = b.iter().rev().find(|&&x| x < pos).copied().unwrap_or(0);
        let next = b.iter().find(|&&x| x > pos).copied().unwrap_or(len);
        if pos < len {
            let cluster = snap_to_cluster(slice, co(pos)).expect("a non-empty slice");
            assert_eq!(
                (cluster.start, cluster.end, cluster.first),
                (co(floor), co(next), slice.char(floor)),
                "snap at {pos} in {label}"
            );
        }
        assert_eq!(
            prev_grapheme_boundary(slice, co(pos)),
            co(prev),
            "prev at {pos} in {label}"
        );
        assert_eq!(
            next_grapheme_boundary(slice, co(pos)),
            co(next),
            "next at {pos} in {label}"
        );
    }
}

#[test]
fn cluster_queries_on_a_sub_slice_match_segmentation_of_its_text() {
    for sample in test_fixtures::unicode::ALL {
        for text in corpus_texts(sample) {
            let r = rope(&text);
            let b = segmentation_boundaries(&r.to_string());
            for (i, &from) in b.iter().enumerate() {
                for &to in &b[i..] {
                    let slice = r.slice(from..to);
                    let label = format!("{:?}[{from}..{to}]", r.to_string());
                    assert_queries_match_segmentation(slice, 0..=slice.len_chars(), &label);
                }
            }
        }
    }
}

#[test]
fn cluster_queries_on_a_sub_slice_across_chunk_boundaries_match_segmentation() {
    let (r, probes) = chunk_straddling_text();
    let b = segmentation_boundaries(&r.to_string());
    let (from, to) = (b[b.len() / 4], b[3 * b.len() / 4]);
    let slice = r.slice(from..to);
    let local = probes
        .into_iter()
        .filter(|&p| from <= p && p <= to)
        .map(|p| p - from);
    assert_queries_match_segmentation(slice, local.chain([0, to - from]), "a sub-slice");
    assert_queries_match_segmentation(r.slice(from..from), [0], "an empty sub-slice");
}

#[test]
fn cluster_queries_keep_crlf_whole_at_chunk_edges() {
    for unit in ["\r\n", "x\r\n"] {
        let r = Rope::from_str(&unit.repeat(2000));
        assert!(r.chunks().count() > 1, "the text must span several chunks");
        let mut probes = Vec::new();
        let mut byte = 0;
        for chunk in r.chunks() {
            byte += chunk.len();
            let at = r.byte_to_char(byte);
            probes.extend(at.saturating_sub(3)..=(at + 3).min(r.len_chars()));
        }
        assert_queries_match_segmentation(r.slice(..), probes, unit);
    }
}

#[test]
fn graphemes_at_from_every_position_matches_segmentation_over_the_corpus() {
    for sample in test_fixtures::unicode::ALL {
        for text in corpus_texts(sample) {
            let r = rope(&text);
            for pos in 0..=r.len_chars() {
                assert_eq!(
                    walk(&r, pos),
                    segmentation_clusters(&r, pos),
                    "{:?} from {pos}",
                    r.to_string()
                );
            }
        }
    }
}

#[test]
fn char_pos_at_display_col_walks_combining_wide_and_astral_clusters() {
    use test_fixtures::unicode::{ASTRAL, CJK, COMBINING, ZWJ_FAMILY};
    let at = |text: &str, col: u32| {
        let r = rope(text);
        char_pos_at_display_col(r.slice(..), ContentLine::new(0), dc(col), 4).offset()
    };
    // `e◌́` is one cell wide.
    let t = format!("a{COMBINING}b");
    assert_eq!((at(&t, 1), at(&t, 2), at(&t, 3)), (co(1), co(3), co(4)));
    // A two-cell cluster is skipped whole and never split by an odd target.
    for wide in [CJK, ASTRAL] {
        let t = format!("a{wide}b");
        let after = 1 + wide.chars().count();
        assert_eq!(at(&t, 1), co(1), "{wide:?}");
        assert_eq!(at(&t, 2), co(1), "{wide:?}: target inside the cluster");
        assert_eq!(at(&t, 3), co(after), "{wide:?}");
    }
    let t = format!("a{ZWJ_FAMILY}b");
    assert_eq!((at(&t, 2), at(&t, 3)), (co(1), co(6)));
}

#[test]
fn display_col_in_line_matches_str_width_across_chunk_boundaries() {
    let (r, probes) = chunk_straddling_text();
    let text = r.to_string();
    let boundaries = segmentation_boundaries(&text);
    let chars: Vec<char> = text.chars().collect();
    let slice = r.slice(..);
    let mut checked = 0;
    for pos in probes.into_iter().filter(|p| boundaries.contains(p)) {
        let prefix: String = chars[..pos].iter().collect();
        assert_eq!(
            display_col_in_line(slice, ContentLine::new(0), co(pos), 4),
            dc(crate::width::str_width(&prefix, 0, 4) as u32),
            "at {pos}"
        );
        checked += 1;
    }
    assert!(checked > 0);
}
