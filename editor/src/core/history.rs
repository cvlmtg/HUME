use std::time::Instant;

use crate::core::changeset::{ChangeSet, ChangeSetBuilder};
use crate::core::selection::SelectionSet;
use crate::core::transaction::Transaction;

// ── Arena index ───────────────────────────────────────────────────────────────

/// A lightweight index into the History revision arena.
///
/// Using `usize` as an arena index is idiomatic Rust for tree structures:
/// it avoids `Rc<RefCell<...>>` reference cycles and the borrow-checker
/// friction that comes with self-referential structs, while still allowing
/// O(1) parent/child traversal via a `Vec`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RevisionId(pub(crate) usize);

// ── Revision ──────────────────────────────────────────────────────────────────

/// A single node in the undo tree.
///
/// Each revision stores both a forward Transaction (parent → this state, for
/// redo) and an inverse Transaction (this state → parent, for undo). No buffer
/// snapshot is stored — undo reconstructs the previous state by applying the
/// inverse Transaction.
///
/// The `children` vec records all revisions that branch from this one. The
/// **last** child (highest index) is the most recently created branch and is
/// the default redo target: after undoing and making a new edit, redo goes
/// to the most recent edit.
struct Revision {
    /// Apply this to move from the current state back to the parent state (undo).
    /// Its `selection` is the pre-edit selection — where cursors were before
    /// this revision was created.
    inverse: Transaction,
    /// Apply this to move from the parent state forward to this state (redo).
    /// Its `selection` is the post-edit selection.
    forward: Transaction,
    /// The parent revision. `None` only for the root.
    parent: Option<RevisionId>,
    /// Child revisions — branches created from this state.
    /// The last entry is the most recently created child (default redo target).
    children: Vec<RevisionId>,
    /// When this revision was created. Reserved for `:earlier`/`:later` time travel.
    #[allow(dead_code)]
    timestamp: Instant,
}

// ── History ───────────────────────────────────────────────────────────────────

/// Tree-structured undo/redo history.
///
/// ## Structure
///
/// Revisions are stored in an arena (`Vec<Revision>`) and linked by
/// [`RevisionId`] indices. The root revision (index 0) represents the initial
/// document state and has identity changesets. `current` tracks the active
/// revision — the state that matches the document's current buffer and
/// selections.
///
/// ## Branching
///
/// When the user undoes to state A and then makes a new edit C, the old redo
/// path (B) is preserved as a sibling of C. No edit is ever discarded. The
/// tree grows monotonically — revisions are never deleted.
///
/// ```text
///  root
///   └─ A        (first edit)
///       ├─ B    (second edit, later undone)
///       └─ C    (new edit after undoing to A — C is now the redo target)
/// ```
///
/// ## Undo/Redo
///
/// - **Undo**: apply `current.inverse`, set `current = current.parent`.
/// - **Redo**: pick the last child of `current`, apply its `forward`, set
///   `current` to that child.
///
/// ## What History does NOT own
///
/// Buffers. The caller ([`crate::editor::buffer::Buffer`]) holds the current
/// buffer. History stores only Transactions (changeset + selections). This
/// keeps History a pure data structure with no Buffer dependency.
pub(crate) struct History {
    /// Arena of all revisions. Index 0 is always the root.
    revisions: Vec<Revision>,
    /// The currently active revision.
    current: RevisionId,
}

impl History {
    /// Create a new history rooted at the initial document state.
    ///
    /// The root revision has identity changesets (all Retain) and carries
    /// `initial_sels` as its selection — this is the state before any edit.
    /// `buf_len` is the character length of the initial buffer (needed to
    /// build the identity ChangeSet).
    pub(crate) fn new(initial_sels: SelectionSet, buf_len: usize) -> Self {
        // Build an identity ChangeSet: retain every character unchanged.
        let mut b = ChangeSetBuilder::new(buf_len);
        b.retain_rest();
        let identity_cs = b.finish();

        // The root's forward and inverse are both identity transactions.
        // The selection is the initial cursor state.
        let root = Revision {
            inverse: Transaction::new(identity_cs.clone(), initial_sels.clone()),
            forward: Transaction::new(identity_cs, initial_sels),
            parent: None,
            children: Vec::new(),
            timestamp: Instant::now(),
        };

        Self {
            revisions: vec![root],
            current: RevisionId(0),
        }
    }

