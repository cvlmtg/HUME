use super::*;
use crate::editor::commands::open_pane;
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
    use hume_editing::selection::{Selection, SelectionSet};

    let mut ed = editor_from("-[h]>ello world\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id;

    let pid_b = open_pane(&mut ed.state, &mut ed.view, bid);

    // Pane A → position 2 ('l').
    ed.switch_focused_pane(pid_a);
    ed.set_current_selections(SelectionSet::single(Selection::collapsed(2)));

    // Pane B → position 6 ('w').
    ed.switch_focused_pane(pid_b);
    ed.set_current_selections(SelectionSet::single(Selection::collapsed(6)));

    // Back to pane A: head must be 2, not 6.
    ed.switch_focused_pane(pid_a);
    assert_eq!(
        ed.current_selections().primary().head(),
        2,
        "pane A head after switch"
    );

    // Back to pane B: head must be 6, not 2.
    ed.switch_focused_pane(pid_b);
    assert_eq!(
        ed.current_selections().primary().head(),
        6,
        "pane B head after switch"
    );
}

/// D4a — `Buffer.search_pattern` is shared across all panes on the same buffer;
/// each pane has its own `SearchCursor` in `pane_state`.
#[test]
fn d4a_search_pattern_is_per_buffer() {
    use crate::editor::search::SearchCursor;

    let mut ed = editor_from("-[f]>oo foo foo\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id;
    let pid_b = open_pane(&mut ed.state, &mut ed.view, bid);

    // Both panes see Buffer.search_pattern — it's a single field on `doc`.
    // Verify independence of search_cursor: write distinct values per pane.
    ed.state.panes.state[pid_a][bid].search_cursor = SearchCursor {
        match_count: Some((1, 3)),
        wrapped: false,
        ..SearchCursor::default()
    };
    ed.state.panes.state[pid_b][bid].search_cursor = SearchCursor {
        match_count: Some((2, 3)),
        wrapped: true,
        ..SearchCursor::default()
    };

    // Pane A and pane B see different cursors even though they share the buffer.
    assert_eq!(
        ed.state.panes.state[pid_a][bid].search_cursor.match_count,
        Some((1, 3))
    );
    assert!(!ed.state.panes.state[pid_a][bid].search_cursor.wrapped);

    assert_eq!(
        ed.state.panes.state[pid_b][bid].search_cursor.match_count,
        Some((2, 3))
    );
    assert!(ed.state.panes.state[pid_b][bid].search_cursor.wrapped);
}

/// D4b — `Selection.horiz` travels with the selection; resets when its line
/// is touched by an edit; survives translate_in_place on untouched lines.
#[test]
fn d4b_sticky_col_is_per_selection() {
    use hume_editing::changeset::ChangeSetBuilder;
    use hume_editing::selection::{Selection, SelectionSet};
    use hume_editing::text::Text;

    // "abc\ndef\n" — two lines.
    let text = Text::from("abc\ndef\n");

    // Selection on line 1 (char offset 4 = 'd'), horiz = 0.
    let sel = Selection::with_horiz(4, 4, 0);
    let mut sels = SelectionSet::single(sel);

    // CS that inserts at the start of line 0 only: "abc\n" → "Xabc\n"
    // This touches line 0 but not line 1, so horiz on line-1 head should survive.
    let mut b = ChangeSetBuilder::new(text.len_chars());
    b.insert("X"); // insert at start
    b.retain_rest();
    let cs = b.finish();

    sels.translate_in_place(&cs, &text);
    // Head moved from 4 to 5 (past the inserted 'X'), horiz preserved.
    assert_eq!(sels.primary().head(), 5, "head mapped past insert");
    assert_eq!(
        sels.primary().horiz(),
        Some(0),
        "horiz preserved on untouched line"
    );

    // Now a CS that touches line 1 (inserts at position of 'd'): horiz should reset.
    // Re-build sels with the updated head but set horiz back to show it was latched.
    let sel2 = Selection::with_horiz(5, 5, 0);
    let mut sels2 = SelectionSet::single(sel2);

    // "Xabc\ndef\n" (after first edit) — "d" is now at char 5 (line 1).
    // Insert at char 5 (start of "def" in new rope); use the pre-edit Text for
    // translate_in_place (buf_pre = before-this-edit text).
    let text2 = Text::from("Xabc\ndef\n");
    let mut b2 = ChangeSetBuilder::new(text2.len_chars());
    b2.retain(5); // skip "Xabc\n"
    b2.insert("Y"); // insert at line 1
    b2.retain_rest();
    let cs2 = b2.finish();

    sels2.translate_in_place(&cs2, &text2);
    // Head moved past insert; horiz must be reset because line 1 was touched.
    assert_eq!(
        sels2.primary().horiz(),
        None,
        "horiz reset when head's line is touched"
    );
}

/// D5 — `EditGroup` is per-(pane, buffer); insert sessions are independent across
/// panes on the same buffer.  Two separate i…Esc sessions each produce one revision.
#[test]
fn d5_insert_session_is_pane_buffer_scoped() {
    let mut ed = editor_from("-[a]>bc\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id;
    let pid_b = open_pane(&mut ed.state, &mut ed.view, bid);

    // Pane A insert session: type 'X' at the start.
    ed.switch_focused_pane(pid_a);
    assert!(
        ed.state.panes.state[pid_a][bid].edit_group.is_none(),
        "no group before i"
    );
    ed.handle_key(key('i'));
    assert!(
        ed.state.panes.state[pid_a][bid].edit_group.is_some(),
        "group open after i"
    );
    ed.handle_key(key('X'));
    ed.handle_key(key_esc());
    assert!(
        ed.state.panes.state[pid_a][bid].edit_group.is_none(),
        "group committed on Esc"
    );

    let rev_after_a = ed.doc().revision_id();

    // Pane B insert session: type 'Y'.
    ed.switch_focused_pane(pid_b);
    assert!(
        ed.state.panes.state[pid_b][bid].edit_group.is_none(),
        "pane B starts with no group"
    );
    ed.handle_key(key('i'));
    assert!(
        ed.state.panes.state[pid_b][bid].edit_group.is_some(),
        "pane B group opens"
    );
    ed.handle_key(key('Y'));
    ed.handle_key(key_esc());
    assert!(
        ed.state.panes.state[pid_b][bid].edit_group.is_none(),
        "pane B group committed"
    );

    let rev_after_b = ed.doc().revision_id();

    // Each session produced a distinct revision.
    assert_ne!(rev_after_a, rev_after_b, "pane B produced a new revision");

    // Two undos restore original content.
    ed.switch_focused_pane(pid_a);
    ed.handle_key(key('u'));
    ed.handle_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        "abc\n",
        "two undos restore original"
    );
}

/// D6 — `panes.transient[pid]` snapshots are per-pane and never aliased.
#[test]
fn d6_search_mode_snapshot_is_per_pane() {
    use hume_editing::selection::{Selection, SelectionSet};

    let mut ed = editor_from("-[h]>ello\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id;
    let pid_b = open_pane(&mut ed.state, &mut ed.view, bid);

    let sels_a = SelectionSet::single(Selection::collapsed(1));
    let sels_b = SelectionSet::single(Selection::collapsed(3));

    ed.state.panes.transient[pid_a].pre_search_sels = Some(sels_a.clone());
    ed.state.panes.transient[pid_b].pre_search_sels = Some(sels_b.clone());

    // Pane A snapshot is independent of pane B.
    assert_eq!(
        ed.state.panes.transient[pid_a]
            .pre_search_sels
            .as_ref()
            .unwrap()
            .primary()
            .head(),
        1,
        "pane A pre_search_sels head"
    );
    assert_eq!(
        ed.state.panes.transient[pid_b]
            .pre_search_sels
            .as_ref()
            .unwrap()
            .primary()
            .head(),
        3,
        "pane B pre_search_sels head"
    );

    // Clearing pane A's snapshot does not affect pane B.
    ed.state.panes.transient[pid_a].pre_search_sels = None;
    assert!(ed.state.panes.transient[pid_a].pre_search_sels.is_none());
    assert!(
        ed.state.panes.transient[pid_b].pre_search_sels.is_some(),
        "pane B unaffected"
    );
}

/// D2 — An edit in the focused pane translates non-acting pane selections via the CS.
///
/// Pane A deletes char 0; pane B's cursor at position 9 must slide to 8.
#[test]
fn d2_edit_in_pane_a_translates_pane_b_selections() {
    use hume_editing::selection::{Selection, SelectionSet};

    // "abcdefghij\n" (11 chars including trailing \n); cursor on 'a'.
    let mut ed = editor_from("-[a]>bcdefghij\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id;
    let pid_b = open_pane(&mut ed.state, &mut ed.view, bid);

    // Position pane B's cursor at char 9 ('j').
    ed.switch_focused_pane(pid_b);
    ed.set_current_selections(SelectionSet::single(Selection::collapsed(9)));

    // Switch to pane A and delete char 0 ('a').
    ed.switch_focused_pane(pid_a);
    ed.handle_key(key('d')); // delete selection (covers 'a')

    // Pane A's cursor is now at 0 (post-delete); pane B's should be at 8.
    assert_eq!(
        ed.selections_for(pid_b, bid).unwrap().primary().head(),
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
    use hume_editing::selection::{Selection, SelectionSet};

    let mut ed = editor_from("-[a]>bcdefghij\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id;
    let pid_b = open_pane(&mut ed.state, &mut ed.view, bid);

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
        ed.current_selections().primary().head(),
        0,
        "pane A cursor restored by undo"
    );
    // Pane B's cursor is translated back to 9 by the inverse CS.
    assert_eq!(
        ed.selections_for(pid_b, bid).unwrap().primary().head(),
        9,
        "pane B selection translated by inverse CS (undo)"
    );
}

/// Multi-cursor propagation: a deletion that spans two selections in pane B
/// merges them into one (proves translate_in_place calls merge_overlapping_in_place).
#[test]
fn propagate_cs_merges_collapsed_non_acting_pane_selections() {
    use hume_editing::selection::{Selection, SelectionSet};

    // "abcde\n" — 6 chars.
    let mut ed = editor_from("-[a]>bcde\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id;
    let pid_b = open_pane(&mut ed.state, &mut ed.view, bid);

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
    assert_eq!(
        pane_b_sels.len(),
        1,
        "collapsed selections must merge after propagation"
    );
    assert_eq!(
        pane_b_sels.primary().head(),
        1,
        "merged cursor at deletion point"
    );
}

/// Non-focused pane engine mirror is updated by `sync_all_pane_mirrors` after
/// an edit translates the pane's authoritative `SelectionSet`.
///
/// Guards the removal of the immediate engine-mirror write from
/// `propagate_cs_to_panes`: the mirror must stay consistent with `pane_state`
/// when synced via the per-frame path.
#[test]
fn pane_engine_mirror_synced_for_non_focused_pane_after_edit() {
    use hume_editing::selection::{Selection, SelectionSet};

    // "abcdefghij\n" — cursor on 'a'.
    let mut ed = editor_from("-[a]>bcdefghij\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id;
    let pid_b = open_pane(&mut ed.state, &mut ed.view, bid);

    // Position pane B's cursor at char 5 ('f').
    ed.switch_focused_pane(pid_b);
    ed.set_current_selections(SelectionSet::single(Selection::collapsed(5)));

    // Switch to pane A and delete char 0 ('a'); this calls propagate_cs_to_panes
    // which translates pane B's authoritative SelectionSet but (post-fix) does NOT
    // write the engine mirror directly.
    ed.switch_focused_pane(pid_a);
    ed.handle_key(key('d'));

    // Authoritative selection in pane_state must be at 4 (translated by CS).
    assert_eq!(
        ed.selections_for(pid_b, bid).unwrap().primary().head(),
        4,
        "pane B pane_state selection translated to 4"
    );

    // Simulate the per-frame sync — this is what write the engine mirror.
    ed.sync_all_pane_mirrors();

    // Engine mirror for pane B must now reflect the translated position.
    let mirror_head = ed.view.panes[pid_b].selections[0].head;
    assert_eq!(
        mirror_head, 4,
        "pane B engine mirror head reflects translated position"
    );
}

// ── ensure() contract tests ────────────────────────────────────────────────────

/// ensure() is idempotent: calling it twice on the same (pid, bid) does not
/// overwrite existing state (e.g. selections moved away from initial).
#[test]
fn ensure_is_idempotent() {
    use crate::editor::pane_state;
    use hume_editing::selection::{Selection, SelectionSet};

    let mut ed = editor_from("-[h]>ello\n");
    let pid = ed.state.focused_pane_id;
    let bid = ed.focused_buffer_id();

    // Move the cursor away from its initial position.
    ed.set_current_selections(SelectionSet::single(Selection::collapsed(3)));

    // ensure() on an already-seeded entry must not reset to initial_sels.
    pane_state::ensure(&mut ed.state.panes.state, &ed.state.buffers, pid, bid);
    assert_eq!(
        ed.current_selections().primary().head(),
        3,
        "ensure must not overwrite existing pane_state entry",
    );
}

/// ensure() on a new (pid, bid) pair seeds the entry with the buffer's initial
/// selections, matching the same value that fresh_from_buf() would produce.
#[test]
fn ensure_seeds_new_entry_with_initial_sels() {
    use crate::editor::pane_state;

    let mut ed = editor_from("-[h]>ello\n");
    let pid = ed.state.focused_pane_id;

    // Open a second buffer; the focused pane has never viewed it.
    let doc2 = Buffer::scratch();
    let expected_sels = doc2.initial_sels();
    let bid2 = crate::editor::buffer::lifecycle::open_buffer(
        &mut ed.view,
        &mut ed.state.buffers,
        &mut ed.state.panes.state,
        pid,
        doc2,
        0,
    );

    // open_buffer already calls ensure internally; a second call is idempotent
    // and returns a state with the initial selections.
    let state = pane_state::ensure(&mut ed.state.panes.state, &ed.state.buffers, pid, bid2);
    assert_eq!(
        state.selections, expected_sels,
        "ensure must seed with buffer's initial_sels on first visit",
    );
}

// ── T2: `:split` / `:vsplit` typed commands ────────────────────────────────────

/// `:split` stacks a new pane below the focused one, viewing the same buffer,
/// and moves focus there. Vim naming is inverted from the engine's `Direction`:
/// stacked panes are `Direction::Vertical` (it divides height).
#[test]
fn split_stacks_pane_on_same_buffer() {
    use hume_engine::pipeline::{Direction, LayoutTree};

    let mut ed = editor_from("-[h]>ello\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id;

    ed.execute_typed("split", None).unwrap();

    assert_eq!(ed.view.panes.len(), 2, "split creates exactly one new pane");
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_b, pid_a, "focus moves to the new pane");
    assert_eq!(
        ed.view.panes[pid_b].buffer_id, bid,
        "new pane views the same buffer"
    );

    match &ed.view.layout {
        LayoutTree::Split {
            direction,
            children,
            ..
        } => {
            assert_eq!(*direction, Direction::Vertical, ":split stacks panes");
            let leaves: Vec<_> = [&children.0, &children.1]
                .into_iter()
                .map(|c| match c {
                    LayoutTree::Leaf(id) => *id,
                    other => panic!("expected two leaves, got {other:?}"),
                })
                .collect();
            assert!(
                leaves.contains(&pid_a) && leaves.contains(&pid_b),
                "layout's two leaves are the original and new pane"
            );
        }
        other => panic!("expected Split layout, got {other:?}"),
    }
}

/// `:vsplit` places the new pane side by side with the focused one —
/// `Direction::Horizontal` in the engine (it divides width).
#[test]
fn vsplit_places_pane_side_by_side() {
    use hume_engine::pipeline::{Direction, LayoutTree};

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("vsplit", None).unwrap();

    match &ed.view.layout {
        LayoutTree::Split { direction, .. } => {
            assert_eq!(*direction, Direction::Horizontal, ":vsplit is side-by-side")
        }
        other => panic!("expected Split layout, got {other:?}"),
    }
}

/// End-to-end regression guard: `:split` typed through the real command-mode
/// dispatch path (`Mode::Command`, not `Mode::Normal`) must still move focus
/// to the new pane. `execute_typed`-based tests above run with the editor
/// already in Normal mode and would not catch a regression that routed focus
/// through `switch_focused_pane` (whose Normal-mode debug_assert would panic
/// here, since mode flips back to Normal only after dispatch completes).
#[test]
fn split_via_command_mode_moves_focus() {
    use hume_engine::pipeline::LayoutTree;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focused_pane_id;

    type_cmd(&mut ed, ":split");

    let pid_b = ed.state.focused_pane_id;
    assert_ne!(
        pid_b, pid_a,
        "focus moves to the new pane through the real command-mode path"
    );
    assert!(
        matches!(ed.view.layout, LayoutTree::Split { .. }),
        "layout is a Split"
    );
}

// ── T3: Multi-pane render / prepare_frame ──────────────────────────────────────

/// After `:vsplit`, `prepare_frame` must size both panes from the layout tree,
/// not just the focused one. Regression guard: before the fix the sibling
/// pane kept its `Pane::new` default viewport instead of tiling its half of
/// the terminal.
#[test]
fn vsplit_sizes_both_panes_from_layout() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focused_pane_id;
    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_a, pid_b);

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(100, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx); // 25 rows → 24 usable after statusline

    let wa = ed.view.panes[pid_a].viewport.width;
    let wb = ed.view.panes[pid_b].viewport.width;
    assert_eq!(
        wa + wb,
        99,
        "vsplit halves plus the 1-column seam must tile the full terminal width"
    );
    assert!(
        wa < 100 && wb < 100,
        "neither pane keeps the full terminal width"
    );
    assert_eq!(ed.view.panes[pid_a].viewport.height, 24);
    assert_eq!(ed.view.panes[pid_b].viewport.height, 24);
}

/// `:split` stacks panes: height is partitioned, width stays full for both.
#[test]
fn split_sizes_both_panes_stacked() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focused_pane_id;
    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focused_pane_id;

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(80, 41);
    ed.settle();
    ed.prepare_frame(&mut ctx); // 41 rows → 40 usable after statusline

    let ha = ed.view.panes[pid_a].viewport.height;
    let hb = ed.view.panes[pid_b].viewport.height;
    assert_eq!(
        ha + hb,
        39,
        "split halves plus the 1-row seam must tile the full usable height"
    );
    assert_eq!(ed.view.panes[pid_a].viewport.width, 80);
    assert_eq!(ed.view.panes[pid_b].viewport.width, 80);
}

// ── T4: Split-too-small guard ────────────────────────────────────────────────

/// `:vsplit` on a pane too narrow to fit two minimum-width panes plus the
/// seam divider is a noop with a warning, not a degraded split.
#[test]
fn vsplit_too_narrow_is_noop_with_warning() {
    use hume_engine::pipeline::LayoutTree;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focused_pane_id;

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(20, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx); // width 20 < 2*MIN_PANE_WIDTH(10)+1 = 21

    ed.execute_typed("vsplit", None).unwrap();

    assert_eq!(
        ed.state.focused_pane_id, pid_a,
        "focus does not move — split was rejected"
    );
    assert!(
        matches!(ed.view.layout, LayoutTree::Leaf(_)),
        "layout is unchanged"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("pane too small to split")
    );
}

/// `:split` on a pane too short is likewise a noop with a warning.
#[test]
fn split_too_short_is_noop_with_warning() {
    use hume_engine::pipeline::LayoutTree;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focused_pane_id;

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(80, 7);
    ed.settle();
    ed.prepare_frame(&mut ctx); // 7 rows -> 6 usable after statusline < 2*MIN_PANE_HEIGHT(3)+1 = 7

    ed.execute_typed("split", None).unwrap();

    assert_eq!(
        ed.state.focused_pane_id, pid_a,
        "focus does not move — split was rejected"
    );
    assert!(
        matches!(ed.view.layout, LayoutTree::Leaf(_)),
        "layout is unchanged"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("pane too small to split")
    );
}

/// The guard is a threshold, not a blanket restriction: a pane at exactly
/// the minimum-fitting size still splits.
#[test]
fn vsplit_at_minimum_width_still_splits() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focused_pane_id;

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(21, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx); // exactly 2*MIN_PANE_WIDTH(10)+1

    ed.execute_typed("vsplit", None).unwrap();

    assert_ne!(
        ed.state.focused_pane_id, pid_a,
        "split succeeds at the threshold"
    );
}

/// Symmetric threshold check for `:split`'s height axis.
#[test]
fn split_at_minimum_height_still_splits() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focused_pane_id;

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(80, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx); // 8 rows -> 7 usable = exactly 2*MIN_PANE_HEIGHT(3)+1

    ed.execute_typed("split", None).unwrap();

    assert_ne!(
        ed.state.focused_pane_id, pid_a,
        "split succeeds at the threshold"
    );
}

/// A zero-height render area (e.g. a terminal reporting height 0 on early
/// startup) must not panic. `EngineView::render` must guard the statusline
/// (and tab bar) provider's synthesized `Rect` on `area.height` — an
/// unguarded `Rect` claiming a row regardless of `area.height` would send
/// the provider's `Buffer::set_string` calls out-of-bounds and panic, unlike
/// the background fill which clamps.
///
/// Fail oracle: drop the `area.height > 0` guard around the chrome-row
/// rendering in `EngineView::render` — this test panics instead of returning
/// an empty string.
#[test]
fn zero_height_render_does_not_panic() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    let rect = ratatui::layout::Rect::new(0, 0, 20, 0);
    assert_eq!(render_to_styled_string(&mut ed, rect), "");
}

