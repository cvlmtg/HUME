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

// ── wire_to_char ─────────────────────────────────────────────────────────

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
