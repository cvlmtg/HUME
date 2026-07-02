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
    use hume_editing::selection::{Selection, SelectionSet};

    let mut ed = editor_from("-[h]>ello world\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id;

    let pid_b = ed.open_pane(bid);

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
    use crate::editor::search_state::SearchCursor;

    let mut ed = editor_from("-[f]>oo foo foo\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id;
    let pid_b = ed.open_pane(bid);

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
    let pid_b = ed.open_pane(bid);

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
    let pid_b = ed.open_pane(bid);

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
    let pid_b = ed.open_pane(bid);

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
    let pid_b = ed.open_pane(bid);

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
    let bid2 = crate::editor::ops::open_buffer(
        &mut ed.view,
        &mut ed.state.buffers,
        &mut ed.state.panes.state,
        pid,
        doc2,
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

/// `:vsplit <path>` opens the given file in the new pane instead of mirroring
/// the focused pane's buffer.
#[test]
#[cfg(not(windows))]
fn vsplit_path_opens_that_buffer() {
    use hume_engine::pipeline::{Direction, LayoutTree};

    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), "other file\n").unwrap();
    let path = f.path().to_path_buf();
    let _tmp_path = f.into_temp_path();

    let mut ed = editor_from("-[h]>ello\n");
    let bid_a = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id;

    ed.execute_typed("vsplit", Some(path.to_str().unwrap()))
        .unwrap();

    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_b, pid_a, "focus moves to the new pane");
    let bid_b = ed.view.panes[pid_b].buffer_id;
    assert_ne!(
        bid_b, bid_a,
        "new pane views the opened file, not the original buffer"
    );

    // macOS temp paths differ from their canonical form (/var vs /private/var);
    // canonicalize before comparing against the stored buffer path.
    let canonical = hume_platform::fs::canonicalize(&path).unwrap();
    assert_eq!(
        ed.state.buffers.find_by_path(&canonical),
        Some(bid_b),
        "new pane's buffer resolves to the opened file"
    );

    match &ed.view.layout {
        LayoutTree::Split { direction, .. } => assert_eq!(*direction, Direction::Horizontal),
        other => panic!("expected Split layout, got {other:?}"),
    }
}

/// `:split <missing-path>` reports the path exactly as the user typed it, not
/// its tilde-expanded form — a symlinked or relative path resolved to an
/// unrecognizable absolute path would otherwise make the error more
/// confusing, not less.
///
/// Uses a `~`-prefixed path rather than a plain relative one: `expand()` is a
/// no-op on inputs with no `~`/env-var sigil, so a plain relative path (e.g.
/// `./foo.txt`) round-trips identically through both "show what was typed"
/// and "show the expanded-but-unresolved path" — it can't tell the two
/// implementations apart. Only an input `expand()` actually rewrites, like
/// `~/...`, can prove which one the error message is built from.
#[test]
#[cfg(not(windows))]
fn split_missing_file_error_shows_raw_typed_path() {
    let home = hume_platform::dirs::home_dir().expect("HOME must be set for this test");
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed
        .execute_typed("split", Some("~/no-such-file-xyz.txt"))
        .unwrap_err();
    assert!(
        err.message().starts_with("~/no-such-file-xyz.txt: "),
        "error must lead with the raw typed path, got: {}",
        err.message()
    );
    assert!(
        !err.message().contains(&home.to_string_lossy().to_string()),
        "error must not leak the expanded $HOME path, got: {}",
        err.message()
    );
}

/// Same guarantee as `split_missing_file_error_shows_raw_typed_path`, for
/// `:vsplit`. Both commands share `open_path_arg`, but each has its own
/// dispatch entry point (`typed_split`/`typed_vsplit`), so both are covered.
#[test]
#[cfg(not(windows))]
fn vsplit_missing_file_error_shows_raw_typed_path() {
    let home = hume_platform::dirs::home_dir().expect("HOME must be set for this test");
    let mut ed = editor_from("-[h]>ello\n");
    let err = ed
        .execute_typed("vsplit", Some("~/no-such-file-xyz.txt"))
        .unwrap_err();
    assert!(
        err.message().starts_with("~/no-such-file-xyz.txt: "),
        "error must lead with the raw typed path, got: {}",
        err.message()
    );
    assert!(
        !err.message().contains(&home.to_string_lossy().to_string()),
        "error must not leak the expanded $HOME path, got: {}",
        err.message()
    );
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
    ed.prepare_frame(100, 25, &mut ctx); // 25 rows → 24 usable after statusline

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
    ed.prepare_frame(80, 41, &mut ctx); // 41 rows → 40 usable after statusline

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
    ed.prepare_frame(20, 25, &mut ctx); // width 20 < 2*MIN_PANE_WIDTH(10)+1 = 21

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
    ed.prepare_frame(80, 7, &mut ctx); // 7 rows -> 6 usable after statusline < 2*MIN_PANE_HEIGHT(3)+1 = 7

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
    ed.prepare_frame(21, 25, &mut ctx); // exactly 2*MIN_PANE_WIDTH(10)+1

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
    ed.prepare_frame(80, 8, &mut ctx); // 8 rows -> 7 usable = exactly 2*MIN_PANE_HEIGHT(3)+1

    ed.execute_typed("split", None).unwrap();

    assert_ne!(
        ed.state.focused_pane_id, pid_a,
        "split succeeds at the threshold"
    );
}

