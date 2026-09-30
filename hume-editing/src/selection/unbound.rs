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
    pub fn kept(sel: Selection) -> Self {
        Self::kept_with(sel, Assoc::After)
    }

    /// `sel` carried through the edit, each end on `assoc`'s side of text
    /// inserted at it. One side for both ends keeps the selection's facing.
    pub fn kept_with(sel: Selection, assoc: Assoc) -> Self {
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
            let start = text.line_to_char(old.into());
            LinePlace::Col(CharCol::new(end.offset().chars_since(start)))
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
        let line_break = crate::lines::line_break(text, self.line);
        let LinePlace::Col(col) = self.place else {
            return line_break;
        };
        let start = text.line_to_char(self.line.into());
        let content_len = line_break.offset().chars_since(start);
        if content_len == 0 {
            return line_break;
        }
        text.snap(CharOffset::new(
            start.index() + col.index().min(content_len - 1),
        ))
    }
}

/// Resolves positions of the old text against the new one. Its walk through
/// the change is monotone, so it restarts only when a position sits behind
/// the last one asked about.
pub(crate) struct Resolver<'a> {
    change: Option<&'a TextChange<'a>>,
    text: &'a BufferText,
    walk: Option<(PosMapCursor<'a>, CharOffset)>,
}

impl<'a> Resolver<'a> {
    pub(crate) fn new(change: Option<&'a TextChange<'a>>, text: &'a BufferText) -> Self {
        let walk = change.map(|change| {
            (
                PosMapCursor::new(change.changes().ops()),
                CharOffset::default(),
            )
        });
        Self { change, text, walk }
    }

    pub(crate) fn text(&self) -> &'a BufferText {
        self.text
    }

    /// `pos` of the old text in the new one; `pos` itself with no change.
    pub(crate) fn map_old(&mut self, pos: CharOffset, assoc: Assoc) -> CharOffset {
        let (Some(change), Some((cursor, last))) = (self.change, self.walk.as_mut()) else {
            return pos;
        };
        if pos < *last {
            *cursor = PosMapCursor::new(change.changes().ops());
        }
        *last = pos;
        cursor.map(pos, assoc)
    }

    /// `sel` carried through the change, each end on `assoc`'s side of text
    /// inserted at it and landed on the cluster of the new text holding it.
    pub(crate) fn carry(&mut self, sel: Selection, assoc: Assoc) -> Selection {
        let first = self.map_old(sel.first().offset(), assoc);
        let last = self.map_old(sel.last().offset(), assoc);
        sel.with_ends(self.text.snap(first), self.text.snap(last))
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
