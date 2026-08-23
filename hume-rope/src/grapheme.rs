use std::borrow::Cow;

use ropey::RopeSlice;
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete, UnicodeSegmentation};

/// Returns the char offset of the start of the *next* grapheme cluster after
/// `char_offset`, or `slice.len_chars()` when already at (or past) the end.
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
pub fn next_grapheme_boundary(slice: RopeSlice<'_>, char_offset: usize) -> usize {
    let len_chars = slice.len_chars();
    if char_offset >= len_chars {
        return len_chars;
    }

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
/// Returns `0` when `char_offset` is already at the start of the slice.
pub fn prev_grapheme_boundary(slice: RopeSlice<'_>, char_offset: usize) -> usize {
    if char_offset == 0 {
        return 0;
    }

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
/// `grapheme_count(slice, line_start, c)` returns the number of grapheme
/// clusters that precede the cursor on that line — its 0-based grapheme
/// column.
///
/// If `to_char < from_char` the range is treated as empty and 0 is returned.
///
/// # Why chunk-based?
///
/// The naïve alternative is `slice.slice(from..to).to_string().graphemes(true).count()`,
/// which allocates a heap String proportional to line length. Long lines
/// (minified JSON, generated files, log files with no newlines) can be
/// arbitrarily wide. This implementation uses the same chunk-at-a-time
/// `GraphemeCursor` strategy as `next_grapheme_boundary` — O(log n) per
/// cluster with no heap allocation.
/// Byte offset of the start of the grapheme cluster ending at `byte_pos` —
/// the `&str` sibling of [`prev_grapheme_boundary`], for the short, already
/// contiguous strings the UI edits in place (a minibuffer prompt, a picker
/// query) rather than a rope. `0` when `byte_pos` is already 0.
///
/// A plain `&str` needs none of the rope walker's chunk machinery: one
/// backwards `grapheme_indices` step over the prefix is exact and
/// allocation-free. Both exist so no caller is tempted to hand-roll the
/// `next_back()` walk and land mid-cluster on a combining sequence or a ZWJ
/// emoji.
pub fn prev_str_boundary(s: &str, byte_pos: usize) -> usize {
    s[..byte_pos]
        .grapheme_indices(true)
        .next_back()
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Byte offset just past the grapheme cluster starting at `byte_pos` — the
/// `&str` sibling of [`next_grapheme_boundary`]. `s.len()` when `byte_pos` is
/// at or past the end. See [`prev_str_boundary`] for why these exist.
pub fn next_str_boundary(s: &str, byte_pos: usize) -> usize {
    s[byte_pos..]
        .grapheme_indices(true)
        .next()
        .map(|(_, g)| byte_pos + g.len())
        .unwrap_or(s.len())
}

pub(crate) fn grapheme_count(slice: RopeSlice<'_>, from_char: usize, to_char: usize) -> usize {
    let to_char = to_char.max(from_char);
    if from_char == to_char {
        return 0;
    }

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
pub fn grapheme_col_in_line(slice: RopeSlice<'_>, line_idx: usize, char_pos: usize) -> usize {
    grapheme_count(slice, slice.line_to_char(line_idx), char_pos)
}

/// Grapheme cluster `[start, end)` of `slice`, as text — the shape
/// `width::grapheme_width` needs to measure it, since `unicode-width`'s
/// context-sensitive rules (e.g. combining marks folding into a base
/// character's width) need the whole cluster, not just its first char.
///
/// Borrowed with zero copy when the cluster lies entirely inside one rope
/// chunk — true for the overwhelming majority of clusters, since chunks run
/// hundreds of bytes and a cluster is rarely more than a handful of
/// codepoints. Copied only for the rare cluster that straddles a chunk
/// boundary.
fn cluster_str(slice: RopeSlice<'_>, start: usize, end: usize) -> Cow<'_, str> {
    let start_byte = slice.char_to_byte(start);
    let end_byte = slice.char_to_byte(end);
    let (chunk, chunk_byte_start, _, _) = slice.chunk_at_byte(start_byte);
    let local_start = start_byte - chunk_byte_start;
    let local_end = end_byte - chunk_byte_start;
    if local_end <= chunk.len() {
        Cow::Borrowed(&chunk[local_start..local_end])
    } else {
        Cow::Owned(slice.slice(start..end).chars().collect())
    }
}

/// 0-based display column of `char_pos` within line `line_idx`, with `\t`
/// expanded to tab stops of width `tab_width` and every other grapheme
/// weighted by [`crate::width::grapheme_width`] — the same convention the
/// renderer uses, so this and `hume_engine::format::grapheme_display` always
/// agree on where a given position lands on screen.
///
/// Used by `insert_tab` (Soft style: insert spaces to the next tab stop) and
/// dedent-on-Backspace (compute the previous tab stop). Vertical motion uses
/// `hume_engine::rows::RowMap` instead, which measures through the
/// decoration layer this rope-only function can't see.
pub fn display_col_in_line(
    slice: RopeSlice<'_>,
    line_idx: usize,
    char_pos: usize,
    tab_width: u8,
) -> usize {
    let line_start = slice.line_to_char(line_idx);
    let mut display_col = 0usize;
    let mut pos = line_start;
    while pos < char_pos {
        let next = next_grapheme_boundary(slice, pos);
        if next > char_pos || next == pos {
            break;
        }
        display_col +=
            crate::width::grapheme_width(&cluster_str(slice, pos, next), display_col, tab_width);
        pos = next;
    }
    display_col
}

/// Return the char offset on `line_idx` at which the display column first
/// reaches `target_display_col`, walking forward from the line start with
/// `\t` expanded to tab stops of width `tab_width`.
///
/// For `target_display_col == 0` this is the line start. When
/// `target_display_col` is a tab stop and the line's leading content is
/// whitespace — dedent-on-Backspace's case, this function's only caller —
/// the position is exact: tabs jump to multiples of `tab_width` and spaces
/// step by one, so every tab stop along the way is hit. Otherwise the result
/// is the closest position not exceeding `target_display_col`: a grapheme
/// that would overshoot (a tab when not aligned, a double-width cluster
/// straddling the target) leaves the walk at the position before it.
///
/// The walk never leaves the line: a `target_display_col` beyond the line's
/// width stops on the line's `\n`. A caller that wants a cursor position
/// clamped back onto the last real character instead — vertical motion's
/// case — wants `hume_engine::rows::RowMap::char_at_line_display_col`, which
/// also sees the decoration layer this rope-only function can't.
pub fn char_pos_at_display_col(
    slice: RopeSlice<'_>,
    line_idx: usize,
    target_display_col: usize,
    tab_width: u8,
) -> usize {
    let line_start = slice.line_to_char(line_idx);
    if target_display_col == 0 {
        return line_start;
    }
    let mut display_col = 0usize;
    let mut pos = line_start;
    loop {
        let next = next_grapheme_boundary(slice, pos);
        if next == pos {
            break; // end of buffer
        }
        if slice.get_char(pos) == Some('\n') {
            break; // end of line — never walk onto the next line
        }
        let w =
            crate::width::grapheme_width(&cluster_str(slice, pos, next), display_col, tab_width);
        if display_col + w > target_display_col {
            break; // this grapheme would overshoot — stop here
        }
        display_col += w;
        pos = next;
        if display_col == target_display_col {
            break;
        }
    }
    pos
}

#[cfg(test)]
mod tests;
