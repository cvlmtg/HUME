//! Grapheme-cluster positions and ranges.
//!
//! A selection's ends are cluster starts and it covers whole clusters. These
//! types make that a property of the value: none has a public constructor
//! from a raw [`CharOffset`], so only this crate's grapheme and line
//! primitives, which find boundaries by segmenting the text, produce them.
//! A position from a foreign coordinate system (a regex byte offset, a
//! tree-sitter node, an LSP wire position, a char-by-char scan) enters
//! through [`crate::grapheme::snap_to_cluster`], [`ClusterRange::covering`],
//! [`ClusterRange::covering_bytes`] or [`ClusterRange::within`].
//!
//! A value describes the text it was computed from. The type does not name
//! that text; a caller holding one across an edit maps it through the edit
//! first.
//!
//! There is no arithmetic on these types. Moving to a neighbouring cluster is
//! a text query ([`crate::grapheme::next_cluster`],
//! [`crate::grapheme::prev_cluster`]), and [`ClusterStart::offset`] is a
//! one-way escape to a plain [`CharOffset`] for ropey and foreign APIs.

use std::ops::Range;

use ropey::RopeSlice;

use crate::grapheme::{
    ceil_boundary, cluster_end, floor_with_prev_start, prev_cluster, snap_covering_bytes,
    snap_to_cluster, text_end,
};
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

    /// The one cluster starting at `cluster`.
    pub fn of(slice: RopeSlice<'_>, cluster: ClusterStart) -> Self {
        Self::mint(cluster, cluster, cluster_end(slice, cluster))
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
        let first = snap_to_cluster(slice, chars.start)?;
        let last = snap_to_cluster(slice, end.retreat(1))?;
        Some(Self::mint(first.start(), last.start(), last.end()))
    }

    /// [`Self::covering`] for a byte range. A bound inside a codepoint moves
    /// to that codepoint's start, as ropey's `byte_to_char` does.
    ///
    /// # Panics
    /// Panics if either bound is past the slice.
    pub fn covering_bytes(slice: RopeSlice<'_>, bytes: Range<usize>) -> Option<Self> {
        let (first, last) = snap_covering_bytes(slice, bytes)?;
        Some(Self::mint(first.start(), last.start(), last.end()))
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
        let (end, last) = if end.index() < slice.len_chars() {
            let (floor, last) = floor_with_prev_start(slice, end);
            (ClusterBound::mint(floor), last)
        } else {
            let end = text_end(slice);
            (end, prev_cluster(slice, end))
        };
        if end <= start {
            return None;
        }
        let last = last.expect("a bound past a boundary has a cluster before it");
        Some(Self::mint(ClusterStart::mint(start.offset()), last, end))
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

#[cfg(test)]
mod tests;