/// After `:vsplit`, `render_into` must draw both panes at their own rects —
/// not just the focused one. Both panes view the same buffer here, so the
/// same content must appear in both halves of the styled-frame snapshot.
#[test]
fn vsplit_renders_content_in_both_halves() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    ed.execute_typed("vsplit", None).unwrap();

    let rect = ratatui::layout::Rect::new(0, 0, 20, 4);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

/// Where a horizontal seam meets a vertical seam, the crossing cell must get
/// a proper junction glyph (`┬`), not whichever straight glyph drew last.
/// `:split` stacks A/B, then `:vsplit` on B splits it into B|C — the seam
/// below A meets the seam between B and C in a T shape.
#[test]
fn split_then_vsplit_renders_t_junction_glyph() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    ed.execute_typed("split", None).unwrap();
    ed.execute_typed("vsplit", None).unwrap();

    let rect = ratatui::layout::Rect::new(0, 0, 20, 8);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

/// A 2×2 grid of panes (both rows split at the same ratio, so their vertical
/// seams align in the same column) must render a full cross (`┼`) where the
/// horizontal and vertical seams meet — not two overlapping straight lines.
/// Same grid shape as `quit_in_grid_promotes_correct_sibling`.
#[test]
fn grid_of_four_panes_renders_cross_junction_glyph() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    let pid_a = ed.state.focused_pane_id;

    ed.execute_typed("split", None).unwrap(); // A/B stacked.
    let pid_b = ed.state.focused_pane_id;

    ed.switch_focused_pane(pid_a);
    ed.execute_typed("vsplit", None).unwrap(); // A/D side by side — top row.

    ed.switch_focused_pane(pid_b);
    ed.execute_typed("vsplit", None).unwrap(); // B/C side by side — bottom row.

    let rect = ratatui::layout::Rect::new(0, 0, 20, 8);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

