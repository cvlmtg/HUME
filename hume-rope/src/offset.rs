//! A domain-typed char offset.
//!
//! Every position in a HUME buffer is an index into the rope's sequence of
//! Unicode scalar values — a char offset. Stepping one by a raw `+ 1`/`- 1`
//! can land mid-grapheme-cluster: a combining sequence (`é` = U+0065 +
//! U+0301) or a ZWJ emoji is more than one char wide, so the position right
//! after its first char is not a cluster boundary at all.
//!
//! [`CharOffset`] makes that mistake a compile error: the field is private
//! and the type implements no `Add`/`Sub`/`AddAssign`, so `offset + 1`
//! doesn't type-check. The only ways to produce
//! one are the boundary-walking functions in [`crate::grapheme`], the
//! offset-returning functions in [`crate::lines`], a validated mint against a
//! rope ([`CharOffset::checked`]), or [`CharOffset::new`] — a trusted mint
//! for a value a caller has already proven valid by some other means (an
//! ASCII delimiter scan, a value read back from another `CharOffset`).
//!
//! Deliberately not a bare tuple struct with a `pub` field, matching
//! [`crate::line`]'s `RopeyLine`/`ContentLine`: a `pub` field would make
//! `CharOffset(offset.index() + 1)` writable again, exactly the derivation
//! this type exists to forbid.
//!
//! What this type does **not** guarantee: grapheme-cluster alignment. It
//! guarantees only "a valid char index into some rope" — [`crate::cursor::CharCursor`]
//! is codepoint-level by design and can legitimately yield a `CharOffset`
//! that sits mid-cluster, and `Selection::end_inclusive` (in `hume-editing`)
//! deliberately lands on a cluster's *last* codepoint, not its start. A
//! caller that needs a cluster boundary asks for one explicitly — via
//! [`crate::grapheme::next_grapheme_boundary`]/[`crate::grapheme::prev_grapheme_boundary`]
//! or [`crate::grapheme::snap_to_cluster_start`] — rather than assuming the
//! type provides it.

use ropey::Rope;

/// A char offset into a buffer — an index into its sequence of Unicode
/// scalar values, never a byte offset or a display column.
///
/// `Default` is offset 0 — always valid, since every HUME buffer holds at
/// least the structural trailing `\n` — for a caller that needs a `CharOffset`
/// with no rope in hand to mint from (e.g. `#[derive(Default)]` on a
/// containing struct).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct CharOffset(usize);

impl CharOffset {
    /// Mint an offset already known to be valid — e.g. one just read back
    /// from another `CharOffset`, or a value a caller has proven safe by
    /// some other means (an ASCII delimiter scan). Does not check against
    /// any rope; a caller minting from unvalidated input wants
    /// [`CharOffset::checked`] instead — there is no `clamped` counterpart,
    /// see that method's own doc for why.
    pub fn new(idx: usize) -> Self {
        Self(idx)
    }

    /// `idx` if it names a valid char position in `rope` (`idx <=
    /// rope.len_chars()`), else `None` — the mint for input whose validity
    /// isn't yet established (a Steel builtin argument, a wire-supplied
    /// position). `CharOffset` has no `clamped`/`snapped` counterpart: a
    /// blind `idx.min(rope.len_chars())` is the wrong tool everywhere a
    /// position needs clamping — an LSP wire position clamps line-then-column
    /// ([`crate::position_encoding::wire_to_char`]), and landing on a
    /// grapheme boundary is [`crate::grapheme::snap_to_cluster_start`]'s job
    /// directly, taken on the type the caller already has rather than routed
    /// back through this one.
    pub fn checked(rope: &Rope, idx: usize) -> Option<Self> {
        (idx <= rope.len_chars()).then_some(Self(idx))
    }

    /// The bare 0-based char index, for handing to ropey itself, tree-sitter,
    /// or an LSP wire-position conversion — every foreign coordinate system
    /// that has no domain of its own.
    pub fn index(self) -> usize {
        self.0
    }

