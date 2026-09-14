//! Display-line address types. Zero references to `DisplayLineMap`
//! internals.

use hume_rope::line::ContentLine;

/// The address of one display line: `slot` indexes into `line`'s visual
/// block (`before`-virtuals, content/wrap display lines, `after`-virtuals —
/// in that order).
///
/// `Ord` is lexicographic on `(line, slot)`, which is document order, so
/// comparing two addresses answers "which comes first on screen".
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct DisplayLinePos {
    pub line: ContentLine,
    pub slot: usize,
}

impl DisplayLinePos {
    pub fn new(line: ContentLine, slot: usize) -> Self {
        Self { line, slot }
    }
}

/// Which slot of a line's visual block a display line falls in — virtual
/// display lines anchored before it, its own wrap/content display lines, or
/// virtual display lines anchored after. The payload is the display line's
/// index within its own group, so `Content(2)` is a line's third content
/// display line and `Before(0)` is the first virtual display line above it.
///
/// Named `BlockSlot` rather than `DisplayLineKind` to stay distinct from
/// [`crate::types::DisplayLineKind`] (`LineStart`/`Wrap`/`Virtual`/`Filler`)
/// — a different question about the same display line: that one classifies
/// how a display line was produced, this one where it sits within its
/// line's block.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BlockSlot {
    Before(usize),
    Content(usize),
    After(usize),
}

/// Display-line breakdown of one buffer line's visual block: virtual
/// display lines anchored `Before` it, its own wrap/content display lines,
/// and virtual display lines anchored `After` it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlockBreakdown {
    pub before: usize,
    pub content: usize,
    pub after: usize,
}

impl BlockBreakdown {
    /// Total display lines this line's whole visual block occupies.
    pub fn total(&self) -> usize {
        self.before + self.content + self.after
    }
}

/// Which grapheme a display column resolves to in
/// [`DisplayLineMap::char_at`](super::DisplayLineMap::char_at).
///
/// The two variants are different questions, not different implementations —
/// a click and a sticky-column vertical move optimise different things, and
/// collapsing them regresses one or the other.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DisplayColTarget {
    /// The cell that *contains* this column — what a mouse click asks. A
    /// column inside a wide cell (a tab's expanse, a double-width glyph)
    /// resolves to that cell, and the end-of-line sentinel is a valid landing
    /// spot, so clicking past a line's text puts the cursor on its `\n`
    /// (a real cursor position in HUME's inclusive selection model).
    Cell,
    /// The real grapheme whose start column is *nearest* this one — what a
    /// sticky-column `j`/`k` asks, since it minimises display column drift.
    /// The end-of-line sentinel is skipped unless it is the display line's
    /// only grapheme (an empty line), so vertical movement stays on content.
    NearestContent,
}
