//! Domain-typed line-relative columns.
//!
//! "Column" means four different things in this codebase, and mixing them up
//! silently produces the wrong char position or the wrong screen cell: a
//! **display column** (terminal cells, tab-expanded), a **char column** (char
//! index within a line), a **grapheme column** (grapheme-cluster index within
//! a line), and a **byte column** (byte offset within a line). The five types
//! below give each its own type — [`DisplayLineCol`]/[`BufferLineCol`] for
//! the first (split further, see below), [`CharCol`], [`GraphemeCol`],
//! [`ByteCol`] for the rest — so a function typed to take one can't be handed
//! another, matching [`crate::offset::CharOffset`] and
//! [`crate::line::{RopeyLine, ContentLine}`](crate::line). A bare `*LineCol`
//! is always display cells; a unit prefix (`Char`/`Grapheme`/`Byte`) names
//! any other unit — so the type itself says which of the two questions
//! ("how many cells" vs. "how many chars/graphemes/bytes") a column answers.
//!
//! # Display columns have an origin
//!
//! A display column is also measured *from* somewhere, and that "from" is
//! itself two different things under soft wrap: a continuation display line
//! renumbers its columns from its own left edge (its indent, under
//! `WrapMode::Indent`), so the same character has a different column
//! depending on whether it's counted from its own display line
//! ([`DisplayLineCol`]) or from its whole buffer line ([`BufferLineCol`]).
//! The two coincide exactly when a buffer line occupies one display line —
//! no wrap, or a line short enough not to wrap regardless — which is why
//! [`BufferLineCol::as_display_line_unwrapped`] exists as a named, doc'd escape hatch
//! rather than a silent reinterpretation.
//!
//! # Positions, not counts
//!
//! Unlike [`CharOffset`](crate::offset::CharOffset), which forbids arithmetic
//! outright, a display column is a genuine accumulator: formatting a line
//! advances one grapheme's width at a time, and tab-stop math divides and
//! multiplies by the tab width. The two display-column types keep a private
//! field like every domain type in this crate, but expose named arithmetic
//! (`advance`/`cells_since`/`cells_since_saturating`/`abs_diff`/`shift`) instead of forbidding
//! it — the compile-time win here is keeping `DisplayLineCol` and
//! `BufferLineCol` from being silently interchanged, not banning `+`/`-` on
//! a column itself. [`CharCol`]/[`GraphemeCol`]/[`ByteCol`] see no such
//! arithmetic in the codebase today and stay as strict as `CharOffset`.
//!
//! # No `checked`/`clamped` mint for the display types
//! A display column has no fixed upper bound of its own to validate against
//! — unlike a char offset (bounded by a rope's length) or a line index
//! (bounded by a rope's line count), a column past a line's actual content is
//! simply where the cursor would sit if the line were that long. Every
//! resolver that turns one back into a [`CharOffset`](crate::offset::CharOffset)
//! (`DisplayLineMap::char_at`, `DisplayLineMap::char_at_buffer_line_col`) clamps internally.
//! [`DisplayLineCol::new`]/[`BufferLineCol::new`] are the only mints.

/// A display column measured from its display line's left edge — what
/// `hume_engine::display_lines::DisplayLineMap::locate` returns. See the module doc's
/// "Display columns have an origin" section.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct DisplayLineCol(u32);

/// A display column measured from its buffer line's start — what
/// `hume_engine::display_lines::DisplayLineMap::buffer_line_col` returns, and what a
/// numeric-prefixed vertical move (`9j`/`9k`) latches, since it targets the
/// same buffer-line column on its landing line regardless of which display
/// line it lands on. See the module doc's "Display columns have an origin"
/// section.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct BufferLineCol(u32);

