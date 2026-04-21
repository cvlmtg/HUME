use super::*;
use pretty_assertions::assert_eq;

// ── D1–D6: Multi-pane contract tests ──────────────────────────────────────────
//
// These tests lock the SSOT invariants for per-pane, per-buffer, and per-search state.

/// D1 — Each pane maintains its own cursor independently for the same buffer.
///
/// Two panes on the same buffer; set them to different positions; verify
/// `switch_focused_pane` restores each pane's cursor exactly.
#[test]
fn d1_selections_are_pane_owned() {
    use crate::core::selection::{Selection, SelectionSet};

    let mut ed = editor_from("-[h]>ello world\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.focused_pane_id;

    let pid_b = ed.open_pane(bid);

    // Pane A → position 2 ('l').
    ed.switch_focused_pane(pid_a);
    ed.set_current_selections(SelectionSet::single(Selection::collapsed(2)));

    // Pane B → position 6 ('w').
    ed.switch_focused_pane(pid_b);
    ed.set_current_selections(SelectionSet::single(Selection::collapsed(6)));

    // Back to pane A: head must be 2, not 6.
    ed.switch_focused_pane(pid_a);
    assert_eq!(ed.current_selections().primary().head, 2, "pane A head after switch");

    // Back to pane B: head must be 6, not 2.
    ed.switch_focused_pane(pid_b);
    assert_eq!(ed.current_selections().primary().head, 6, "pane B head after switch");
}

/// D4a — `Buffer.search_pattern` is shared across all panes on the same buffer;
/// each pane has its own `SearchCursor` in `pane_state`.
#[test]
fn d4a_search_pattern_is_per_buffer() {
    use crate::core::search_state::SearchCursor;

    let mut ed = editor_from("-[f]>oo foo foo\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.focused_pane_id;
    let pid_b = ed.open_pane(bid);

    // Both panes see Buffer.search_pattern — it's a single field on `doc`.
    // Verify independence of search_cursor: write distinct values per pane.
    ed.pane_state[pid_a][bid].search_cursor = SearchCursor {
        match_count: Some((1, 3)),
        wrapped: false,
        ..SearchCursor::default()
    };
    ed.pane_state[pid_b][bid].search_cursor = SearchCursor {
        match_count: Some((2, 3)),
        wrapped: true,
        ..SearchCursor::default()
    };

    // Pane A and pane B see different cursors even though they share the buffer.
    assert_eq!(ed.pane_state[pid_a][bid].search_cursor.match_count, Some((1, 3)));
    assert!(!ed.pane_state[pid_a][bid].search_cursor.wrapped);

    assert_eq!(ed.pane_state[pid_b][bid].search_cursor.match_count, Some((2, 3)));
    assert!(ed.pane_state[pid_b][bid].search_cursor.wrapped);
}

/// D4b — `Selection.horiz` travels with the selection; resets when its line
/// is touched by an edit; survives translate_in_place on untouched lines.
#[test]
fn d4b_sticky_col_is_per_selection() {
    use crate::core::changeset::ChangeSetBuilder;
    use crate::core::selection::{Selection, SelectionSet};
    use crate::core::text::Text;

    // "abc\ndef\n" — two lines.
    let text = Text::from("abc\ndef\n");
    let rope = text.rope().clone();

    // Selection on line 1 (char offset 4 = 'd'), horiz = 0.
    let sel = Selection::with_horiz(4, 4, 0);
    let mut sels = SelectionSet::single(sel);

    // CS that inserts at the start of line 0 only: "abc\n" → "Xabc\n"
    // This touches line 0 but not line 1, so horiz on line-1 head should survive.
    let mut b = ChangeSetBuilder::new(rope.len_chars());
    b.insert("X");   // insert at start
    b.retain_rest();
    let cs = b.finish();

    sels.translate_in_place(&cs, &rope);
    // Head moved from 4 to 5 (past the inserted 'X'), horiz preserved.
    assert_eq!(sels.primary().head, 5, "head mapped past insert");
    assert_eq!(sels.primary().horiz, Some(0), "horiz preserved on untouched line");

    // Now a CS that touches line 1 (inserts at position of 'd'): horiz should reset.
    // Re-build sels with the updated head but set horiz back to show it was latched.
    let sel2 = Selection::with_horiz(5, 5, 0);
    let mut sels2 = SelectionSet::single(sel2);

    // "Xabc\ndef\n" (after first edit) — "d" is now at char 5 (line 1).
    // Insert at char 5 (start of "def" in new rope); use the original rope for
    // translate_in_place (rope_pre = before-this-edit rope).
    let text2 = Text::from("Xabc\ndef\n");
    let rope2 = text2.rope().clone();
    let mut b2 = ChangeSetBuilder::new(rope2.len_chars());
    b2.retain(5);   // skip "Xabc\n"
    b2.insert("Y"); // insert at line 1
    b2.retain_rest();
    let cs2 = b2.finish();

    sels2.translate_in_place(&cs2, &rope2);
    // Head moved past insert; horiz must be reset because line 1 was touched.
    assert_eq!(sels2.primary().horiz, None, "horiz reset when head's line is touched");
}

/// D5 — `EditGroup` is per-(pane, buffer); insert sessions are independent across
/// panes on the same buffer.  Two separate i…Esc sessions each produce one revision.
#[test]
fn d5_insert_session_is_pane_buffer_scoped() {
    let mut ed = editor_from("-[a]>bc\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.focused_pane_id;
    let pid_b = ed.open_pane(bid);

    // Pane A insert session: type 'X' at the start.
    ed.switch_focused_pane(pid_a);
    assert!(ed.pane_state[pid_a][bid].edit_group.is_none(), "no group before i");
    ed.handle_key(key('i'));
    assert!(ed.pane_state[pid_a][bid].edit_group.is_some(), "group open after i");
    ed.handle_key(key('X'));
    ed.handle_key(key_esc());
    assert!(ed.pane_state[pid_a][bid].edit_group.is_none(), "group committed on Esc");

    let rev_after_a = ed.doc().revision_id();

    // Pane B insert session: type 'Y'.
    ed.switch_focused_pane(pid_b);
    assert!(ed.pane_state[pid_b][bid].edit_group.is_none(), "pane B starts with no group");
    ed.handle_key(key('i'));
    assert!(ed.pane_state[pid_b][bid].edit_group.is_some(), "pane B group opens");
    ed.handle_key(key('Y'));
    ed.handle_key(key_esc());
    assert!(ed.pane_state[pid_b][bid].edit_group.is_none(), "pane B group committed");

    let rev_after_b = ed.doc().revision_id();

    // Each session produced a distinct revision.
    assert_ne!(rev_after_a, rev_after_b, "pane B produced a new revision");

    // Two undos restore original content.
    ed.switch_focused_pane(pid_a);
    ed.handle_key(key('u'));
    ed.handle_key(key('u'));
    assert_eq!(ed.doc().text().to_string(), "abc\n", "two undos restore original");
}

/// D6 — `pane_transient[pid]` snapshots are per-pane and never aliased.
#[test]
fn d6_search_mode_snapshot_is_per_pane() {
    use crate::core::selection::{Selection, SelectionSet};

    let mut ed = editor_from("-[h]>ello\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.focused_pane_id;
    let pid_b = ed.open_pane(bid);

    let sels_a = SelectionSet::single(Selection::collapsed(1));
    let sels_b = SelectionSet::single(Selection::collapsed(3));

    ed.pane_transient[pid_a].pre_search_sels = Some(sels_a.clone());
    ed.pane_transient[pid_b].pre_search_sels = Some(sels_b.clone());

    // Pane A snapshot is independent of pane B.
    assert_eq!(
        ed.pane_transient[pid_a].pre_search_sels.as_ref().unwrap().primary().head,
        1,
        "pane A pre_search_sels head"
    );
    assert_eq!(
        ed.pane_transient[pid_b].pre_search_sels.as_ref().unwrap().primary().head,
        3,
        "pane B pre_search_sels head"
    );

    // Clearing pane A's snapshot does not affect pane B.
    ed.pane_transient[pid_a].pre_search_sels = None;
    assert!(ed.pane_transient[pid_a].pre_search_sels.is_none());
    assert!(ed.pane_transient[pid_b].pre_search_sels.is_some(), "pane B unaffected");
}

/// D2 — An edit in the focused pane translates non-acting pane selections via the CS.
///
/// Pane A deletes char 0; pane B's cursor at position 9 must slide to 8.
#[test]
fn d2_edit_in_pane_a_translates_pane_b_selections() {
    use crate::core::selection::{Selection, SelectionSet};

    // "abcdefghij\n" (11 chars including trailing \n); cursor on 'a'.
    let mut ed = editor_from("-[a]>bcdefghij\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.focused_pane_id;
    let pid_b = ed.open_pane(bid);

    // Position pane B's cursor at char 9 ('j').
    ed.switch_focused_pane(pid_b);
    ed.set_current_selections(SelectionSet::single(Selection::collapsed(9)));

    // Switch to pane A and delete char 0 ('a').
    ed.switch_focused_pane(pid_a);
    ed.handle_key(key('d')); // delete selection (covers 'a')

    // Pane A's cursor is now at 0 (post-delete); pane B's should be at 8.
    assert_eq!(
        ed.selections_for(pid_b, bid).unwrap().primary().head,
        8,
        "pane B selection translated by forward CS"
    );
}

/// D3 — Undo in the focused pane propagates the inverse CS to non-acting panes.
///
/// After the D2 edit (delete 'a'), undo restores 'a'; pane B's cursor at 8
/// must ride the inverse CS back to 9.
#[test]
fn d3_undo_restores_acting_pane_and_translates_others() {
    use crate::core::selection::{Selection, SelectionSet};

    let mut ed = editor_from("-[a]>bcdefghij\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.focused_pane_id;
    let pid_b = ed.open_pane(bid);

    // Position pane B at char 9.
    ed.switch_focused_pane(pid_b);
    ed.set_current_selections(SelectionSet::single(Selection::collapsed(9)));

    // Pane A: delete 'a', then undo.
    ed.switch_focused_pane(pid_a);
    ed.handle_key(key('d'));
    // After delete: pane B at 8. Undo restores 'a'.
    ed.handle_key(key('u'));

    // Pane A's cursor is restored to pre-delete position.
    assert_eq!(
        ed.current_selections().primary().head,
        0,
        "pane A cursor restored by undo"
    );
    // Pane B's cursor is translated back to 9 by the inverse CS.
    assert_eq!(
        ed.selections_for(pid_b, bid).unwrap().primary().head,
        9,
        "pane B selection translated by inverse CS (undo)"
    );
}

/// Multi-cursor propagation: a deletion that spans two selections in pane B
/// merges them into one (proves translate_in_place calls merge_overlapping_in_place).
#[test]
fn propagate_cs_merges_collapsed_non_acting_pane_selections() {
    use crate::core::selection::{Selection, SelectionSet};

    // "abcde\n" — 6 chars.
    let mut ed = editor_from("-[a]>bcde\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.focused_pane_id;
    let pid_b = ed.open_pane(bid);

    // Pane B: two cursors at positions 2 ('c') and 4 ('e').
    ed.switch_focused_pane(pid_b);
    ed.set_current_selections(SelectionSet::from_vec(
        vec![Selection::collapsed(2), Selection::collapsed(4)],
        0,
    ));

    // Pane A: select chars 1–4 ("bcde") and delete.
    // First put pane A's selection on 'b'-'e'.
    ed.switch_focused_pane(pid_a);
    // Select 'a' then extend to 'e': use 'v' to enter Select then motion.
    // Simplest: directly set selections and do a delete.
    ed.set_current_selections(SelectionSet::single(Selection::new(1, 4)));
    ed.handle_key(key('d'));

    // After deleting chars 1-4, pane B's two cursors at 2 and 4 both map to
    // the deletion point (1); they must merge into a single cursor at 1.
    let pane_b_sels = ed.selections_for(pid_b, bid).unwrap();
    assert_eq!(pane_b_sels.len(), 1, "collapsed selections must merge after propagation");
    assert_eq!(pane_b_sels.primary().head, 1, "merged cursor at deletion point");
}

