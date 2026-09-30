use super::*;
use pretty_assertions::assert_eq;

// ── Pane selection sync ───────────────────────────────────────────────────────
//
// The engine pane's `selections` field must stay in sync with `doc.sels()` so
// the renderer always shows the correct cursor. `sync_all_pane_mirrors` is
// called once per frame in the run loop; these tests call it explicitly (as
// the run loop would) and verify the pane reflects the post-operation state.

/// Return the pane's primary cursor as an absolute char offset.
fn pane_head(ed: &Editor) -> hume_rope::offset::CharOffset {
    ed.view.panes[ed.state.focus.id()].selections[0]
        .cursor
        .offset()
}

/// After `c` (change): the selection is deleted and Insert mode entered; the
/// pane must reflect the post-deletion cursor, not the stale pre-deletion
/// selection.
#[test]
fn pane_selections_synced_after_change_command() {
    let mut ed = editor_from("-[hell]>o\n");
    ed.handle_key(key('c'));
    // `c` enters Insert; buffer is now "o\n" with cursor at char 0.
    assert_eq!(ed.state.mode(), Mode::Insert);

    // Simulate the per-frame sync that happens in the run loop.
    ed.sync_all_pane_mirrors(&ed.view.active_pane_ids());

    // Cursor must be at char offset 0 (start of "o\n").
    assert_eq!(
        pane_head(&ed),
        co(0),
        "pane head must be at char 0 after 'c' deletes selection"
    );
}

/// After typing a character in Insert mode: the pane cursor must advance,
/// which requires `apply_edit_grouped` to call `sync_all_pane_mirrors`.
#[test]
fn pane_selections_synced_after_insert_typing() {
    let mut ed = editor_from("-[a]>b\n");
    ed.handle_key(key('c')); // delete "a", enter Insert, cursor at byte 0
    ed.handle_key(key('x')); // type 'x', cursor advances past 'x' to byte 1

    ed.sync_all_pane_mirrors(&ed.view.active_pane_ids());

    // Text is now "xb\n"; cursor sits after 'x', at byte offset 1.
    assert_eq!(
        pane_head(&ed),
        co(1),
        "pane head must be at char 1 after typing 'x'"
    );
}

/// After `Esc` (exit Insert): pane must reflect the final cursor position,
/// which requires `end_insert_session` to call `sync_all_pane_mirrors`.
#[test]
fn pane_selections_synced_after_exit_insert() {
    let mut ed = editor_from("ab-[c]>\n");
    ed.handle_key(key('i')); // enter Insert at 'c' (byte 2)
    ed.handle_key(key('x')); // type 'x' before 'c' → "abxc\n", cursor at byte 3
    ed.handle_key(key_esc()); // exit Insert; select-inserted-text selects 'x'

    ed.sync_all_pane_mirrors(&ed.view.active_pane_ids());

    // 'x' was inserted at byte 2; Esc selects the typed run, so the (anchor
    // == head) selection's head sits back on 'x' itself, at byte 2.
    assert_eq!(
        pane_head(&ed),
        co(2),
        "pane head must be at char 2 (on 'x') after Esc"
    );
}

/// When the primary selection is NOT the earliest in the document,
/// `pane.primary_idx` must still identify the actual primary (not the
/// earliest).
///
/// Iterating sorted by position without preserving `primary_idx` would make
/// the engine treat the earliest selection as primary.
#[test]
fn pane_selections_primary_is_first_even_when_not_earliest() {
    let mut ed = editor_from("-[a]>b\n");

    // Two cursors: one at "a" (char 0) and one at "b" (char 1).
    // Primary is index 1: the "b" cursor, which is LATER in document order.
    let two_sels = sels_at(ed.doc().text(), &[(0, 0), (1, 1)], 1);
    ed.set_current_selections(two_sels);

    // Simulate the per-frame sync.
    ed.sync_all_pane_mirrors(&ed.view.active_pane_ids());

    // Selections are passed in sorted document order; a flag marks the primary.
    let pane = &ed.view.panes[ed.state.focus.id()];
    assert_eq!(
        pane.selections[0].cursor.offset(),
        co(0),
        "pane.selections[0] is the earliest in document order (char 0, 'a')"
    );
    assert_eq!(
        pane.selections[1].cursor.offset(),
        co(1),
        "pane.selections[1] is 'b' at char 1"
    );
    assert!(
        pane.selections[1].is_primary && !pane.selections[0].is_primary,
        "the primary flag must be on 'b' (index 1)"
    );
}

/// A backward selection whose far end is `e` plus a combining accent paints
/// that whole cluster and stops before the next one: the mirror's covered
/// range ends past the accent, not on the cluster's first char.
#[test]
fn backward_selection_ending_on_a_combining_cluster_paints_the_whole_cluster() {
    let mut ed = editor_from("a<[be\u{301}]-c\n");
    ed.view.theme = crate::testing::build_snapshot_theme();
    let snap =
        super::render_snapshot::render_to_styled_string(&mut ed, hume_grid::Rect::new(0, 0, 20, 3));
    insta::assert_snapshot!(snap);
}