/// Entering Insert mode must hide the fake block cursor only in the focused
/// pane (which real terminal bar cursor overlays) — not in every pane.
/// `resolve_pane_settings` (lifecycle.rs) forces a block-cursor mode for
/// unfocused panes regardless of the editor's global mode; this locks that
/// per-pane behavior at the render level. `:vsplit` moves focus to the new
/// (right) pane, so the left pane's cursor cell must keep its block style
/// after `i`, while the right pane's cursor cell goes transparent.
#[test]
fn insert_mode_hides_cursor_only_in_focused_pane() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    ed.execute_typed("vsplit", None).unwrap();
    assert_eq!(ed.state.mode(), Mode::Normal, "sanity: starts in Normal");

    ed.feed_key(key('i'));
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: entered Insert mode");

    let rect = ratatui::layout::Rect::new(0, 0, 20, 4);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

/// A pane created via `open_pane` (the shared core of `:split`/`:vsplit` and the
/// keymap-bound `pane-split`/`pane-vsplit`) must get the same gutter column as
/// the initial pane — not the empty `ProviderSet` `Pane::new` alone would give
/// it. Uses the real `Editor::open` constructor (not the bare-pane `for_testing`
/// harness used elsewhere in this file) so the initial pane reflects actual
/// production setup. Independent oracle: compare the split pane's
/// `gutter_columns().count()` against the pre-existing initial pane's, rather
/// than asserting a hardcoded count that could pass even if both were wrongly
/// empty.
#[test]
fn split_pane_gets_gutter_column() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    let pid_a = ed.state.focused_pane_id;
    let initial_gutter_cols = ed.view.panes[pid_a].providers.gutter_columns().count();
    assert!(
        initial_gutter_cols > 0,
        "sanity: the initial pane must itself have a gutter column"
    );

    let bid = ed.focused_buffer_id();
    let pid_b = open_pane(&mut ed.state, &mut ed.view, bid);

    assert_eq!(
        ed.view.panes[pid_b].providers.gutter_columns().count(),
        initial_gutter_cols,
        "split pane must have the same gutter columns as the initial pane"
    );
}

