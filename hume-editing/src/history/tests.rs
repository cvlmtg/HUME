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
