//! Reads of a selection set paired with the text it was computed for.

use hume_rope::cluster::{ClusterRange, ClusterStart};
use hume_rope::grapheme::Cluster;
use hume_rope::line::ContentLine;
use hume_rope::offset::InclusiveRange;
use ropey::RopeSlice;

use super::{Facing, Selection, SelectionSet, assert_fits, check_fit};
use crate::error::InvariantViolation;
use crate::text::BufferText;

/// A selection set read against its text. Borrowed from an
/// [`crate::state::EditState`], or bound from a stored set with
/// [`Self::bind`], which checks the set belongs to the text.
pub struct EditView<'a> {
    text: &'a BufferText,
    selections: &'a SelectionSet,
}

impl<'a> EditView<'a> {
    /// `selections` read against `text`.
    ///
    /// # Panics
    /// Panics if the set was tagged for another text.
    pub fn bind(text: &'a BufferText, selections: &'a SelectionSet) -> Self {
        assert_fits(text, selections);
        Self::fitted(text, selections)
    }

    /// A view over a set already known to fit `text`.
    pub(crate) fn fitted(text: &'a BufferText, selections: &'a SelectionSet) -> Self {
        Self { text, selections }
    }

    pub fn text(&self) -> &'a BufferText {
        self.text
    }

    /// The number of selections; at least one.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.selections.selections().len()
    }

    pub fn primary(&self) -> SelectionView<'a> {
        let index = self.selections.primary_pos();
        self.at(index)
    }

    /// The primary selection's position in [`iter`](Self::iter)'s order.
    pub fn primary_index(&self) -> usize {
        self.selections.primary_pos()
    }

    /// Every selection in document order.
    pub fn iter(&self) -> impl Iterator<Item = SelectionView<'a>> + '_ {
        (0..self.len()).map(|index| self.at(index))
    }

    /// Whether every selection is a cursor.
    pub fn all_cursors(&self) -> bool {
        self.selections
            .selections()
            .iter()
            .all(|sel| sel.is_cursor())
    }

    /// Every invariant of the set against the text, the version tag
    /// included.
    pub fn check(&self) -> Result<(), InvariantViolation> {
        check_fit(self.text, self.selections)
    }

    fn at(&self, index: usize) -> SelectionView<'a> {
        SelectionView {
            text: self.text,
            sel: self.selections.selections()[index],
            index,
            primary: index == self.selections.primary_pos(),
        }
    }
}

/// What a selection covers of one line it touches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineSpan {
    pub line: ContentLine,
    /// The covered clusters before the line's `\n`.
    pub content: Option<ClusterRange>,
    /// Whether the line's `\n` is covered.
    pub has_break: bool,
}

/// One selection read against its text: what it covers, where its lines
/// are, its content. Every extent a command needs is a method here, so no
/// command derives one from positions.
#[derive(Clone, Copy, Debug)]
pub struct SelectionView<'a> {
    text: &'a BufferText,
    sel: Selection,
    index: usize,
    primary: bool,
}

impl<'a> SelectionView<'a> {
    pub fn selection(self) -> Selection {
        self.sel
    }

