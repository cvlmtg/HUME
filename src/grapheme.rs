use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

use crate::buffer::Buffer;

/// Returns the char offset of the start of the *next* grapheme cluster after
/// `char_offset`.
///
/// A grapheme cluster is what a user perceives as a single character — an
/// ASCII letter, a combining sequence like `é` (U+0065 + U+0301), or a
/// multi-codepoint emoji like `👨‍👩‍👧` (joined via Zero Width Joiner). Using
/// grapheme boundaries ensures cursor movement never lands mid-cluster.
///
/// Returns `buf.len_chars()` when `char_offset` is already at (or past) the
/// end of the buffer.
///
/// # Why byte offsets internally?
///
/// `GraphemeCursor` (from `unicode-segmentation`) operates in *byte* space
/// because Unicode break algorithms work on UTF-8 encoded bytes. We convert
/// the caller-facing char offset to a byte offset, run the cursor, then
/// convert the result back — byte offsets never leave this module.
///
/// # Why chunks instead of a full `&str`?
///
/// Ropey stores the rope as a B-tree of `&str` chunks. Materializing the
/// whole buffer into a single `String` just to walk one boundary would be
/// O(n) in space and time. `GraphemeCursor` supports a chunk-at-a-time API
/// (`next_boundary` / `provide_context`) that lets us stay O(log n) and
/// allocation-free.
pub(crate) fn next_grapheme_boundary(buf: &Buffer, char_offset: usize) -> usize {
    let len_chars = buf.len_chars();
    if char_offset >= len_chars {
        return len_chars;
    }

    let slice = buf.full_slice();
    let len_bytes = slice.len_bytes();
    let byte_offset = slice.char_to_byte(char_offset);

    // Start with the chunk that contains `byte_offset`.
    let (mut chunk, mut chunk_byte_start, _, _) = slice.chunk_at_byte(byte_offset);

    let mut gc = GraphemeCursor::new(byte_offset, len_bytes, true);

    loop {
        match gc.next_boundary(chunk, chunk_byte_start) {
            Ok(None) => return len_chars,
            Ok(Some(b)) => return slice.byte_to_char(b),

            // The cursor needs the next chunk of the rope.
            Err(GraphemeIncomplete::NextChunk) => {
                let next_byte = chunk_byte_start + chunk.len();
                if next_byte >= len_bytes {
                    // No more chunks — treat as end.
                    return len_chars;
                }
                let (c, s, _, _) = slice.chunk_at_byte(next_byte);
                chunk = c;
                chunk_byte_start = s;
            }

            // The cursor needs context from *before* the current position to
            // resolve a boundary that depends on a preceding codepoint (e.g.
            // Regional Indicator pairs, ZWJ sequences).
            Err(GraphemeIncomplete::PreContext(n)) => {
                let (ctx_chunk, ctx_start, _, _) = slice.chunk_at_byte(n - 1);
                gc.provide_context(ctx_chunk, ctx_start);
            }

            // All other variants are unreachable when using the public API
            // correctly — `next_boundary` only returns the three above.
            Err(_) => unreachable!("unexpected GraphemeIncomplete variant"),
        }
    }
}