// ── T5: `:q` pane-awareness + close-pane semantics ──────────────────────────
//
// `close_focused_pane` backs both `:q` (multi-pane branch, exercised here)
// and the keymap-bound `pane-close` (`Ctrl+p c`, exercised in kitty.rs).

/// With multiple panes open, `:q` closes the focused pane instead of the
/// editor, and moves focus to the promoted sibling.
#[test]
fn quit_with_multiple_panes_closes_focused_pane_not_editor() {
    use hume_engine::pipeline::LayoutTree;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focused_pane_id;
    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_a, pid_b);

    ed.execute_typed("quit", None).unwrap();

    assert!(
        !ed.state.should_quit,
        ":q with panes open must not quit the editor"
    );
    assert_eq!(ed.view.panes.len(), 1, "the focused pane is closed");
    assert_eq!(
        ed.state.focused_pane_id, pid_a,
        "focus returns to the surviving pane"
    );
    assert!(
        matches!(ed.view.layout, LayoutTree::Leaf(id) if id == pid_a),
        "layout collapses back to a single leaf"
    );
}

/// The multi-pane `:q` branch skips the dirty check entirely — closing a pane
/// never loses edits because the buffer stays open in the buffer list. This
/// is a deliberate difference from the single-pane path, which still refuses
/// on unsaved changes (covered by `colon_q_on_dirty_buffer_refuses`).
#[test]
fn quit_with_multiple_panes_ignores_dirty_buffer() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("split", None).unwrap();

    // Dirty the buffer both panes are viewing.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty(), "sanity: buffer is dirty");

    let result = ed.execute_typed("quit", None);
    assert!(
        result.is_ok(),
        "multi-pane :q must not be blocked by unsaved changes: {result:?}"
    );
    assert_eq!(ed.view.panes.len(), 1);
}

