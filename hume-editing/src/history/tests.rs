use hume_rope::offset::CharOffset;

use super::children::Children;
use super::*;
use crate::changeset::ChangeSetBuilder;
use crate::selection::SelectionSet;
use crate::state::EditState;
use crate::text::BufferText;

// ── Helpers ───────────────────────────────────────────────────────────────

/// A cursor at offset `pos` of one text shared by every call, wide enough
/// that every char below 60 starts a cluster, so two calls with one `pos`
/// give equal sets.
fn sel_at(pos: usize) -> SelectionSet {
    static TEXT: std::sync::OnceLock<BufferText> = std::sync::OnceLock::new();
    let text = TEXT.get_or_init(|| BufferText::from(" ".repeat(60).as_str()));
    let at = text.snap(CharOffset::new(pos));
    assert_eq!(at.offset(), CharOffset::new(pos));
    EditState::with_cursor(text.clone(), at).into_selections()
}

/// Build a simple ChangeSet that inserts `text` at offset 0 in a buffer
/// of `buf_len` characters.
fn insert_cs(buf_len: usize, text: &str) -> ChangeSet {
    let mut b = ChangeSetBuilder::new(CharOffset::new(buf_len));
    b.insert(text);
    b.retain_rest();
    b.finish()
}

/// Build a simple ChangeSet that deletes the first `n` characters from a
/// buffer of `buf_len` characters.
fn delete_cs(buf_len: usize, n: usize) -> ChangeSet {
    let mut b = ChangeSetBuilder::new(CharOffset::new(buf_len));
    b.delete(n);
    b.retain_rest();
    b.finish()
}

// ── Basic undo/redo ───────────────────────────────────────────────────────

#[test]
fn new_history_has_one_revision() {
    let h = History::new();
    assert_eq!(h.len(), 1);
    assert!(!h.can_undo());
    assert!(!h.can_redo());
}

#[test]
fn record_advances_current() {
    let mut h = History::new();
    let cs = insert_cs(6, "x");
    let inv = delete_cs(7, 1);
    h.record(cs, inv, sel_at(0), sel_at(1));
    assert_eq!(h.len(), 2);
    assert!(h.can_undo());
    assert!(!h.can_redo());
}

#[test]
fn undo_returns_inverse_and_moves_to_parent() {
    let mut h = History::new();
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
    let mut h = History::new();
    assert!(h.undo().is_none());
}

#[test]
fn redo_returns_forward_and_moves_to_child() {
    let mut h = History::new();
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
    let mut h = History::new();
    assert!(h.redo().is_none());
}

#[test]
fn undo_redo_roundtrip() {
    let mut h = History::new();
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
    let mut h = History::new();
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
    // Root still has children, so can redo.
    assert!(h.can_redo());
}

// ── undo_n / redo_n (composed multi-step walk) ───────────────────────────

#[test]
fn undo_n_walks_multiple_steps_and_lands_on_target() {
    let mut h = History::new();
    for i in 0..5 {
        h.record(
            insert_cs(6 + i, "x"),
            delete_cs(7 + i, 1),
            sel_at(i),
            sel_at(i + 1),
        );
    }
    assert_eq!(h.current, RevisionId(5));

    let txns = h.undo_n(3);
    assert_eq!(txns.len(), 3, "three real ancestors are available");
    assert_eq!(h.current, RevisionId(2));
    // The last transaction in the up-path is rev3's own inverse, whose
    // selection is rev3's pre-edit selection.
    assert_eq!(*txns.last().expect("non-empty").selection(), sel_at(2));
}

#[test]
fn undo_n_clamps_at_root_short_of_the_requested_count() {
    let mut h = History::new();
    h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1));
    h.record(insert_cs(7, "b"), delete_cs(8, 1), sel_at(1), sel_at(2));

    let txns = h.undo_n(10);
    assert_eq!(txns.len(), 2, "only 2 ancestors exist above the root");
    assert_eq!(h.current, RevisionId(0));
}

#[test]
fn undo_n_zero_count_is_empty_and_current_unmoved() {
    let mut h = History::new();
    h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1));
    let before = h.current;

    assert!(h.undo_n(0).is_empty());
    assert_eq!(h.current, before);
}

