use std::borrow::Cow;
use std::ops::Range;

use ropey::RopeSlice;
use unicode_segmentation::UnicodeSegmentation;

use crate::cluster::{ClusterBound, ClusterRange, ClusterStart};
use crate::column::{BufferLineCol, GraphemeCol};
use crate::line::ContentLine;
use crate::offset::CharOffset;

mod chunk_cursor;

use chunk_cursor::ChunkCursor;

/// One grapheme cluster yielded by [`graphemes_at`]: chars `[start, end)`,
/// and the first of them, which is what classifies the cluster (a base
/// letter, not the combining mark after it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cluster {
    pub(crate) start: CharOffset,
    pub(crate) end: CharOffset,
    pub(crate) first: char,
}

impl Cluster {
    pub fn start(&self) -> ClusterStart {
        ClusterStart::mint(self.start)
    }

    pub fn end(&self) -> ClusterBound {
        ClusterBound::mint(self.end)
    }

    /// This cluster alone, as a range.
    pub fn range(&self) -> ClusterRange {
        ClusterRange::mint(self.start(), self.start(), self.end())
    }

    /// The char that classifies the cluster: a base letter, not the
    /// combining mark after it.
    pub fn first(&self) -> char {
        self.first
    }
}

/// The grapheme clusters of `slice` from `pos` to its end, in order. The one
/// forward grapheme stepper: [`cluster_end`] is its first step, and a loop
/// that walks forward cluster by cluster iterates this instead of calling
/// that once per step, which would re-seek the rope every time.
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
/// space. `GraphemeCursor` takes the text a chunk at a time, and the walk
/// keeps its place in the chunk sequence, so the rope is descended once,
/// when the walk starts.
pub fn graphemes_at(slice: RopeSlice<'_>, from: ClusterBound) -> Graphemes<'_> {
    walk_from(slice, from.offset())
}

/// [`graphemes_at`] from any char offset up to the text end: a start inside
/// a cluster yields that cluster's tail as the first item. For the
/// offset-based functions below, whose callers may hold such a position.
fn walk_from(slice: RopeSlice<'_>, pos: CharOffset) -> Graphemes<'_> {
    let (cur, byte) = ChunkCursor::at_char(slice, pos.index());
    Graphemes {
        cur,
        byte,
        char: pos,
    }
}

/// See [`graphemes_at`].
pub struct Graphemes<'a> {
    cur: ChunkCursor<'a>,
    byte: usize,
    char: CharOffset,
}

impl<'a> Graphemes<'a> {
    /// Steps over one cluster: the cluster, and its text when it lies inside
    /// one chunk.
    fn step(&mut self) -> Option<(Cluster, Option<&'a str>)> {
        if self.byte == self.cur.slice().len_bytes() {
            return None;
        }
        let first = self.cur.char_at(self.byte);
        let (chunk, chunk_start) = (self.cur.chunk(), self.cur.chunk_byte_start());
        let end_byte = self.cur.next_boundary(self.byte);
        let text = chunk.get(self.byte - chunk_start..end_byte - chunk_start);
        let end = match text {
            // Trusted mint: this module is the grapheme-boundary authority.
            Some(text) => CharOffset::new(self.char.index() + text.chars().count()),
            None => CharOffset::new(self.cur.byte_to_char(end_byte)),
        };
        let cluster = Cluster {
            start: self.char,
            end,
            first,
        };
        self.byte = end_byte;
        self.char = end;
        Some((cluster, text))
    }

    /// The next cluster with its text: the shape `width::grapheme_width`
    /// needs, since `unicode-width`'s context-sensitive rules (a combining
    /// mark folding into its base's width) need the whole cluster, not just
    /// its first char. Borrowed from the rope unless the cluster straddles a
    /// chunk boundary, which is rare: chunks run hundreds of bytes.
    fn next_with_text(&mut self) -> Option<(Cluster, Cow<'a, str>)> {
        let start = self.byte;
        let (cluster, text) = self.step()?;
        let text = match text {
            Some(text) => Cow::Borrowed(text),
            None => Cow::Owned(self.cur.slice().byte_slice(start..self.byte).to_string()),
        };
        Some((cluster, text))
    }
}

impl Iterator for Graphemes<'_> {
    type Item = Cluster;

    fn next(&mut self) -> Option<Cluster> {
        self.step().map(|(cluster, _)| cluster)
    }
}

/// Returns the char offset of the start of the *next* grapheme cluster after
/// `char_offset`, or `slice.len_chars()` when already at the end: the first
/// step of [`graphemes_at`], for a caller taking a single step. Panics past
/// the end.
pub(crate) fn next_grapheme_boundary(slice: RopeSlice<'_>, char_offset: CharOffset) -> CharOffset {
    walk_from(slice, char_offset)
        .next()
        .map_or(CharOffset::new(slice.len_chars()), |cluster| cluster.end)
}

