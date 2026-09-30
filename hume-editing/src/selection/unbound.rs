//! Selections an edit leaves behind, described against the text before it.

use hume_rope::cluster::ClusterStart;
use hume_rope::column::CharCol;
use hume_rope::line::ContentLine;
use hume_rope::offset::CharOffset;

use super::{Selection, SelectionView};
use crate::changeset::{Assoc, PosMapCursor};
use crate::edit::TextChange;
use crate::text::BufferText;

/// One selection an edit leaves behind, described in terms of the text
/// before the edit: a selection to carry through it, or one moved to other
/// lines. It has no reads; the resolver turns it into a [`Selection`] of the
/// new text, landing every end on a cluster start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnboundSelection(Kind);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Kept {
        sel: Selection,
        assoc: Assoc,
    },
    AtLines {
        sel: Selection,
        first: LineEnd,
        last: LineEnd,
    },
}

impl UnboundSelection {
    /// `sel` carried through the edit, each end past text inserted at it.
    pub(crate) fn kept(sel: Selection) -> Self {
        Self::kept_with(sel, Assoc::After)
    }

    /// `sel` carried through the edit, each end on `assoc`'s side of text
    /// inserted at it. One side for both ends keeps the selection's facing.
    pub(crate) fn kept_with(sel: Selection, assoc: Assoc) -> Self {
        Self(Kind::Kept {
            sel: sel.without_sticky(),
            assoc,
        })
    }

    /// `sel` with its first end moved to line `first` of the new text and its
    /// last end to line `last`. Each end keeps its column, clamped to its new
    /// line's last content cluster; an end on its line's `\n` stays on the
    /// new line's `\n`. For an edit that moves whole lines.
    ///
    /// # Panics
    /// Panics if `first` is after `last`.
    pub fn at_lines(sel: SelectionView<'_>, first: ContentLine, last: ContentLine) -> Self {
        assert!(
            first <= last,
            "at_lines: the first end's line is after the last's"
        );
        let text = sel.text();
        Self(Kind::AtLines {
            sel: sel.selection().without_sticky(),
            first: LineEnd::of(text, sel.start(), first),
            last: LineEnd::of(text, sel.last(), last),
        })
    }
}

/// Where a selection end sits on a line of the new text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LineEnd {
    line: ContentLine,
    place: LinePlace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinePlace {
    Break,
    Col(CharCol),
}

impl LineEnd {
    /// `end`, a cluster of `text`, moved to `line` of the new text.
    fn of(text: &BufferText, end: ClusterStart, line: ContentLine) -> Self {
        let old = text.char_to_line(end.offset());
        let place = if end == crate::lines::line_break(text, old) {
            LinePlace::Break
        } else {
            LinePlace::Col(crate::lines::char_col_in_line(text, old, end.offset()))
        };
        Self { line, place }
    }

    /// The cluster of `text` this end names.
    ///
    /// # Panics
    /// Panics if the line is past the last line of `text`.
    fn resolve(self, text: &BufferText) -> ClusterStart {
        assert!(
            self.line <= text.last_content_line(),
            "an end moved to line {}, past the last line of the text",
            self.line.index()
        );
        match self.place {
            LinePlace::Break => crate::lines::line_break(text, self.line),
            LinePlace::Col(col) => crate::lines::place_char_column(text, self.line.into(), col),
        }
    }
}

/// Resolves positions of the old text against the new one. Its walk through
/// the change is monotone, so it restarts only when a position sits behind
/// the last one asked about.
pub(crate) struct Resolver<'a> {
    change: &'a TextChange<'a>,
    text: &'a BufferText,
    cursor: PosMapCursor<'a>,
    last: CharOffset,
}

impl<'a> Resolver<'a> {
    pub(crate) fn new(change: &'a TextChange<'a>, text: &'a BufferText) -> Self {
        Self {
            change,
            text,
            cursor: PosMapCursor::new(change.changes().ops()),
            last: CharOffset::default(),
        }
    }

    pub(crate) fn text(&self) -> &'a BufferText {
        self.text
    }

    /// `pos` of the old text in the new one.
    pub(crate) fn map_old(&mut self, pos: CharOffset, assoc: Assoc) -> CharOffset {
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
        sel.with_ends(self.text.snap(start), self.text.snap(last))
    }

    pub(crate) fn selection(&mut self, UnboundSelection(kind): UnboundSelection) -> Selection {
        let text = self.text;
        match kind {
            Kind::Kept { sel, assoc } => self.carry(sel, assoc),
            Kind::AtLines { sel, first, last } => {
                sel.with_ends(first.resolve(text), last.resolve(text))
            }
        }
    }
}

#[cfg(test)]
mod tests;