#[test]
fn undo_n_at_root_is_empty() {
    let mut h = History::new();
    assert!(h.undo_n(3).is_empty());
    assert_eq!(h.current, RevisionId(0));
}

#[test]
fn redo_n_walks_multiple_steps_and_lands_on_target() {
    let mut h = History::new();
    for i in 0..4 {
        h.record(
            insert_cs(6 + i, "x"),
            delete_cs(7 + i, 1),
            sel_at(i),
            sel_at(i + 1),
        );
    }
    for _ in 0..4 {
        h.undo();
    }
    assert_eq!(h.current, RevisionId(0));

    let txns = h.redo_n(3);
    assert_eq!(txns.len(), 3);
    assert_eq!(h.current, RevisionId(3));
    assert_eq!(*txns.last().expect("non-empty").selection(), sel_at(3));
}

#[test]
fn redo_n_clamps_at_leaf_short_of_the_requested_count() {
    let mut h = History::new();
    h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1));
    h.record(insert_cs(7, "b"), delete_cs(8, 1), sel_at(1), sel_at(2));
    h.undo();
    h.undo();

    let txns = h.redo_n(10);
    assert_eq!(txns.len(), 2, "only 2 descendants exist below the root");
    assert_eq!(h.current, RevisionId(2));
}

#[test]
fn redo_n_follows_last_walked_child_through_multiple_hops() {
    // branching_history: root → rev1 → rev2 → rev3, then rev1 gains a second
    // child rev4 (the newest, so rev1's redo target). Current is rev4 after
    // construction.
    let mut h = branching_history();
    h.undo(); // rev4 -> rev1
    h.undo(); // rev1 -> root
    assert_eq!(h.current, RevisionId(0));

    let txns = h.redo_n(2);
    assert_eq!(txns.len(), 2);
    assert_eq!(
        h.current,
        RevisionId(4),
        "must follow rev1's redo child (rev4), not the older rev2"
    );
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
    let mut h = History::new();
    h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1)); // rev1
    h.record(insert_cs(7, "b"), delete_cs(8, 1), sel_at(1), sel_at(2)); // rev2
    h.record(insert_cs(8, "c"), delete_cs(9, 1), sel_at(2), sel_at(3)); // rev3
    h.undo(); // back to rev2
    h.undo(); // back to rev1
    h.record(insert_cs(7, "d"), delete_cs(8, 1), sel_at(1), sel_at(9)); // rev4 (branch)
    h
}

#[test]
fn goto_current_revision_walks_nothing() {
    let mut h = History::new();
    h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1));
    let current = h.current;
    let txns = h
        .goto_revision(current)
        .expect("the current revision is a known id");
    assert!(txns.is_empty());
    assert_eq!(h.current, current);
}

#[test]
fn goto_out_of_bounds_returns_none() {
    let mut h = History::new();
    assert!(h.goto_revision(RevisionId(999)).is_none());
}

#[test]
fn goto_parent_is_one_inverse() {
    let mut h = History::new();
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
    let mut h = History::new();
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
    let mut h = History::new();
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
    let mut h = History::new();
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
    let mut h = History::new();
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
    // A cap of 0 means unlimited. Trimming here would leave 1 revision
    // instead of 6.
    let mut h = History::new();
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
    // Lowering the cap takes effect on the next record. An immediate trim
    // would drop len() to 3 right after the call.
    let mut h = History::new();
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
    // Promoting `a` into the root drops len() from 4 (root+a+b+c) to 3 and
    // leaves `a` with no parent at all.
    let mut h = History::new();
    h.set_undo_levels(2);
    let a = h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1)); // a = RevisionId(1)
    h.record(insert_cs(7, "b"), delete_cs(8, 1), sel_at(1), sel_at(2)); // b
    let promoted = h.record(insert_cs(8, "c"), delete_cs(9, 1), sel_at(2), sel_at(3)); // c

    assert_eq!(h.len(), 3); // root(now=a) + b + c
    assert_eq!(promoted, Some(RevisionId(1))); // a promoted into root
    assert_eq!(a, None); // a's own record didn't trigger a promotion
    assert!(h.parent(RevisionId(1)).is_none()); // a is gone, not just re-parented

    h.undo(); // c -> b
    h.undo(); // b -> new root (was a's parent slot, now root itself)
    assert!(!h.can_undo());
}

