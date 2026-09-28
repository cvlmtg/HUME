use crate::changeset::ChangeSet;
use crate::error::TransactionError;
use crate::selection::SelectionSet;
use crate::text::BufferText;

/// A text change bundled with the resulting selection state: the unit of
/// editing and of undo. `selection` is always the post-apply selection, for
/// forward and inverse transactions alike.
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
    /// Create a transaction from a changeset and the resulting selection.
    pub fn new(changes: ChangeSet, selection: SelectionSet) -> Self {
        Self { changes, selection }
    }

    /// Apply this transaction to a buffer, returning the new buffer and the
    /// new selection state.
    ///
    /// Takes `text` by reference so the original buffer remains available to
    /// the caller on the error path, with no undo needed. On success the caller
    /// should drop the old buffer (or push an inverse transaction to the undo
    /// stack before doing so).
    ///
    /// This is the trust boundary for plugin-constructed transactions. Internal
    /// named commands build changesets by construction and call
    /// [`ChangeSet::apply`] directly. A plugin assembling a [`Transaction`]
    /// manually goes through here and gets a clear error instead of silent
    /// corruption or a crash.
    ///
    /// # Errors
    /// - [`TransactionError::Apply`] if the changeset is invalid for `text`
    ///   (length mismatch or deleted the structural trailing `\n`).
    /// - [`TransactionError::Validation`] if any selection head or anchor is
    ///   out of bounds for the post-apply buffer.
    pub fn apply(&self, text: &BufferText) -> Result<(BufferText, SelectionSet), TransactionError> {
        let new_text = self.changes.apply(text)?;
        self.selection.validate(new_text.len_chars())?;
        // Canonicalize before handing the set to the editor: a plugin-built
        // Transaction can carry unsorted or overlapping selections, which
        // downstream code only debug-asserts against. Identity on sets that
        // are already canonical (every internally-built one), so undo/redo
        // round-trips are unaffected.
        let mut sels = self.selection.clone();
        sels.merge_overlapping_in_place();
        Ok((new_text, sels))
    }

    /// The selection state recorded in this transaction.
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
