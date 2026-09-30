//! Selections recorded for one text, to be bound again to a text with the
//! same content (an undo step restores them).

use super::fit::{check_positions, refit_parts};
use super::{Selection, SelectionSet};
use crate::error::InvariantViolation;
use crate::state::EditState;
use crate::text::BufferText;

/// The selections of a text as it stood: every end a position of that text,
/// sticky columns included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedSelections {
    selections: Vec<Selection>,
    primary: usize,
}

impl RecordedSelections {
    /// # Panics
    /// Panics if `selections` is empty or `primary` is out of range.
    pub fn new(selections: Vec<Selection>, primary: usize) -> Self {
        assert!(
            !selections.is_empty(),
            "recorded selections must not be empty"
        );
        assert!(primary < selections.len(), "primary index out of bounds");
        Self {
            selections,
            primary,
        }
    }

    /// These selections against `text`, which has the content they were
    /// recorded against.
    ///
    /// # Errors
    /// Returns the invariant a position breaks when `text` is not such a text.
    pub fn bind(self, text: BufferText) -> Result<EditState, InvariantViolation> {
        let set = SelectionSet::from_parts(self.selections, self.primary, text.version());
        check_positions(&text, &set)?;
        Ok(EditState::from_set(text, set))
    }

    /// These selections fitted to `text`, which they were not recorded
    /// against: each end clamped into it and floored to the start of the
    /// cluster holding it.
    pub fn refit(self, text: BufferText) -> EditState {
        let set = refit_parts(&text, &self.selections, self.primary);
        EditState::from_set(text, set)
    }
}

#[cfg(test)]
mod tests;
