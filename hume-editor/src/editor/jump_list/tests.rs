use super::*;
use hume_editing::selection::{Selection, SelectionSet};

/// Helper: build a JumpEntry with a cursor at `char_pos` on `line`.
/// Bypasses `JumpEntry::new` since unit tests don't have a BufferText.
fn entry(char_pos: usize, line: usize) -> JumpEntry {
    JumpEntry {
        buffer_id: hume_engine::pipeline::BufferId::default(),
        selections: SelectionSet::single(Selection::collapsed(char_pos)),
        primary_line: line,
    }
}

#[test]
fn push_and_backward() {
    let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
    jl.push(entry(0, 0));
    jl.push(entry(10, 5));
    jl.push(entry(20, 10));

    let current = entry(30, 15);
    let e = jl.backward(current).unwrap();
    assert_eq!(e.primary_line, 10);

    let e = jl.backward(entry(0, 0)).unwrap();
    assert_eq!(e.primary_line, 5);

    let e = jl.backward(entry(0, 0)).unwrap();
    assert_eq!(e.primary_line, 0);

    assert!(jl.backward(entry(0, 0)).is_none());
}

#[test]
fn forward_after_backward() {
    let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
    jl.push(entry(0, 0));
    jl.push(entry(10, 5));
    jl.push(entry(20, 10));

    let current = entry(30, 15);
    jl.backward(current).unwrap();
    jl.backward(entry(0, 0)).unwrap();

    let e = jl.forward().unwrap();
    assert_eq!(e.primary_line, 10);

    let e = jl.forward().unwrap();
    assert_eq!(e.primary_line, 15);

    assert!(jl.forward().is_none());
}

#[test]
fn truncation_on_new_push() {
    let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
    jl.push(entry(0, 0));
    jl.push(entry(10, 5));
    jl.push(entry(20, 10));

    jl.backward(entry(30, 15)).unwrap();
    jl.backward(entry(0, 0)).unwrap();

    // New jump from here — forward history (lines 10, 15) is discarded.
    jl.push(entry(50, 25));

    assert!(jl.forward().is_none());

    let e = jl.backward(entry(60, 30)).unwrap();
    assert_eq!(e.primary_line, 25);

    let e = jl.backward(entry(0, 0)).unwrap();
    assert_eq!(e.primary_line, 0);

    assert!(jl.backward(entry(0, 0)).is_none());
}

#[test]
fn capacity_cap() {
    const CAP: usize = DEFAULT_JUMP_LIST_CAPACITY;
    let mut jl = JumpList::new(CAP);
    for i in 0..=CAP {
        jl.push(entry(i * 10, i));
    }
    assert_eq!(jl.len(), CAP);

    let e = jl.backward(entry(9999, 9999)).unwrap();
    assert_eq!(e.primary_line, CAP);

    let mut oldest = e.primary_line;
    while let Some(e) = jl.backward(entry(0, 0)) {
        oldest = e.primary_line;
    }
    // `backward`'s own "save current position" append enforces the same cap
    // as `push` — it evicts one more entry (line 1) to make room for the
    // saved position, so the true oldest survivor is line 2, not line 1.
    // Fail oracle: dropping that trim (leaving `backward` free to grow the
    // list to `CAP + 1`) would keep line 1 reachable and this assertion red.
    assert_eq!(oldest, 2);
}

#[test]
fn set_capacity_defers_trim_to_next_push() {
    // Fail oracle: if set_capacity trimmed immediately, jl.len() would drop
    // to 2 right after the call instead of only on the next push.
    let mut jl = JumpList::new(10);
    for i in 0..5 {
        jl.push(entry(i * 10, i));
    }
    assert_eq!(jl.len(), 5);

    jl.set_capacity(2);
    assert_eq!(jl.len(), 5, "lowering the cap must not retroactively trim");

    // The overshoot (5 -> new cap 2) is more than one entry — a single push
    // must still converge to the cap in this one call, proving push's trim
    // loop is a `while`, not an `if`.
    jl.push(entry(50, 5));
    assert_eq!(jl.len(), 2);

    // `backward`'s own "save current position" append enforces the same cap
    // as `push` — with capacity 2 already full (line 4, line 5), saving the
    // current position evicts line 4 to make room, so only one step back is
    // reachable. Fail oracle: without that trim, the list would transiently
    // hold 3 entries and line 4 would still be reachable as a second step.
    let e = jl.backward(entry(9999, 9999)).unwrap();
    assert_eq!(
        e.primary_line, 5,
        "the surviving pushed entry is the newest push"
    );
    assert!(
        jl.backward(entry(0, 0)).is_none(),
        "capacity 2 holds only the newest push and the saved current position \
         — line 4 must have been evicted to make room, not line 5"
    );
}

