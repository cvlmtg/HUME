//! Edits: building one from operations on the text it changes, the finished
//! [`Edited`], and carrying positions through a text change.

mod builder;

pub use builder::{EditBuilder, Landing, Landings, Mark, NewPos, edit};

use std::cell::OnceCell;

use hume_rope::offset::{CharOffset, ExclusiveRange};

use crate::changeset::{Assoc, ChangeSet};
use crate::error::ApplyError;
use crate::selection::{Resolver, SelectionSet};
use crate::state::EditState;
use crate::text::{BufferText, TextVersion};

/// `text` changed by the changeset `drive` builds for it, with the changeset
/// and whatever else `drive` returned.
///
/// The result ends with a `\n`. `drive` is called with no ceiling first. When
/// that result would not end with a `\n`, `drive` is called again with the
/// structural `\n`'s position as the ceiling, and must then follow one rule:
/// text inserted at the text end gets a `\n` of its own, and when none is
/// inserted there, every deletion stops at the ceiling.
///
/// # Panics
/// Panics if a changeset `drive` builds is for another text, or if the one
/// built under the ceiling still drops the final `\n`.
pub fn apply_keeping_final_break<T>(
    text: &BufferText,
    mut drive: impl FnMut(Option<CharOffset>) -> (ChangeSet, T),
) -> (BufferText, ChangeSet, T) {
    let (changes, out) = drive(None);
    match changes.apply(text) {
        Ok(new) => (new, changes, out),
        Err(ApplyError::TrailingNewlineMissing) => {
            let (changes, out) = drive(Some(text.last_char()));
            let new = changes
                .apply(text)
                .expect("a changeset built under the ceiling keeps the final newline");
            (new, changes, out)
        }
        Err(e) => panic!("apply_keeping_final_break: {e}"),
    }
}

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
        let before = state.text().clone();
        let base = before.version();
        let mut selections = state.into_selections();
        selections.translate(&TextChange::new(&before, &text, &changes));
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
    edited_old_ranges: OnceCell<Vec<ExclusiveRange<CharOffset>>>,
}

impl<'a> TextChange<'a> {
    /// # Panics
    /// Panics if `changes` does not map `before`'s length to `after`'s.
    pub fn new(before: &'a BufferText, after: &'a BufferText, changes: &'a ChangeSet) -> Self {
        assert_eq!(
            changes.len_before(),
            before.len_chars(),
            "TextChange: the changes do not start from `before`"
        );
        assert_eq!(
            changes.len_after(),
            after.len_chars(),
            "TextChange: the changes do not lead to `after`"
        );
        Self {
            before,
            after,
            changes,
            edited_old_ranges: OnceCell::new(),
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

    /// The changeset's edited old ranges, computed on first use and shared by
    /// every set translated through this change.
    fn edited_old_ranges(&self) -> &[ExclusiveRange<CharOffset>] {
        self.edited_old_ranges
            .get_or_init(|| self.changes.edited_old_ranges())
    }
}

impl SelectionSet {
    /// This set carried through `change`. Each end maps past text inserted
    /// at it and lands on the cluster of the new text holding it; selections
    /// the change folds together merge. A sticky column survives only when
    /// the change left the head's line alone.
    ///
    /// # Panics
    /// Panics if this set was not computed for `change`'s old text: carrying
    /// it would land its positions wherever the change happens to map them.
    pub fn translate(&mut self, change: &TextChange<'_>) {
        crate::selection::assert_fits(change.before(), self);
        let untouched = self.heads_untouched(change.edited_old_ranges(), change.before());
        let mut resolver = Resolver::new(change, change.after());
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