#[test]
fn cap_one_current_never_evicted() {
    // Current is never evicted, so len() holds at 2 (root + current).
    let mut h = History::new();
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
    let mut h = History::new();
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
    let mut h = History::new();
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
    // node, well under the cap of 3, matching Vim's overshoot behavior.
    let mut h = History::new();
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
    let mut h = History::new();
    h.set_undo_levels(1);
    h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1)); // a: len 2, no trim
    let promoted_b = h.record(insert_cs(7, "b"), delete_cs(8, 1), sel_at(1), sel_at(2)); // b promotes a
    assert_eq!(promoted_b, Some(RevisionId(1))); // a
    let promoted_c = h.record(insert_cs(8, "c"), delete_cs(9, 1), sel_at(2), sel_at(3)); // c promotes b
    assert_eq!(promoted_c, Some(RevisionId(2))); // b, not a
    assert_eq!(h.len(), 2);
}

// ── Time-travel step resolution (:earlier/:later) ────────────────────────────

use std::time::{Duration, SystemTime};

/// Backdate revision `id` so it reads as `age` old.
fn backdate(h: &mut History, id: RevisionId, age: Duration) {
    h.revisions.get_mut(&id).expect("revision exists").timestamp = SystemTime::now() - age;
}

fn mins(n: u64) -> Duration {
    Duration::from_secs(n * 60)
}

/// Linear chain root(20m) → rev1(15m) → rev2(8m) → rev3(1m, current).
fn aged_chain() -> History {
    let mut h = History::new();
    h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1)); // rev1
    h.record(insert_cs(7, "b"), delete_cs(8, 1), sel_at(1), sel_at(2)); // rev2
    h.record(insert_cs(8, "c"), delete_cs(9, 1), sel_at(2), sel_at(3)); // rev3
    backdate(&mut h, RevisionId(0), mins(20));
    backdate(&mut h, RevisionId(1), mins(15));
    backdate(&mut h, RevisionId(2), mins(8));
    backdate(&mut h, RevisionId(3), mins(1));
    h
}

#[test]
fn undo_steps_older_than_walks_to_state_as_of_age() {
    let h = aged_chain();
    assert_eq!(
        h.undo_steps_older_than(mins(5)),
        Ok(1),
        ":earlier 5m from 1m-old tip must step once onto rev2 (8m)"
    );
    assert_eq!(
        h.undo_steps_older_than(mins(10)),
        Ok(2),
        ":earlier 10m must step twice onto rev1 (15m)"
    );
    assert_eq!(
        h.undo_steps_older_than(mins(30)),
        Err(3),
        ":earlier older than the root is unsatisfiable: Err carries the real depth (3), \
         for the shared undo loop's own exhaustion check to report"
    );
    assert_eq!(
        h.undo_steps_older_than(Duration::ZERO),
        Ok(0),
        ":earlier 0s is already satisfied: no steps"
    );
}

#[test]
fn redo_steps_newer_than_walks_last_child_chain() {
    let mut h = aged_chain();
    h.undo();
    h.undo(); // back to rev1 (15m)
    assert_eq!(
        h.redo_steps_newer_than(mins(5)),
        Ok(1),
        ":later 5m from rev1 must step once onto rev2 (8m), stopping before rev3 (1m)"
    );
    assert_eq!(
        h.redo_steps_newer_than(Duration::ZERO),
        Err(2),
        ":later 0s walks the whole last-child chain to the tip (2 real hops), still \
         older than now: unsatisfiable, Err carries the real hop count"
    );
    assert_eq!(
        h.redo_steps_newer_than(mins(30)),
        Ok(0),
        ":later older than every descendant is already satisfied: no steps"
    );
}