    /// Record a new edit and advance the current position to it.
    ///
    /// Creates a new [`Revision`] as a child of the current revision and makes
    /// it the new `current`. The caller provides both the forward and inverse
    /// changesets — the inverse must have been computed against the pre-edit
    /// buffer before that buffer was replaced.
    ///
    /// # Arguments
    ///
    /// - `forward_cs`: the ChangeSet that was applied to produce the new state.
    /// - `inverse_cs`: `forward_cs.invert(&pre_edit_buf)` — reverses the edit.
    /// - `pre_edit_sels`: cursor positions before the edit (stored in `inverse`
    ///   so undo restores them).
    /// - `post_edit_sels`: cursor positions after the edit (stored in `forward`
    ///   so redo restores them).
    pub(crate) fn record(
        &mut self,
        forward_cs: ChangeSet,
        inverse_cs: ChangeSet,
        pre_edit_sels: SelectionSet,
        post_edit_sels: SelectionSet,
    ) {
        let new_id = RevisionId(self.revisions.len());
        let parent_id = self.current;

        let revision = Revision {
            // inverse carries pre-edit sels: after undoing, cursors return there.
            inverse: Transaction::new(inverse_cs, pre_edit_sels),
            // forward carries post-edit sels: after redoing, cursors land there.
            forward: Transaction::new(forward_cs, post_edit_sels),
            parent: Some(parent_id),
            children: Vec::new(),
            timestamp: Instant::now(),
        };

        self.revisions.push(revision);
        self.revisions[parent_id.0].children.push(new_id);
        self.current = new_id;
    }

    /// Undo: return the inverse Transaction for the current revision and move
    /// to the parent. Returns `None` if already at the root (nothing to undo).
    ///
    /// The returned Transaction carries the pre-edit buffer transform and the
    /// pre-edit selections. The caller applies it to the current buffer to
    /// restore the previous state and selections.
    ///
    /// Returns an owned `Transaction` (cloned from the arena) rather than a
    /// reference, to avoid lifetime conflicts when the caller also holds a
    /// reference to other fields of the owning struct (e.g. `Buffer::text`).
    /// `Transaction` is cheap to clone: its ChangeSet is a `Vec<Operation>`.
    pub(crate) fn undo(&mut self) -> Option<Transaction> {
        let old_current = self.current;
        // Copy out the parent index before mutating current.
        let parent = self.revisions[old_current.0].parent?;
        self.current = parent;
        // Clone the inverse from the revision we just stepped out of.
        // The arena is append-only — old_current is still valid.
        Some(self.revisions[old_current.0].inverse.clone())
    }

    /// Redo: return the forward Transaction of the most recent child and move
    /// to it. Returns `None` if the current revision has no children.
    ///
    /// The most recent child (last in `children`) is chosen to match
    /// Vim/Helix behaviour: after undoing and making a new edit, redo goes
    /// to the most recent edit, not the historically first one.
    ///
    /// Returns an owned `Transaction` for the same reason as [`undo`].
    pub(crate) fn redo(&mut self) -> Option<Transaction> {
        // Copy out child_id before mutating current.
        let child_id = *self.revisions[self.current.0].children.last()?;
        self.current = child_id;
        Some(self.revisions[child_id.0].forward.clone())
    }

    #[cfg(test)]
    /// True if there is at least one revision above the current position.
    pub(crate) fn can_undo(&self) -> bool {
        self.revisions[self.current.0].parent.is_some()
    }

    #[cfg(test)]
    /// True if the current revision has at least one child.
    pub(crate) fn can_redo(&self) -> bool {
        !self.revisions[self.current.0].children.is_empty()
    }

