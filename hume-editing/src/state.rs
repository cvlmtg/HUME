//! A text paired with the selections computed for it.

use hume_rope::cluster::ClusterStart;

use crate::selection::{
    EditView, RecordedSelections, Selection, SelectionSet, SelectionView, assert_fits,
    check_positions,
};
use crate::text::BufferText;

/// A text and a selection set that fits it: the input and output of every
/// motion and selection command. Its selections are read through
/// [`Self::view`], so a read cannot be made against another text.
#[derive(Debug, Clone)]
pub struct EditState {
    text: BufferText,
    selections: SelectionSet,
}

impl EditState {
    /// `selections` paired with `text`.
    ///
    /// # Panics
    /// Panics if the set was tagged for another text.
    pub fn bind(text: &BufferText, selections: SelectionSet) -> Self {
        assert_fits(text, &selections);
        Self {
            text: text.clone(),
            selections,
        }
    }

    /// `text` with one cursor on the cluster starting at `at`.
    pub fn with_cursor(text: BufferText, at: ClusterStart) -> Self {
        Self::from_text(text, vec![Selection::cursor(at)], 0)
    }

    /// `text` with one cursor on its first cluster.
    pub fn at_text_start(text: BufferText) -> Self {
        let first =
            hume_rope::grapheme::first_cluster(text.full_slice()).expect("a buffer is never empty");
        Self::with_cursor(text, first)
    }

    /// `text` with one selection per `(anchor, head)` pair of cluster
    /// indices, each wrapped into the text's cluster count, for generators
    /// that pick selections without knowing the text's clusters.
    ///
    /// # Panics
    /// Panics if `picks` is empty or `primary` is out of range.
    #[cfg(any(test, feature = "test-util"))]
    pub fn from_cluster_indices(
        text: BufferText,
        picks: &[(usize, usize)],
        primary: usize,
    ) -> Self {
        let starts: Vec<ClusterStart> = std::iter::successors(
            hume_rope::grapheme::first_cluster(text.full_slice()),
            |&s| hume_rope::grapheme::next_cluster(text.full_slice(), s),
        )
        .collect();
        let selections = picks
            .iter()
            .map(|&(anchor, head)| {
                Selection::new(starts[anchor % starts.len()], starts[head % starts.len()])
            })
            .collect();
        Self::from_text(text, selections, primary)
    }

    /// `text` with a set computed for it.
    pub(crate) fn from_set(text: BufferText, selections: SelectionSet) -> Self {
        Self::checked(text, selections)
    }

    /// `text` with `selections`, which must have been computed for it.
    pub(crate) fn from_text(text: BufferText, selections: Vec<Selection>, primary: usize) -> Self {
        let selections = SelectionSet::from_parts(selections, primary, text.version());
        Self::checked(text, selections)
    }

    pub fn view(&self) -> EditView<'_> {
        EditView::fitted(&self.text, &self.selections)
    }

    pub fn text(&self) -> &BufferText {
        &self.text
    }

    /// Each selection replaced by `f`'s result for it.
    #[must_use]
    pub fn map(self, mut f: impl FnMut(SelectionView<'_>) -> Selection) -> Self {
        let primary = self.selections.primary_pos();
        let selections = self.view().iter().map(&mut f).collect();
        self.replaced(selections, primary)
    }

    /// Each selection replaced by the selections `f` returns for it. A
    /// selection `f` returns nothing for is dropped. The primary becomes the
    /// first selection produced for the primary, or the first selection
    /// overall when nothing was produced for it.
    ///
    /// # Panics
    /// Panics if `f` returns nothing for every selection.
    #[must_use]
    pub fn flat_map<I: IntoIterator<Item = Selection>>(
        self,
        mut f: impl FnMut(SelectionView<'_>) -> I,
    ) -> Self {
        let mut selections = Vec::new();
        let mut primary = None;
        for view in self.view().iter() {
            let before = selections.len();
            selections.extend(f(view));
            if view.is_primary() && selections.len() > before {
                primary = Some(before);
            }
        }
        self.replaced(selections, primary.unwrap_or(0))
    }

    /// The same text with `selections`, which must have been computed for
    /// it, sorted and with overlapping ones merged.
    ///
    /// # Panics
    /// Panics if `selections` is empty or `primary` is out of range.
    #[must_use]
    pub fn with_selections(self, selections: Vec<Selection>, primary: usize) -> Self {
        self.replaced(selections, primary)
    }

    /// The primary selection replaced by `sel`.
    #[must_use]
    pub fn replace_primary(self, sel: Selection) -> Self {
        let (mut selections, primary) = self.selections.clone().into_parts();
        selections[primary] = sel;
        self.replaced(selections, primary)
    }

    /// Only the primary selection.
    #[must_use]
    pub fn keep_primary(self) -> Self {
        let primary = self.selections.primary_selection();
        self.replaced(vec![primary], 0)
    }

    /// The selection at `idx` removed, unless it is the only one. Removing the
    /// primary makes the next selection primary, wrapping to the first.
    ///
    /// # Panics
    /// Panics if `idx` is out of range.
    #[must_use]
    pub fn remove(self, idx: usize) -> Self {
        let (mut selections, primary) = self.selections.clone().into_parts();
        assert!(idx < selections.len(), "remove index out of bounds");
        if selections.len() == 1 {
            return self;
        }
        selections.remove(idx);
        let primary = if idx < primary {
            primary - 1
        } else if idx == primary {
            idx % selections.len()
        } else {
            primary
        };
        self.replaced(selections, primary)
    }

    /// The primary moved `delta` selections along document order, wrapping.
    #[must_use]
    pub fn cycle_primary(self, delta: isize) -> Self {
        let (selections, primary) = self.selections.clone().into_parts();
        let len = selections.len() as isize;
        let primary = (primary as isize + delta).rem_euclid(len) as usize;
        self.replaced(selections, primary)
    }

    /// The selections, to store detached from the text. Reading them again
    /// needs this text: see [`EditView::bind`].
    pub fn into_selections(self) -> SelectionSet {
        self.selections
    }

    /// The selections as they stand, sticky columns included, to record and
    /// bind again to a text with this content (an undo step restores them).
    pub fn recorded(&self) -> RecordedSelections {
        let view = self.view();
        let selections = view.iter().map(|v| v.selection()).collect();
        RecordedSelections::new(selections, view.primary().index())
    }

    fn replaced(self, selections: Vec<Selection>, primary: usize) -> Self {
        let selections = SelectionSet::from_parts(selections, primary, self.text.version());
        Self::checked(self.text, selections)
    }

    fn checked(text: BufferText, selections: SelectionSet) -> Self {
        debug_assert_eq!(
            check_positions(&text, &selections),
            Ok(()),
            "EditState: selections do not fit the text they were paired with"
        );
        Self { text, selections }
    }
}

#[cfg(test)]
mod tests;