#[test]
fn set_capacity_shrink_converges_on_a_deduplicated_push() {
    // Fail oracle: if the dedup branch returned before the trim loop, a
    // shrink would only converge on a push that landed a genuinely new
    // entry — never on one that overwrote the last entry in place.
    let mut jl = JumpList::new(10);
    for i in 0..5 {
        jl.push(entry(i * 10, i));
    }
    assert_eq!(jl.len(), 5);

    jl.set_capacity(2);

    // Same line AND buffer as the last push (line 4) — hits the dedup
    // branch, not the plain append.
    jl.push(entry(50, 4));
    assert_eq!(
        jl.len(),
        2,
        "a deduplicated push must still converge to the shrunk cap"
    );
}

#[test]
fn set_capacity_raising_it_does_not_drop_entries() {
    let mut jl = JumpList::new(2);
    jl.push(entry(0, 0));
    jl.push(entry(10, 1));
    assert_eq!(jl.len(), 2);

    jl.set_capacity(10);
    assert_eq!(
        jl.len(),
        2,
        "raising the cap must not trim existing entries"
    );

    jl.push(entry(20, 2));
    assert_eq!(
        jl.len(),
        3,
        "new capacity must take effect on the next push"
    );
}

/// A shrink immediately followed by a raise, with no push in between, must
/// not lose entries in the transient window — `:reload-config` resets
/// `jump-list-capacity` to its compiled-in default before `init.scm`
/// re-raises it, and an eager trim would have discarded everything past the
/// default before the raise had a chance to take effect. Drives the shrink
/// and the raise directly against `JumpList` (no `Editor`/`:reload-config`
/// involved) to isolate that exact sequence.
#[test]
fn shrink_then_raise_with_no_push_between_resurrects_every_entry() {
    const OVER_DEFAULT: usize = DEFAULT_JUMP_LIST_CAPACITY + 200;
    let mut jl = JumpList::new(OVER_DEFAULT);
    for i in 0..OVER_DEFAULT {
        jl.push(entry(i * 10, i));
    }
    assert_eq!(jl.len(), OVER_DEFAULT);

    // The reset: shrink to the compiled-in default. Deferred — no trim yet.
    jl.set_capacity(DEFAULT_JUMP_LIST_CAPACITY);
    assert_eq!(
        jl.len(),
        OVER_DEFAULT,
        "shrinking must not eagerly trim — nothing has pushed since"
    );

    // init.scm re-raising the setting. Still no push — nothing to converge.
    // Fail oracle: make `set_capacity` (or `push`'s trim) eager instead of
    // deferred and the first `set_capacity` call above would have already
    // dropped every entry past 100 — raising the cap back up here can't
    // resurrect what an eager trim already discarded, so `len()` would stay
    // at 100 instead of climbing back to `OVER_DEFAULT`.
    jl.set_capacity(OVER_DEFAULT);
    assert_eq!(
        jl.len(),
        OVER_DEFAULT,
        "raising the cap back up before any push must resurrect every entry, \
         not just whatever survived a (nonexistent) eager trim"
    );
}

#[test]
fn deduplication() {
    let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
    jl.push(entry(0, 5));
    jl.push(entry(3, 5)); // same line — replaces
    assert_eq!(jl.len(), 1);

    jl.push(entry(20, 10));
    let e = jl.backward(entry(99, 99)).unwrap();
    assert_eq!(e.primary_line, 10);
    let e = jl.backward(entry(0, 0)).unwrap();
    assert_eq!(e.primary_line, 5);
    assert_eq!(e.selections.primary().head(), 3);
}

#[test]
fn empty_list() {
    let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
    assert!(jl.backward(entry(0, 0)).is_none());
    assert!(jl.forward().is_none());
}

#[test]
fn backward_after_returning_to_present() {
    let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
    jl.push(entry(0, 0));

    // Go backward, then forward back to the saved "present" entry.
    jl.backward(entry(50, 10)).unwrap();
    jl.forward().unwrap();

    // Now backward again. Since cursor is at the last entry (the saved
    // "present"), not past it, the new current position is NOT saved —
    // matching Vim/Helix: the present is only captured when first entering
    // the jump list from a fresh editing state.
    let e = jl.backward(entry(80, 20)).unwrap();
    assert_eq!(
        e.primary_line, 0,
        "traverses existing history without saving new position"
    );

    // Forward returns to the previously saved "present" (line 10).
    let e = jl.forward().unwrap();
    assert_eq!(e.primary_line, 10);
    assert!(jl.forward().is_none());
}

#[test]
fn backward_saves_current_position() {
    let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
    jl.push(entry(0, 0));

    let e = jl.backward(entry(50, 10)).unwrap();
    assert_eq!(e.primary_line, 0);

    let e = jl.forward().unwrap();
    assert_eq!(e.primary_line, 10);
}

// ── prune_buffer cursor-adjustment arithmetic ─────────────────────────────

