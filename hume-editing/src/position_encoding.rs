//! Rope char offset ↔ LSP wire `(line, character)` position conversion, in
//! both negotiated encodings (see the LSP hub's *Position encoding*
//! decision: negotiate `utf-8`, fall back to UTF-16). `hume-editing` must
//! not depend on `lsp-types`, so [`PositionEncoding`] mirrors the wire
//! `Position`/encoding concept as a plain type; `hume-lsp` converts to/from
//! `lsp_types::Position`.
//!
//! Wire math is **not** motion math: `character` counts code units in the
//! negotiated encoding, never a grapheme or a raw byte. The grapheme
//! helpers in [`crate::grapheme`] are the wrong tool in this module — do not
//! reach for them here (hub: testing playbook).

use ropey::Rope;

/// Wire-format position encoding negotiated with an LSP server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionEncoding {
    Utf8,
    Utf16,
}

/// char offset → `(line, character)` in `enc` code units.
///
/// Total: a `char_idx` past `text.len_chars()` clamps to the document end
/// rather than panicking (ropey's own indexing functions panic past
/// `len_chars()`) — mirrors [`wire_to_char`]'s clamp-don't-error convention.
pub fn char_to_wire(text: &Rope, char_idx: usize, enc: PositionEncoding) -> (usize, usize) {
    let char_idx = char_idx.min(text.len_chars());
    let line = text.char_to_line(char_idx);
    let character = match enc {
        PositionEncoding::Utf8 => text.char_to_byte(char_idx) - text.line_to_byte(line),
        PositionEncoding::Utf16 => {
            let line_start = text.line_to_char(line);
            text.char_to_utf16_cu(char_idx) - text.char_to_utf16_cu(line_start)
        }
    };
    (line, character)
}

/// `(line, character)` → char offset.
///
/// Out-of-range input clamps rather than errors — servers send past-end
/// positions routinely: `line` past EOF clamps to the last line; `character`
/// past the line's content clamps to the line's end. Clamping happens in
/// code-unit space (byte or UTF-16-code-unit) before converting back to a
/// char index, so a `character` that would split a multi-byte char or a
/// UTF-16 surrogate pair clamps down to that char's start rather than
/// landing mid-char — `Rope::byte_to_char`/`utf16_cu_to_char` guarantee this
/// for any in-bounds code-unit index, on-boundary or not.
pub fn wire_to_char(text: &Rope, line: usize, character: usize, enc: PositionEncoding) -> usize {
    let line = line.min(text.len_lines().saturating_sub(1));
    let line_start = text.line_to_char(line);
    let content_end = line_content_end_char(text, line);

    match enc {
        PositionEncoding::Utf8 => {
            let line_start_byte = text.line_to_byte(line);
            let content_end_byte = text.char_to_byte(content_end);
            let byte = (line_start_byte + character).min(content_end_byte);
            text.byte_to_char(byte)
        }
        PositionEncoding::Utf16 => {
            let line_start_utf16 = text.char_to_utf16_cu(line_start);
            let content_end_utf16 = text.char_to_utf16_cu(content_end);
            let cu = (line_start_utf16 + character).min(content_end_utf16);
            text.utf16_cu_to_char(cu)
        }
    }
}

/// The char offset one past the last content char on `line` — the position
/// of `line`'s terminator (`\n` or `\r\n`), or the line's own start when it
/// has none (the buffer's trailing empty line, guaranteed by the "buffer
/// always ends with `\n`" invariant).
///
/// This is the wire-domain "end of line": defined by content length, not by
/// where a cursor may land. `hume_editing::lines`' motion-domain helpers
/// (e.g. `line_content_end`) define the latter — including a position sitting
/// *on* the `\n` — and must not be reused here; the two disagree by design
/// for an empty line, and wire math needs this one.
fn line_content_end_char(text: &Rope, line: usize) -> usize {
    if line + 1 >= text.len_lines() {
        return text.len_chars();
    }
    let next_start = text.line_to_char(line + 1);
    // A non-final line's span always contains at least its terminator, so
    // `next_start > line_to_char(line) >= 0` and this lookup is in-bounds.
    if next_start >= 2 && text.char(next_start - 1) == '\n' && text.char(next_start - 2) == '\r' {
        next_start - 2
    } else {
        next_start - 1
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
}