/// After `:vsplit`, `render_into` must draw both panes at their own rects —
/// not just the focused one. Both panes view the same buffer here, so the
/// same content must appear in both halves of the styled-frame snapshot.
#[test]
fn vsplit_renders_content_in_both_halves() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
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
/// `gutter_columns().len()` against the pre-existing initial pane's, rather
/// than asserting a hardcoded count that could pass even if both were wrongly
/// empty.
#[test]
fn split_pane_gets_gutter_column() {
    let mut ed = Editor::open(None).unwrap();
    let pid_a = ed.state.focused_pane_id;
    let initial_gutter_cols = ed.view.panes[pid_a].providers.gutter_columns().len();
    assert!(
        initial_gutter_cols > 0,
        "sanity: the initial pane must itself have a gutter column"
    );

    let bid = ed.focused_buffer_id();
    let pid_b = ed.open_pane(bid);

    assert_eq!(
        ed.view.panes[pid_b].providers.gutter_columns().len(),
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

// ── wrap_mode: per-pane SSOT (M10 T6) ──────────────────────────────────────────

/// A new pane seeds its `wrap_mode` from `EditorSettings::wrap_mode` (the
/// init-default), not from the buffer it opens onto.
#[test]
fn split_seeds_new_pane_wrap_mode_from_global_default() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.settings.wrap_mode = hume_engine::pane::WrapMode::Soft { width: 40 };

    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focused_pane_id;

    assert_eq!(
        ed.view.panes[pid_b].wrap_mode,
        hume_engine::pane::WrapMode::Soft { width: 40 },
        "new pane inherits the global default at creation time"
    );
}

/// `:wrap` toggles only the focused pane's `wrap_mode` — a sibling pane on
/// the same buffer is untouched. `wrap_mode` lives on `Pane`, not on the
/// buffer, so two panes viewing the same buffer can wrap independently.
#[test]
fn wrap_toggle_affects_only_focused_pane() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.state.settings.wrap_mode = hume_engine::pane::WrapMode::None;
    let pid_a = ed.state.focused_pane_id;
    // A was already constructed before the settings write above (which only
    // seeds panes created from here on) — pin its baseline explicitly.
    ed.view.panes[pid_a].wrap_mode = hume_engine::pane::WrapMode::None;

    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_a, pid_b);
    assert_eq!(
        ed.view.panes[pid_a].buffer_id, ed.view.panes[pid_b].buffer_id,
        "sanity: both panes view the same buffer"
    );

    // Focus is on B (the new pane) after :split — toggle wrap there.
    ed.execute_typed("wrap", None).unwrap();

    assert!(
        ed.view.panes[pid_b].wrap_mode.is_wrapping(),
        "B: wrap toggled on"
    );
    assert!(
        !ed.view.panes[pid_a].wrap_mode.is_wrapping(),
        "A: unaffected by B's toggle, despite sharing a buffer"
    );
}

// ── Geometry regression guards (post-review fixes) ─────────────────────────────
//
// `EngineView` no longer caches a `pane_rects` snapshot across frames — every
// consumer (pane-focus commands, `fits_split`, the bar-cursor lookup)
// recomputes from the live layout tree plus the terminal area cached by the
// last `prepare_frame`. These guard the bug the cache used to allow: a
// close/split earlier in the same macro-replay batch invalidating geometry
// that a later command in the same batch would otherwise still trust.

/// Closing a pane and then focusing the next one, with no `prepare_frame` in
/// between (exactly what a macro-replay batch does — several commands run
/// per frame), must land on the surviving pane. A cached rect list would
/// still list the just-closed pane, handing focus to a dead `PaneId`.
#[test]
fn close_then_focus_next_without_reframe_lands_on_live_pane() {
    use crate::editor::commands::cmd_pane_focus_next;
    use crate::ops::MotionMode;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focused_pane_id;
    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focused_pane_id;
    assert_ne!(pid_a, pid_b);

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.prepare_frame(100, 51, &mut ctx); // establish terminal geometry once

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
    use crate::ops::MotionMode;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focused_pane_id;

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.prepare_frame(100, 51, &mut ctx); // geometry established with one pane

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

/// `:vsplit <path>` opens a different buffer in the new pane, so it must
/// start fresh at the buffer's initial selection rather than inheriting the
/// source pane's (unrelated) cursor position.
#[test]
#[cfg(not(windows))]
fn split_path_arg_does_not_inherit_source_panes_view() {
    use hume_editing::selection::Selection;

    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), "other file\n").unwrap();
    let path = f.path().to_path_buf();
    let _tmp_path = f.into_temp_path();

    let mut ed = editor_from("-[h]>ello\n");
    let bid_a = ed.focused_buffer_id();
    let pid_a = ed.state.focused_pane_id;
    ed.state.panes.state[pid_a][bid_a].selections = SelectionSet::single(Selection::collapsed(2));

    ed.execute_typed("vsplit", Some(path.to_str().unwrap()))
        .unwrap();
    let pid_b = ed.state.focused_pane_id;
    let bid_b = ed.view.panes[pid_b].buffer_id;
    assert_ne!(bid_b, bid_a, "sanity: new pane views a different buffer");

    assert_eq!(
        ed.state.panes.state[pid_b][bid_b].selections,
        ed.state.buffers.get(bid_b).initial_sels(),
        "new pane starts at the opened file's initial selection, not A's cursor"
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
    let ghost_pid = ed.open_pane(bid);
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
    ed.prepare_frame(100, 51, &mut ctx);

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
    ed.state.settings.pane_dividers = false;
    ed.execute_typed("vsplit", None).unwrap();

    let rect = ratatui::layout::Rect::new(0, 0, 20, 4);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}
