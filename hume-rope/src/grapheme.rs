use std::borrow::Cow;

use ropey::RopeSlice;
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete, UnicodeSegmentation};

use crate::column::{BufferLineCol, GraphemeCol};
use crate::line::ContentLine;
use crate::offset::CharOffset;

/// One grapheme cluster yielded by [`graphemes_at`]: chars `[start, end)`,
/// and the first of them, which is what classifies the cluster (a base
/// letter, not the combining mark after it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cluster {
    pub start: CharOffset,
    pub end: CharOffset,
    pub first: char,
}

impl Cluster {
    /// The cluster's last char, including a trailing combining mark: the
    /// inclusive counterpart of `end`.
    pub fn last_char(&self) -> CharOffset {
        self.end.retreat(1)
    }
}

/// The grapheme clusters of `slice` from `pos` to its end, in order. The one
/// forward grapheme stepper: [`next_grapheme_boundary`] is its first step, and
/// a loop that walks forward cluster by cluster iterates this instead of
/// calling that once per step, which would re-seek the rope every time.
///
/// If `pos` is inside a cluster, the first item runs from `pos` to that
/// cluster's end.
///
/// # Why byte offsets internally?
///
/// `GraphemeCursor` (from `unicode-segmentation`) operates in *byte* space
/// because Unicode break algorithms work on UTF-8 encoded bytes. Byte offsets
/// never leave this module: each cluster's char count is taken from its own
/// bytes, so the walk never converts back through the rope per step.
///
/// # Why chunks instead of a full `&str`?
///
/// Ropey stores the rope as a B-tree of `&str` chunks. Materializing the
/// whole buffer into a single `String` just to walk it would be O(n) in
/// space. `GraphemeCursor` takes the text a chunk at a time
/// (`next_boundary` / `provide_context`), and the walk keeps its current chunk
/// between clusters, so the rope is descended once per chunk, not per step.
pub fn graphemes_at(slice: RopeSlice<'_>, pos: CharOffset) -> Graphemes<'_> {
    let char = pos.min(CharOffset::new(slice.len_chars()));
    let byte = slice.char_to_byte(char.index());
    Graphemes {
        slice,
        chunk: "",
        chunk_byte_start: byte,
        byte,
        char,
    }
}

/// See [`graphemes_at`].
pub struct Graphemes<'a> {
    slice: RopeSlice<'a>,
    chunk: &'a str,
    chunk_byte_start: usize,
    byte: usize,
    char: CharOffset,
}

impl Graphemes<'_> {
    fn load_chunk_at(&mut self, byte: usize) {
        let (chunk, start, _, _) = self.slice.chunk_at_byte(byte);
        self.chunk = chunk;
        self.chunk_byte_start = start;
    }
}

impl Iterator for Graphemes<'_> {
    type Item = Cluster;

    fn next(&mut self) -> Option<Cluster> {
        let len_bytes = self.slice.len_bytes();
        if self.byte >= len_bytes {
            return None;
        }
        if self.byte >= self.chunk_byte_start + self.chunk.len() {
            self.load_chunk_at(self.byte);
        }
        // A fresh cursor per cluster: one carried over from the previous
        // cluster splits a regional-indicator pair that straddles a chunk
        // boundary.
        let mut cursor = GraphemeCursor::new(self.byte, len_bytes, true);
        let start_chunk_byte = self.chunk_byte_start;
        let first = self.chunk[self.byte - start_chunk_byte..]
            .chars()
            .next()
            .expect("byte < len_bytes lies inside the current chunk");
        let end_byte = loop {
            match cursor.next_boundary(self.chunk, self.chunk_byte_start) {
                Ok(Some(b)) => break b,
                Ok(None) => break len_bytes,
                Err(GraphemeIncomplete::NextChunk) => {
                    let next_byte = self.chunk_byte_start + self.chunk.len();
                    if next_byte >= len_bytes {
                        break len_bytes;
                    }
                    self.load_chunk_at(next_byte);
                }
                // The cursor needs context from *before* the current position
                // to resolve a boundary that depends on a preceding codepoint
                // (e.g. Regional Indicator pairs, ZWJ sequences).
                Err(GraphemeIncomplete::PreContext(n)) => {
                    let (ctx_chunk, ctx_start, _, _) = self.slice.chunk_at_byte(n - 1);
                    cursor.provide_context(ctx_chunk, ctx_start);
                }
                // `next_boundary` only returns the three variants above.
                Err(_) => unreachable!("unexpected GraphemeIncomplete variant"),
            }
        };
        // A cluster inside one chunk counts its own chars; one straddling a
        // chunk boundary (rare) asks the rope.
        let end_char = if self.chunk_byte_start == start_chunk_byte {
            let chars = self.chunk[self.byte - start_chunk_byte..end_byte - start_chunk_byte]
                .chars()
                .count();
            // Trusted mint: this module is the grapheme-boundary authority.
            CharOffset::new(self.char.index() + chars)
        } else {
            CharOffset::new(self.slice.byte_to_char(end_byte))
        };
        let cluster = Cluster {
            start: self.char,
            end: end_char,
            first,
        };
        self.byte = end_byte;
        self.char = end_char;
        Some(cluster)
    }
}