/// `:wq` delegates to `:q` after a successful write, so it must mirror the
/// same pane-aware close: with panes open it writes then closes the focused
/// pane, rather than tearing down the whole editor.
#[test]
fn wq_with_multiple_panes_closes_focused_pane_not_editor() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    let pid_a = ed.state.focused_pane_id;
    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_a, pid_b);

    // Dirty the buffer both panes are viewing.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(ed.doc().is_dirty(), "sanity: buffer is dirty");

    let expected_content = ed.doc().text().to_string();
    let result = ed.execute_typed("wq", None);

    assert!(
        result.is_ok(),
        ":wq with panes open must write and close the pane: {result:?}"
    );
    assert!(
        !ed.state.should_quit,
        ":wq with panes open must not quit the editor"
    );
    assert_eq!(ed.view.panes.len(), 1, "the focused pane is closed");
    assert_eq!(
        ed.state.focused_pane_id, pid_a,
        "focus returns to the surviving pane"
    );
    assert!(!ed.doc().is_dirty(), "the write must have happened");
    assert_eq!(
        std::fs::read_to_string(&tmp).unwrap(),
        expected_content,
        "the edit must have been written to disk before the pane closed"
    );
}

/// `viewport_debounce`/`last_viewport_key`/`virtual_lines_synced` live on
/// `Editor` rather than `EditorState.panes`, so `drop_pane_state` can't clear
/// them directly — `prepare_frame`'s `prune_closed_pane_caches` sweep is the
/// only place that reclaims a closed pane's entries. Without it these three
/// maps grow without bound over an editor session's lifetime.
#[test]
fn closing_a_pane_reclaims_its_entries_from_the_frame_caches() {
    let mut ed = editor_from("-[h]>ello\n");
    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focused_pane_id;
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    assert!(
        ed.last_viewport_key.contains_key(&pid_b),
        "sanity: prepare_frame populated pane B's scroll-key cache entry"
    );
    assert!(
        ed.virtual_lines_synced.contains_key(&pid_b),
        "sanity: prepare_frame populated pane B's virtual-line-sync cache entry"
    );
    assert!(
        ed.viewport_debounce.contains_key(&pid_b),
        "sanity: prepare_frame armed pane B's viewport-debounce timer"
    );

    // Closes the focused pane (B) and promotes A back to focus.
    ed.execute_typed("quit", None).unwrap();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    assert!(
        !ed.last_viewport_key.contains_key(&pid_b),
        "closed pane's scroll-key entry must be reclaimed"
    );
    assert!(
        !ed.virtual_lines_synced.contains_key(&pid_b),
        "closed pane's virtual-line-sync entry must be reclaimed"
    );
    assert!(
        !ed.viewport_debounce.contains_key(&pid_b),
        "closed pane's viewport-debounce timer must be cancelled and reclaimed"
    );
}

