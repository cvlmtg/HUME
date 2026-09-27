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
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
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
            /// of its own to overflow into (see the module doc). Named
            /// `advance_saturating`, not bare `advance`, to match the
            /// `_saturating` suffix's meaning everywhere else in the
            /// workspace: a bare direction word never silently clamps.
            pub fn advance_saturating(self, cells: u32) -> Self {
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
        }
    };
}

display_col_methods!(DisplayLineCol);
display_col_methods!(BufferLineCol);

impl DisplayLineCol {
    /// Unsigned distance between `self` and `other`, direction
    /// discarded — the "which grapheme is visually closest" metric a
    /// nearest-column resolve needs, where [`Self::cells_since`]'s
    /// ordering requirement would be the wrong tool. Its only caller
    /// (`DisplayLineMap`'s nearest-column resolve) works in display-line-
    /// relative columns, so this lives on `DisplayLineCol` alone rather than
    /// in the shared macro.
    pub fn abs_diff(self, other: Self) -> u32 {
        self.0.abs_diff(other.0)
    }

    /// [`Self::cells_since`] without the ordering precondition — 0 when
    /// `earlier` is actually later, rather than debug-panicking. Its one
    /// caller that can't prove `earlier <= self`: `hume-editor`'s
    /// `cursor::place` positions a completion popup at an LSP completion
    /// session's token-start anchor, which is fixed while the live cursor
    /// (and the viewport's horizontal scroll it drives) keeps moving right —
    /// so the anchor can sit left of `viewport.horizontal_offset` in a way
    /// the live cursor, kept on-screen by `Viewport::reveal_horizontal`,
    /// never does. Display-line-relative only (like `abs_diff` above): its
    /// one caller works in that domain, and `BufferLineCol` has none.
    pub fn cells_since_saturating(self, earlier: Self) -> u32 {
        self.0.saturating_sub(earlier.0)
    }
}

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

    /// `self` shifted by a signed cell delta, saturating at both ends
    /// rather than panicking — unlike `CharOffset::shift`, a display column
    /// legitimately clamps to 0 when a multi-cursor edit's running delta
    /// outpaces the column it's applied to (e.g. a wide deletion collapsing
    /// a later cursor's indent target); that is the caller's intended
    /// behavior, not a bug this type should catch. Named `shift_saturating`,
    /// not `shift`, so the suffix carries the same meaning everywhere in the
    /// workspace: a bare `shift`/`retreat` panics on an out-of-range result
    /// (`CharOffset`'s contract), `_saturating` clamps instead. Widens
    /// through `i64` before clamping back to `u32` rather than calling
    /// `saturating_add_signed` (which takes `i32`, not `isize`) directly —
    /// narrowing `delta` first would wrap a delta outside `i32`'s range
    /// (e.g. `indent_lines`'s `delta_display_col`, `u32`-bounded but built
    /// as `isize`) into a small or negative number instead of saturating.
    ///
    /// Buffer-line-relative only, so this lives on `BufferLineCol` alone
    /// rather than in the shared macro. Two shapes among its callers:
    /// `hume-ops`'s `indent_lines` and `align_selections`'s pass-3 anchor
    /// tracking each fold a running delta accumulated within one buffer
    /// line; `align_selections`'s pass-2 `fit_0`/`fit_k` instead apply a
    /// one-shot cross-slot delta to compute a floor. Both shapes need the
    /// same saturate-don't-panic contract.
    pub fn shift_saturating(self, delta: isize) -> Self {
        Self(
            (self.0 as i64)
                .saturating_add(delta as i64)
                .clamp(0, u32::MAX as i64) as u32,
        )
    }

    /// `self` moved back by `cells` — the unsigned-length counterpart to
    /// [`Self::shift_saturating`], for the common case of retreating by a
    /// `u32` cell count rather than a signed delta a caller would otherwise
    /// negate by hand. Same saturate-at-0 contract as `shift_saturating`.
    pub fn retreat_saturating(self, cells: u32) -> Self {
        self.shift_saturating(-(cells as isize))
    }
}

/// 0-based char index within a line — never a grapheme count or a display
/// column. [`crate::lines::char_col_in_line`]'s result and
/// [`crate::lines::place_char_column`]'s input.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
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

    /// `self` advanced by `bytes` — a matched token's own byte length folded
    /// onto its start column (e.g. a bracket match's end byte from its
    /// start byte plus the matched char's UTF-8 length). Saturates rather
    /// than overflowing, matching every other `advance_saturating` in this
    /// module (`DisplayLineCol`/`BufferLineCol`'s shared one) — the two real
    /// callers this type has for advancing at all (a tree-sitter edit's end
    /// position, a bracket match's end byte) never come close to
    /// `usize::MAX`, so this stays this narrow rather than gaining
    /// `cells_since`/`shift_saturating`'s full arithmetic surface.
    pub fn advance_saturating(self, bytes: usize) -> Self {
        Self(self.0.saturating_add(bytes))
    }
}

impl crate::offset::ExclusiveRange<ByteCol> {
    /// This range as a `std::ops::Range<usize>` — the shape `str`/`&[u8]`
    /// slicing (`s.get(range)`) wants. `ExclusiveRange`'s typed fields cost
    /// two `.index()` calls at a slicing call site otherwise (build the
    /// `Range` from `self.start.index()..self.end.index()` by hand); this
    /// names that crossing once instead of repeating it at every byte-range
    /// slice site in the crate.
    pub fn as_byte_range(self) -> std::ops::Range<usize> {
        self.start.index()..self.end.index()
    }
}

#[cfg(test)]
mod tests;