/// Returns the char offset of the start of the grapheme cluster *before*
/// `char_offset`.
///
/// Returns `0` when `char_offset` is already at the start of the slice.
/// Panics past the end.
pub(crate) fn prev_grapheme_boundary(slice: RopeSlice<'_>, char_offset: CharOffset) -> CharOffset {
    let (mut cur, byte) = ChunkCursor::at_char(slice, char_offset.index());
    if byte == 0 {
        return CharOffset::new(0);
    }
    let prev = cur.prev_boundary(byte);
    CharOffset::new(cur.byte_to_char(prev))
}

/// The start of the cluster holding the char at `offset`, which must be
/// below the text end, and the start of the cluster before that one, or
/// `None` when the first is the text's first cluster.
pub(crate) fn floor_with_prev_start(
    slice: RopeSlice<'_>,
    offset: CharOffset,
) -> (CharOffset, Option<ClusterStart>) {
    let (mut cur, byte) = ChunkCursor::at_char(slice, offset.index());
    let end = cur.next_boundary(byte);
    let floor = cur.prev_boundary(end);
    let floor_char = CharOffset::new(cur.byte_to_char(floor));
    let prev = (floor > 0).then(|| {
        let prev = cur.prev_boundary(floor);
        ClusterStart::mint(CharOffset::new(cur.byte_to_char(prev)))
    });
    (floor_char, prev)
}

/// The boundary after the cluster starting at `start`.
pub fn cluster_end(slice: RopeSlice<'_>, start: ClusterStart) -> ClusterBound {
    ClusterBound::mint(next_grapheme_boundary(slice, start.offset()))
}

/// The cluster after the one starting at `start`, or `None` when `start` is
/// the last cluster.
pub fn next_cluster(slice: RopeSlice<'_>, start: ClusterStart) -> Option<ClusterStart> {
    let end = cluster_end(slice, start).offset();
    (end.index() < slice.len_chars()).then(|| ClusterStart::mint(end))
}

/// The cluster ending at `bound`, or `None` at the text start.
pub fn prev_cluster(slice: RopeSlice<'_>, bound: ClusterBound) -> Option<ClusterStart> {
    (bound.offset().index() > 0)
        .then(|| ClusterStart::mint(prev_grapheme_boundary(slice, bound.offset())))
}

/// The slice's first cluster, or `None` when it is empty.
pub fn first_cluster(slice: RopeSlice<'_>) -> Option<ClusterStart> {
    (slice.len_chars() > 0).then(|| ClusterStart::mint(CharOffset::new(0)))
}

/// The slice's last cluster, or `None` when it is empty.
pub fn last_cluster(slice: RopeSlice<'_>) -> Option<ClusterStart> {
    prev_cluster(slice, text_end(slice))
}

/// The boundary at the slice's end.
pub fn text_end(slice: RopeSlice<'_>) -> ClusterBound {
    ClusterBound::mint(CharOffset::new(slice.len_chars()))
}

/// The cluster holding the char at `offset`; the last cluster when `offset`
/// is at or past the end. `None` only for an empty slice. The entry point for
/// a position from a foreign coordinate system.
pub fn snap_to_cluster(slice: RopeSlice<'_>, offset: CharOffset) -> Option<Cluster> {
    let last = slice.len_chars().checked_sub(1)?;
    let at = offset.min(CharOffset::new(last));
    let (mut cur, byte) = ChunkCursor::at_char(slice, at.index());
    Some(snap_on_cursor(&mut cur, at, byte))
}

/// The cluster holding the char at `at`, which starts at `byte`, read from
/// `cur`.
fn snap_on_cursor(cur: &mut ChunkCursor<'_>, at: CharOffset, byte: usize) -> Cluster {
    // Every char that joins a cluster to its neighbour is non-ASCII, so a
    // boundary between two ASCII chars is missing only inside "\r\n", which
    // ropey never splits across chunks.
    let breaks_between = |before: char, after: char| {
        before.is_ascii() && after.is_ascii() && !(before == '\r' && after == '\n')
    };
    let here = cur.char_at(byte);
    let after = byte + here.len_utf8();
    let before = cur.char_before(byte);
    let (end_byte, end) = match cur.char_after(after) {
        Some(next) if !breaks_between(here, next) => {
            let end_byte = cur.next_boundary(byte);
            (end_byte, CharOffset::new(cur.byte_to_char(end_byte)))
        }
        _ => (after, CharOffset::new(at.index() + 1)),
    };
    let (start, first) = if before.is_none_or(|before| breaks_between(before, here)) {
        (at, here)
    } else {
        let start_byte = cur.prev_boundary(end_byte);
        let start = CharOffset::new(cur.byte_to_char(start_byte));
        (start, cur.char_at(start_byte))
    };
    Cluster { start, end, first }
}