/// `virtual_lines_synced`'s cache key must include the pane's buffer, not
/// just `decorations.generation()` — a generation-only key would keep a pane
/// mirroring its *previous* buffer's virtual lines after a switch, since
/// switching a buffer doesn't bump the generation.
#[test]
fn switching_a_panes_buffer_rebuilds_its_virtual_lines() {
    use crate::editor::decorations::VirtualLineEntry;
    use crate::lock_ext::LockExt;

    let mut ed = editor_from("-[h]>ello\n");
    let bid_a = ed.focused_buffer_id();
    // `editor_from`'s bootstrap pane is built via `Pane::new` directly, with
    // no `panes.render` entry (see `Editor::for_testing`'s comment) — only
    // `open_pane` seeds one, so this test opens a second pane rather than
    // using the bootstrap one.
    let pid = open_pane(&mut ed.state, &mut ed.view, bid_a);
    ed.switch_focused_pane(pid);

    ed.state.config.decorations.set_virtual_lines(
        "test".to_string(),
        bid_a,
        vec![VirtualLineEntry {
            line: 0,
            text: "deleted".to_string(),
            before: false,
            scope: None,
            segments: Vec::new(),
        }],
    );

    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut hume_engine::pipeline::RenderContext::new());

    let virtual_lines_arc = ed
        .state
        .panes
        .render
        .get(pid)
        .unwrap()
        .virtual_lines
        .clone();
    assert!(
        virtual_lines_arc.read_or_panic().contains_key(&0),
        "sanity: pane A mirrors buffer A's virtual line at line 0"
    );

    // Switch the same pane to a fresh buffer with no virtual lines set.
    let bid_b = ed.open_buffer(Buffer::scratch());
    ed.switch_to_buffer_with_jump(bid_b);
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut hume_engine::pipeline::RenderContext::new());

    assert!(
        virtual_lines_arc.read_or_panic().is_empty(),
        "after switching to buffer B, the pane must no longer mirror buffer \
         A's virtual lines — a generation-only sync gate would leave line 0 \
         populated with A's stale entry"
    );
}

/// Closing one leaf of a 2×2 grid promotes the correct sibling and leaves the
/// other three panes untouched. Independent oracle: the expected post-close
/// shape (`Split{Leaf(A), Split{B, C}}`) is derived from how the grid was
/// built, not by re-running the close logic.
#[test]
fn quit_in_grid_promotes_correct_sibling() {
    use hume_engine::pipeline::LayoutTree;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focused_pane_id;

    // A/B stacked.
    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focused_pane_id;

    // A/D side by side — top row.
    ed.switch_focused_pane(pid_a);
    ed.execute_typed("vsplit", None).unwrap();
    let pid_d = ed.state.focused_pane_id;

    // B/C side by side — bottom row. Grid is now: top (A, D), bottom (B, C).
    ed.switch_focused_pane(pid_b);
    ed.execute_typed("vsplit", None).unwrap();
    let pid_c = ed.state.focused_pane_id;

    assert_eq!(ed.view.panes.len(), 4, "sanity: four panes in the grid");

    // Close D (top-right): its sibling A is promoted, collapsing the top row
    // to a single leaf; the bottom row (B, C) is untouched.
    ed.switch_focused_pane(pid_d);
    ed.execute_typed("quit", None).unwrap();

    assert_eq!(ed.view.panes.len(), 3, "one pane closed");
    assert_eq!(
        ed.state.focused_pane_id, pid_a,
        "A is promoted as D's surviving sibling"
    );
    assert!(!ed.view.panes.contains_key(pid_d), "D was closed");

    match &ed.view.layout {
        LayoutTree::Split { children, .. } => {
            assert!(
                matches!(&children.0, LayoutTree::Leaf(id) if *id == pid_a),
                "top row collapses to A"
            );
            match &children.1 {
                LayoutTree::Split {
                    children: bottom, ..
                } => {
                    let leaves: Vec<_> = [&bottom.0, &bottom.1]
                        .into_iter()
                        .map(|c| match c {
                            LayoutTree::Leaf(id) => *id,
                            other => panic!("expected leaf, got {other:?}"),
                        })
                        .collect();
                    assert!(
                        leaves.contains(&pid_b) && leaves.contains(&pid_c),
                        "bottom row keeps B and C untouched"
                    );
                }
                other => panic!("expected bottom row Split, got {other:?}"),
            }
        }
        other => panic!("expected Split layout, got {other:?}"),
    }
}

// ── wrap_mode: pane override → buffer override → global (M11) ─────────────────

/// A same-buffer split (`:split` with no path) inherits the source pane's
/// live override — not the global default. This lets a `:wrap`-toggled
/// pane pass its mode on to a split of itself.
#[test]
fn same_buffer_split_inherits_source_panes_wrap_override() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focused_pane_id;
    ed.view.panes[pid_a].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Soft { width: 40 }),
        saved: None,
    });
    // Global default deliberately differs, to prove it is NOT the source.
    ed.state.settings.wrap_mode = hume_engine::pane::WrapMode::None;

    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focused_pane_id;

    assert_eq!(
        ed.view.panes[pid_b].wrap().mode,
        Some(hume_engine::pane::WrapMode::Soft { width: 40 }),
        "same-buffer split inherits the source pane's live override"
    );
}

/// The other half of the split-inheritance contract: a source pane with *no*
/// pane-level override (still inheriting from the buffer/global setting)
/// splits into a pane that is likewise unpinned — not one frozen at
/// whichever mode the source happened to resolve to. The new pane keeps
/// following later `:set buffer`/`:set global wrap-mode=…` changes, same as
/// the pane it split from.
#[test]
fn same_buffer_split_of_an_unpinned_pane_stays_unpinned() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focused_pane_id;
    assert_eq!(ed.view.panes[pid_a].wrap().mode, None, "sanity: unpinned");

    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focused_pane_id;

    assert_eq!(
        ed.view.panes[pid_b].wrap().mode,
        None,
        "split of an unpinned pane is itself unpinned, not frozen at the \
         resolved mode"
    );
}

