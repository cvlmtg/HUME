//! Edits: building one from operations on the text it changes, the finished
//! [`Edited`], and carrying positions through a text change.

mod builder;

pub use builder::{EditBuilder, Landing, Landings, Mark, NewPos, Removed, edit};

use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::changeset::{Assoc, ChangeSet};
use crate::error::ApplyError;
use crate::selection::{Resolver, SelectionSet};
use crate::state::EditState;
use crate::text::{BufferText, TextVersion};

/// A finished edit: the new text with its selections, and the changeset that
/// produced it from the text it started from.
#[derive(Debug, Clone)]
pub struct Edited {
    state: EditState,
    changes: ChangeSet,
    base: TextVersion,
}

impl Edited {
    /// No change to `state`.
    pub fn unchanged(state: EditState) -> Self {
        let changes = ChangeSet::identity(state.text().len_chars());
        let base = state.text().version();
        Self {
            state,
            changes,
            base,
        }
    }

    /// `changes` applied to `state`'s text, its selections carried through.
    pub fn from_changes(state: EditState, changes: ChangeSet) -> Result<Self, ApplyError> {
        let text = changes.apply(state.text())?;
        let base = state.text().version();
        let primary = state.view().primary().index();
        let selections = {
            let change = TextChange::new(state.text(), &text, &changes);
            let mut resolver = Resolver::new(Some(&change), &text);
            let carried = state
                .view()
                .iter()
                .map(|v| resolver.carry(v.selection().without_sticky(), Assoc::After))
                .collect();
            SelectionSet::from_parts(carried, primary, text.version())
        };
        Ok(Self {
            state: EditState::from_set(text, selections),
            changes,
            base,
        })
    }

    pub fn state(&self) -> &EditState {
        &self.state
    }

    pub fn changes(&self) -> &ChangeSet {
        &self.changes
    }

    /// The version of the text the edit started from.
    pub fn base(&self) -> TextVersion {
        self.base
    }

    pub fn into_parts(self) -> (EditState, ChangeSet) {
        (self.state, self.changes)
    }

    /// This edit followed by `f`'s edit of its result, as one edit.
    ///
    /// # Panics
    /// Panics if `f`'s edit is not of this edit's result.
    pub fn then(self, f: impl FnOnce(EditState) -> Edited) -> Edited {
        let result = self.state.text().version();
        let next = f(self.state);
        assert_eq!(
            next.base, result,
            "then: the next edit is of another text than this edit's result"
        );
        Edited {
            changes: self.changes.compose(next.changes),
            state: next.state,
            base: self.base,
        }
    }
}

/// A change from one text to another, for carrying positions of the first
/// into the second.
pub struct TextChange<'a> {
    before: &'a BufferText,
    after: &'a BufferText,
    changes: &'a ChangeSet,
}

impl<'a> TextChange<'a> {
    /// # Panics
    /// Panics if `changes` does not map `before`'s length to `after`'s.
    pub fn new(before: &'a BufferText, after: &'a BufferText, changes: &'a ChangeSet) -> Self {
        assert_eq!(changes.len_before(), before.len_chars());
        assert_eq!(changes.len_after(), after.len_chars());
        Self {
            before,
            after,
            changes,
        }
    }

    pub fn before(&self) -> &'a BufferText {
        self.before
    }

    pub fn after(&self) -> &'a BufferText {
        self.after
    }

    pub fn changes(&self) -> &'a ChangeSet {
        self.changes
    }
}

impl SelectionSet {
    /// This set carried through `change`. Each end maps past text inserted
    /// at it and lands on the cluster of the new text holding it; selections
    /// the change folds together merge. A sticky column survives only when
    /// the change left the head's line alone.
    pub fn translate(&mut self, change: &TextChange<'_>) {
        self.translate_with(&change.changes().edited_old_ranges(), change);
    }

    /// [`Self::translate`] with `change`'s edited old ranges computed by the
    /// caller, once for many sets.
    ///
    /// # Panics
    /// Panics if this set was not computed for `change`'s old text: carrying
    /// it would land its positions wherever the change happens to map them.
    pub(crate) fn translate_with(
        &mut self,
        edits: &[ExclusiveRange<CharOffset>],
        change: &TextChange<'_>,
    ) {
        assert_eq!(
            self.version(),
            change.before().version(),
            "translate: a selection set carried through a change to another text"
        );
        let untouched = self.heads_untouched(edits, change.before());
        let mut resolver = Resolver::new(Some(change), change.after());
        let carried = self
            .selections()
            .iter()
            .zip(untouched)
            .map(|(sel, keep_sticky)| {
                let sel = if keep_sticky {
                    *sel
                } else {
                    sel.without_sticky()
                };
                resolver.carry(sel, Assoc::After)
            })
            .collect();
        *self = SelectionSet::from_parts(carried, self.primary_pos(), change.after().version());
    }
}

#[cfg(test)]
mod tests;