/// The first and last clusters covering the chars `bytes` fall in, or
/// `None` when `bytes` holds no whole char. A bound inside a codepoint moves
/// to that codepoint's start. Panics if either bound is past the slice.
pub(crate) fn snap_covering_bytes(
    slice: RopeSlice<'_>,
    bytes: Range<usize>,
) -> Option<(Cluster, Cluster)> {
    let mut first = ChunkCursor::at_byte(slice, bytes.start);
    let (first_char, first_byte) = first.char_holding(bytes.start);
    let mut last = ChunkCursor::at_byte(slice, bytes.end);
    let (end_char, end_byte) = last.char_holding(bytes.end);
    if first_char >= end_char {
        return None;
    }
    if end_byte == last.chunk_byte_start() {
        last.retreat();
    }
    let (last_char, last_byte) = last.char_holding(end_byte - 1);
    Some((
        snap_on_cursor(&mut first, CharOffset::new(first_char), first_byte),
        snap_on_cursor(&mut last, CharOffset::new(last_char), last_byte),
    ))
}

/// The boundary at or after `offset`: the text end for an offset at or past
/// it.
pub(crate) fn ceil_boundary(slice: RopeSlice<'_>, offset: CharOffset) -> ClusterBound {
    match snap_to_cluster(slice, offset) {
        Some(cluster) if offset.index() < slice.len_chars() => {
            if cluster.start == offset {
                cluster.start().into()
            } else {
                cluster.end()
            }
        }
        _ => text_end(slice),
    }
}

/// The clusters of `slice` before `bound`, nearest first: the backward
/// counterpart of [`graphemes_at`], descending the rope once when the walk
/// starts.
pub fn clusters_before(slice: RopeSlice<'_>, bound: ClusterBound) -> ClustersBefore<'_> {
    let (cur, byte) = ChunkCursor::at_char(slice, bound.offset().index());
    ClustersBefore {
        cur,
        byte,
        char: bound.offset(),
    }
}

/// See [`clusters_before`].
pub struct ClustersBefore<'a> {
    cur: ChunkCursor<'a>,
    byte: usize,
    char: CharOffset,
}

impl Iterator for ClustersBefore<'_> {
    type Item = Cluster;

    fn next(&mut self) -> Option<Cluster> {
        if self.byte == 0 {
            return None;
        }
        let start_byte = self.cur.prev_boundary(self.byte);
        let chunk_start = self.cur.chunk_byte_start();
        let start = match self
            .cur
            .chunk()
            .get(start_byte - chunk_start..self.byte - chunk_start)
        {
            Some(text) => self.char.retreat(text.chars().count()),
            None => CharOffset::new(self.cur.byte_to_char(start_byte)),
        };
        let cluster = Cluster {
            start,
            end: self.char,
            first: self.cur.char_at(start_byte),
        };
        self.byte = start_byte;
        self.char = start;
        Some(cluster)
    }
}

/// Byte offset of the start of the grapheme cluster ending at `byte_pos`:
/// the `&str` sibling of [`prev_cluster`], for the short, already
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
/// `&str` sibling of [`cluster_end`]. `s.len()` when `byte_pos` is
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
    walk_from(slice, from_char)
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
    let mut walk = walk_from(slice, line_start);
    while let Some((cluster, text)) = walk.next_with_text() {
        if cluster.end > char_pos {
            break;
        }
        let w = crate::width::grapheme_width(&text, display_col.get() as usize, tab_width);
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
/// The text must be LF-normalized and end with a `\n`, as every buffer
/// does: a `\r\n` is one grapheme cluster, so the walk could not tell where
/// the line ends, and a last line with no `\n` would let it run to the text
/// end, which starts no cluster.
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
) -> ClusterStart {
    let line_start = crate::lines::slice_line_start_char(slice, line_idx.into());
    if target_display_col == BufferLineCol::new(0) {
        return ClusterStart::mint(line_start);
    }
    let mut display_col = BufferLineCol::new(0);
    let mut pos = line_start;
    let mut walk = walk_from(slice, line_start);
    while let Some((cluster, text)) = walk.next_with_text() {
        debug_assert!(
            cluster.first != '\r',
            "char_pos_at_display_col: text must be LF-normalized, found a '\\r'"
        );
        if cluster.first == '\n' {
            break; // end of line: never walk onto the next line
        }
        let w = crate::width::grapheme_width(&text, display_col.get() as usize, tab_width);
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
    // A cluster start: the line's start, or the end of a cluster before the
    // line's `\n`.
    ClusterStart::mint(pos)
}

#[cfg(test)]
mod tests;