/// `:wrap` toggles only the focused pane's override — a sibling pane on the
/// same buffer is untouched. The override lives on `Pane`, not on the
/// buffer, so two panes viewing the same buffer can wrap independently once
/// one is pinned.
#[test]
fn wrap_toggle_affects_only_focused_pane() {
    let mut ed = editor_from("-[h]>ello\n");
    // Global is resolved lazily on every read, so this reaches pid_a's
    // effective mode retroactively — no pane pin needed for A to start off.
    ed.state.settings.wrap_mode = hume_engine::pane::WrapMode::None;
    let pid_a = ed.state.focused_pane_id;

    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_a, pid_b);
    assert_eq!(
        ed.view.panes[pid_a].buffer_id, ed.view.panes[pid_b].buffer_id,
        "sanity: both panes view the same buffer"
    );

    // Focus is on B (the new pane) after :split — toggle wrap there.
    ed.execute_typed("wrap", None).unwrap();

    let doc = ed.state.buffers.get(ed.view.panes[pid_a].buffer_id);
    assert!(
        crate::editor::commands::effective_wrap_mode(
            doc,
            &ed.state.settings,
            &ed.view.panes[pid_b]
        )
        .is_wrapping(),
        "B: wrap toggled on"
    );
    assert!(
        !crate::editor::commands::effective_wrap_mode(
            doc,
            &ed.state.settings,
            &ed.view.panes[pid_a]
        )
        .is_wrapping(),
        "A: unaffected by B's toggle, despite sharing a buffer"
    );
}

// ── Geometry regression guards ─────────────────────────────────────────────────
//
// `EngineView` must not cache a `pane_rects` snapshot across frames — every
// consumer (pane-focus commands, `fits_split`, the bar-cursor lookup) must
// recompute from the live layout tree plus the terminal area cached by the
// last `prepare_frame`. A cache would let a close/split earlier in the same
// macro-replay batch invalidate geometry that a later command in the same
// batch would otherwise still trust.

/// Closing a pane and then focusing the next one, with no `prepare_frame` in
/// between (exactly what a macro-replay batch does — several commands run
/// per frame), must land on the surviving pane. A cached rect list would
/// still list the just-closed pane, handing focus to a dead `PaneId`.
#[test]
fn close_then_focus_next_without_reframe_lands_on_live_pane() {
    use crate::editor::commands::cmd_pane_focus_next;
    use hume_ops::MotionMode;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focused_pane_id;
    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_a, pid_b);

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(100, 51);
    ed.settle();
    ed.prepare_frame(&mut ctx); // establish terminal geometry once

    // Close B, then immediately focus-next — no `prepare_frame` in between.
    ed.execute_typed("quit", None).unwrap();
    assert_eq!(ed.view.panes.len(), 1, "sanity: B is closed");
    cmd_pane_focus_next(&mut ed.state, &mut ed.view, 1, MotionMode::Move).unwrap();

    assert_eq!(
        ed.state.focused_pane_id, pid_a,
        "focus-next after a same-batch close must land on the live survivor"
    );
    assert!(ed.view.panes.contains_key(ed.state.focused_pane_id));
}

/// Splitting and then focusing directionally, with no `prepare_frame` in
/// between, must reach the freshly created pane. A cached rect list from
/// before the split would still show only the original pane, so the focus
/// command would silently no-op instead of moving to the new pane.
#[test]
fn split_then_focus_left_without_reframe_reaches_new_pane() {
    use crate::editor::commands::cmd_pane_focus_left;
    use hume_ops::MotionMode;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focused_pane_id;

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(100, 51);
    ed.settle();
    ed.prepare_frame(&mut ctx); // geometry established with one pane

    // :vsplit puts the new pane on the right and moves focus to it — no
    // `prepare_frame` in between.
    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_a, pid_b);

    cmd_pane_focus_left(&mut ed.state, &mut ed.view, 1, MotionMode::Move).unwrap();
    assert_eq!(
        ed.state.focused_pane_id, pid_a,
        "focus-left from the freshly split pane must reach the original pane"
    );
}

/// A bare split (same buffer as the source pane) inherits the source pane's
/// cursor and scroll position, rather than jumping to the top of the file.
#[test]
fn split_inherits_focused_panes_selection_and_scroll() {
    use hume_editing::selection::Selection;

    let content: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let buf = Text::from(content.as_str());
    let sels = SelectionSet::single(Selection::collapsed(0));
    let mut ed = Editor::for_testing(Buffer::new(buf, sels));
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id;

    // Move A's cursor and scroll well away from the top of the file.
    let cursor_pos = ed.doc().text().line_to_char(150);
    ed.state.panes.state[pid_a][bid].selections =
        SelectionSet::single(Selection::collapsed(cursor_pos));
    ed.view.panes[pid_a].viewport.top_line = 140;

    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_a, pid_b);

    assert_eq!(
        ed.state.panes.state[pid_b][bid].selections, ed.state.panes.state[pid_a][bid].selections,
        "new pane inherits the source pane's selection"
    );
    assert_eq!(
        ed.view.panes[pid_b].viewport.top_line, 140,
        "new pane inherits the source pane's scroll position"
    );
}

/// A same-buffer split inherits the source pane's `saved_scrolls` — its
/// memory of where it was in buffers visited *before* the split. Without
/// this, the new pane would reset such a buffer to the top on first visit
/// instead of recalling where the source pane last left it.
#[test]
fn same_buffer_split_inherits_saved_scrolls() {
    use crate::editor::buffer::lifecycle::open_buffer;
    use hume_engine::pane::ScrollPosition;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focused_pane_id;

    // A second buffer the source pane visited (and scrolled) before the
    // split, then switched away from — this is what populates
    // `saved_scrolls` in real usage (see `remember_scroll`).
    let bid2 = open_buffer(
        &mut ed.view,
        &mut ed.state.buffers,
        &mut ed.state.panes.state,
        pid_a,
        Buffer::scratch(),
        0,
    );
    ed.view.panes[pid_a].saved_scrolls.insert(
        bid2,
        ScrollPosition {
            top_line: 42,
            top_row_offset: 0,
            horizontal_offset: 0,
        },
    );

    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_a, pid_b);

    assert_eq!(
        ed.view.panes[pid_b].saved_scrolls.get(bid2),
        ed.view.panes[pid_a].saved_scrolls.get(bid2),
        "new pane inherits the source pane's saved_scrolls history"
    );
}

/// Before the first `prepare_frame`, there is no real terminal geometry to
/// check a split against — `fits_split` must allow it; the next
/// `prepare_frame` sizes the result correctly regardless.
#[test]
fn fits_split_allows_before_first_frame() {
    use hume_engine::pipeline::Direction;

    let ed = editor_from("-[h]>ello\n");
    assert!(crate::editor::commands::fits_split(
        &ed.state,
        &ed.view,
        Direction::Vertical
    ));
    assert!(crate::editor::commands::fits_split(
        &ed.state,
        &ed.view,
        Direction::Horizontal
    ));
}

