use hume_rope::cluster::{ClusterRange, ClusterStart};
use hume_rope::column::{BufferLineCol, DisplayLineCol};

/// A display column together with the frame it was measured in.
///
/// Both variants are `hume_engine::display_lines::DisplayLineMap` quantities from one authority,
/// so both count tab expansion, wide glyphs and inline decorations (inlay
/// hints, ghost text) identically. They differ only in what they're measured
/// *from*: under soft wrap, a continuation display line renumbers its
/// columns from its own left edge (its indent, under `WrapMode::Indent`), so
/// the same character has a different [`DisplayLineCol`] than
/// [`BufferLineCol`]. Reading one as the other sends the cursor sideways,
/// which [`DisplayLineCol`]/[`BufferLineCol`] being distinct types makes a
/// compile error rather than a bug to find at runtime. With wrapping off a
/// display line *is* the whole buffer line, so the two coincide and either
/// variant reads back the same number. This is why a motion switching
/// families (`j` then `2j`, or vice versa) re-derives instead of reusing a
/// latch tagged with the other variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StickyDisplayCol {
    /// Column within the current display line (`DisplayLineMap::locate`): what
    /// `j`/`k`, page/half-page scroll, and the mouse wheel latch.
    DisplayLine {
        display_col: DisplayLineCol,
        /// The wrap column `display_col` was measured against
        /// (`DisplayLineMap::resolved_wrap_width`). A pane resize changes what
        /// column a display-line-relative latch's number means (the same
        /// display-line-relative column addresses a different buffer
        /// position once display lines re-flow at a new width), so a reader
        /// compares this against `DisplayLineMap`'s *current* resolved width and
        /// re-derives on a mismatch instead of reusing a column measured for
        /// a wrap geometry other than the current one.
        wrap_width: Option<u16>,
    },
    /// Column within the buffer line (`DisplayLineMap::buffer_line_col`): what an
    /// explicit numeric prefix (`9j`/`9k`) latches. Carries no wrap width:
    /// a buffer-line column counts a line's own characters and never
    /// depends on wrap geometry, unlike the `DisplayLine` variant above.
    BufferLine { display_col: BufferLineCol },
}

/// Which end of a selection the cursor is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Facing {
    /// The head is at or after the anchor: the user extended towards the end
    /// of the text. A cursor faces forward.
    Forward,
    /// The head is before the anchor.
    Backward,
}

impl Facing {
    /// `Forward` when `forward`, else `Backward`.
    pub fn from_forward(forward: bool) -> Self {
        if forward {
            Self::Forward
        } else {
            Self::Backward
        }
    }
}

/// A selection: an anchor, which stays put when the user extends, and a
/// head, where the cursor is drawn. Both are cluster starts, and the
/// selection covers every cluster from the earlier through the later, both
/// included. `anchor == head` is a cursor covering one cluster, never a
/// zero-width point.
///
/// A selection holds positions, not text. Reading what it covers needs the
/// text it was built for, so those reads live on
/// [`super::SelectionView`], which a paired text and selection set hand out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Selection {
    anchor: ClusterStart,
    head: ClusterStart,
    /// Sticky display column for vertical motion. `None` means "not latched;
    /// recompute on next vertical move." Every constructor and transform
    /// clears it except [`Self::with_sticky`], and a translation through an
    /// edit keeps it only when the edit did not touch the head's line.
    sticky_display_col: Option<StickyDisplayCol>,
}

impl Selection {
    /// A cursor on the cluster starting at `at`.
    pub fn cursor(at: ClusterStart) -> Self {
        Self::new(at, at)
    }

    pub fn new(anchor: ClusterStart, head: ClusterStart) -> Self {
        Self {
            anchor,
            head,
            sticky_display_col: None,
        }
    }

    /// The selection covering `range`, with the cursor on its last cluster
    /// when facing forward and on its first when facing backward.
    pub fn covering(range: ClusterRange, facing: Facing) -> Self {
        match facing {
            Facing::Forward => Self::new(range.start(), range.last()),
            Facing::Backward => Self::new(range.last(), range.start()),
        }
    }

    pub fn anchor(self) -> ClusterStart {
        self.anchor
    }

    pub fn head(self) -> ClusterStart {
        self.head
    }

    pub fn facing(self) -> Facing {
        if self.anchor <= self.head {
            Facing::Forward
        } else {
            Facing::Backward
        }
    }

    pub fn is_cursor(self) -> bool {
        self.anchor == self.head
    }

    /// Anchor and head swapped.
    #[must_use]
    pub fn flip(self) -> Self {
        Self::new(self.head, self.anchor)
    }

    /// The head moved to `head`, the anchor kept.
    #[must_use]
    pub fn with_head(self, head: ClusterStart) -> Self {
        Self::new(self.anchor, head)
    }

    /// A cursor on the head.
    #[must_use]
    pub fn to_head(self) -> Self {
        Self::cursor(self.head)
    }

    /// A cursor on the anchor.
    #[must_use]
    pub fn to_anchor(self) -> Self {
        Self::cursor(self.anchor)
    }

    /// This selection with `col` latched for the next vertical move.
    #[must_use]
    pub fn with_sticky(self, col: StickyDisplayCol) -> Self {
        Self {
            sticky_display_col: Some(col),
            ..self
        }
    }

    pub fn sticky_display_col(self) -> Option<StickyDisplayCol> {
        self.sticky_display_col
    }

    /// The earlier of anchor and head.
    pub(crate) fn start(self) -> ClusterStart {
        self.anchor.min(self.head)
    }

    /// The later of anchor and head: the start of the last covered cluster.
    pub(crate) fn last(self) -> ClusterStart {
        self.anchor.max(self.head)
    }

    /// This selection with `start` and `last` replaced, facing and sticky
    /// column kept.
    pub(crate) fn with_ends(self, start: ClusterStart, last: ClusterStart) -> Self {
        let (anchor, head) = match self.facing() {
            Facing::Forward => (start, last),
            Facing::Backward => (last, start),
        };
        Self {
            anchor,
            head,
            ..self
        }
    }

    pub(crate) fn without_sticky(self) -> Self {
        Self {
            sticky_display_col: None,
            ..self
        }
    }
}

#[cfg(test)]
mod tests;
