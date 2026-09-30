//! A domain-typed char offset.
//!
//! A buffer position is an index into the rope's chars. A raw `+ 1`/`- 1` can
//! land mid-cluster (`é` as U+0065 U+0301, ZWJ emoji), so [`CharOffset`] has a
//! private field and no `Add`/`Sub`/`AddAssign`. Offsets come from
//! [`crate::grapheme`]'s boundary walks, [`crate::lines`], the validated
//! [`CharOffset::checked`], or the trusted [`CharOffset::new`].
//!
//! The type guarantees a valid char index, not cluster alignment:
//! [`crate::cursor::CharCursor`] can yield a mid-cluster offset. A cluster
//! position is [`crate::cluster::ClusterStart`], which only this crate's
//! boundary walks mint ([`crate::grapheme::snap_to_cluster`] for a foreign
//! offset).

use ropey::Rope;

/// A char offset into a buffer: an index into its sequence of Unicode
/// scalar values, never a byte offset or a display column.
///
/// `Default` is offset 0 (always valid, since every HUME buffer holds at
/// least the structural trailing `\n`), for a caller that needs a `CharOffset`
/// with no rope in hand to mint from (e.g. `#[derive(Default)]` on a
/// containing struct).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct CharOffset(usize);

impl CharOffset {
    /// Mint an offset already known to be valid, e.g. one just read back
    /// from another `CharOffset`, or a value a caller has proven safe by
    /// some other means (an ASCII delimiter scan). Does not check against
    /// any rope; a caller minting from unvalidated input wants
    /// [`CharOffset::checked`] instead. There is no `clamped` counterpart
    /// (see that method's own doc for why).
    pub const fn new(idx: usize) -> Self {
        Self(idx)
    }

    /// `idx` if it names a valid char position in `rope` (`idx <=
    /// rope.len_chars()`), else `None`: the mint for input whose validity
    /// isn't yet established (a Steel builtin argument, a wire-supplied
    /// position). `CharOffset` has no `clamped`/`snapped` counterpart: a
    /// blind `idx.min(rope.len_chars())` is the wrong tool everywhere a
    /// position needs clamping. An LSP wire position clamps line-then-column
    /// ([`crate::position_encoding::wire_to_char`]), and landing on a
    /// cluster is [`crate::grapheme::snap_to_cluster`]'s job.
    pub fn checked(rope: &Rope, idx: usize) -> Option<Self> {
        (idx <= rope.len_chars()).then_some(Self(idx))
    }

    /// The bare 0-based char index, for handing to ropey itself, tree-sitter,
    /// or an LSP wire-position conversion: every foreign coordinate system
    /// that has no domain of its own.
    pub fn index(self) -> usize {
        self.0
    }

    /// Chars between `earlier` and `self` (`earlier <= self`). Debug-panics
    /// on inversion. A raw `self - earlier` would silently wrap instead.
    ///
    /// The named form for "how many chars does this span cover".
    /// `self` is the *later* offset because every call site today is a
    /// subtraction (`end - start`, `p - b.old_pos()`), and keeping `self` on
    /// the same side as the minuend preserves that written order. A
    /// receiver/argument swap is exactly the mistake a raw subtraction hides
    /// silently, caught (if at all) only by the debug-build assert below.
    pub fn chars_since(self, earlier: CharOffset) -> usize {
        debug_assert!(
            earlier <= self,
            "chars_since: {earlier:?} is after {self:?}, chars_since measures forward only"
        );
        self.0.saturating_sub(earlier.0)
    }

    /// `self` shifted by `delta` chars (may be negative), for repositioning
    /// a selection across an edit by a delta already known to preserve
    /// grapheme alignment (the shifted span's content is unchanged, only its
    /// offset is, e.g. an edit-delta reposition of `anchor`/`head` across a
    /// retained span). Panics if the result would be negative.
    pub fn shift(self, delta: isize) -> CharOffset {
        Self(
            self.0
                .checked_add_signed(delta)
                .expect("CharOffset::shift: result would be negative"),
        )
    }

    /// `self` moved back by `n` chars: the unsigned-length counterpart to
    /// [`Self::shift`], for the common case of retreating by a `usize`
    /// count (a removed run's length, a typed-char count) rather than a
    /// signed delta a caller would otherwise negate by hand. Same contract
    /// as `shift`: panics if `n` exceeds `self`.
    pub fn retreat(self, n: usize) -> CharOffset {
        Self(
            self.0
                .checked_sub(n)
                .expect("CharOffset::retreat: result would be negative"),
        )
    }

    /// [`Self::retreat`] clamped to 0 instead of panicking, for a caller
    /// retreating by a count that may legitimately exceed `self` (a
    /// cramped cursor near the buffer start, an edit delta larger than the
    /// position it lands on). Reach for `retreat` when the result is known
    /// non-negative and a violation should panic loudly instead of
    /// silently clamping.
    pub fn retreat_saturating(self, n: usize) -> CharOffset {
        Self(self.0.saturating_sub(n))
    }
}

/// A half-open range `[start, end)`: `end` is one past the last position
/// covered. Matches `BufferText::slice`, `ChangeSet`'s position-mapping API,
/// and LSP wire ranges (half-open by protocol). A range of whole clusters is
/// a [`crate::cluster::ClusterRange`] instead.
///
/// Fields are `pub`, unlike `CharOffset`: nothing here prevents arithmetic
/// misuse the way `CharOffset`'s private field does. This type exists so
/// "half-open" is a name rather than a bare tuple whose meaning depends on
/// which function produced it, and public fields let it read as
/// ergonomically as `std::ops::Range` does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExclusiveRange<T> {
    pub start: T,
    pub end: T,
}

impl<T> ExclusiveRange<T> {
    pub fn new(start: T, end: T) -> Self {
        Self { start, end }
    }
}

impl<T: Copy + PartialOrd> ExclusiveRange<T> {
    /// `hume-editor`'s `DecoratedPane.lines` (an `ExclusiveRange<RopeyLine>`)
    /// and `hume_engine::format::FormatBound::reached`'s `ToByte` arm (an
    /// `ExclusiveRange<ByteCol>`) are the two production callers.
    pub fn contains(&self, pos: T) -> bool {
        pos >= self.start && pos < self.end
    }
}

impl<T: PartialEq> ExclusiveRange<T> {
    /// Whether the range covers nothing (`start == end`).
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// An inclusive range `[start, end]`: both ends are covered. Used for line
/// ranges (`InclusiveRange<ContentLine>`). Never empty: a single position is
/// `start == end`. See [`ExclusiveRange`]'s doc for why this is a named type
/// rather than a bare tuple.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InclusiveRange<T> {
    pub start: T,
    pub end: T,
}

impl<T> InclusiveRange<T> {
    pub fn new(start: T, end: T) -> Self {
        Self { start, end }
    }
}

impl<T: Copy + PartialOrd> InclusiveRange<T> {
    pub fn contains(&self, pos: T) -> bool {
        pos >= self.start && pos <= self.end
    }
}

#[cfg(test)]
mod tests;
