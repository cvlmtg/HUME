use super::*;
use crate::selection::{Selection, SelectionSet};

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
        h.record(
            insert_cs(6 + i, "x"),
            delete_cs(7 + i, 1),
            sel_at(i),
            sel_at(i + 1),
        );
    }
    // Jump from rev5 to root in one call: 5 inverses.
    let txns = h
        .goto_revision(RevisionId(0))
        .expect("should navigate to root");
    assert_eq!(txns.len(), 5);
    assert_eq!(h.current, RevisionId(0));
}

#[test]
fn goto_distant_descendant() {
    let mut h = History::new(sel_at(0), 6);
    for i in 0..5 {
        h.record(
            insert_cs(6 + i, "x"),
            delete_cs(7 + i, 1),
            sel_at(i),
            sel_at(i + 1),
        );
    }
    h.undo();
    h.undo();
    h.undo();
    h.undo();
    h.undo(); // back to root
    assert_eq!(h.current, RevisionId(0));

    // Jump from root to rev5 in one call: 5 forwards.
    let txns = h
        .goto_revision(RevisionId(5))
        .expect("should navigate to leaf");
    assert_eq!(txns.len(), 5);
    assert_eq!(h.current, RevisionId(5));
}

#[test]
fn multiple_sequential_undos() {
    let mut h = History::new(sel_at(0), 6);
    for i in 0..5 {
        h.record(
            insert_cs(6 + i, "x"),
            delete_cs(7 + i, 1),
            sel_at(i),
            sel_at(i + 1),
        );
    }
    assert_eq!(h.len(), 6); // root + 5 revisions
    assert_eq!(h.current, RevisionId(5));

    for expected in (0..5).rev() {
        h.undo();
        assert_eq!(h.current, RevisionId(expected));
    }
    assert!(!h.can_undo());
}

// ── undo-levels cap ──────────────────────────────────────────────────────

#[test]
fn cap_zero_never_evicts() {
    // Fail oracle: if enforce_undo_levels ran regardless of the 0 sentinel,
    // this would trim down to 1 revision instead of staying at 6.
    let mut h = History::new(sel_at(0), 6);
    for i in 0..5 {
        h.record(
            insert_cs(6 + i, "x"),
            delete_cs(7 + i, 1),
            sel_at(i),
            sel_at(i + 1),
        );
    }
    assert_eq!(h.len(), 6);
}

#[test]
fn set_undo_levels_does_not_trim_until_next_record() {
    // Fail oracle: if set_undo_levels trimmed immediately, len() would drop
    // to 3 right after the call instead of only on the next record.
    let mut h = History::new(sel_at(0), 6);
    for i in 0..5 {
        h.record(
            insert_cs(6 + i, "x"),
            delete_cs(7 + i, 1),
            sel_at(i),
            sel_at(i + 1),
        );
    }
    assert_eq!(h.len(), 6);

    h.set_undo_levels(2);
    assert_eq!(h.len(), 6, "lowering the cap must not retroactively trim");

    let promoted = h.record(insert_cs(11, "x"), delete_cs(12, 1), sel_at(5), sel_at(6));
    assert_eq!(h.len(), 3); // root + last 2 revisions
    assert!(promoted.is_some());
}

#[test]
fn linear_chain_promotes_oldest() {
    // Fail oracle: without promotion, initial_sels would still read the
    // original root's identity selection (sel_at(0)) instead of a's
    // post-edit selection, and undo would walk one level too many.
    let mut h = History::new(sel_at(0), 6);
    h.set_undo_levels(2);
    h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1)); // a
    h.record(insert_cs(7, "b"), delete_cs(8, 1), sel_at(1), sel_at(2)); // b
    h.record(insert_cs(8, "c"), delete_cs(9, 1), sel_at(2), sel_at(3)); // c

    assert_eq!(h.len(), 3); // root(now=a) + b + c
    assert_eq!(*h.initial_sels(), sel_at(1)); // a's post-edit selection

    h.undo(); // c -> b
    h.undo(); // b -> new root (was a's parent slot, now root itself)
    assert!(!h.can_undo());
}

#[test]
fn cap_one_current_never_evicted() {
    // Fail oracle: if current could be evicted, len() would collapse to 1
    // (root only) instead of holding at 2 (root + current).
    let mut h = History::new(sel_at(0), 6);
    h.set_undo_levels(1);
    for i in 0..4 {
        h.record(
            insert_cs(6 + i, "x"),
            delete_cs(7 + i, 1),
            sel_at(i),
            sel_at(i + 1),
        );
        assert_eq!(h.len(), 2);
    }
    h.undo();
    assert!(h.can_redo());
}