/// A leaf has nothing newer to redo onto regardless of the request: the
/// tip's own age must never be compared against `age` the way a genuine
/// over-travel (walking onto a leaf that is still too old) is.
/// `undo_steps_older_than` has no such case: its clamp only ever fires from
/// inside the walk, at the root.
///
/// Comparing the *current* revision's own age against `age` when it has no
/// children would return `Err(1)`, a false "Already at newest change" extra
/// step, where `Ok(0)` is correct.
#[test]
fn redo_steps_newer_than_at_the_tip_is_satisfied_regardless_of_the_tip_s_own_age() {
    let h = aged_chain(); // current = rev3 (tip), backdated to 1m, no children
    assert_eq!(
        h.redo_steps_newer_than(Duration::from_secs(5)),
        Ok(0),
        ":later 5s at the tip is already satisfied; there is nothing newer to redo \
         onto, no matter how old the tip itself is relative to the requested age"
    );
}

#[test]
fn redo_steps_newer_than_follows_last_walked_child() {
    let mut h = aged_chain();
    h.undo();
    h.undo(); // back to rev1
    h.record(insert_cs(7, "d"), delete_cs(8, 1), sel_at(1), sel_at(9)); // rev4, last child of rev1
    backdate(&mut h, RevisionId(4), mins(2));
    h.undo(); // back to rev1, the fork point the queries run from
    // Sibling rev2 is 8m old but is not rev1's redo target; the count must
    // follow rev4 (2m), so `:later 5m` takes no steps.
    assert_eq!(
        h.redo_steps_newer_than(mins(5)),
        Ok(0),
        ":later must follow the redo child (rev4, 2m), not the older sibling"
    );
    assert_eq!(
        h.redo_steps_newer_than(Duration::ZERO),
        Err(1),
        ":later 0s steps once along the rev4 branch (1 real hop), whose tip is still \
         older than now: unsatisfiable, Err carries the real hop count"
    );
    h.redo();
    assert_eq!(
        h.current,
        RevisionId(4),
        "the counted step must land on the redo child"
    );
}

// ── Redo follows the last-walked child ───────────────────────────────────────

#[test]
fn undo_makes_the_undone_child_the_redo_target() {
    let mut h = History::new();
    h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1)); // rev1
    h.undo();
    h.record(insert_cs(6, "c"), delete_cs(7, 1), sel_at(0), sel_at(2)); // rev2, newer sibling
    h.goto_revision(RevisionId(1));
    h.undo();
    assert_eq!(h.current, History::ROOT);

    h.redo();
    assert_eq!(h.current, RevisionId(1));
}

#[test]
fn goto_into_older_branch_redirects_redo_along_it() {
    // branching_history: rev1 has children rev2 (→ rev3) and rev4; current is rev4.
    let mut h = branching_history();
    h.goto_revision(RevisionId(3));
    h.undo_n(2);
    assert_eq!(h.current, RevisionId(1));

    let txns = h.redo_n(2);
    assert_eq!(txns.len(), 2);
    assert_eq!(h.current, RevisionId(3));
}

#[test]
fn redo_steps_newer_than_follows_the_walked_branch() {
    let mut h = aged_chain();
    h.undo();
    h.undo(); // rev1
    h.record(insert_cs(7, "d"), delete_cs(8, 1), sel_at(1), sel_at(9)); // rev4
    backdate(&mut h, RevisionId(4), mins(2));
    h.goto_revision(RevisionId(2));
    h.undo(); // rev1, redo target now rev2 (8m), not rev4 (2m)

    assert_eq!(
        h.redo_steps_newer_than(mins(5)),
        Ok(1),
        ":later 5m must step onto rev2 (8m) and stop before rev3 (1m)"
    );
    h.redo();
    assert_eq!(h.current, RevisionId(2));
}

#[test]
fn promotion_carries_the_redo_target_into_the_root() {
    let mut h = History::new();
    h.set_undo_levels(3);
    h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1)); // rev1
    h.record(insert_cs(7, "b"), delete_cs(8, 1), sel_at(1), sel_at(2)); // rev2
    h.undo();
    h.record(insert_cs(7, "c"), delete_cs(8, 1), sel_at(1), sel_at(3)); // rev3
    h.goto_revision(RevisionId(2));
    let promoted = h.record(insert_cs(8, "d"), delete_cs(9, 1), sel_at(2), sel_at(4)); // rev4
    assert_eq!(promoted, Some(RevisionId(1)));
    h.undo_n(2);
    assert_eq!(h.current, History::ROOT);

    h.redo();
    assert_eq!(h.current, RevisionId(2));
}

