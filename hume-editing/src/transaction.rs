use crate::changeset::ChangeSet;
use crate::error::TransactionError;
use crate::selection::SelectionSet;
use crate::text::BufferText;

/// A `Transaction` bundles a text change with the resulting selection state.
///
/// This is the unit of editing: every user action (insert, delete, motion
/// that modifies text) produces a `Transaction`. `selection` is always the
/// **post-apply** selection — where cursors land *after* applying `changes`
/// — for both forward and inverse Transactions.
///
/// ## Undo pattern
///
/// Build **two** Transactions from the same `ChangeSet`, inverting before
/// applying since `invert` reads the original rope to reconstruct deleted
/// text:
///
/// ```text
/// let inv_cs = cs.invert(&old_buf);          // must happen BEFORE apply
/// let new_buf = cs.apply(&old_buf);          // borrows old_buf; original intact
///
/// let forward = Transaction::new(cs,     post_edit_sels);  // for redo
/// let inverse = Transaction::new(inv_cs, pre_edit_sels);   // push to undo stack
/// ```
///
/// The inverse's `selection` is the pre-edit selection, since that's where
/// cursors land after applying it — the history manager stores `inverse`
/// and applying it later restores text and cursor state in one step.
///
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
    /// the caller on the error path — no undo needed. On success the caller
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
        let new_buf = self.changes.apply(text)?;
        self.selection.validate(new_buf.len_chars())?;
        // Canonicalize before handing the set to the editor: a plugin-built
        // Transaction can carry unsorted or overlapping selections, which
        // downstream code only debug-asserts against. Identity on sets that
        // are already canonical (every internally-built one), so undo/redo
        // round-trips are unaffected.
        let mut sels = self.selection.clone();
        sels.merge_overlapping_in_place();
        Ok((new_buf, sels))
    }

    /// The selection state recorded in this transaction.
    pub fn selection(&self) -> &SelectionSet {
        &self.selection
    }

    /// Consume this transaction and return just the `ChangeSet`.
    ///
    /// Used by `Buffer::undo` / `Buffer::redo` to extract the CS for
    /// propagation to non-acting panes after `apply` has already validated and
    /// applied the transaction.
    pub fn into_changes(self) -> ChangeSet {
        self.changes
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
