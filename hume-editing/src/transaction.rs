use crate::changeset::ChangeSet;
use crate::error::TransactionError;
use crate::selection::RecordedSelections;
use crate::state::EditState;
use crate::text::BufferText;

/// A text change bundled with the resulting selections: the unit of editing
/// and of undo. `selection` is always the post-apply selection, for forward
/// and inverse transactions alike, recorded as positions of the text the
/// transaction produces.
///
/// For undo, build two transactions from one `ChangeSet`: `forward` from
/// `cs` with the post-edit selections, and `inverse` from
/// `cs.invert(&old_text)` (which needs the original text to recover deleted
/// chars) with the pre-edit selections, so applying `inverse` restores text
/// and cursors in one step.
#[derive(Debug, Clone)]
pub struct Transaction {
    changes: ChangeSet,
    selection: RecordedSelections,
}

impl Transaction {
    /// Create a transaction from a changeset and the resulting selection.
    pub fn new(changes: ChangeSet, selection: RecordedSelections) -> Self {
        Self { changes, selection }
    }

    /// Apply this transaction to a buffer, returning the new text paired with
    /// the recorded selections.
    ///
    /// Takes `text` by reference so the original buffer remains available to
    /// the caller on the error path, with no undo needed.
    ///
    /// # Errors
    /// - [`TransactionError::Apply`] if the changeset is invalid for `text`
    ///   (length mismatch or deleted the structural trailing `\n`).
    /// - [`TransactionError::Selections`] if the recorded selections do not
    ///   fit the text the changes produce.
    pub fn apply(&self, text: &BufferText) -> Result<EditState, TransactionError> {
        let new_text = self.changes.apply(text)?;
        self.selection
            .clone()
            .bind(new_text)
            .map_err(TransactionError::Selections)
    }

    /// The selection state recorded in this transaction.
    pub fn selection(&self) -> &RecordedSelections {
        &self.selection
    }

    /// Consume this transaction and return just the `ChangeSet`.
    pub fn into_changes(self) -> ChangeSet {
        self.changes
    }
}

#[cfg(test)]
mod tests;
