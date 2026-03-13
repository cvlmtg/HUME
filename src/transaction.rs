use crate::buffer::Buffer;
use crate::changeset::ChangeSet;
use crate::selection::SelectionSet;

/// A `Transaction` bundles a text change with the resulting selection state.
///
/// This is the unit of editing: every user action (insert, delete, motion
/// that modifies text) produces a `Transaction`. Undo/redo will operate on
/// transactions — the `ChangeSet` can be inverted to undo the text change,
/// and the stored selection can be restored.
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

    /// Apply this transaction to a buffer, returning the new buffer and
    /// the new selection state. Consumes the buffer — the old buffer is not
    /// needed because undo uses changeset inversion, not buffer snapshots.
    pub(crate) fn apply(&self, buf: Buffer) -> (Buffer, SelectionSet) {
        let new_buf = self.changes.apply(buf);
        (new_buf, self.selection.clone())
    }

    /// The text-change portion of this transaction.
    pub(crate) fn changes(&self) -> &ChangeSet {
        &self.changes
    }

    /// The selection state after this transaction.
    pub(crate) fn selection(&self) -> &SelectionSet {
        &self.selection
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::ChangeSetBuilder;
    use crate::selection::Selection;
    use pretty_assertions::assert_eq;

    #[test]
    fn transaction_apply() {
        let buf = Buffer::from_str("hello");
        let mut b = ChangeSetBuilder::new(5);
        b.insert("!");
        b.retain_rest();
        let cs = b.finish();

        let sels = SelectionSet::single(Selection::cursor(1));
        let txn = Transaction::new(cs, sels.clone());

        let (new_buf, new_sels) = txn.apply(buf);
        assert_eq!(new_buf.to_string(), "!hello");
        assert_eq!(new_sels.primary().head, 1);
    }
}
