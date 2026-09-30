//! Grapheme-cluster positions and ranges.
//!
//! A selection's ends are cluster starts and it covers whole clusters. These
//! types make that a property of the value: none has a public constructor
//! from a raw [`CharOffset`], so only this crate's grapheme and line
//! primitives, which find boundaries by segmenting the text, produce them.
//! A position from a foreign coordinate system (a regex byte offset, a
//! tree-sitter node, an LSP wire position, a char-by-char scan) enters
//! through [`crate::grapheme::snap_to_cluster`], [`ClusterRange::covering`]
//! or [`ClusterRange::within`].
//!
//! A value describes the text it was computed from. The type does not name
//! that text; a caller holding one across an edit maps it through the edit
//! first.
//!
//! There is no arithmetic on these types. Moving to a neighbouring cluster is
//! a text query ([`crate::grapheme::next_cluster`],
//! [`crate::grapheme::prev_cluster`]), and [`ClusterStart::offset`] is a
//! one-way escape to a plain [`CharOffset`] for ropey and foreign APIs.

use ropey::RopeSlice;

use crate::grapheme::{ceil_boundary, cluster_end, floor_boundary, prev_cluster};
use crate::offset::{CharOffset, ExclusiveRange};

/// The first char of a grapheme cluster: a position a cursor can sit on,
/// always below the text's length.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ClusterStart(CharOffset);

impl ClusterStart {
    /// For this crate's primitives, which have just found `offset` to be a
    /// cluster start below the text end.
    pub(crate) fn mint(offset: CharOffset) -> Self {
        Self(offset)
    }

    pub fn offset(self) -> CharOffset {
        self.0
    }
}

/// A boundary between clusters: a cluster start or the text end. The bound
/// of a half-open range of whole clusters.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ClusterBound(CharOffset);

impl ClusterBound {
    /// Offset 0 is a boundary of every text.
    pub const TEXT_START: ClusterBound = ClusterBound(CharOffset::new(0));

    /// For this crate's primitives, which have just found `offset` to be a
    /// cluster boundary.
    pub(crate) fn mint(offset: CharOffset) -> Self {
        Self(offset)
    }

    pub fn offset(self) -> CharOffset {
        self.0
    }
}

impl From<ClusterStart> for ClusterBound {
    fn from(start: ClusterStart) -> Self {
        Self(start.0)
    }
}

impl From<ClusterStart> for CharOffset {
    fn from(start: ClusterStart) -> Self {
        start.0
    }
}

impl From<ClusterBound> for CharOffset {
    fn from(bound: ClusterBound) -> Self {
        bound.0
    }
}

/// One or more whole clusters, in order. Both faces of the far end are
/// carried: [`Self::last`] is the start of the last covered cluster (where a
/// cursor on it sits) and [`Self::end`] is the boundary after it (where a
/// half-open char range stops), so neither is derived by arithmetic.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ClusterRange {
    start: ClusterStart,
    last: ClusterStart,
    end: ClusterBound,
}

impl ClusterRange {
    /// For this crate's primitives, which have just found all three values
    /// in `slice`, with `start <= last` and `end` the boundary after `last`.
    pub(crate) fn mint(start: ClusterStart, last: ClusterStart, end: ClusterBound) -> Self {
        Self { start, last, end }
    }

    /// The clusters from `first` through `last`, both included. `None` when
    /// `last` is before `first`.
    pub fn through(slice: RopeSlice<'_>, first: ClusterStart, last: ClusterStart) -> Option<Self> {
        (first <= last).then(|| Self::mint(first, last, cluster_end(slice, last)))
    }

    /// The clusters from `start` up to `end`. `None` when `end` is not past
    /// `start`.
    pub fn between(slice: RopeSlice<'_>, start: ClusterStart, end: ClusterBound) -> Option<Self> {
        if end <= ClusterBound::from(start) {
            return None;
        }
        let last = prev_cluster(slice, end)?;
        Some(Self::mint(start, last, end))
    }

    /// The fewest whole clusters covering every char of `chars`: the start
    /// widens back to its cluster's start and the end widens forward to a
    /// boundary. `chars` is clamped to the slice first. `None` when nothing
    /// is left to cover.
    pub fn covering(slice: RopeSlice<'_>, chars: ExclusiveRange<CharOffset>) -> Option<Self> {
        let end = chars.end.min(CharOffset::new(slice.len_chars()));
        if chars.start >= end {
            return None;
        }
        let start = floor_boundary(slice, chars.start);
        Self::between(
            slice,
            ClusterStart::mint(start.offset()),
            ceil_boundary(slice, end),
        )
    }

    /// The most whole clusters lying inside `chars`: the start narrows forward
    /// to a boundary and the end narrows back to one. `chars` is clamped to
    /// the slice first. `None` when no whole cluster fits.
    pub fn within(slice: RopeSlice<'_>, chars: ExclusiveRange<CharOffset>) -> Option<Self> {
        let end = chars.end.min(CharOffset::new(slice.len_chars()));
        if chars.start >= end {
            return None;
        }
        let start = ceil_boundary(slice, chars.start);
        let end = floor_boundary(slice, end);
        if end <= start {
            return None;
        }
        Self::between(slice, ClusterStart::mint(start.offset()), end)
    }

    pub fn start(self) -> ClusterStart {
        self.start
    }

    pub fn last(self) -> ClusterStart {
        self.last
    }

    pub fn end(self) -> ClusterBound {
        self.end
    }

    /// The covered chars as a half-open range.
    pub fn chars(self) -> ExclusiveRange<CharOffset> {
        ExclusiveRange::new(self.start.offset(), self.end.offset())
    }

    pub fn contains(self, pos: ClusterStart) -> bool {
        self.start <= pos && pos <= self.last
    }

    /// The smallest range covering both.
    pub fn hull(self, other: ClusterRange) -> ClusterRange {
        let (last, end) = if other.last > self.last {
            (other.last, other.end)
        } else {
            (self.last, self.end)
        };
        Self::mint(self.start.min(other.start), last, end)
    }
}

impl From<ClusterRange> for ExclusiveRange<CharOffset> {
    fn from(range: ClusterRange) -> Self {
        range.chars()
    }
}

#[cfg(test)]
mod tests;
