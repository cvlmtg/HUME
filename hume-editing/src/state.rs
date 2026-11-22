//! A text paired with the selections computed for it.

use hume_rope::cluster::ClusterStart;

use crate::selection::{
    EditView, Selection, SelectionSet, SelectionView, assert_fits, assert_positions,
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

/// What one selection became when a command searched from it.
#[derive(Debug, Clone, Copy)]
pub enum Sought {
    /// The search found its target.
    Found(Selection),
    /// The search found nothing.
    Missed(Selection),
}

impl Sought {
    /// What the selection became, found or not.
    pub fn selection(self) -> Selection {
        match self {
            Self::Found(sel) | Self::Missed(sel) => sel,
        }
    }
}

impl EditState {
    /// `selections` paired with `text`.
    ///
    /// # Panics
    /// Panics if the set was tagged for another text.
    pub fn bind(text: BufferText, selections: SelectionSet) -> Self {
        assert_fits(&text, &selections);
        Self { text, selections }
    }

    /// `text` with one cursor on the cluster starting at `at`.
    pub fn with_cursor(text: BufferText, at: ClusterStart) -> Self {
        let selections = SelectionSet::cursor(&text, at);
        Self { text, selections }
    }

    /// `text` with one cursor on its first cluster.
    pub fn at_text_start(text: BufferText) -> Self {
        let selections = SelectionSet::at_text_start(&text);
        Self { text, selections }
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
        let starts: Vec<ClusterStart> =
            std::iter::successors(Some(text.clusters().first()), |&s| text.clusters().next(s))
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
        assert_positions(&text, &selections);
        Self { text, selections }
    }

    /// `text` with `selections`, which must have been computed for it, as
    /// [`SelectionSet::new`] makes them a set.
    pub(crate) fn from_text(text: BufferText, selections: Vec<Selection>, primary: usize) -> Self {
        let selections = SelectionSet::new(&text, selections, primary);
        Self { text, selections }
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
        self.with_selections(selections, primary)
    }

    /// Each selection replaced by `f`'s result for it, for a command that
    /// searches from each selection. `None` when no selection found its
    /// target, so a command that found nothing changes nothing.
    #[must_use]
    pub fn map_sought(self, mut f: impl FnMut(SelectionView<'_>) -> Sought) -> Option<Self> {
        let primary = self.selections.primary_pos();
        let mut found = false;
        let selections = self
            .view()
            .iter()
            .map(|view| match f(view) {
                Sought::Found(sel) => {
                    found = true;
                    sel
                }
                Sought::Missed(sel) => sel,
            })
            .collect();
        found.then(|| self.with_selections(selections, primary))
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
        self.with_selections(selections, primary.unwrap_or(0))
    }

    /// The same text with `selections`, which must have been computed for
    /// it, sorted and with overlapping ones merged.
    ///
    /// # Panics
    /// Panics if `selections` is empty or `primary` is out of range.
    #[must_use]
    pub fn with_selections(self, selections: Vec<Selection>, primary: usize) -> Self {
        Self::from_text(self.text, selections, primary)
    }

    /// The primary selection replaced by `sel`.
    #[must_use]
    pub fn replace_primary(self, sel: Selection) -> Self {
        self.edit_parts(|selections, primary| selections[*primary] = sel)
    }

    /// Only the primary selection.
    #[must_use]
    pub fn keep_primary(self) -> Self {
        let primary = self.selections.primary_selection();
        self.with_selections(vec![primary], 0)
    }

    /// `count` selections removed, starting at the primary and going on in
    /// document order, wrapping, but never the last one left. The selection
    /// after the removed run becomes primary.
    #[must_use]
    pub fn remove_primary(self, count: usize) -> Self {
        let len = self.selections.selections().len();
        let removed = count.min(len - 1);
        if removed == 0 {
            return self;
        }
        self.edit_parts(|selections, primary| {
            let end = *primary + removed;
            selections.drain(*primary..end.min(len));
            // A run past the last selection goes on from the first.
            selections.drain(..end.saturating_sub(len));
            if end >= len {
                *primary = 0;
            }
        })
    }

    /// The primary moved `count` selections forward in document order,
    /// wrapping.
    #[must_use]
    pub fn cycle_primary_forward(self, count: usize) -> Self {
        self.edit_parts(|selections, primary| {
            let len = selections.len();
            *primary = (*primary + count % len) % len;
        })
    }

    /// The primary moved `count` selections back in document order,
    /// wrapping.
    #[must_use]
    pub fn cycle_primary_backward(self, count: usize) -> Self {
        self.edit_parts(|selections, primary| {
            let len = selections.len();
            *primary = (*primary + len - count % len) % len;
        })
    }

    /// `f` applied to the selections and the primary index, then the result
    /// re-fitted to the same text.
    fn edit_parts(self, f: impl FnOnce(&mut Vec<Selection>, &mut usize)) -> Self {
        let Self { text, selections } = self;
        let (mut selections, mut primary) = selections.into_parts();
        f(&mut selections, &mut primary);
        Self::from_text(text, selections, primary)
    }

    /// The selections, to store detached from the text. Reading them again
    /// needs this text: see [`EditView::bind`].
    pub fn into_selections(self) -> SelectionSet {
        self.selections
    }

    /// The text and the selections, for a caller that keeps both.
    pub fn into_parts(self) -> (BufferText, SelectionSet) {
        (self.text, self.selections)
    }
}

#[cfg(test)]
mod tests;
