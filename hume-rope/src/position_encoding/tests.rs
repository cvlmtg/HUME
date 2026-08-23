use super::*;

/// Char layout (9 chars, 4 lines):
/// ```text
/// idx:  0  1  2   3  4   5   6    7  8
/// char: a  b  \n  c  é   \n  😀   d  \n
/// line: 0  0  0   1  1   1   2    2  2      (line 3 = trailing empty line)
/// ```
/// `é` is 1 char / 2 UTF-8 bytes / 1 UTF-16 unit.
/// `😀` is 1 char / 4 UTF-8 bytes / 2 UTF-16 units (a surrogate pair).
fn fixture() -> Rope {
    Rope::from_str("ab\ncé\n\u{1F600}d\n")
}

// ── char_to_wire ─────────────────────────────────────────────────────────

#[test]
fn char_to_wire_pure_ascii() {
    let text = fixture();
    assert_eq!(char_to_wire(&text, 0, PositionEncoding::Utf8), (0, 0));
    assert_eq!(char_to_wire(&text, 1, PositionEncoding::Utf8), (0, 1));
    assert_eq!(char_to_wire(&text, 0, PositionEncoding::Utf16), (0, 0));
    assert_eq!(char_to_wire(&text, 1, PositionEncoding::Utf16), (0, 1));
}

#[test]
fn char_to_wire_two_byte_char_diverges_utf8_vs_utf16() {
    let text = fixture();
    // char_idx=5 is the '\n' right after 'é' — UTF-8 counts é as 2 bytes,
    // UTF-16 counts it as 1 unit, so the two encodings diverge here.
    assert_eq!(char_to_wire(&text, 5, PositionEncoding::Utf8), (1, 3));
    assert_eq!(char_to_wire(&text, 5, PositionEncoding::Utf16), (1, 2));
}

#[test]
fn char_to_wire_astral_char_diverges_utf8_vs_utf16() {
    let text = fixture();
    // char_idx=6 is 😀 itself — both encodings agree at its start (0 code
    // units consumed yet).
    assert_eq!(char_to_wire(&text, 6, PositionEncoding::Utf8), (2, 0));
    assert_eq!(char_to_wire(&text, 6, PositionEncoding::Utf16), (2, 0));
    // char_idx=7 is 'd', right after 😀 — 4 bytes vs. a 2-unit surrogate
    // pair.
    assert_eq!(char_to_wire(&text, 7, PositionEncoding::Utf8), (2, 4));
    assert_eq!(char_to_wire(&text, 7, PositionEncoding::Utf16), (2, 2));
}

#[test]
fn char_to_wire_line_start_and_on_the_newline() {
    let text = fixture();
    // Line starts.
    assert_eq!(char_to_wire(&text, 3, PositionEncoding::Utf8), (1, 0));
    assert_eq!(char_to_wire(&text, 6, PositionEncoding::Utf8), (2, 0));
    // Sitting exactly on a '\n' reports that line's content length.
    assert_eq!(char_to_wire(&text, 2, PositionEncoding::Utf8), (0, 2));
}

#[test]
fn char_to_wire_eof_is_the_trailing_empty_line() {
    let text = fixture();
    assert_eq!(text.len_chars(), 9);
    assert_eq!(char_to_wire(&text, 9, PositionEncoding::Utf8), (3, 0));
    assert_eq!(char_to_wire(&text, 9, PositionEncoding::Utf16), (3, 0));
}

#[test]
fn char_to_wire_clamps_past_end_char_idx_instead_of_panicking() {
    let text = fixture();
    assert_eq!(
        char_to_wire(&text, 9_999, PositionEncoding::Utf8),
        char_to_wire(&text, 9, PositionEncoding::Utf8)
    );
}

#[test]
fn char_to_wire_minimum_buffer_is_a_bare_newline() {
    // The buffer invariant: every buffer ends with '\n'; "\n" alone is
    // the minimum possible buffer.
    let text = Rope::from_str("\n");
    assert_eq!(char_to_wire(&text, 0, PositionEncoding::Utf8), (0, 0));
    assert_eq!(char_to_wire(&text, 1, PositionEncoding::Utf8), (1, 0));
}

// ── char_range_to_wire_range ─────────────────────────────────────────────

#[test]
fn char_range_to_wire_range_is_char_to_wire_on_each_end() {
    let text = fixture();
    assert_eq!(
        char_range_to_wire_range(&text, 4, 7, PositionEncoding::Utf16),
        (
            char_to_wire(&text, 4, PositionEncoding::Utf16),
            char_to_wire(&text, 7, PositionEncoding::Utf16),
        )
    );
}