/// The arithmetic shared by [`DisplayLineCol`] and [`BufferLineCol`] — a
/// macro rather than a trait, since neither type gains a caller from the
/// other implementing some shared interface; each is generated inline so the
/// two stay textually adjacent to their own doc comments above.
macro_rules! display_col_methods {
    ($ty:ident) => {
        impl $ty {
            /// Mint a column already known to be valid — e.g. one just read
            /// back from another value of this type, or the result of
            /// `hume_engine::display_lines::DisplayLineMap` walking a formatted line. See the
            /// module doc for why there is no `checked`/`clamped` form.
            pub fn new(col: u32) -> Self {
                Self(col)
            }

            /// The bare 0-based column, for handing to `hume_rope::width`
            /// (which is origin-agnostic — see its own module doc) or a
            /// terminal-cell coordinate conversion.
            pub fn get(self) -> u32 {
                self.0
            }

            /// `self` advanced by `cells` — a grapheme's width folded into a
            /// running format cursor. Saturates: a column has no upper bound
            /// of its own to overflow into (see the module doc).
            pub fn advance(self, cells: u32) -> Self {
                Self(self.0.saturating_add(cells))
            }

            /// Cells between `earlier` and `self` (`earlier <= self`) —
            /// the named form for "how wide is this span", mirroring
            /// `CharOffset::chars_since`. Debug-panics on inversion, where a
            /// raw subtraction would silently wrap.
            pub fn cells_since(self, earlier: Self) -> u32 {
                debug_assert!(
                    earlier <= self,
                    "cells_since: {earlier:?} is after {self:?} — cells_since measures forward only"
                );
                self.0.saturating_sub(earlier.0)
            }

            /// [`Self::cells_since`] without the ordering precondition — 0
            /// when `earlier` is actually later, rather than debug-panicking.
            /// For the one caller that cannot prove `earlier <= self`:
            /// `hume-editor`'s `cursor::place` positions a completion popup
            /// at an LSP completion session's token-start anchor, which is
            /// fixed while the live cursor (and the viewport's horizontal
            /// scroll it drives) keeps moving right — so the anchor can sit
            /// left of `viewport.horizontal_offset` in a way the live cursor,
            /// kept on-screen by `ensure_cursor_visible_horizontal`, never
            /// does. Every other caller of `cells_since` has that guarantee
            /// and keeps the assert.
            pub fn cells_since_saturating(self, earlier: Self) -> u32 {
                self.0.saturating_sub(earlier.0)
            }

            /// Unsigned distance between `self` and `other`, direction
            /// discarded — the "which grapheme is visually closest" metric a
            /// nearest-column resolve needs, where [`Self::cells_since`]'s
            /// ordering requirement would be the wrong tool.
            pub fn abs_diff(self, other: Self) -> u32 {
                self.0.abs_diff(other.0)
            }

            /// `self` shifted by a signed cell delta, saturating at 0 rather
            /// than panicking on a negative result — unlike
            /// `CharOffset::shift`, a display column legitimately clamps to 0
            /// when a multi-cursor edit's running delta outpaces the column
            /// it's applied to (e.g. a wide deletion collapsing a later
            /// cursor's indent target); that is the caller's intended
            /// behavior, not a bug this type should catch.
            ///
            /// `delta as i32` narrows before `saturating_add_signed` (which
            /// takes `i32`, not `isize`) — silently wrapping, not saturating,
            /// for a `delta` outside `i32`'s range: a sufficiently large
            /// positive delta could wrap negative and shift `self` *down*
            /// instead of saturating upward. Not reachable today — the sole
            /// caller (`hume-ops/src/edit/insert.rs`) accumulates a running
            /// delta within one buffer line, far short of `i32::MAX` cells —
            /// but a future caller summing deltas across a whole buffer
            /// should not assume this saturates the way the rest of this
            /// method's doc does.
            pub fn shift(self, delta: isize) -> Self {
                Self(self.0.saturating_add_signed(delta as i32))
            }
        }
    };
}

display_col_methods!(DisplayLineCol);
display_col_methods!(BufferLineCol);

impl BufferLineCol {
    /// This column read as a display-line-relative one — sound only where
    /// the buffer line occupies a single display line, where the two
    /// origins coincide (see the module doc). Two callers rely on that:
    /// `DisplayLineMap::char_at_buffer_line_col`
    /// (while wrapping, `ensure_formatted` promotes the format bound to
    /// `Full` and never consults the column this produces, so the
    /// non-coincident case is never actually reached there) and
    /// `hume-editor`'s vertical-motion sticky column, which resolves a
    /// `BufferLine`-family latch through the no-wrap-only `buffer_line_col` in
    /// the first place — the same coincidence, from the other direction.
    pub fn as_display_line_unwrapped(self) -> DisplayLineCol {
        DisplayLineCol(self.0)
    }
}

/// 0-based char index within a line — never a grapheme count or a display
/// column. [`crate::lines::char_col_in_line`]'s result and
/// [`crate::lines::place_char_column`]'s input.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct CharCol(usize);

impl CharCol {
    /// Mint a char column already known to be valid — e.g. one just read
    /// back from another `CharCol`, or `char_pos.chars_since(line_start)`.
    pub fn new(col: usize) -> Self {
        Self(col)
    }

    /// The bare 0-based index.
    pub fn index(self) -> usize {
        self.0
    }
}

/// 0-based grapheme-cluster index within a line — matches how many times the
/// user pressed → to reach a position from the line's start.
/// [`crate::grapheme::grapheme_col_in_line`]'s result and
/// [`crate::lines::place_grapheme_column`]'s input.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct GraphemeCol(usize);

impl GraphemeCol {
    /// Mint a grapheme column already known to be valid.
    pub fn new(col: usize) -> Self {
        Self(col)
    }

    /// Decode a 1-based column number (CLI `path:line:col`, a statusline
    /// value round-tripped from a user) into the 0-based index it addresses,
    /// or `None` if `n` is `0` — there is no column 0 to land on.
    pub fn from_number(n: usize) -> Option<Self> {
        n.checked_sub(1).map(Self)
    }

    /// The bare 0-based index.
    pub fn index(self) -> usize {
        self.0
    }

    /// The 1-based column number a user reads back (statusline, CLI
    /// `path:line:col`, `:diagnostics`) — CLAUDE.md's "Displayed value"
    /// invariant: every column shown to a user is this, on an open buffer.
    pub fn number(self) -> usize {
        self.0 + 1
    }
}

/// 0-based byte offset within a line — tree-sitter `Point`s and line-relative
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
}

#[cfg(test)]
mod tests;
