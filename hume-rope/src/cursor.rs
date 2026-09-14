use ropey::Rope;

use crate::offset::CharOffset;

/// A char-level cursor for scanning a contiguous range of a [`Rope`] without
/// re-paying ropey's O(log n) tree descent on every step. Each `next()` /
/// `prev()` call is amortized O(1) after the initial O(log n) seek in
/// [`chars_at`] — an O(span × log n) loop of indexed char lookups becomes
/// O(log n + span).
///
/// **Char-level, not grapheme-level** — intended for ASCII delimiter scanning
/// (brackets, quotes, argument commas). Motion and selection logic must keep
/// using [`crate::grapheme`]'s boundary helpers; a multi-codepoint cluster
/// (e.g. `e` + U+0301) is yielded here as two separate chars.
pub struct CharCursor<'a> {
    iter: ropey::iter::Chars<'a>,
    /// Char offset of the position the cursor currently sits at — the
    /// position `next()` would yield and `prev()` would land on.
    pos: CharOffset,
}

/// A cursor over `rope`'s chars starting at `pos`. See [`CharCursor`].
///
/// Takes [`CharOffset`] so a caller holding a buffer position hands it over
/// directly instead of unwrapping to a bare index at the call site.
///
/// # Panics
/// Panics if `pos > rope.len_chars()`.
pub fn chars_at(rope: &Rope, pos: CharOffset) -> CharCursor<'_> {
    CharCursor {
        iter: rope.chars_at(pos.index()),
        pos,
    }
}

impl Iterator for CharCursor<'_> {
    type Item = (CharOffset, char);

    /// Yield the char at the cursor position, then advance forward.
    fn next(&mut self) -> Option<(CharOffset, char)> {
        let ch = self.iter.next()?;
        let pos = self.pos;
        self.pos = self.pos.shift(1);
        Some((pos, ch))
    }
}

impl CharCursor<'_> {
    /// Step back and yield the char just before the cursor position.
    ///
    /// Not a [`DoubleEndedIterator`] impl —
    /// that trait means "consume from the far end of the same forward
    /// sequence," not "walk backward from here," which is what callers
    /// (bracket-pair scans) actually need.
    pub fn prev(&mut self) -> Option<(CharOffset, char)> {
        let ch = self.iter.prev()?;
        self.pos = self.pos.retreat(1);
        Some((self.pos, ch))
    }
}

#[cfg(test)]
mod tests;