/// Returns the char offset of the start of the *next* grapheme cluster after
/// `char_offset`, or `slice.len_chars()` when already at (or past) the end:
/// the first step of [`graphemes_at`], for a caller taking a single step.
pub fn next_grapheme_boundary(slice: RopeSlice<'_>, char_offset: CharOffset) -> CharOffset {
    graphemes_at(slice, char_offset)
        .next()
        .map_or(CharOffset::new(slice.len_chars()), |cluster| cluster.end)
}

/// Returns the char offset of the start of the grapheme cluster *before*
/// `char_offset`.
///
/// Returns `0` when `char_offset` is already at the start of the slice.
pub fn prev_grapheme_boundary(slice: RopeSlice<'_>, char_offset: CharOffset) -> CharOffset {
    let char_offset = char_offset.index();
    if char_offset == 0 {
        return CharOffset::new(0);
    }

    let len_bytes = slice.len_bytes();
    let byte_offset = slice.char_to_byte(char_offset);

    // Start one byte before `byte_offset` to land inside the preceding
    // cluster. We want the chunk that *contains* the last byte of that
    // cluster, not the chunk that starts exactly at `byte_offset`.
    let (mut chunk, mut chunk_byte_start, _, _) = slice.chunk_at_byte(byte_offset - 1);

    let mut gc = GraphemeCursor::new(byte_offset, len_bytes, true);

    loop {
        match gc.prev_boundary(chunk, chunk_byte_start) {
            Ok(None) => return CharOffset::new(0),
            Ok(Some(b)) => return CharOffset::new(slice.byte_to_char(b)),

            // The cursor needs the previous chunk.
            Err(GraphemeIncomplete::PrevChunk) => {
                if chunk_byte_start == 0 {
                    return CharOffset::new(0);
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

/// Floor `char_offset` to the start of its own grapheme cluster: a no-op
/// when it's already a cluster start, otherwise the start of the cluster it
/// sits inside.
///
/// `next` then `prev` rather than `prev` alone: [`prev_grapheme_boundary`]
/// answers "where does the *preceding* cluster start," which is one cluster
/// too far back when `char_offset` is already a boundary. Advancing to the
/// next boundary first (identity if already on one), then retreating, lands
/// on the boundary that actually opens `char_offset`'s own cluster.
pub fn snap_to_cluster_start(slice: RopeSlice<'_>, char_offset: CharOffset) -> CharOffset {
    prev_grapheme_boundary(slice, next_grapheme_boundary(slice, char_offset))
}

/// Whether `char_offset` is a cluster boundary: the start of a cluster, or the
/// end of the slice.
///
/// Two adjacent ASCII chars are always a boundary except for `\r\n`, which
/// answers without a grapheme walk. Any non-ASCII neighbour takes the walk:
/// a Prepend char before an ASCII base glues to it.
pub fn is_cluster_boundary(slice: RopeSlice<'_>, char_offset: CharOffset) -> bool {
    if char_offset.index() == 0 || char_offset.index() >= slice.len_chars() {
        return true;
    }
    let prev = slice.char(char_offset.retreat(1).index());
    let next = slice.char(char_offset.index());
    if prev.is_ascii() && next.is_ascii() {
        return !(prev == '\r' && next == '\n');
    }
    snap_to_cluster_start(slice, char_offset) == char_offset
}

/// Last codepoint of the grapheme cluster starting at `cluster_start`: the
/// inverse of [`snap_to_cluster_start`], and the inclusive counterpart to
/// [`next_grapheme_boundary`]'s exclusive one.
///
/// For a single-codepoint cluster (the common case) this equals
/// `cluster_start`. For a multi-codepoint cluster such as `e` + U+0301
/// (combining acute) it includes the trailing combining mark, so a selection
/// or delete range built from this end never orphans it. A caller already
/// holding a [`Cluster`] uses [`Cluster::last_char`] instead. Past the end of
/// the slice this clamps to its last char.
pub fn cluster_last_char(slice: RopeSlice<'_>, cluster_start: CharOffset) -> CharOffset {
    graphemes_at(slice, cluster_start).next().map_or(
        CharOffset::new(slice.len_chars()).retreat_saturating(1),
        |cluster| cluster.last_char(),
    )
}

/// Byte offset of the start of the grapheme cluster ending at `byte_pos`:
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

/// Byte offset just past the grapheme cluster starting at `byte_pos`: the
/// `&str` sibling of [`next_grapheme_boundary`]. `s.len()` when `byte_pos` is
/// at or past the end. See [`prev_str_boundary`] for why these exist.
pub fn next_str_boundary(s: &str, byte_pos: usize) -> usize {
    s[byte_pos..]
        .grapheme_indices(true)
        .next()
        .map(|(_, g)| byte_pos + g.len())
        .unwrap_or(s.len())
}

/// Count grapheme clusters in the char range `[from_char, to_char)`.
///
/// `to_char` is an **exclusive** upper bound: the character at `to_char` is
/// not itself counted. For example, if the cursor sits at char offset `c`,
/// `grapheme_count(slice, line_start, c)` returns the number of grapheme
/// clusters that precede the cursor on that line, i.e. its 0-based grapheme
/// column. A cluster that `to_char` falls inside is not counted.
///
/// If `to_char < from_char` the range is treated as empty and 0 is returned.
pub(crate) fn grapheme_count(
    slice: RopeSlice<'_>,
    from_char: CharOffset,
    to_char: CharOffset,
) -> usize {
    graphemes_at(slice, from_char)
        .take_while(|cluster| cluster.end <= to_char)
        .count()
}

/// 0-based grapheme column of `char_pos` within line `line_idx`.
///
/// This is a logical position (grapheme index), not a display column: wide
/// characters count as one, not two. The value matches how many times the
/// user pressed → to reach the cursor from the start of the line.
pub fn grapheme_col_in_line(
    slice: RopeSlice<'_>,
    line_idx: ContentLine,
    char_pos: CharOffset,
) -> GraphemeCol {
    GraphemeCol::new(grapheme_count(
        slice,
        crate::lines::slice_line_start_char(slice, line_idx.into()),
        char_pos,
    ))
}

/// Grapheme cluster `[start, end)` of `slice`, as text: the shape
/// `width::grapheme_width` needs to measure it, since `unicode-width`'s
/// context-sensitive rules (e.g. combining marks folding into a base
/// character's width) need the whole cluster, not just its first char.
///
/// Borrowed with zero copy when the cluster lies entirely inside one rope
/// chunk, true for the overwhelming majority of clusters, since chunks run
/// hundreds of bytes and a cluster is rarely more than a handful of
/// codepoints. Copied only for the rare cluster that straddles a chunk
/// boundary.
fn cluster_str(slice: RopeSlice<'_>, start: CharOffset, end: CharOffset) -> Cow<'_, str> {
    let start_byte = slice.char_to_byte(start.index());
    let end_byte = slice.char_to_byte(end.index());
    let (chunk, chunk_byte_start, _, _) = slice.chunk_at_byte(start_byte);
    let local_start = start_byte - chunk_byte_start;
    let local_end = end_byte - chunk_byte_start;
    if local_end <= chunk.len() {
        Cow::Borrowed(&chunk[local_start..local_end])
    } else {
        Cow::Owned(slice.slice(start.index()..end.index()).to_string())
    }
}

/// 0-based display column of `char_pos` within line `line_idx`, with `\t`
/// expanded to tab stops of width `tab_width` and every other grapheme
/// weighted by [`crate::width::grapheme_width`]. That is the same convention the
/// renderer uses, so this and `hume_engine::format::grapheme_display` always
/// agree on where a given position lands on screen.
///
/// Vertical motion uses
/// `hume_engine::display_lines::DisplayLineMap` instead, which measures through the
/// decoration layer this rope-only function can't see.
pub fn display_col_in_line(
    slice: RopeSlice<'_>,
    line_idx: ContentLine,
    char_pos: CharOffset,
    tab_width: u8,
) -> BufferLineCol {
    let line_start = crate::lines::slice_line_start_char(slice, line_idx.into());
    let mut display_col = BufferLineCol::new(0);
    for cluster in graphemes_at(slice, line_start) {
        if cluster.end > char_pos {
            break;
        }
        let w = crate::width::grapheme_width(
            &cluster_str(slice, cluster.start, cluster.end),
            display_col.get() as usize,
            tab_width,
        );
        display_col = display_col.advance_saturating(w as u32);
    }
    display_col
}

/// Return the char offset on `line_idx` at which the display column first
/// reaches `target_display_col`, walking forward from the line start with
/// `\t` expanded to tab stops of width `tab_width`.
///
/// For `target_display_col == 0` this is the line start. When
/// `target_display_col` is a tab stop and the line's leading content is
/// whitespace (dedent-on-Backspace's case), the position is exact: tabs
/// jump to multiples of `tab_width` and spaces step by one, so every tab
/// stop along the way is hit. Otherwise the result is the closest position
/// not exceeding `target_display_col`: a grapheme that would overshoot (a
/// tab when not aligned, a double-width cluster straddling the target)
/// leaves the walk at the position before it. `hume-ops`'s
/// `align_selections` relies on exactly this non-tab-stop case to resolve
/// its removable-run cell target back to a char position, padding any
/// resulting overshoot with spaces.
///
/// The text must be LF-normalized: a `\r\n` is one grapheme cluster, so the
/// walk could not tell where the line ends.
///
/// The walk never leaves the line: a `target_display_col` beyond the line's
/// width stops on the line's `\n`. A caller that wants a cursor position
/// clamped back onto the last real character instead (vertical motion's
/// case) wants `hume_engine::display_lines::DisplayLineMap::char_at_buffer_line_col`, which
/// also sees the decoration layer this rope-only function can't.
pub fn char_pos_at_display_col(
    slice: RopeSlice<'_>,
    line_idx: ContentLine,
    target_display_col: BufferLineCol,
    tab_width: u8,
) -> CharOffset {
    let line_start = crate::lines::slice_line_start_char(slice, line_idx.into());
    if target_display_col == BufferLineCol::new(0) {
        return line_start;
    }
    let mut display_col = BufferLineCol::new(0);
    let mut pos = line_start;
    for cluster in graphemes_at(slice, line_start) {
        debug_assert!(
            cluster.first != '\r',
            "char_pos_at_display_col: text must be LF-normalized, found a '\\r'"
        );
        if cluster.first == '\n' {
            break; // end of line: never walk onto the next line
        }
        let w = crate::width::grapheme_width(
            &cluster_str(slice, cluster.start, cluster.end),
            display_col.get() as usize,
            tab_width,
        );
        let advanced = display_col.advance_saturating(w as u32);
        if advanced > target_display_col {
            break; // this grapheme would overshoot, stop here
        }
        display_col = advanced;
        pos = cluster.end;
        if display_col == target_display_col {
            break;
        }
    }
    pos
}

#[cfg(test)]
mod tests;