/// Returns the char offset of the start of the grapheme cluster *before*
/// `char_offset`.
///
/// Returns `0` when `char_offset` is already at the start of the buffer.
pub(crate) fn prev_grapheme_boundary(buf: &Buffer, char_offset: usize) -> usize {
    if char_offset == 0 {
        return 0;
    }

    let slice = buf.full_slice();
    let len_bytes = slice.len_bytes();
    let byte_offset = slice.char_to_byte(char_offset);

    // Start one byte before `byte_offset` to land inside the preceding
    // cluster — we want the chunk that *contains* the last byte of that
    // cluster, not the chunk that starts exactly at `byte_offset`.
    let (mut chunk, mut chunk_byte_start, _, _) = slice.chunk_at_byte(byte_offset - 1);

    let mut gc = GraphemeCursor::new(byte_offset, len_bytes, true);

    loop {
        match gc.prev_boundary(chunk, chunk_byte_start) {
            Ok(None) => return 0,
            Ok(Some(b)) => return slice.byte_to_char(b),

            // The cursor needs the previous chunk.
            Err(GraphemeIncomplete::PrevChunk) => {
                if chunk_byte_start == 0 {
                    return 0;
                }
                let (c, s, _, _) = slice.chunk_at_byte(chunk_byte_start - 1);
                chunk = c;
                chunk_byte_start = s;
            }

            Err(GraphemeIncomplete::PreContext(n)) => {
                let (ctx_chunk, ctx_start, _, _) = slice.chunk_at_byte(n - 1);
                gc.provide_context(ctx_chunk, ctx_start);
            }

            Err(_) => unreachable!("unexpected GraphemeIncomplete variant"),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    // ── ASCII ─────────────────────────────────────────────────────────────────

    #[test]
    fn ascii_next_single_step() {
        let buf = Buffer::from_str("hello");
        assert_eq!(next_grapheme_boundary(&buf, 0), 1);
        assert_eq!(next_grapheme_boundary(&buf, 1), 2);
        assert_eq!(next_grapheme_boundary(&buf, 4), 5);
    }

    #[test]
    fn ascii_next_walk() {
        // Walk forward through every grapheme in an ASCII string.
        // Each ASCII char is its own grapheme, so boundaries are 0,1,2,…,5.
        let buf = Buffer::from_str("hello");
        let boundaries: Vec<usize> =
            std::iter::successors(Some(0usize), |&c| {
                let n = next_grapheme_boundary(&buf, c);
                if n > c { Some(n) } else { None }
            })
            .collect();
        assert_eq!(boundaries, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn ascii_prev_single_step() {
        let buf = Buffer::from_str("hello");
        assert_eq!(prev_grapheme_boundary(&buf, 5), 4);
        assert_eq!(prev_grapheme_boundary(&buf, 1), 0);
    }

    // ── Combining character (é = U+0065 + U+0301) ─────────────────────────────

    #[test]
    fn combining_char_next() {
        // "e\u{0301}x" is 3 chars but 2 grapheme clusters: ["é", "x"].
        // next(0) must skip both chars of the combining cluster and return 2.
        let buf = Buffer::from_str("e\u{0301}x");
        assert_eq!(buf.len_chars(), 3);
        assert_eq!(next_grapheme_boundary(&buf, 0), 2); // skip the whole é cluster
        assert_eq!(next_grapheme_boundary(&buf, 2), 3); // x
    }

    #[test]
    fn combining_char_next_mid_cluster() {
        // Offset 1 is *inside* the é cluster (between 'e' and U+0301).
        // next() should still find the next boundary at 2, not at 1+1=2
        // by coincidence — it must consult the grapheme algorithm.
        let buf = Buffer::from_str("e\u{0301}x");
        assert_eq!(next_grapheme_boundary(&buf, 1), 2);
    }

    #[test]
    fn combining_char_prev_mid_cluster() {
        // prev(1) from inside the é cluster should return 0 (start of cluster),
        // not 1-1=0 by coincidence — test with a prefix to break the coincidence.
        // "ae\u{0301}x" — offset 2 is inside the é cluster (between 'e' and U+0301).
        let buf = Buffer::from_str("ae\u{0301}x");
        assert_eq!(buf.len_chars(), 4);
        assert_eq!(prev_grapheme_boundary(&buf, 2), 1); // back to start of é, not to 'a'
    }

    #[test]
    fn combining_char_prev() {
        // prev from end of "é" (char offset 2) must jump back to 0, not to 1.
        let buf = Buffer::from_str("e\u{0301}x");
        assert_eq!(prev_grapheme_boundary(&buf, 2), 0);
        assert_eq!(prev_grapheme_boundary(&buf, 3), 2);
    }

    // ── ZWJ emoji (👨‍👩‍👧 = 5 codepoints joined by ZWJ) ──────────────────────────

    #[test]
    fn zwj_emoji_next() {
        // U+1F468 ZWJ U+1F469 ZWJ U+1F467 — 5 chars, 1 grapheme cluster.
        // next(0) must return 5 — the whole family is one cluster.
        let buf = Buffer::from_str("👨‍👩‍👧");
        assert_eq!(buf.len_chars(), 5);
        assert_eq!(next_grapheme_boundary(&buf, 0), 5);
    }

    #[test]
    fn zwj_emoji_prev() {
        let buf = Buffer::from_str("👨‍👩‍👧");
        assert_eq!(prev_grapheme_boundary(&buf, 5), 0);
    }

    // ── Mixed string with multiple grapheme types ─────────────────────────────

    #[test]
    fn mixed_string_boundaries() {
        // "Hello 👨‍👩‍👧!" — chars: H(0) e(1) l(2) l(3) o(4) (space)(5)
        //                         👨(6) ZWJ(7) 👩(8) ZWJ(9) 👧(10) !(11)
        // Graphemes: H, e, l, l, o, ' ', [👨‍👩‍👧], !
        // Boundaries: 0, 1, 2, 3, 4, 5, 6, 11, 12
        let buf = Buffer::from_str("Hello 👨‍👩‍👧!");
        assert_eq!(buf.len_chars(), 12);

        let expected = vec![0usize, 1, 2, 3, 4, 5, 6, 11, 12];
        let got: Vec<usize> =
            std::iter::successors(Some(0usize), |&c| {
                let n = next_grapheme_boundary(&buf, c);
                if n > c { Some(n) } else { None }
            })
            .collect();
        assert_eq!(got, expected);
    }

    // ── Edge cases ────────────────────────────────────────────────────────────

    #[test]
    fn next_at_end_returns_len() {
        let buf = Buffer::from_str("hi");
        assert_eq!(next_grapheme_boundary(&buf, 2), 2); // at end
        assert_eq!(next_grapheme_boundary(&buf, 99), 2); // past end — clamped
    }

    #[test]
    fn prev_at_start_returns_zero() {
        let buf = Buffer::from_str("hi");
        assert_eq!(prev_grapheme_boundary(&buf, 0), 0);
    }

    #[test]
    fn empty_buffer_next() {
        let buf = Buffer::empty();
        assert_eq!(next_grapheme_boundary(&buf, 0), 0);
    }

    #[test]
    fn empty_buffer_prev() {
        let buf = Buffer::empty();
        assert_eq!(prev_grapheme_boundary(&buf, 0), 0);
    }
}
