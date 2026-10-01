//! Positions of the text before a change, resolved against the text after it.

use hume_rope::offset::CharOffset;

use super::Selection;
use crate::changeset::{Assoc, PosMapCursor};
use crate::edit::TextChange;
use crate::text::BufferText;

/// Resolves positions of the old text against the new one. Its walk through
/// the change is monotone, so it restarts only when a position sits behind
/// the last one asked about.
pub(crate) struct Resolver<'a> {
    change: &'a TextChange<'a>,
    cursor: PosMapCursor<'a>,
    last: CharOffset,
}

impl<'a> Resolver<'a> {
    pub(crate) fn new(change: &'a TextChange<'a>) -> Self {
        Self {
            change,
            cursor: PosMapCursor::new(change.changes().ops()),
            last: CharOffset::default(),
        }
    }

    pub(crate) fn text(&self) -> &'a BufferText {
        self.change.after()
    }

    /// `pos` of the old text in the new one.
    fn map_old(&mut self, pos: CharOffset, assoc: Assoc) -> CharOffset {
        if pos < self.last {
            self.cursor = PosMapCursor::new(self.change.changes().ops());
        }
        self.last = pos;
        self.cursor.map(pos, assoc)
    }

    /// `sel` carried through the change, each end on `assoc`'s side of text
    /// inserted at it and landed on the cluster of the new text holding it.
    pub(crate) fn carry(&mut self, sel: Selection, assoc: Assoc) -> Selection {
        let start = self.map_old(sel.start().offset(), assoc);
        let last = self.map_old(sel.last().offset(), assoc);
        sel.with_ends(self.text().snap(start), self.text().snap(last))
    }
}
