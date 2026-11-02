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
/// `Default` is offset 0, valid in every buffer since each holds at least the
/// structural trailing `\n`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct CharOffset(usize);

impl CharOffset {
    /// Mint an offset already known to be valid (read back from another
    /// `CharOffset`, the result of an ASCII delimiter scan). Unchecked;
    /// unvalidated input goes through [`CharOffset::checked`].
    pub const fn new(idx: usize) -> Self {
        Self(idx)
    }

    /// `idx` if `idx <= rope.len_chars()`, else `None`: the mint for
    /// unvalidated input (a Steel builtin argument).
    ///
    /// There is no `clamped` form: an LSP wire position clamps
    /// line-then-column ([`crate::position_encoding::wire_range_to_char_range`]),
    /// and landing on a cluster is [`crate::grapheme::snap_to_cluster`]'s job.
    pub fn checked(rope: &Rope, idx: usize) -> Option<Self> {
        (idx <= rope.len_chars()).then_some(Self(idx))
    }

    /// The bare 0-based char index, for ropey, tree-sitter, or an LSP
    /// wire-position conversion.
    pub fn index(self) -> usize {
        self.0
    }

    /// Chars between `earlier` and `self`. Debug-panics if `earlier > self`.
    /// `self` is the later offset, matching the written order `end - start`.
    pub fn chars_since(self, earlier: CharOffset) -> usize {
        debug_assert!(
            earlier <= self,
            "chars_since: {earlier:?} is after {self:?}, chars_since measures forward only"
        );
        self.0.saturating_sub(earlier.0)
    }

    /// `self` shifted by a signed `delta` already known to preserve grapheme
    /// alignment (repositioning across an edit whose span content is
    /// unchanged).
    ///
    /// # Panics
    /// Panics if the result would be negative.
    pub fn shift(self, delta: isize) -> CharOffset {
        Self(
            self.0
                .checked_add_signed(delta)
                .expect("CharOffset::shift: result would be negative"),
        )
    }

    /// `self` moved back by `n` chars (a removed run's length, a typed-char
    /// count).
    ///
    /// # Panics
    /// Panics if `n` exceeds `self`.
    pub fn retreat(self, n: usize) -> CharOffset {
        Self(
            self.0
                .checked_sub(n)
                .expect("CharOffset::retreat: result would be negative"),
        )
    }

    /// [`Self::retreat`] clamped to 0, for a count that may exceed `self` (a
    /// cursor near the buffer start, an edit delta larger than the position
    /// it lands on).
    pub fn retreat_saturating(self, n: usize) -> CharOffset {
        Self(self.0.saturating_sub(n))
    }
}

/// A half-open range `[start, end)`: `end` is one past the last position
/// covered. Matches `BufferText::slice`, `ChangeSet`'s position-mapping API,
/// and LSP wire ranges (half-open by protocol). A range of whole clusters is
/// a [`crate::cluster::ClusterRange`] instead.
///
/// Fields are `pub`: the type names "half-open", it does not guard
/// arithmetic.
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
/// `start == end`.
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