#[test]
fn oldest_branch_evicted_first() {
    // Tree: root -> A (rev1) -> B (rev2); undo to A; record C (rev3, branch).
    // current is under C. Capping to 1 must drop the whole {A, B} branch
    // and promote C, not touch C's own subtree.
    // Fail oracle: evicting C's branch instead of A/B would break current.
    let mut h = History::new(sel_at(0), 6);
    h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1)); // rev1 = A
    h.record(insert_cs(7, "b"), delete_cs(8, 1), sel_at(1), sel_at(2)); // rev2 = B
    h.undo(); // back to A
    h.undo(); // back to root
    h.set_undo_levels(1);
    h.record(insert_cs(6, "c"), delete_cs(7, 1), sel_at(0), sel_at(1)); // rev3 = C

    assert_eq!(h.len(), 2); // root(now=C) + nothing else
    assert!(h.parent(RevisionId(1)).is_none()); // A evicted
    assert!(h.parent(RevisionId(2)).is_none()); // B evicted (subtree of A)
    assert!(h.goto_revision(RevisionId(1)).is_none());
}

#[test]
fn protected_branch_skipped() {
    // Tree: root -> A (rev1) -> B (rev2, current); undo to root; record C
    // (rev3, branch), undo to root; record D (rev4, branch, current).
    // Root has 3 children [A, C, D] (chronological). D is on current's
    // path. Eviction must remove A's branch (oldest non-protected), not D's.
    let mut h = History::new(sel_at(0), 6);
    h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1)); // rev1 = A
    h.record(insert_cs(7, "b"), delete_cs(8, 1), sel_at(1), sel_at(2)); // rev2 = B
    h.undo();
    h.undo(); // back to root
    h.record(insert_cs(6, "c"), delete_cs(7, 1), sel_at(0), sel_at(1)); // rev3 = C
    h.undo(); // back to root
    h.set_undo_levels(2);
    let promoted = h.record(insert_cs(6, "d"), delete_cs(7, 1), sel_at(0), sel_at(1)); // rev4 = D

    // Non-root count must be <= 2: A's branch {A, B} (2 nodes) discarded,
    // leaving {C, D} = 2 nodes. No promotion needed since root already had
    // more than one child.
    assert_eq!(h.len(), 3);
    assert!(promoted.is_none());
    assert!(h.parent(RevisionId(1)).is_none()); // A evicted
    assert!(h.parent(RevisionId(2)).is_none()); // B evicted
    assert!(h.parent(RevisionId(3)).is_some()); // C survives
    assert_eq!(h.current, RevisionId(4)); // D survives, still current
}

#[test]
fn subtree_eviction_may_overshoot() {
    // Tree: root -> A -> B -> C (chain of 3), undo to root, record D
    // (branch, current). Cap 3 with 4 non-root nodes triggers eviction;
    // discarding the whole {A, B, C} branch in one step drops to 1 non-root
    // node, well under the cap of 3 — matches Vim's overshoot behavior.
    let mut h = History::new(sel_at(0), 6);
    h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1)); // rev1 = A
    h.record(insert_cs(7, "b"), delete_cs(8, 1), sel_at(1), sel_at(2)); // rev2 = B
    h.record(insert_cs(8, "c"), delete_cs(9, 1), sel_at(2), sel_at(3)); // rev3 = C
    h.undo();
    h.undo();
    h.undo(); // back to root
    h.set_undo_levels(3);
    h.record(insert_cs(6, "d"), delete_cs(7, 1), sel_at(0), sel_at(1)); // rev4 = D

    assert_eq!(h.len(), 2); // root + D only; overshot below the cap of 3
}

#[test]
fn promotion_reports_last_promoted_only() {
    // A single record call can trigger multiple promotions in the trim
    // loop (linear chain with a very low cap). Only the final promoted id
    // is meaningful (it's the node root now represents), so earlier
    // promotions in the same loop must not leak out.
    let mut h = History::new(sel_at(0), 6);
    h.set_undo_levels(1);
    h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1)); // a: len 2, no trim
    let promoted_b = h.record(insert_cs(7, "b"), delete_cs(8, 1), sel_at(1), sel_at(2)); // b promotes a
    assert_eq!(promoted_b, Some(RevisionId(1))); // a
    let promoted_c = h.record(insert_cs(8, "c"), delete_cs(9, 1), sel_at(2), sel_at(3)); // c promotes b
    assert_eq!(promoted_c, Some(RevisionId(2))); // b, not a
    assert_eq!(h.len(), 2);
}
