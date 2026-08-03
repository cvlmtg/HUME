use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

use crate::text::Text;

/// Returns the char offset of the start of the *next* grapheme cluster after
/// `char_offset`, or `buf.len_chars()` when already at (or past) the end.
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
pub fn next_grapheme_boundary(buf: &Text, char_offset: usize) -> usize {
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
pub fn prev_grapheme_boundary(buf: &Text, char_offset: usize) -> usize {
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
pub fn grapheme_count(buf: &Text, from_char: usize, to_char: usize) -> usize {
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
pub fn grapheme_col_in_line(buf: &Text, line_idx: usize, char_pos: usize) -> usize {
    grapheme_count(buf, buf.line_to_char(line_idx), char_pos)
}

/// Columns a `\t` at display column `col` occupies — the distance to the next
/// tab stop of width `tw`. Always in `[1, tw]`: a tab already sitting on a stop
/// advances a full `tw` rather than zero.
fn tab_advance(col: usize, tw: usize) -> usize {
    tw - col % tw
}

/// 0-based display column of `char_pos` within line `line_idx`, with `\t`
/// expanded to tab stops of width `tab_width`.
///
/// Non-tab graphemes count as one column each (matching the logical-column
/// convention of [`grapheme_col_in_line`]); wide CJK characters are therefore
/// undercounted by one. This is acceptable for tab-stop alignment — the only
/// place display width matters for editing — since CJK-plus-tab mixtures are
/// rare and any error there is bounded. A `'\t'` advances the column to the
/// next multiple of `tab_width`.
///
/// The tab-stop arithmetic here is intentionally duplicated with the
/// renderer's `hume_engine::format::grapheme_display`. `hume-engine` does not
/// depend on `hume-editing` (the engine doesn't know about the text model), so
/// the primitive lives in each crate. The two also diverge on purpose: the
/// renderer uses `unicode-width` so wide CJK chars take 2 columns for
/// display, while this helper counts every non-tab grapheme as 1 — see the
/// comment in `grapheme_display` for the rationale.
///
/// Used by `insert_tab` (Soft style: insert spaces to the next tab stop) and
/// by dedent-on-Backspace (compute the previous tab stop).
pub fn display_col_in_line(buf: &Text, line_idx: usize, char_pos: usize, tab_width: u8) -> usize {
    let line_start = buf.line_to_char(line_idx);
    let tw = tab_width.max(1) as usize;
    let mut col = 0usize;
    let mut pos = line_start;
    while pos < char_pos {
        let next = next_grapheme_boundary(buf, pos);
        if next > char_pos || next == pos {
            break;
        }
        let ch = buf.char_at(pos);
        col += if ch == Some('\t') {
            tab_advance(col, tw)
        } else {
            1
        };
        pos = next;
    }
    col
}

/// Return the char offset on `line_idx` at which the display column first
/// reaches `target_col`, walking forward from the line start with `\t`
/// expanded to tab stops of width `tab_width`.
///
/// For `target_col == 0` this is the line start. When `target_col` is a tab
/// stop and the line's leading content is whitespace (the only context in
/// which this helper is called — dedent-on-Backspace), the position is exact:
/// tabs jump to multiples of `tab_width` and spaces step by one, so every
/// tab stop along the way is hit. If a grapheme would overshoot `target_col`
/// (e.g. a tab when not aligned), the walk stops at the current position —
/// the closest position not exceeding `target_col`. The walk never leaves the
/// line: a `target_col` beyond the line's width stops on the line's `\n`.
pub fn char_pos_at_display_col(
    buf: &Text,
    line_idx: usize,
    target_col: usize,
    tab_width: u8,
) -> usize {
    let line_start = buf.line_to_char(line_idx);
    if target_col == 0 {
        return line_start;
    }
    let tw = tab_width.max(1) as usize;
    let mut col = 0usize;
    let mut pos = line_start;
    loop {
        let next = next_grapheme_boundary(buf, pos);
        if next == pos {
            break; // end of buffer
        }
        let ch = buf.char_at(pos);
        if ch == Some('\n') {
            break; // end of line — never walk onto the next line
        }
        let w = if ch == Some('\t') {
            tab_advance(col, tw)
        } else {
            1
        };
        if col + w > target_col {
            break; // this grapheme would overshoot — stop here
        }
        col += w;
        pos = next;
        if col == target_col {
            break;
        }
    }
    pos
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