    pub fn text(self) -> &'a BufferText {
        self.text
    }

    pub fn anchor(self) -> ClusterStart {
        self.sel.anchor()
    }

    pub fn head(self) -> ClusterStart {
        self.sel.head()
    }

    /// The first covered cluster.
    pub fn start(self) -> ClusterStart {
        self.sel.start()
    }

    /// The last covered cluster.
    pub fn last(self) -> ClusterStart {
        self.sel.last()
    }

    pub fn facing(self) -> Facing {
        self.sel.facing()
    }

    pub fn is_cursor(self) -> bool {
        self.sel.is_cursor()
    }

    /// This selection's place in document order.
    pub fn index(self) -> usize {
        self.index
    }

    pub fn is_primary(self) -> bool {
        self.primary
    }

    /// Every cluster this selection covers.
    pub fn covered(self) -> ClusterRange {
        ClusterRange::through(self.text.full_slice(), self.sel.start(), self.sel.last())
            .expect("a selection's first cluster is not after its last")
    }

    /// Whether the last covered cluster is a `\n`.
    pub fn ends_on_break(self) -> bool {
        self.text.char_at(self.sel.last().offset()) == Some('\n')
    }

    /// The covered clusters without the `\n` the selection ends on, if it
    /// ends on one: what `c` replaces. `None` when that `\n` is all it covers.
    pub fn content(self) -> Option<ClusterRange> {
        if self.ends_on_break() {
            ClusterRange::between(
                self.text.full_slice(),
                self.sel.start(),
                self.sel.last().into(),
            )
        } else {
            Some(self.covered())
        }
    }

    /// Where text appended after this selection goes: after the last covered
    /// cluster, or before the `\n` the selection ends on, so appending never
    /// crosses a line break.
    pub fn append_point(self) -> ClusterStart {
        if self.ends_on_break() {
            self.sel.last()
        } else {
            crate::grapheme::next_cluster(self.text, self.sel.last())
                .expect("a cluster other than the final `\\n` has a successor")
        }
    }

    /// The lines of the first and last covered clusters.
    pub fn lines(self) -> InclusiveRange<ContentLine> {
        InclusiveRange::new(
            self.text.char_to_line(self.sel.start().offset()),
            self.text.char_to_line(self.sel.last().offset()),
        )
    }

    /// The lines the selection touches, in order, each with what it covers
    /// of them.
    pub fn line_spans(self) -> impl Iterator<Item = LineSpan> + 'a {
        let text = self.text;
        let covered = self.covered();
        let lines = self.lines();
        // Bare-`usize` range, `ContentLine` re-minted each iteration:
        // `ContentLine` has no `Step` impl to range over, and both endpoints
        // are already valid lines.
        (lines.start.index()..=lines.end.index()).map(move |index| {
            let line = ContentLine::new(index);
            let start = crate::lines::line_start(text, line);
            let line_break = crate::lines::line_break(text, line);
            let content = ClusterRange::between(
                text.full_slice(),
                covered.start().max(start),
                covered.end().min(line_break.into()),
            );
            LineSpan {
                line,
                content,
                has_break: covered.start() <= line_break && line_break <= covered.last(),
            }
        })
    }

    pub fn head_line(self) -> ContentLine {
        self.text.char_to_line(self.sel.head().offset())
    }

    /// Whether the first covered cluster starts its line.
    pub fn starts_line(self) -> bool {
        let start = self.sel.start();
        start == crate::lines::line_start(self.text, self.text.char_to_line(start.offset()))
    }

    /// Whether this selection covers whole lines: it starts a line and ends
    /// on a `\n`. A cursor on an empty line counts; see
    /// [`Self::linewise_classification`] for when that matters.
    pub fn is_linewise(self) -> bool {
        self.ends_on_break() && self.starts_line()
    }

    /// Whether the user meant whole lines: `None` for a cursor on an empty
    /// line, which covers one whole line whether or not that was the intent.
    pub fn linewise_classification(self) -> Option<bool> {
        match self.is_linewise() {
            true if self.is_cursor() => None,
            linewise => Some(linewise),
        }
    }

    /// The covered text.
    pub fn slice(self) -> RopeSlice<'a> {
        self.text.slice(self.covered().chars())
    }

    /// The covered clusters, in order.
    pub fn clusters(self) -> impl Iterator<Item = Cluster> + 'a {
        let end = self.covered().end();
        hume_rope::grapheme::graphemes_at(self.text.full_slice(), self.sel.start().into())
            .take_while(move |cluster| cluster.end() <= end)
    }

    /// `sel` read against this view's text, which it must have been computed
    /// for: the next step of a motion that moves a selection several times.
    pub fn with_selection(self, sel: Selection) -> Self {
        debug_assert_eq!(
            super::fit::check_selection(self.text, &sel, self.index()),
            Ok(()),
            "with_selection: a selection that does not fit this text"
        );
        Self { sel, ..self }
    }

    /// This selection widened to also cover `range`, facing `facing`.
    pub fn union(self, range: ClusterRange, facing: Facing) -> Selection {
        Selection::covering(self.covered().hull(range), facing)
    }
}

#[cfg(test)]
mod tests;