    /// Chars between `earlier` and `self` (`earlier <= self`). Debug-panics
    /// on inversion — a raw `self - earlier` would silently wrap instead.
    ///
    /// The named form for "how many chars does this span cover" —
    /// `ChangeSetBuilder::retain`/`delete`, span-width comparisons, and
    /// similar length derivations all read `later.chars_since(earlier)`.
    /// `self` is the *later* offset deliberately: every call site today is a
    /// subtraction (`end - start`, `p - b.old_pos()`), and keeping `self` on
    /// the same side as the minuend preserves that written order — a
    /// receiver/argument swap is exactly the mistake a raw subtraction hides
    /// silently, caught (if at all) only by the debug-build assert below.
    pub fn chars_since(self, earlier: CharOffset) -> usize {
        debug_assert!(
            earlier <= self,
            "chars_since: {earlier:?} is after {self:?} — chars_since measures backward only"
        );
        self.0.saturating_sub(earlier.0)
    }

    /// `self` shifted by `delta` chars (may be negative) — for repositioning
    /// a selection across an edit by a delta already known to preserve
    /// grapheme alignment (the shifted span's content is unchanged, only its
    /// offset is — e.g. an edit-delta reposition of `anchor`/`head` across a
    /// retained span). Panics if the result would be negative.
    pub fn shift(self, delta: isize) -> CharOffset {
        Self(
            self.0
                .checked_add_signed(delta)
                .expect("CharOffset::shift: result would be negative"),
        )
    }

    /// `self` shifted by `delta` chars, clamping to 0 instead of panicking —
    /// for a caller subtracting a count that may legitimately exceed `self`
    /// (a cramped cursor near the buffer start, an undo/redo delta larger
    /// than the position it lands on). Reach for [`Self::shift`] when the
    /// result is known non-negative and a violation should panic loudly
    /// instead of silently clamping.
    pub fn shift_saturating(self, delta: isize) -> CharOffset {
        Self(self.0.saturating_add_signed(delta))
    }
}

/// A half-open range `[start, end)` — `end` is one past the last position
/// covered. Matches `BufferText::slice`, `ChangeSet`'s position-mapping API,
/// and LSP wire ranges (half-open by protocol).
///
/// Exists alongside [`InclusiveRange`] so the two `(T, T)`-shaped
/// conventions this codebase uses for a char range — inclusive
/// (`Selection`, every text-object/bracket/quote/tag/search finder) and
/// exclusive (everything above) — are two distinct, named types instead of
/// one bare tuple shape whose meaning depends on which function produced it.
/// Fields are `pub`, unlike `CharOffset`: nothing here prevents arithmetic
/// misuse the way `CharOffset`'s private field does — this type exists
/// purely so "which convention" is a name, not a lookup — so it reads as
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
    /// is the one production caller — a per-line decoration filter checking
    /// a resolved line against the pane's visible range.
    pub fn contains(&self, pos: T) -> bool {
        pos >= self.start && pos < self.end
    }
}

/// An inclusive range `[start, end]` — both ends are covered. Matches
/// `Selection::start()`/`end()`, every text-object/bracket/quote/tag/search
/// finder, and `ObjectSpans`. Never empty: a single position is `start ==
/// end`. See [`ExclusiveRange`]'s doc for why this is a named type rather
/// than a bare tuple.
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

impl InclusiveRange<CharOffset> {
    /// This range as a half-open one — same span, exclusive end one char
    /// past `self.end` (via [`CharOffset::shift`], not a raw `.index() + 1`),
    /// for handing an inclusive result (`Selection`, a
    /// text-object/bracket/quote finder) to `text.slice()`, which wants an
    /// [`ExclusiveRange`].
    pub fn to_exclusive(self) -> ExclusiveRange<CharOffset> {
        ExclusiveRange::new(self.start, self.end.shift(1))
    }
}

#[cfg(test)]
mod tests;
