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

    /// Close `folds`, each replacing a closed fold starting on its line.
    pub(in crate::editor) fn close(&mut self, folds: impl IntoIterator<Item = Fold>) {
        let mut closed = std::mem::take(&mut self.closed);
        for fold in folds {
            closed.retain(|c| c.first() != fold.first());
            closed.push(fold);
        }
        self.set(closed);
    }

    /// Open the closed fold starting at `line`, if there is one.
    pub(in crate::editor) fn open_at(&mut self, line: ContentLine) {
        let mut closed = std::mem::take(&mut self.closed);
        closed.retain(|c| c.first() != line);
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
        let mut open = vec![false; self.closed.len()];
        for sel in view.iter() {
            for line in end_lines(sel) {
                for (i, fold) in self.closed.iter().enumerate() {
                    open[i] |= hides_from(*fold, sel, line);
                }
            }
        }
        if open.contains(&true) {
            let mut open = open.into_iter();
            let closed = std::mem::take(&mut self.closed)
                .into_iter()
                .filter(|_| !open.next().expect("one flag per fold"))
                .collect();
            self.set(closed);
        }
    }

    /// `view`'s selections with every end a closed fold hides from them
    /// moved to the indent of the line shown in its place, `None` when no
    /// end moves.
    pub(in crate::editor) fn shown_ends(&self, view: EditView<'_>) -> Option<SelectionSet> {
        let text = view.text();
        let shown = |sel: SelectionView<'_>, end: ClusterStart| {
            let line = text.char_to_line(end.offset());
            match self.closed.iter().any(|fold| hides_from(*fold, sel, line)) {
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

    fn set(&mut self, mut closed: Vec<Fold>) {
        closed.sort_unstable_by_key(|fold| fold.first());
        closed.dedup_by_key(|fold| fold.first());
        self.hidden = HiddenLines::from_folds(closed.iter().copied());
        self.closed = closed;
    }
}

/// The lines `sel`'s anchor and head are on.
fn end_lines(sel: SelectionView<'_>) -> [ContentLine; 2] {
    let text = sel.text();
    [sel.anchor(), sel.head()].map(|end| text.char_to_line(end.offset()))
}

/// Whether closed `fold` hides `line`, an end of `sel`, from `sel`: it hides
/// the line, and `sel` does not cover every line it hides.
fn hides_from(fold: Fold, sel: SelectionView<'_>, line: ContentLine) -> bool {
    let hidden = fold.hidden();
    let lines = sel.lines();
    hidden.contains(line) && !(lines.contains(hidden.start) && lines.contains(hidden.end))
}