#[test]
fn char_range_to_wire_range_astral_char_diverges_utf8_vs_utf16() {
    let text = fixture();
    // Range spanning 😀 (char_idx 6..7) — UTF-8 counts it as 4 bytes,
    // UTF-16 as a 2-unit surrogate pair.
    assert_eq!(
        char_range_to_wire_range(&text, 6, 7, PositionEncoding::Utf8),
        ((2, 0), (2, 4))
    );
    assert_eq!(
        char_range_to_wire_range(&text, 6, 7, PositionEncoding::Utf16),
        ((2, 0), (2, 2))
    );
}

// ── CRLF line terminator ─────────────────────────────────────────────────
//
// `hume_editing::text::Text` normalizes `\r\n` to `\n` on load, but a `\r\n`
// can still reach a live rope in one edge case: `Text::from`'s single-pass
// CRLF strip leaves a literal `\r\n` behind when the input has a bare `\r`
// immediately before a `\r\n` pair (e.g. `"\r\r\n"` → `"\r\n"` — see
// `text::tests::from_str_cr_then_crlf_leaves_bare_cr`). These functions take
// a raw `&Rope`, not a `&Text`, so a `\r\n`-bearing rope is constructed
// directly here rather than routing through that edge case.

#[test]
fn line_content_end_stops_before_crlf_not_just_lf() {
    // "ab\r\ncd\n": line 0's terminator is the 2-char "\r\n" pair, not a bare
    // '\n'. content_end must land on the '\r' (char 2), one further back than
    // it would for a plain '\n' terminator (which would land on char 3) —
    // this is `line_content_end_char`'s CRLF-aware `next_start - 2` branch,
    // not the "\n"-only `next_start - 1` branch.
    let text = Rope::from_str("ab\r\ncd\n");
    for enc in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
        assert_eq!(
            wire_to_char(&text, 0, 9_999, enc),
            2,
            "character past line end must clamp before the \\r, not the \\n, for {enc:?}"
        );
    }
}

#[test]
fn wire_to_char_matches_char_to_wire_for_exact_positions() {
    let text = fixture();
    for &idx in &[0usize, 1, 2, 3, 4, 5, 6, 7, 8, 9] {
        for enc in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
            let (line, character) = char_to_wire(&text, idx, enc);
            assert_eq!(
                wire_to_char(&text, line, character, enc),
                idx,
                "round trip failed for idx={idx} enc={enc:?}"
            );
        }
    }
}

#[test]
fn wire_to_char_clamps_line_past_eof_to_last_line() {
    let text = fixture();
    assert_eq!(
        wire_to_char(&text, 999, 0, PositionEncoding::Utf8),
        text.len_chars()
    );
}

#[test]
fn wire_to_char_clamps_character_past_line_end_to_line_content_end() {
    let text = fixture();
    // Line 1 ("cé") — character way past its length clamps to the same
    // char position as an exact request for the line's content end.
    assert_eq!(
        wire_to_char(&text, 1, 9_999, PositionEncoding::Utf8),
        5 // the '\n' position — see the fixture layout above
    );
}

#[test]
fn wire_to_char_clamps_surrogate_pair_split_down_not_mid_char() {
    let text = fixture();
    // Line 2 starts with 😀 (a 2-unit surrogate pair at UTF-16 units
    // [0, 2) within the line). character=1 lands on the low surrogate —
    // must clamp down to the astral char's own start (char_idx 6), never
    // to a position "inside" it.
    assert_eq!(wire_to_char(&text, 2, 1, PositionEncoding::Utf16), 6);
}

#[test]
fn wire_to_char_minimum_buffer_is_a_bare_newline() {
    let text = Rope::from_str("\n");
    assert_eq!(wire_to_char(&text, 0, 0, PositionEncoding::Utf8), 0);
    // Past-end line/character both clamp to the sole valid EOF position.
    assert_eq!(wire_to_char(&text, 5, 5, PositionEncoding::Utf8), 1);
}

// ── wire_range_to_char_range ─────────────────────────────────────────────

#[test]
fn wire_range_to_char_range_is_wire_to_char_on_each_end() {
    let text = fixture();
    assert_eq!(
        wire_range_to_char_range(&text, (0, 0), (1, 1), PositionEncoding::Utf8),
        (
            wire_to_char(&text, 0, 0, PositionEncoding::Utf8),
            wire_to_char(&text, 1, 1, PositionEncoding::Utf8),
        )
    );
}

