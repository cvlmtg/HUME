use super::*;
use pretty_assertions::assert_eq;

// ── Pane selection sync (Bug 3) ──────────────────────────────────────────────
//
// The engine pane's `selections` field must stay in sync with `doc.sels()` so
// the renderer always shows the correct cursor. `sync_all_pane_mirrors` is
// called once per frame in the run loop; these tests call it explicitly (as
// the run loop would) and verify the pane reflects the post-operation state.

/// Return the pane's primary cursor as an absolute char offset — the engine's
/// representation after Phase 2 unified the selection types.
fn pane_head(ed: &Editor) -> usize {
    ed.engine_view.panes[ed.focused_pane_id].selections[0].head
}

/// After `c` (change): the selection is deleted and Insert mode entered.
/// Before the fix, the pane still held the pre-deletion selection after `c`.
#[test]
fn pane_selections_synced_after_change_command() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('c'));
    // `c` enters Insert; buffer is now "o\n" with cursor at char 0.
    assert_eq!(ed.mode, Mode::Insert);

    // Simulate the per-frame sync that happens in the run loop.
    ed.sync_all_pane_mirrors();

    // Cursor must be at char offset 0 (start of "o\n").
    assert_eq!(
        pane_head(&ed),
        0,
        "pane head must be at char 0 after 'c' deletes selection"
    );
}

/// After typing a character in Insert mode: the pane cursor must advance.
/// Before the fix, `apply_edit_grouped` never called `sync_all_pane_mirrors`.
#[test]
fn pane_selections_synced_after_insert_typing() {
    let mut ed = editor_from("-[a]>b\n");
    ed.handle_key(key('c')); // delete "a", enter Insert — cursor at byte 0
    ed.handle_key(key('x')); // type 'x' — cursor advances past 'x' to byte 1

    ed.sync_all_pane_mirrors();

    // Text is now "xb\n"; cursor sits after 'x', at byte offset 1.
    assert_eq!(
        pane_head(&ed),
        1,
        "pane head must be at char 1 after typing 'x'"
    );
}

/// After `Esc` (exit Insert): pane must reflect the final cursor position.
/// Before the fix, `end_insert_session` never called `sync_all_pane_mirrors`.
#[test]
fn pane_selections_synced_after_exit_insert() {
    let mut ed = editor_from("ab-[c]>\n");
    ed.handle_key(key('i')); // enter Insert at 'c' (byte 2)
    ed.handle_key(key('x')); // type 'x' before 'c' → "abxc\n", cursor at byte 3
    ed.handle_key(key_esc()); // exit Insert

    ed.sync_all_pane_mirrors();

    // 'x' was inserted at byte 2; cursor now sits just after 'x' at byte 3.
    assert_eq!(
        pane_head(&ed),
        3,
        "pane head must be at char 3 (after 'x') after Esc"
    );
}

/// When the primary selection is NOT the earliest in the document,
/// `pane.selections[0]` must still be the primary (not the earliest).
///
/// Before the fix, `sync_all_pane_mirrors` used `iter_sorted()`, which lost
/// primary info, so the engine always treated the earliest selection as primary.
#[test]
fn pane_selections_primary_is_first_even_when_not_earliest() {
    use crate::core::selection::{Selection, SelectionSet};

    let mut ed = editor_from("-[a]>b\n");

    // Two cursors: one at "a" (char 0) and one at "b" (char 1).
    // Primary is index 1 — the "b" cursor, which is LATER in document order.
    let two_sels = SelectionSet::from_vec(
        vec![
            Selection::collapsed(0), // at "a" — NOT primary
            Selection::collapsed(1), // at "b" — IS primary
        ],
        1,
    );
    ed.set_current_selections(two_sels);

    // Simulate the per-frame sync.
    ed.sync_all_pane_mirrors();

    // Selections are passed in sorted document order; primary_idx identifies the primary.
    let pane = &ed.engine_view.panes[ed.focused_pane_id];
    assert_eq!(
        pane.selections[0].head, 0,
        "pane.selections[0] is the earliest in document order (char 0, 'a')"
    );
    assert_eq!(
        pane.selections[1].head, 1,
        "pane.selections[1] is 'b' at char 1"
    );
    assert_eq!(
        pane.primary_idx, 1,
        "primary_idx must point to 'b' (index 1)"
    );
}

/// Backward selections (head < anchor) can cause start()-order to differ from
/// head-order. Before the fix, pane selections were passed in start()-order, which
/// triggered the engine's `debug_assert!(selections sorted by head)`.
///
/// Reproduction: two selections where their start() order differs from head order:
///   A: anchor=10, head=3  → start()=3, head=3   (backward)
///   B: anchor=0,  head=8  → start()=0, head=8   (forward)
/// start() order: [B(0), A(3)]  → heads [8, 3]  — NOT sorted → panic
/// head  order:   [A(3), B(8)]  → heads [3, 8]  — sorted     → OK
#[test]
fn pane_selections_sorted_by_head_not_start() {
    use crate::core::selection::{Selection, SelectionSet};

    // Text needs at least 11 chars. The -[h]> marker satisfies editor_from's
    // requirement; we replace the selection set immediately after.
    let mut ed = editor_from("-[h]>ello world\n");

    // A: backward selection, anchor=10, head=3  → start()=3
    // B: forward  selection, anchor=0,  head=8  → start()=0
    // In start() order: [B, A].  In head order: [A, B].
    let two_sels = SelectionSet::from_vec(
        vec![
            Selection {
                anchor: 10,
                head: 3,
                horiz: None,
            }, // A — primary
            Selection {
                anchor: 0,
                head: 8,
                horiz: None,
            }, // B
        ],
        0, // primary is A
    );
    ed.set_current_selections(two_sels);

    ed.sync_all_pane_mirrors();

    let pane = &ed.engine_view.panes[ed.focused_pane_id];
    // After sort-by-head: [A(head=3), B(head=8)]
    assert_eq!(pane.selections[0].head, 3, "first in head order is A");
    assert_eq!(pane.selections[1].head, 8, "second in head order is B");
    // Primary (A) ends up at index 0 after sorting.
    assert_eq!(
        pane.primary_idx, 0,
        "primary_idx follows A to its new position"
    );
}
