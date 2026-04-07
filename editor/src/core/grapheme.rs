use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

use crate::core::buffer::Buffer;


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
    // chunk_at_byte returns (chunk, byte_start, char_start, line_start); we only
    // need the chunk text and its byte offset — the char/line starts are unused.
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

/// Count grapheme clusters in the char range `[from_char, to_char)`.
///
/// `to_char` is an **exclusive** upper bound — the character at `to_char` is
/// not itself counted. For example, if the cursor sits at char offset `c`,
/// `grapheme_count(buf, line_start, c)` returns the number of grapheme
/// clusters that precede the cursor on that line — its 0-based grapheme
/// column.
///
/// If `to_char < from_char` the range is treated as empty and 0 is returned.
///
/// # Why chunk-based?
///
/// The naïve alternative is `buf.slice(from..to).to_string().graphemes(true).count()`,
/// which allocates a heap String proportional to line length. Long lines
/// (minified JSON, generated files, log files with no newlines) can be
/// arbitrarily wide. This implementation uses the same chunk-at-a-time
/// `GraphemeCursor` strategy as `next_grapheme_boundary` — O(log n) per
/// cluster with no heap allocation.
pub(crate) fn grapheme_count(buf: &Buffer, from_char: usize, to_char: usize) -> usize {
    let to_char = to_char.max(from_char);
    if from_char == to_char {
        return 0;
    }

    let slice = buf.full_slice();
    let len_bytes = slice.len_bytes();
    let from_byte = slice.char_to_byte(from_char);
    let to_byte = slice.char_to_byte(to_char);

    let (mut chunk, mut chunk_byte_start, _, _) = slice.chunk_at_byte(from_byte);
    let mut gc = GraphemeCursor::new(from_byte, len_bytes, true);
    let mut count = 0;

    loop {
        match gc.next_boundary(chunk, chunk_byte_start) {
            Ok(None) => return count,
            Ok(Some(b)) => {
                if b > to_byte {
                    return count;
                }
                count += 1;
                if b == to_byte {
                    return count;
                }
            }
            Err(GraphemeIncomplete::NextChunk) => {
                let next_byte = chunk_byte_start + chunk.len();
                if next_byte >= len_bytes {
                    return count;
                }
                let (c, s, _, _) = slice.chunk_at_byte(next_byte);
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

/// 0-based grapheme column of `char_pos` within line `line_idx`.
///
/// This is a logical position (grapheme index), not a display column: wide
/// characters count as one, not two. The value matches how many times the
/// user pressed → to reach the cursor from the start of the line.
pub(crate) fn grapheme_col_in_line(buf: &Buffer, line_idx: usize, char_pos: usize) -> usize {
    grapheme_count(buf, buf.line_to_char(line_idx), char_pos)
}


// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    // ── ASCII ─────────────────────────────────────────────────────────────────

    #[test]
    fn ascii_next_single_step() {
        let buf = Buffer::from("hello");
        assert_eq!(next_grapheme_boundary(&buf, 0), 1);
        assert_eq!(next_grapheme_boundary(&buf, 1), 2);
        assert_eq!(next_grapheme_boundary(&buf, 4), 5);
    }

    #[test]
    fn ascii_next_walk() {
        // Walk forward through every grapheme in "hello\n" (6 chars).
        // Each char is its own grapheme, so boundaries are 0,1,2,…,6.
        let buf = Buffer::from("hello");
        let boundaries: Vec<usize> =
            std::iter::successors(Some(0usize), |&c| {
                let n = next_grapheme_boundary(&buf, c);
                if n > c { Some(n) } else { None }
            })
            .collect();
        assert_eq!(boundaries, vec![0, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn ascii_prev_single_step() {
        let buf = Buffer::from("hello");
        assert_eq!(prev_grapheme_boundary(&buf, 5), 4);
        assert_eq!(prev_grapheme_boundary(&buf, 1), 0);
    }

    // ── Combining character (é = U+0065 + U+0301) ─────────────────────────────

    #[test]
    fn combining_char_next() {
        // "e\u{0301}x\n" is 4 chars, 3 grapheme clusters: ["é", "x", "\n"].
        // next(0) must skip both chars of the combining cluster and return 2.
        let buf = Buffer::from("e\u{0301}x");
        assert_eq!(buf.len_chars(), 4);
        assert_eq!(next_grapheme_boundary(&buf, 0), 2); // skip the whole é cluster
        assert_eq!(next_grapheme_boundary(&buf, 2), 3); // x → \n boundary
    }

    #[test]
    fn combining_char_next_mid_cluster() {
        // Offset 1 is *inside* the é cluster (between 'e' and U+0301).
        // next() should still find the next boundary at 2, not at 1+1=2
        // by coincidence — it must consult the grapheme algorithm.
        let buf = Buffer::from("e\u{0301}x");
        assert_eq!(next_grapheme_boundary(&buf, 1), 2);
    }

    #[test]
    fn combining_char_prev_mid_cluster() {
        // prev(1) from inside the é cluster should return 0 (start of cluster),
        // not 1-1=0 by coincidence — test with a prefix to break the coincidence.
        // "ae\u{0301}x\n" — offset 2 is inside the é cluster (between 'e' and U+0301).
        let buf = Buffer::from("ae\u{0301}x");
        assert_eq!(buf.len_chars(), 5);
        assert_eq!(prev_grapheme_boundary(&buf, 2), 1); // back to start of é, not to 'a'
    }

    #[test]
    fn combining_char_prev() {
        // prev from end of "é" (char offset 2) must jump back to 0, not to 1.
        let buf = Buffer::from("e\u{0301}x");
        assert_eq!(prev_grapheme_boundary(&buf, 2), 0);
        assert_eq!(prev_grapheme_boundary(&buf, 3), 2);
    }

    // ── ZWJ emoji (👨‍👩‍👧 = 5 codepoints joined by ZWJ) ──────────────────────────

    #[test]
    fn zwj_emoji_next() {
        // U+1F468 ZWJ U+1F469 ZWJ U+1F467 — 5 chars, 1 grapheme cluster; + "\n".
        // next(0) must return 5 — the whole family is one cluster.
        let buf = Buffer::from("👨‍👩‍👧");
        assert_eq!(buf.len_chars(), 6); // 5 emoji chars + \n
        assert_eq!(next_grapheme_boundary(&buf, 0), 5);
    }

    #[test]
    fn zwj_emoji_prev() {
        let buf = Buffer::from("👨‍👩‍👧");
        assert_eq!(prev_grapheme_boundary(&buf, 5), 0);
    }

    // ── Mixed string with multiple grapheme types ─────────────────────────────

    #[test]
    fn mixed_string_boundaries() {
        // "Hello 👨‍👩‍👧!\n" — chars: H(0) e(1) l(2) l(3) o(4) (space)(5)
        //                           👨(6) ZWJ(7) 👩(8) ZWJ(9) 👧(10) !(11) \n(12)
        // Graphemes: H, e, l, l, o, ' ', [👨‍👩‍👧], !, \n
        // Boundaries: 0, 1, 2, 3, 4, 5, 6, 11, 12, 13
        let buf = Buffer::from("Hello 👨‍👩‍👧!");
        assert_eq!(buf.len_chars(), 13);

        let expected = vec![0usize, 1, 2, 3, 4, 5, 6, 11, 12, 13];
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
        // "hi\n" is 3 chars. next(2) steps past '\n' to len_chars=3.
        let buf = Buffer::from("hi");
        assert_eq!(next_grapheme_boundary(&buf, 2), 3); // '\n' → one past it = len_chars
        assert_eq!(next_grapheme_boundary(&buf, 99), 3); // past end — clamped to len_chars
    }

    #[test]
    fn prev_at_start_returns_zero() {
        let buf = Buffer::from("hi");
        assert_eq!(prev_grapheme_boundary(&buf, 0), 0);
    }

    #[test]
    fn empty_buffer_next() {
        // Buffer::empty() = "\n" (1 char). next(0) steps past '\n' to len_chars=1.
        let buf = Buffer::empty();
        assert_eq!(next_grapheme_boundary(&buf, 0), 1);
    }

    #[test]
    fn empty_buffer_prev() {
        let buf = Buffer::empty();
        assert_eq!(prev_grapheme_boundary(&buf, 0), 0);
    }

    // ── Complex Unicode grapheme clusters ─────────────────────────────────────

    #[test]
    fn regional_indicator_flag_emoji() {
        // 🇺🇸 is U+1F1FA (regional indicator U) + U+1F1F8 (regional indicator S).
        // Both codepoints form a single grapheme cluster. next from 0 must skip
        // both to land at 2.
        let buf = Buffer::from("\u{1F1FA}\u{1F1F8}");
        // buf: U+1F1FA(0) U+1F1F8(1) '\n'(2) = 3 chars
        assert_eq!(next_grapheme_boundary(&buf, 0), 2);
        assert_eq!(prev_grapheme_boundary(&buf, 2), 0);
    }

    #[test]
    fn devanagari_vowel_sign() {
        // "क" (U+0915) + "ा" (U+093E vowel sign aa) form one grapheme cluster.
        let buf = Buffer::from("\u{0915}\u{093E}");
        // buf: U+0915(0) U+093E(1) '\n'(2) = 3 chars
        assert_eq!(next_grapheme_boundary(&buf, 0), 2);
        assert_eq!(prev_grapheme_boundary(&buf, 2), 0);
    }

    // ── grapheme_count ────────────────────────────────────────────────────────

    #[test]
    fn grapheme_count_ascii() {
        let buf = Buffer::from("hello\n");
        // "hello" = 5 graphemes; line starts at 0
        assert_eq!(grapheme_count(&buf, 0, 5), 5);
    }

    #[test]
    fn grapheme_count_zero_range() {
        let buf = Buffer::from("hello\n");
        assert_eq!(grapheme_count(&buf, 2, 2), 0);
    }

    #[test]
    fn grapheme_count_combining_char() {
        // "e\u{0301}x" = 3 chars but 2 grapheme clusters ("é", "x") + structural \n
        let buf = Buffer::from("e\u{0301}x\n");
        // from char 0 to char 2 (past the combining cluster): 1 grapheme
        assert_eq!(grapheme_count(&buf, 0, 2), 1);
        // from char 0 to char 3 (past "x"): 2 graphemes
        assert_eq!(grapheme_count(&buf, 0, 3), 2);
    }

    #[test]
    fn grapheme_count_zwj_emoji() {
        // 👨‍👩‍👧 = 5 codepoints, 1 grapheme cluster.
        // Buffer::from("👨‍👩‍👧\n"): the string already ends with \n so no extra is
        // added — total 6 chars (5 emoji codepoints + \n).
        let buf = Buffer::from("👨‍👩‍👧\n");
        assert_eq!(buf.len_chars(), 6); // 5 emoji chars + \n
        // from 0 to 5 (past the whole emoji): 1 grapheme
        assert_eq!(grapheme_count(&buf, 0, 5), 1);
    }

    #[test]
    fn grapheme_count_multiline_offset() {
        // "ab\ncd\n" — "cd" starts at char 3
        let buf = Buffer::from("ab\ncd\n");
        // from line 1 start (char 3) to char 5 (past "cd"): 2 graphemes
        assert_eq!(grapheme_count(&buf, 3, 5), 2);
        // from 3 to 3: 0
        assert_eq!(grapheme_count(&buf, 3, 3), 0);
    }

    #[test]
    fn grapheme_count_reversed_range_returns_zero() {
        // to_char < from_char is clamped to an empty range.
        let buf = Buffer::from("hello\n");
        assert_eq!(grapheme_count(&buf, 3, 1), 0);
    }

    #[test]
    fn grapheme_count_to_buffer_end() {
        // to_char == len_chars (the structural \n is the last char).
        // "hi\n" has len_chars = 3; counting from 0 to 3 covers h, i, \n = 3 graphemes.
        let buf = Buffer::from("hi\n");
        assert_eq!(buf.len_chars(), 3);
        assert_eq!(grapheme_count(&buf, 0, 3), 3);
    }

    // ── Invariant enforcement ─────────────────────────────────────────────────

    /// Scan motion-related source files for raw char-level stepping.
    ///
    /// The grapheme cluster invariant (CLAUDE.md) requires that all position
    /// advances in motion and selection code go through `next_grapheme_boundary`
    /// or `prev_grapheme_boundary` — never raw `pos += 1` / `pos -= 1`.
    ///
    /// The bug that prompted this test: word motions used `pos += 1`, causing
    /// combining codepoints (e.g. U+0301, which classify_char sees as Punctuation)
    /// to be treated as false word boundaries inside a grapheme cluster.
    ///
    /// This test reads the source files at compile time, skips test blocks and
    /// comment lines, and fails if any active code contains a forbidden stepping
    /// pattern on a char-position variable.
    #[test]
    fn no_raw_char_stepping_in_motion_code() {
        let manifest = std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR not set — run via `cargo test`");

        // All files whose position-manipulation code must use grapheme boundaries.
        let files = [
            "src/ops/motion.rs",
            "src/ops/text_object.rs",
            "src/ops/selection_cmd.rs",
            "src/ops/edit.rs",
            "src/auto_pairs.rs",
            "src/helpers.rs",
        ];

        // Forbidden patterns — raw +1/-1 steps on char-position variables.
        // Stepping by 1 skips over combining codepoints (e.g. é = U+0065 + U+0301)
        // instead of advancing by a full grapheme cluster.
        //
        // Assignment forms: caught directly.
        // char_at() forms: explicitly forbidden by CLAUDE.md — char_at(pos + 1) and
        //   char_at(pos - 1) were the original motivating footguns.
        let forbidden = [
            // ── Assignment forms ───────────────────────────────────────────────
            "pos += 1",   "pos -= 1",
            "start += 1", "start -= 1",
            "end += 1",   "end -= 1",
            "head += 1",  "head -= 1",
            "anchor += 1","anchor -= 1",
            // ── char_at() expression forms ─────────────────────────────────────
            "char_at(pos + 1)",    "char_at(pos - 1)",
            "char_at(head + 1)",   "char_at(head - 1)",
            "char_at(anchor + 1)", "char_at(anchor - 1)",
        ];

        let mut violations: Vec<String> = Vec::new();

        for file in &files {
            let path = format!("{manifest}/{file}");
            let src = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {file}: {e}"));

            // Track whether we are inside a `#[cfg(test)] mod tests { … }` block
            // so we don't flag historical references in test comments.
            let mut in_test_block = false;
            let mut brace_depth: i64 = 0;
            let mut test_entry_depth: i64 = 0;
            let mut saw_cfg_test = false;

            for (lineno, line) in src.lines().enumerate() {
                let trimmed = line.trim();

                // Detect `#[cfg(test)]` on its own line.
                if trimmed == "#[cfg(test)]" {
                    saw_cfg_test = true;
                }
                // The very next `mod tests` after that attribute opens the block.
                if saw_cfg_test && trimmed.starts_with("mod tests") {
                    in_test_block = true;
                    test_entry_depth = brace_depth;
                    saw_cfg_test = false;
                }

                // Track brace depth so we know when the test block closes.
                let opens = line.chars().filter(|&c| c == '{').count() as i64;
                let closes = line.chars().filter(|&c| c == '}').count() as i64;
                brace_depth += opens - closes;
                if in_test_block && brace_depth <= test_entry_depth {
                    in_test_block = false;
                }

                // Skip everything inside the test module.
                if in_test_block {
                    continue;
                }

                // Skip pure comment lines.
                if trimmed.starts_with("//") {
                    continue;
                }

                // `// grapheme-safe: <reason>` opt-out: lines where raw +1/-1 is
                // intentional and safe (e.g. ASCII-only delimiter arithmetic, or
                // converting a grapheme-boundary-aligned exclusive end to inclusive).
                // The reason after the colon must explain *why* it is safe.
                if line.contains("// grapheme-safe:") {
                    continue;
                }

                // Strip any remaining inline comment before pattern-matching.
                // This prevents explanatory comments like `// was: pos += 1` from
                // triggering false positives.
                let code = match line.find("//") {
                    Some(idx) => &line[..idx],
                    None => line,
                };

                for pattern in &forbidden {
                    if code.contains(pattern) {
                        violations.push(format!(
                            "  {file}:{} — `{pattern}` in: {trimmed}",
                            lineno + 1,
                        ));
                    }
                }
            }
        }

        assert!(
            violations.is_empty(),
            "\nRaw char-level stepping detected in motion/selection code.\n\
             Use next_grapheme_boundary(buf, pos) or prev_grapheme_boundary(buf, pos) instead.\n\
             Violations:\n{}\n",
            violations.join("\n")
        );
    }
}