#[test]
fn children_push_makes_newest_the_redo_target() {
    let mut c = Children::default();
    assert_eq!(c.redo(), None);
    c.push(RevisionId(1));
    c.push(RevisionId(2));
    assert_eq!(c.redo(), Some(RevisionId(2)));
    assert_eq!(c.iter().collect::<Vec<_>>(), [RevisionId(1), RevisionId(2)]);
    assert_eq!(c.len(), 2);
}

#[test]
fn children_remove_of_redo_target_falls_back_to_newest_remaining() {
    let mut c = Children::default();
    c.push(RevisionId(1));
    c.push(RevisionId(2));
    c.push(RevisionId(3));
    c.set_redo(RevisionId(2));

    c.remove(RevisionId(1));
    assert_eq!(
        c.redo(),
        Some(RevisionId(2)),
        "a non-target removal keeps the target"
    );

    c.remove(RevisionId(2));
    assert_eq!(c.redo(), Some(RevisionId(3)));

    c.remove(RevisionId(3));
    assert_eq!(c.redo(), None);
    assert_eq!(c.len(), 0);
}

#[test]
#[should_panic(expected = "redo target must be a child")]
fn children_set_redo_rejects_a_non_child() {
    let mut c = Children::default();
    c.push(RevisionId(1));
    c.set_redo(RevisionId(7));
}

// ── Enumeration and ids ──────────────────────────────────────────────────────

#[test]
fn revision_id_checked_accepts_live_and_rejects_evicted_ids() {
    let mut h = History::new();
    h.set_undo_levels(1);
    h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1)); // rev1
    h.record(insert_cs(7, "b"), delete_cs(8, 1), sel_at(1), sel_at(2)); // rev2, promotes rev1

    assert_eq!(RevisionId::checked(&h, 0), Some(History::ROOT));
    assert_eq!(RevisionId::checked(&h, 1), None, "rev1 was promoted away");
    let live = RevisionId::checked(&h, 2).expect("rev2 is live");
    assert_eq!(live.index(), 2);
    assert_eq!(RevisionId::checked(&h, 99), None);
}

#[test]
fn nodes_enumerate_every_revision_in_id_order_with_parent_and_age() {
    let mut h = branching_history(); // rev1 → rev2 → rev3, rev1 → rev4
    for i in 0..=4 {
        backdate(&mut h, RevisionId(i), mins(20 - 4 * i as u64));
    }
    let nodes = h.nodes(SystemTime::now());

    let shape: Vec<_> = nodes
        .iter()
        .map(|n| (n.id().index(), n.parent().map(RevisionId::index)))
        .collect();
    assert_eq!(
        shape,
        [
            (0, None),
            (1, Some(0)),
            (2, Some(1)),
            (3, Some(2)),
            (4, Some(1))
        ]
    );
    for (i, node) in nodes.iter().enumerate() {
        let want = mins(20 - 4 * i as u64);
        assert!(
            node.age() >= want && node.age() < want + Duration::from_secs(5),
            "rev{i} age {:?} should be about {want:?}",
            node.age()
        );
    }
}

#[test]
fn change_seq_moves_on_every_navigation_and_record_only() {
    let mut h = History::new();
    let mut seen = h.change_seq();
    let mut moved = |h: &History| {
        let now = h.change_seq();
        let changed = now != seen;
        seen = now;
        changed
    };

    h.record(insert_cs(6, "a"), delete_cs(7, 1), sel_at(0), sel_at(1));
    assert!(moved(&h), "record");
    h.undo();
    assert!(moved(&h), "undo");
    assert!(h.undo().is_none());
    assert!(!moved(&h), "undo at the root");
    h.redo();
    assert!(moved(&h), "redo");
    assert!(h.redo().is_none());
    assert!(!moved(&h), "redo at a leaf");
    h.goto_revision(h.current);
    assert!(!moved(&h), "goto the current revision");
    h.undo();
    moved(&h);
    h.goto_revision(RevisionId(1));
    assert!(moved(&h), "goto another revision");
    h.undo_n(0);
    h.set_undo_levels(5);
    assert!(!moved(&h), "a zero-step walk and a cap change");
}