/// If `split_leaf` can't find the focused pane in the layout tree — an
/// invariant violation that should never happen — `split_pane_onto` must
/// roll back the pane it speculatively created instead of leaving an
/// orphaned pane with no layout leaf (which would later violate
/// `close_focused_pane`'s precondition on `remove_leaf`).
#[test]
fn split_pane_onto_rolls_back_when_focused_pane_missing_from_layout() {
    use hume_engine::pipeline::Direction;

    let mut ed = editor_from("-[h]>ello\n");
    let bid = ed.focused_buffer_id();

    // Fabricate the desync directly: focus a pane that was never attached to
    // `view.layout` (only the original pane's `Leaf` exists there).
    let ghost_pid = open_pane(&mut ed.state, &mut ed.view, bid);
    ed.state.focused_pane_id = ghost_pid;
    let panes_before = ed.view.panes.len();

    let result = crate::editor::commands::split_pane_onto(
        &mut ed.state,
        &mut ed.view,
        bid,
        Direction::Vertical,
    );

    assert!(
        result.is_err(),
        "split_leaf failure must surface as an error"
    );
    assert_eq!(
        ed.view.panes.len(),
        panes_before,
        "the speculatively created pane is rolled back, not leaked"
    );
}

/// `pane-dividers=false` reclaims the seam column: sibling panes tile
/// edge-to-edge instead of leaving an unpainted 1-cell gap between them.
#[test]
fn dividers_off_pane_rects_tile_with_no_gap() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.settings.pane_dividers = false;
    ed.execute_typed("vsplit", None).unwrap();

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(100, 51);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let mut rects = ed.view.pane_rects();
    assert_eq!(rects.len(), 2);
    rects.sort_by_key(|(_, r)| r.x);
    let (left, right) = (rects[0].1, rects[1].1);
    assert_eq!(
        left.width + right.width,
        100,
        "no column is reserved for a seam when pane-dividers is off"
    );
    assert_eq!(right.x, left.x + left.width, "panes are adjacent, no gap");
}

/// Visual lock-in companion to `dividers_off_pane_rects_tile_with_no_gap`:
/// with `pane-dividers` off the two halves render edge-to-edge (no gap
/// column between them), and the non-focused pane is still visibly dimmed —
/// dimming is the focus cue, independent of the divider glyph.
#[test]
fn vsplit_dividers_off_tiles_edge_to_edge_and_still_dims() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    ed.state.settings.pane_dividers = false;
    ed.execute_typed("vsplit", None).unwrap();

    let rect = ratatui::layout::Rect::new(0, 0, 20, 4);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

/// A same-buffer split inherits the source pane's jump history so the new
/// pane can Ctrl+O back to positions visited before the split. The two lists
/// then diverge: a new jump in either pane does not affect the other.
#[test]
fn split_same_buffer_clones_jump_list_then_diverges() {
    let mut ed = jump_editor(10);
    let pid_a = ed.state.focused_pane_id;

    // `gg` (goto-first-line) is a jump command: records the pre-jump position.
    ed.handle_key(key('g'));
    ed.handle_key(key('g'));
    assert_eq!(
        ed.state.panes.jumps[pid_a].len(),
        1,
        "source pane has one jump entry after gg"
    );

    // Same-buffer split — new pane inherits the source pane's jump history.
    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_a, pid_b, "focus moved to the new pane");
    assert_eq!(
        ed.state.panes.jumps[pid_b].len(),
        1,
        "new pane inherited the source pane's jump entry"
    );

    // A new jump in the new pane must not leak back into the source pane.
    // `ge` (goto-last-line) is a jump command; cursor is at line 0 (inherited
    // from the source pane's `gg`), so it records {line 0} and moves to line 19.
    ed.handle_key(key('g'));
    ed.handle_key(key('e'));
    assert_eq!(
        ed.state.panes.jumps[pid_b].len(),
        2,
        "new pane recorded its own jump after the split"
    );
    assert_eq!(
        ed.state.panes.jumps[pid_a].len(),
        1,
        "source pane's jump list is unchanged after the new pane's jump"
    );
}

// ── Per-pane highlight isolation ────────────────────────────────────────────
//
// `update_highlight_providers` must not write into globally-shared highlight
// state read by every pane's `SharedHighlighter` — each pane owns its own
// highlight buffers (`PaneHighlights`), computed from that pane's own buffer
// and viewport. Global, focused-buffer-only state would render the focused
// pane's highlight bytes (bracket/search matches) onto every other pane's
// unrelated text, including one viewing a different buffer or the same
// buffer scrolled elsewhere.

/// A search match that spans a `\n` must produce one highlight span per line
/// it touches, each clipped to that line's own content — not a single span
/// computed by converting the match's absolute end offset through whichever
/// line the *start* happened to be on (which, before the fix, produced a
/// corrupt or inverted span whenever a match crossed a line boundary).
///
/// Independent oracle: byte offsets are hand-computed from the known ASCII
/// content ("abc\ndef\n"), not derived by calling the code under test.
#[test]
fn multiline_search_match_splits_into_per_line_highlight_spans() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    let pid = ed.state.focused_pane_id;

    ed.feed_key(key('i'));
    for ch in "abc".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_enter());
    for ch in "def".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    assert_eq!(
        ed.doc().text().to_string(),
        "abc\ndef\n",
        "sanity: buffer content"
    );

    // Matches "c\ndef": char 2 ('c') through char 6 ('f'), crossing the
    // line-0/line-1 boundary at the '\n' (char 3).
    ed = ed.with_search_regex("c\ndef");

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let matches = ed.state.panes.render[pid]
        .highlights
        .search
        .read()
        .unwrap()
        .clone();
    assert_eq!(
        matches,
        vec![(0, 2, 3), (1, 0, 3)],
        "line 0 gets 'c' clipped before its own '\\n' (byte 2..3); \
         line 1 gets 'def' from its own start (byte 0..3)"
    );
}
