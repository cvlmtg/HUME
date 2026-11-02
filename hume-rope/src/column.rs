//! Domain-typed line-relative columns.
//!
//! One type per column sense, so a function typed for one can't be handed
//! another: [`DisplayLineCol`]/[`BufferLineCol`] (terminal cells), [`CharCol`],
//! [`GraphemeCol`], [`ByteCol`]. A bare `*LineCol` is always display cells; a
//! unit prefix names any other unit.
//!
//! # Display columns have an origin
//!
//! Under soft wrap a continuation display line numbers its columns from its
//! own left edge, so one character has a [`DisplayLineCol`] and a different
//! [`BufferLineCol`]. They coincide only when the buffer line fits on one
//! display line; [`BufferLineCol::as_display_line_unwrapped`] is the named
//! crossing for that case.
//!
//! # Arithmetic
//!
//! A display column is an accumulator (formatting advances it one grapheme
//! width at a time), so both display types expose named saturating
//! arithmetic; the type win is keeping the two origins apart. [`CharCol`] and
//! [`GraphemeCol`] have none. [`ByteCol`] has only `advance_saturating`.
//! Display columns have no `checked`/`clamped` mint: they have no upper bound,
//! and resolvers back to a char offset clamp internally.

/// A display column measured from its display line's left edge: what
/// `hume_engine::display_lines::DisplayLineMap::locate` returns.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct DisplayLineCol(u32);

/// A display column measured from its buffer line's start: what
/// `hume_engine::display_lines::DisplayLineMap::buffer_line_col` returns, and
/// what a counted vertical move (`9j`/`9k`) latches.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BufferLineCol(u32);

/// The arithmetic shared by [`DisplayLineCol`] and [`BufferLineCol`]. A
/// macro, since no caller is generic over the two.
macro_rules! display_col_methods {
    ($ty:ident) => {
        impl $ty {
            /// Mint a column already known to be valid, such as one read
            /// back from a formatted line.
            pub fn new(col: u32) -> Self {
                Self(col)
            }

            /// The bare 0-based column, for `hume_rope::width` or a
            /// terminal-cell conversion.
            pub fn get(self) -> u32 {
                self.0
            }

            /// `self` advanced by `cells`, saturating: a grapheme's width
            /// folded into a running format cursor.
            pub fn advance_saturating(self, cells: u32) -> Self {
                Self(self.0.saturating_add(cells))
            }

            /// Cells between `earlier` and `self`. Debug-panics if
            /// `earlier > self`.
            pub fn cells_since(self, earlier: Self) -> u32 {
                debug_assert!(
                    earlier <= self,
                    "cells_since: {earlier:?} is after {self:?}, cells_since measures forward only"
                );
                self.0.saturating_sub(earlier.0)
            }
        }
    };
}

display_col_methods!(DisplayLineCol);
display_col_methods!(BufferLineCol);

impl DisplayLineCol {
    /// Unsigned distance between `self` and `other`: the "which grapheme is
    /// visually closest" metric of a nearest-column resolve.
    pub fn abs_diff(self, other: Self) -> u32 {
        self.0.abs_diff(other.0)
    }

    /// [`Self::cells_since`], but 0 when `earlier` is later. Its one caller,
    /// `hume-editor`'s `cursor::place`, anchors a completion popup at a fixed
    /// token start that horizontal scroll can leave left of the viewport.
    pub fn cells_since_saturating(self, earlier: Self) -> u32 {
        self.0.saturating_sub(earlier.0)
    }
}

impl BufferLineCol {
    /// This column read as a display-line-relative one. Sound only where the
    /// buffer line occupies a single display line. Its two callers:
    /// `DisplayLineMap::char_at_buffer_line_col`, which never consults the
    /// result while wrapping (the format bound is promoted to `Full`), and
    /// `hume-editor`'s vertical-motion sticky column, which calls it only on
    /// its no-wrap path.
    pub fn as_display_line_unwrapped(self) -> DisplayLineCol {
        DisplayLineCol(self.0)
    }

    /// `self` shifted by a signed cell delta, saturating at both ends. A
    /// multi-cursor edit's running delta can outpace the column it applies
    /// to (a wide deletion collapsing a later indent target), and clamping
    /// to 0 is the intended result there.
    ///
    /// Widens through `i64`: narrowing `delta` to `i32` for
    /// `saturating_add_signed` would wrap a delta outside `i32`'s range
    /// instead of saturating.
    pub fn shift_saturating(self, delta: isize) -> Self {
        Self(
            (self.0 as i64)
                .saturating_add(delta as i64)
                .clamp(0, u32::MAX as i64) as u32,
        )
    }

    /// `self` moved back by `cells`, saturating at 0.
    pub fn retreat_saturating(self, cells: u32) -> Self {
        self.shift_saturating(-(cells as isize))
    }
}

/// 0-based char index within a line, never a grapheme count or a display
/// column. [`crate::lines::char_col_in_line`]'s result and
/// [`crate::lines::place_char_column`]'s input.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct CharCol(usize);

impl CharCol {
    /// Mint a char column already known to be valid, such as
    /// `char_pos.chars_since(line_start)`.
    pub fn new(col: usize) -> Self {
        Self(col)
    }

    /// The bare 0-based index.
    pub fn index(self) -> usize {
        self.0
    }
}

/// 0-based grapheme-cluster index within a line.
/// [`crate::grapheme::grapheme_col_in_line`]'s result and
/// [`crate::lines::place_grapheme_column`]'s input.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct GraphemeCol(usize);

impl GraphemeCol {
    /// Mint a grapheme column already known to be valid.
    pub fn new(col: usize) -> Self {
        Self(col)
    }

    /// Decode a 1-based column number (CLI `path:line:col`), or `None` for
    /// `0`.
    pub fn from_number(n: usize) -> Option<Self> {
        n.checked_sub(1).map(Self)
    }

    /// The bare 0-based index.
    pub fn index(self) -> usize {
        self.0
    }

    /// The 1-based column number shown to a user (statusline,
    /// `:diagnostics`). Every column displayed for an open buffer is this.
    pub fn number(self) -> usize {
        self.0 + 1
    }
}

/// 0-based byte offset within a line: tree-sitter `Point`s and line-relative
/// highlight spans. [`crate::lines::char_to_line_byte`]'s result.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct ByteCol(usize);

impl ByteCol {
    /// Mint a byte column already known to be valid.
    pub fn new(col: usize) -> Self {
        Self(col)
    }

    /// The bare 0-based byte offset.
    pub fn index(self) -> usize {
        self.0
    }

    /// `self` advanced by `bytes`, saturating: a byte length folded onto a
    /// start column (a tree-sitter edit's end position, a bracket match's
    /// end byte, a span moved past prepended text).
    pub fn advance_saturating(self, bytes: usize) -> Self {
        Self(self.0.saturating_add(bytes))
    }
}

impl crate::offset::ExclusiveRange<ByteCol> {
    /// This range as the `std::ops::Range<usize>` that `str`/`&[u8]`
    /// slicing wants.
    pub fn as_byte_range(self) -> std::ops::Range<usize> {
        self.start.index()..self.end.index()
    }
}

#[cfg(test)]
mod tests;