/// Helper to create a JumpEntry for a specific BufferId (for prune tests).
fn entry_for(char_pos: usize, line: usize, bid: BufferId) -> JumpEntry {
    JumpEntry {
        buffer_id: bid,
        selections: SelectionSet::single(Selection::collapsed(char_pos)),
        primary_line: line,
    }
}

/// Helper: allocate two distinct real BufferIds via a temporary SlotMap.
fn two_buffer_ids() -> (BufferId, BufferId) {
    let mut sm: slotmap::SlotMap<BufferId, ()> = slotmap::SlotMap::with_key();
    let a = sm.insert(());
    let b = sm.insert(());
    (a, b)
}

/// Cursor decrements by the number of pruned entries that were before it.
#[test]
fn prune_buffer_decrements_cursor_by_removed_before_count() {
    let (bid_a, bid_b) = two_buffer_ids();
    let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
    // [A:0, B:1, A:2, B:3]  cursor=4 (at present)
    jl.push(entry_for(0, 0, bid_a));
    jl.push(entry_for(1, 1, bid_b));
    jl.push(entry_for(2, 2, bid_a));
    jl.push(entry_for(3, 3, bid_b));

    // Prune A: removes indices 0 and 2 (both before cursor=4).
    // remaining = [B:1, B:3], cursor = 4 − 2 = 2 (at present of 2-entry list).
    jl.prune_buffer(bid_a);

    assert_eq!(jl.len(), 2);
    assert_eq!(
        jl.cursor, 2,
        "cursor clamped to end after removing 2 entries before it"
    );
    assert!(!jl.entries_for_buffer(bid_a));
}

/// When the cursor points mid-list and only entries AFTER it are pruned,
/// the cursor position is unchanged.
#[test]
fn prune_buffer_leaves_cursor_unchanged_when_removed_entries_are_after() {
    let (bid_a, bid_b) = two_buffer_ids();
    let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
    // [B:0, B:1, A:2, A:3]  cursor=2 (mid-list, pointing at A:2)
    jl.push(entry_for(0, 0, bid_b));
    jl.push(entry_for(1, 1, bid_b));
    jl.push(entry_for(2, 2, bid_a));
    jl.push(entry_for(3, 3, bid_a));
    jl.cursor = 2; // position mid-list manually

    // Prune A: removes indices 2 and 3 (both at/after cursor=2, so 0 are before).
    // remaining = [B:0, B:1], cursor = 2 − 0 = 2, then clamped to min(2, 2) = 2.
    jl.prune_buffer(bid_a);

    assert_eq!(jl.len(), 2);
    assert_eq!(jl.cursor, 2, "cursor clamped to list len (= at present)");
}

/// `saturating_sub` prevents underflow: removing all entries before cursor=0 is a no-op.
#[test]
fn prune_buffer_saturating_sub_at_zero_cursor() {
    let (bid_a, bid_b) = two_buffer_ids();
    let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
    jl.push(entry_for(0, 0, bid_b));
    jl.push(entry_for(1, 1, bid_a));
    jl.cursor = 0; // at oldest entry

    // Prune A: only the entry at index 1 is removed; 0 were before cursor=0.
    jl.prune_buffer(bid_a);

    assert_eq!(jl.len(), 1);
    assert_eq!(
        jl.cursor, 0,
        "cursor stays at 0 — saturating_sub prevents underflow"
    );
}

/// When all entries belong to the pruned buffer, list and cursor both become 0.
#[test]
fn prune_buffer_all_entries_removed_resets_cursor() {
    let (bid_a, _bid_b) = two_buffer_ids();
    let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
    jl.push(entry_for(0, 0, bid_a));
    jl.push(entry_for(1, 1, bid_a));
    jl.push(entry_for(2, 2, bid_a));
    // cursor = 3 (at present)

    jl.prune_buffer(bid_a);

    assert_eq!(jl.len(), 0);
    assert_eq!(jl.cursor, 0, "cursor = 0 when all entries removed");
}

/// After pruning, backward/forward still work correctly on the remaining entries.
#[test]
fn prune_buffer_remaining_entries_navigable() {
    let (bid_a, bid_b) = two_buffer_ids();
    let mut jl = JumpList::new(DEFAULT_JUMP_LIST_CAPACITY);
    jl.push(entry_for(0, 0, bid_b));
    jl.push(entry_for(1, 1, bid_a));
    jl.push(entry_for(2, 2, bid_b));
    // cursor = 3

    jl.prune_buffer(bid_a);
    // remaining = [B:0, B:2], cursor = 2 (3 − 1 removed before = 2, clamped to min(2,2))

    let e = jl.backward(entry_for(99, 99, bid_b)).unwrap();
    assert_eq!(
        e.primary_line, 2,
        "backward from present lands on last remaining entry"
    );
    let e = jl.backward(entry_for(0, 0, bid_b)).unwrap();
    assert_eq!(e.primary_line, 0, "backward again reaches the oldest entry");
}
