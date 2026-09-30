use crate::changeset::ChangeSet;
use crate::error::ApplyError;
use crate::selection::SelectionSet;
use crate::state::EditState;
use crate::text::BufferText;

/// A text change bundled with the resulting selections: the unit of editing
/// and of undo. `selection` is always the post-apply selection, for forward
/// and inverse transactions alike, tagged with the version of the text the
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
    selection: SelectionSet,
}

impl Transaction {
    /// `changes`, which reproduce the content of the text `selection` is
    /// tagged for, with that selection.
    pub(crate) fn new(changes: ChangeSet, selection: SelectionSet) -> Self {
        Self { changes, selection }
    }

    /// `txns`, each applying to the text the one before it produces, as one
    /// transaction landing on the last one's selections. `None` when `txns`
    /// is empty.
    pub fn compose_all(txns: Vec<Transaction>) -> Option<Transaction> {
        let selection = txns.last()?.selection.clone();
        let changes = ChangeSet::compose_all(txns.into_iter().map(|txn| txn.changes))?;
        Some(Self::new(changes, selection))
    }

    /// Apply this transaction to `text`: the text it produces, carrying the
    /// version the selections were recorded for, paired with them.
    ///
    /// Takes `text` by reference so the original buffer remains available to
    /// the caller on the error path, with no undo needed.
    ///
    /// # Errors
    /// [`ApplyError`] if the changeset is invalid for `text` (length
    /// mismatch, or it deletes the structural trailing `\n`).
    pub fn apply(&self, text: &BufferText) -> Result<EditState, ApplyError> {
        let new_text = self.changes.apply_as(text, self.selection.version())?;
        Ok(EditState::bind(&new_text, self.selection.clone()))
    }

    /// The selections this transaction lands on.
    pub fn selection(&self) -> &SelectionSet {
        &self.selection
    }

    /// Consume this transaction and return just the `ChangeSet`.
    pub fn into_changes(self) -> ChangeSet {
        self.changes
    }
}

#[cfg(test)]
mod tests;