    /// Total number of revisions in the tree (including the root).
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.revisions.len()
    }

    /// The currently active revision.
    pub(crate) fn current_id(&self) -> RevisionId {
        self.current
    }

    /// The initial selections stored in the root revision.
    ///
    /// Used by `Buffer::initial_sels()` to seed `PaneBufferState` when a pane
    /// first views this buffer or when `:e!` reloads it.
    pub(crate) fn initial_sels(&self) -> &SelectionSet {
        self.revisions[0].forward.selection()
    }

    /// Parent of a revision. `None` for the root.
    #[cfg(test)]
    pub(crate) fn parent(&self, id: RevisionId) -> Option<RevisionId> {
        self.revisions[id.0].parent
    }

    /// Ancestor chain from `id` up to and including the root.
    ///
    /// Returns `[id, parent, grandparent, ..., root]`.
    fn ancestors(&self, mut id: RevisionId) -> Vec<RevisionId> {
        let mut chain = vec![id];
        while let Some(parent) = self.revisions[id.0].parent {
            chain.push(parent);
            id = parent;
        }
        chain
    }

    #[allow(dead_code)]
    /// Jump to an arbitrary revision in the undo tree.
    ///
    /// Returns the sequence of [`Transaction`]s that must be applied
    /// **in order** to transform the current buffer into the target state.
    /// The caller is responsible for applying each transaction sequentially —
    /// do **not** try to compose them, since each was computed against the
    /// buffer state at its specific point in history.
    ///
    /// Returns `None` if `target` equals the current revision (no-op) or is
    /// out of bounds.
    ///
    /// ## How it works
    ///
    /// The path from `current` to `target` passes through their Lowest Common
    /// Ancestor (LCA):
    ///
    /// - **Up leg** (`current` → LCA): for each node stepped out of, use its
    ///   `inverse` transaction (same as [`undo`]).
    /// - **Down leg** (LCA → `target`): for each node stepped into, use its
    ///   `forward` transaction (same as [`redo`]).
    pub(crate) fn goto_revision(&mut self, target: RevisionId) -> Option<Vec<Transaction>> {
        if target == self.current {
            return None;
        }
        if target.0 >= self.revisions.len() {
            return None;
        }

        let ancestors_from = self.ancestors(self.current);
        let ancestors_to = self.ancestors(target);

        // Put the "from" ancestor set in a HashSet for O(1) lookup.
        // We need to find the first node in ancestors_to that also appears
        // in ancestors_from — that is the LCA.
        let from_set: std::collections::HashSet<RevisionId> =
            ancestors_from.iter().copied().collect();

        // Find the LCA: walk ancestors_to until we hit a node in from_set.
        let lca = *ancestors_to
            .iter()
            .find(|id| from_set.contains(id))
            .expect("all revisions share at least the root ancestor");

        // Up leg: nodes from `current` up to (not including) LCA.
        // ancestors_from = [current, ..., lca, ...]
        let up_path: Vec<RevisionId> = ancestors_from
            .iter()
            .copied()
            .take_while(|&id| id != lca)
            .collect();

        // Down leg: nodes from LCA's child down to `target`.
        // ancestors_to = [target, ..., lca_child, lca, ...]
        // Take everything before lca, then reverse so it goes lca_child → target.
        let mut down_path: Vec<RevisionId> = ancestors_to
            .iter()
            .copied()
            .take_while(|&id| id != lca)
            .collect();
        down_path.reverse();

        // Build the transaction list.
        let mut txns = Vec::with_capacity(up_path.len() + down_path.len());
        for id in &up_path {
            txns.push(self.revisions[id.0].inverse.clone());
        }
        for id in &down_path {
            txns.push(self.revisions[id.0].forward.clone());
        }

        self.current = target;
        Some(txns)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::selection::{Selection, SelectionSet};

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Build a collapsed SelectionSet at offset `pos`.
    fn sel_at(pos: usize) -> SelectionSet {
        SelectionSet::single(Selection::collapsed(pos))
    }

    /// Build a simple ChangeSet that inserts `text` at offset 0 in a buffer
    /// of `buf_len` characters.
    fn insert_cs(buf_len: usize, text: &str) -> ChangeSet {
        let mut b = ChangeSetBuilder::new(buf_len);
        b.insert(text);
        b.retain_rest();
        b.finish()
    }

    /// Build a simple ChangeSet that deletes the first `n` characters from a
    /// buffer of `buf_len` characters.
    fn delete_cs(buf_len: usize, n: usize) -> ChangeSet {
        let mut b = ChangeSetBuilder::new(buf_len);
        b.delete(n);
        b.retain_rest();
        b.finish()
    }

    // ── Basic undo/redo ───────────────────────────────────────────────────────

    #[test]
    fn new_history_has_one_revision() {
        let h = History::new(sel_at(0), 6);
        assert_eq!(h.len(), 1);
        assert!(!h.can_undo());
        assert!(!h.can_redo());
    }

    #[test]
    fn record_advances_current() {
        let mut h = History::new(sel_at(0), 6);
        let cs = insert_cs(6, "x");
        let inv = delete_cs(7, 1);
        h.record(cs, inv, sel_at(0), sel_at(1));
        assert_eq!(h.len(), 2);
        assert!(h.can_undo());
        assert!(!h.can_redo());
    }

    #[test]
    fn undo_returns_inverse_and_moves_to_parent() {
        let mut h = History::new(sel_at(0), 6);
        let cs = insert_cs(6, "x");
        let inv = delete_cs(7, 1);
        h.record(cs, inv.clone(), sel_at(0), sel_at(1));

        let txn = h.undo().expect("should have something to undo");
        // The inverse Transaction's selection is the pre-edit selection (sel_at(0)).
        assert_eq!(*txn.selection(), sel_at(0));
        assert!(!h.can_undo()); // back at root
    }

    #[test]
    fn undo_at_root_returns_none() {
        let mut h = History::new(sel_at(0), 6);
        assert!(h.undo().is_none());
    }

    #[test]
    fn redo_returns_forward_and_moves_to_child() {
        let mut h = History::new(sel_at(0), 6);
        let cs = insert_cs(6, "x");
        let inv = delete_cs(7, 1);
        h.record(cs.clone(), inv, sel_at(0), sel_at(1));

        h.undo(); // back to root

        let txn = h.redo().expect("should have something to redo");
        assert_eq!(*txn.selection(), sel_at(1)); // post-edit selection
        assert!(!h.can_redo()); // at leaf again
    }

    #[test]
    fn redo_with_no_children_returns_none() {
        let mut h = History::new(sel_at(0), 6);
        assert!(h.redo().is_none());
    }

    #[test]
    fn undo_redo_roundtrip() {
        let mut h = History::new(sel_at(0), 6);
        h.record(insert_cs(6, "x"), delete_cs(7, 1), sel_at(0), sel_at(1));
        h.record(insert_cs(7, "y"), delete_cs(8, 1), sel_at(1), sel_at(2));

        assert_eq!(h.current, RevisionId(2));
        h.undo();
        assert_eq!(h.current, RevisionId(1));
        h.undo();
        assert_eq!(h.current, RevisionId(0));
        h.redo();
        assert_eq!(h.current, RevisionId(1));
        h.redo();
        assert_eq!(h.current, RevisionId(2));
        assert!(!h.can_redo());
    }

    #[test]
    fn branching_preserves_old_path() {
        // Record A (rev 1) then B (rev 2). Undo to root. Record C (rev 3).
        // Tree:  root → A → B
        //            ↘ C
        // Redo from root should go to C (last child), not B.
        let mut h = History::new(sel_at(0), 6);
        h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1)); // rev 1
        h.record(insert_cs(7, "b"), delete_cs(8, 1), sel_at(1), sel_at(2)); // rev 2
        h.undo(); // to rev 1
        h.undo(); // to root
        h.record(insert_cs(6, "c"), delete_cs(7, 1), sel_at(0), sel_at(1)); // rev 3

        // Tree has 4 nodes: root, A, B, C.
        assert_eq!(h.len(), 4);

        // current is rev 3.
        assert_eq!(h.current, RevisionId(3));

        // Undo to root.
        h.undo();
        assert_eq!(h.current, RevisionId(0));

        // Root has 2 children: A (rev 1) and C (rev 3). Redo goes to last = C.
        let txn = h.redo().expect("should redo to C");
        assert_eq!(*txn.selection(), sel_at(1)); // C's post-edit selection
        assert_eq!(h.current, RevisionId(3));

        // From C, undo gets us back to root, then we can redo to C again.
        h.undo();
        // Root still has children — can redo.
        assert!(h.can_redo());
    }

    // ── goto_revision ─────────────────────────────────────────────────────────

    /// Build a branching tree for goto tests:
    ///
    /// ```text
    ///      * rev3
    ///      |
    /// *r4  * rev2
    /// |    |
    /// `----* rev1
    ///      |
    ///      * root (rev0)
    /// ```
    ///
    /// rev1 = first edit, rev2 = second edit, rev3 = third edit.
    /// Undo to rev1, then record rev4 = branch C.
    fn branching_history() -> History {
        let mut h = History::new(sel_at(0), 6);
        h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1)); // rev1
        h.record(insert_cs(7, "b"), delete_cs(8, 1), sel_at(1), sel_at(2)); // rev2
        h.record(insert_cs(8, "c"), delete_cs(9, 1), sel_at(2), sel_at(3)); // rev3
        h.undo(); // back to rev2
        h.undo(); // back to rev1
        h.record(insert_cs(7, "d"), delete_cs(8, 1), sel_at(1), sel_at(9)); // rev4 (branch)
        h
    }

    #[test]
    fn goto_same_revision_is_none() {
        let mut h = History::new(sel_at(0), 6);
        h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1));
        assert!(h.goto_revision(h.current).is_none());
    }

    #[test]
    fn goto_out_of_bounds_returns_none() {
        let mut h = History::new(sel_at(0), 6);
        assert!(h.goto_revision(RevisionId(999)).is_none());
    }

    #[test]
    fn goto_parent_is_one_inverse() {
        let mut h = History::new(sel_at(0), 6);
        let inv = delete_cs(7, 1);
        h.record(insert_cs(6, "a"), inv.clone(), sel_at(0), sel_at(1));
        let rev0 = RevisionId(0);
        let txns = h.goto_revision(rev0).expect("should move to parent");
        // Should be one transaction: the inverse of rev1.
        assert_eq!(txns.len(), 1);
        // After goto, current is root.
        assert_eq!(h.current, RevisionId(0));
    }

    #[test]
    fn goto_child_is_one_forward() {
        let mut h = History::new(sel_at(0), 6);
        h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1));
        h.undo(); // back to root
        let rev1 = RevisionId(1);
        let txns = h.goto_revision(rev1).expect("should move to child");
        assert_eq!(txns.len(), 1);
        assert_eq!(h.current, RevisionId(1));
    }

    #[test]
    fn goto_across_branches_via_lca() {
        // Tree: root → rev1 → rev2 → rev3
        //                  ↘ rev4 (current)
        // Jump from rev4 to rev3: up to rev1 (LCA), down to rev2, down to rev3.
        // Expected: 1 inverse (rev4) + 2 forwards (rev2, rev3) = 3 transactions.
        let mut h = branching_history();
        assert_eq!(h.current, RevisionId(4));

        let txns = h
            .goto_revision(RevisionId(3))
            .expect("should navigate across branches");
        assert_eq!(txns.len(), 3);
        assert_eq!(h.current, RevisionId(3));
    }

    #[test]
    fn goto_distant_ancestor() {
        let mut h = History::new(sel_at(0), 6);
        for i in 0..5 {
            h.record(insert_cs(6 + i, "x"), delete_cs(7 + i, 1), sel_at(i), sel_at(i + 1));
        }
        // Jump from rev5 to root in one call: 5 inverses.
        let txns = h.goto_revision(RevisionId(0)).expect("should navigate to root");
        assert_eq!(txns.len(), 5);
        assert_eq!(h.current, RevisionId(0));
    }

    #[test]
    fn goto_distant_descendant() {
        let mut h = History::new(sel_at(0), 6);
        for i in 0..5 {
            h.record(insert_cs(6 + i, "x"), delete_cs(7 + i, 1), sel_at(i), sel_at(i + 1));
        }
        h.undo();
        h.undo();
        h.undo();
        h.undo();
        h.undo(); // back to root
        assert_eq!(h.current, RevisionId(0));

        // Jump from root to rev5 in one call: 5 forwards.
        let txns = h.goto_revision(RevisionId(5)).expect("should navigate to leaf");
        assert_eq!(txns.len(), 5);
        assert_eq!(h.current, RevisionId(5));
    }

    #[test]
    fn multiple_sequential_undos() {
        let mut h = History::new(sel_at(0), 6);
        for i in 0..5 {
            h.record(insert_cs(6 + i, "x"), delete_cs(7 + i, 1), sel_at(i), sel_at(i + 1));
        }
        assert_eq!(h.len(), 6); // root + 5 revisions
        assert_eq!(h.current, RevisionId(5));

        for expected in (0..5).rev() {
            h.undo();
            assert_eq!(h.current, RevisionId(expected));
        }
        assert!(!h.can_undo());
    }
}
