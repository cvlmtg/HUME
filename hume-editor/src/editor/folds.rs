//! A pane's closed folds on one buffer.
//!
//! A closed fold is a line range, fixed when it closes and carried through
//! every later change line by line, so what a pane hides never waits on a
//! parse. A selection end is never on a hidden line unless its selection
//! covers every line the fold hides: [`FoldSet::unhide`] opens any fold that
//! would hide one, and closing moves such an end to the line shown in its
//! place.

use hume_editing::edit::TextChange;
use hume_editing::selection::{EditView, Selection, SelectionSet, SelectionView};
use hume_engine::folds::{Fold, HiddenLines};
use hume_rope::cluster::ClusterStart;
use hume_rope::line::ContentLine;
use hume_rope::offset::InclusiveRange;

#[derive(Clone, Default)]
pub(in crate::editor) struct FoldSet {
    /// Sorted by first line. At most one per first line.
    closed: Vec<Fold>,
    /// The lines `closed` hides.
    hidden: HiddenLines,
}

impl FoldSet {
    #[cfg(test)]
    pub(in crate::editor) fn closed(&self) -> &[Fold] {
        &self.closed
    }

    pub(in crate::editor) fn hidden(&self) -> &HiddenLines {
        &self.hidden
    }

    /// The line a fold command at `line` acts at: the line shown in place
    /// of a hidden line, and the first line of the closed fold whose shown
    /// closing line `line` is.
    pub(in crate::editor) fn fold_line(&self, line: ContentLine) -> ContentLine {
        let line = self
            .closed
            .iter()
            .find(|fold| fold.shows_last() && fold.last() == line)
            .map_or(line, |fold| fold.first());
        self.hidden.shown_at_or_before(line)
    }

    /// Close `folds`, each replacing a closed fold starting on its line.
    pub(in crate::editor) fn close(&mut self, folds: impl IntoIterator<Item = Fold>) {
        let closed = std::mem::take(&mut self.closed)
            .into_iter()
            .chain(folds)
            .collect();
        self.set(closed);
    }

    /// Open the closed folds starting at `lines`.
    pub(in crate::editor) fn open_at(&mut self, lines: &[ContentLine]) {
        let mut closed = std::mem::take(&mut self.closed);
        closed.retain(|c| !lines.contains(&c.first()));
        self.set(closed);
    }

    pub(in crate::editor) fn open_all(&mut self) {
        self.set(Vec::new());
    }

    /// Open every closed fold hiding an end of a selection of `view` that
    /// does not cover every line the fold hides.
    pub(in crate::editor) fn unhide(&mut self, view: EditView<'_>) {
        if self.closed.is_empty() {
            return;
        }
        // A selection's ends lie on its first and last line.
        let hidden_ends: Vec<_> = view
            .iter()
            .flat_map(|sel| {
                let lines = sel.lines();
                [lines.start, lines.end].map(|line| (lines, line))
            })
            .filter(|&(_, line)| self.hidden.is_hidden(line))
            .collect();
        if hidden_ends.is_empty() {
            return;
        }
        let kept: Vec<Fold> = self
            .closed
            .iter()
            .copied()
            .filter(|&fold| {
                !hidden_ends
                    .iter()
                    .any(|&(lines, line)| hides_from(fold, lines, line))
            })
            .collect();
        if kept.len() < self.closed.len() {
            self.set(kept);
        }
    }

    /// `view`'s selections with every end a closed fold hides from them
    /// moved to the indent of the line shown in its place, `None` when no
    /// end moves.
    pub(in crate::editor) fn shown_ends(&self, view: EditView<'_>) -> Option<SelectionSet> {
        let text = view.text();
        let shown = |sel: SelectionView<'_>, end: ClusterStart| {
            let line = text.char_to_line(end.offset());
            let lines = sel.lines();
            let hidden = self.hidden.is_hidden(line)
                && self
                    .closed
                    .iter()
                    .any(|fold| hides_from(*fold, lines, line));
            match hidden {
                true => text
                    .lines()
                    .indent_end(self.hidden.shown_at_or_before(line)),
                false => end,
            }
        };
        let mut moved = false;
        let sels: Vec<Selection> = view
            .iter()
            .map(|sel| {
                let (anchor, head) = (shown(sel, sel.anchor()), shown(sel, sel.head()));
                if anchor == sel.anchor() && head == sel.head() {
                    sel.selection()
                } else {
                    moved = true;
                    Selection::new(anchor, head)
                }
            })
            .collect();
        moved.then(|| SelectionSet::new(text, sels, view.primary().index()))
    }

    /// Carry the closed folds through `change`. A fold whose first or last
    /// line's `\n` the change deletes, or that hides no line after it, opens.
    pub(in crate::editor) fn carry(&mut self, change: &TextChange<'_>) {
        if self.closed.is_empty() {
            return;
        }
        let mut lines: Vec<ContentLine> = self
            .closed
            .iter()
            .flat_map(|fold| [fold.first(), fold.last()])
            .collect();
        lines.sort_unstable();
        lines.dedup();
        let mut cursor = change.line_cursor();
        let mapped: Vec<Option<ContentLine>> =
            lines.iter().map(|&line| cursor.map_line(line)).collect();
        if lines
            .iter()
            .zip(&mapped)
            .all(|(&line, &to)| to == Some(line))
        {
            return;
        }
        let map = |line: ContentLine| {
            mapped[lines
                .binary_search(&line)
                .expect("every fold line is listed")]
        };
        let closed = self
            .closed
            .iter()
            .filter_map(|fold| Fold::new(map(fold.first())?, map(fold.last())?, fold.shows_last()))
            .collect();
        self.set(closed);
    }

    /// Make `closed` the closed folds, a later fold replacing an earlier one
    /// on its first line.
    fn set(&mut self, mut closed: Vec<Fold>) {
        // Reversed so the stable sort puts each first line's latest fold
        // first, the one `dedup_by_key` keeps.
        closed.reverse();
        closed.sort_by_key(|fold| fold.first());
        closed.dedup_by_key(|fold| fold.first());
        self.hidden = HiddenLines::from_folds(closed.iter().copied());
        self.closed = closed;
    }
}

/// Whether closed `fold` hides `line`, an end of a selection on `lines`,
/// from that selection: it hides the line, and `lines` does not cover every
/// line it hides.
fn hides_from(fold: Fold, lines: InclusiveRange<ContentLine>, line: ContentLine) -> bool {
    let hidden = fold.hidden();
    hidden.contains(line) && !(lines.contains(hidden.start) && lines.contains(hidden.end))
}
