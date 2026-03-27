use crate::buffer::Buffer;
use crate::changeset::ChangeSet;
use crate::error::TransactionError;
use crate::selection::SelectionSet;

/// A `Transaction` bundles a text change with the resulting selection state.
///
/// This is the unit of editing: every user action (insert, delete, motion
/// that modifies text) produces a `Transaction`. `selection` is always the
/// **post-apply** selection — where the cursors land *after* applying
/// `changes` to the document. This invariant holds for both forward and
/// inverse Transactions.
///
/// ## Undo pattern
///
/// At edit time, build **two** Transactions from the same `ChangeSet`:
///
/// ```text
/// let inv_cs = cs.invert(&old_buf);          // must happen BEFORE apply
/// let new_buf = cs.apply(&old_buf);          // borrows old_buf; original intact
///
/// let forward = Transaction::new(cs,     post_edit_sels);  // for redo
/// let inverse = Transaction::new(inv_cs, pre_edit_sels);   // push to undo stack
/// ```
///
/// The inverse Transaction's `selection` is the pre-edit selection because
/// that is where cursors land after applying the inverse changeset. The
/// history manager stores `inverse`; applying it later restores both text
/// and cursor state in one step.
///
/// **Timing constraint:** `invert(&old_buf)` must be called *before*
/// discarding `old_buf` — `invert` reads the original rope to reconstruct
/// deleted text, and that content is gone once you move on to the new buffer.
///
/// ## Separation of concerns
///
/// Separating `ChangeSet` (pure text transform) from `Transaction` (text +
/// selections) keeps the changeset algebra clean: `compose`, `invert`, and
/// `map_pos` are document-level operations that don't need to know about
/// cursors. The `Transaction` adds cursor semantics on top.
#[derive(Debug, Clone)]
pub(crate) struct Transaction {
    changes: ChangeSet,
    selection: SelectionSet,
}

impl Transaction {
    /// Create a transaction from a changeset and the resulting selection.
    pub(crate) fn new(changes: ChangeSet, selection: SelectionSet) -> Self {
        Self { changes, selection }
    }

    /// Apply this transaction to a buffer, returning the new buffer and the
    /// new selection state.
    ///
    /// Takes `buf` by reference so the original buffer remains available to
    /// the caller on the error path — no undo needed. On success the caller
    /// should drop the old buffer (or push an inverse transaction to the undo
    /// stack before doing so).
    ///
    /// This is the trust boundary for plugin-constructed transactions. Named
    /// commands in `edit.rs` build changesets by construction and call
    /// [`ChangeSet::apply`] directly, bypassing this method. A plugin
    /// assembling a [`Transaction`] manually goes through here and gets a
    /// clear error instead of silent corruption or a crash.
    ///
    /// # Errors
    /// - [`TransactionError::Apply`] if the changeset is invalid for `buf`
    ///   (length mismatch or deleted the structural trailing `\n`).
    /// - [`TransactionError::Validation`] if any selection head or anchor is
    ///   out of bounds for the post-apply buffer.
    /// The selection state after this transaction.
    #[cfg(test)]
    pub(crate) fn selection(&self) -> &SelectionSet {
        &self.selection
    }

    pub(crate) fn apply(&self, buf: &Buffer) -> Result<(Buffer, SelectionSet), TransactionError> {
        let new_buf = self.changes.apply(buf)?;
        self.selection.validate(new_buf.len_chars())?;
        Ok((new_buf, self.selection.clone()))
    }

}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::ChangeSetBuilder;
    use crate::error::{ApplyError, ValidationError};
    use crate::selection::Selection;
    use pretty_assertions::assert_eq;

    #[test]
    fn transaction_apply() {
        // "hello\n" = 6 chars; insert "!" at start → "!hello\n".
        let buf = Buffer::from("hello");
        let mut b = ChangeSetBuilder::new(6);
        b.insert("!");
        b.retain_rest();
        let cs = b.finish();

        let sels = SelectionSet::single(Selection::cursor(1));
        let txn = Transaction::new(cs, sels.clone());

        let (new_buf, new_sels) = txn.apply(&buf).unwrap();
        assert_eq!(new_buf.to_string(), "!hello\n");
        assert_eq!(new_sels.primary().head, 1);
    }

    #[test]
    fn transaction_apply_rejects_out_of_bounds_selection() {
        // "hi\n" = 3 chars; a no-op changeset; but selection points to index 99.
        let buf = Buffer::from("hi");
        let mut b = ChangeSetBuilder::new(3);
        b.retain_rest();
        let cs = b.finish();

        // Cursor at 99 is way past buf_len (3) — this is what a buggy plugin
        // might produce.
        let sels = SelectionSet::single(Selection::cursor(99));
        let txn = Transaction::new(cs, sels);

        let err = txn.apply(&buf).unwrap_err();
        assert!(
            matches!(
                err,
                TransactionError::Validation(
                    ValidationError::SelectionOutOfBounds { index: 0, field: "head", value: 99, buf_len: 3 }
                )
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn transaction_apply_rejects_length_mismatch() {
        // Changeset built for 10 chars, but buffer is 3 chars.
        let buf = Buffer::from("hi");
        let mut b = ChangeSetBuilder::new(10);
        b.retain_rest();
        let cs = b.finish();

        let txn = Transaction::new(cs, SelectionSet::single(Selection::cursor(0)));
        let err = txn.apply(&buf).unwrap_err();
        assert!(
            matches!(
                err,
                TransactionError::Apply(ApplyError::LengthMismatch { buf_len: 3, expected: 10 })
            ),
            "unexpected error: {err}"
        );
    }
}