#[test]
fn wire_range_to_char_range_each_end_clamps_independently() {
    let text = fixture();
    // Line 1 ("cé") — character way past its length clamps to that line's
    // content end, same as a lone wire_to_char call.
    assert_eq!(
        wire_range_to_char_range(&text, (1, 0), (1, 9_999), PositionEncoding::Utf8),
        (3, 5)
    );
}

#[test]
fn wire_range_to_char_range_reversed_input_is_not_reordered() {
    let text = fixture();
    // end before start on the wire — the pair comes back reversed too, not
    // swapped into order. Callers that must reject this check it themselves.
    let (start, end) = wire_range_to_char_range(&text, (1, 1), (0, 0), PositionEncoding::Utf8);
    assert_eq!((start, end), (4, 0));
    assert!(end < start);
}

#[test]
fn wire_range_to_char_range_round_trips_through_char_range_to_wire_range() {
    let text = fixture();
    for enc in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
        let (start, end) = char_range_to_wire_range(&text, 1, 8, enc);
        assert_eq!(wire_range_to_char_range(&text, start, end, enc), (1, 8));
    }
}

// ── wire_offset_to_char ──────────────────────────────────────────────────

/// A `SignatureInformation.label`-shaped string: no newlines, and all three
/// code-unit counts different past the first char.
/// ```text
/// char:   a   é    😀      b
/// byte:   0   1    3       7      (end 8)
/// char:   0   1    2       3      (end 4)
/// utf16:  0   1    2       4      (end 5)
/// ```
const LABEL: &str = "aé\u{1F600}b";

#[test]
fn wire_offset_to_char_counts_bytes_in_utf8_and_code_units_in_utf16() {
    let text = RopeSlice::from(LABEL);
    // 😀 starts at byte 3 and at UTF-16 unit 2 — one char index, two
    // different offsets naming it.
    assert_eq!(wire_offset_to_char(text, 3, PositionEncoding::Utf8), 2);
    assert_eq!(wire_offset_to_char(text, 2, PositionEncoding::Utf16), 2);
    // 'b' follows it by 4 bytes, but by only 2 UTF-16 units.
    assert_eq!(wire_offset_to_char(text, 7, PositionEncoding::Utf8), 3);
    assert_eq!(wire_offset_to_char(text, 4, PositionEncoding::Utf16), 3);
}

#[test]
fn wire_offset_to_char_rounds_a_split_char_down_to_its_start() {
    let text = RopeSlice::from(LABEL);
    // Byte 2 is é's continuation byte; UTF-16 unit 3 is 😀's low surrogate.
    // Both name a position inside a char, and both round back to its start.
    assert_eq!(wire_offset_to_char(text, 2, PositionEncoding::Utf8), 1);
    assert_eq!(wire_offset_to_char(text, 3, PositionEncoding::Utf16), 2);
}

#[test]
fn wire_offset_to_char_clamps_past_the_end_of_the_text() {
    let text = RopeSlice::from(LABEL);
    for enc in [PositionEncoding::Utf8, PositionEncoding::Utf16] {
        assert_eq!(wire_offset_to_char(text, 9_999, enc), 4);
    }
}

// ── wire_offsets_to_byte_range ───────────────────────────────────────────

#[test]
fn wire_offsets_to_byte_range_slices_the_same_text_from_either_encoding() {
    // One 😀, named by two different offset pairs — the divergence any
    // hardcoded-UTF-16 scan gets wrong the moment a server negotiates utf-8.
    assert_eq!(
        &LABEL[wire_offsets_to_byte_range(LABEL, 3, 7, PositionEncoding::Utf8)],
        "\u{1F600}"
    );
    assert_eq!(
        &LABEL[wire_offsets_to_byte_range(LABEL, 2, 4, PositionEncoding::Utf16)],
        "\u{1F600}"
    );
}

#[test]
fn wire_offsets_to_byte_range_clamps_a_past_end_pair_to_the_whole_tail() {
    assert_eq!(
        &LABEL[wire_offsets_to_byte_range(LABEL, 1, 9_999, PositionEncoding::Utf16)],
        "é\u{1F600}b"
    );
}

#[test]
fn wire_offsets_to_byte_range_orders_a_reversed_pair_into_an_empty_range() {
    // Indexing a `&str` with a reversed range panics, so unlike
    // `wire_range_to_char_range` this one must not pass one through.
    let range = wire_offsets_to_byte_range(LABEL, 4, 2, PositionEncoding::Utf16);
    assert!(range.is_empty());
    assert_eq!(&LABEL[range], "");
}
