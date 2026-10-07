use super::*;
use crate::editor::commands::open_pane_in_layout;
use hume_engine::providers::HighlightTier;
use hume_grid::Rect;
use pretty_assertions::assert_eq;

// ── Multi-pane contract tests ──────────────────────────────────────────────────
//
// These tests lock the SSOT invariants for per-pane, per-buffer, and per-search state.

/// Each pane maintains its own cursor independently for the same buffer.
///
/// Two panes on the same buffer; set them to different positions; verify
/// `switch_focused_pane` restores each pane's cursor exactly.
#[test]
fn d1_selections_are_pane_owned() {
    let mut ed = editor_from("-[h]>ello world\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focus.id();

    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();

    // Pane A → position 2 ('l').
    ed.switch_focused_pane(pid_a);
    select(&mut ed, &[(2, 2)], 0);

    // Pane B → position 6 ('w').
    ed.switch_focused_pane(pid_b);
    select(&mut ed, &[(6, 6)], 0);

    // Back to pane A: head must be 2, not 6.
    ed.switch_focused_pane(pid_a);
    assert_eq!(
        ed.current_view().primary().head().offset(),
        co(2),
        "pane A head after switch"
    );

    // Back to pane B: head must be 6, not 2.
    ed.switch_focused_pane(pid_b);
    assert_eq!(
        ed.current_view().primary().head().offset(),
        co(6),
        "pane B head after switch"
    );
}

/// D4a: `Buffer.search_pattern` is shared across all panes on the same buffer;
/// each pane has its own `SearchCursor` in `pane_state`.
#[test]
fn d4a_search_pattern_is_per_buffer() {
    use crate::editor::search::SearchCursor;

    let mut ed = editor_from("-[f]>oo foo foo\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focus.id();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();

    // Both panes see Buffer.search_pattern: it's a single field on `doc`.
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

/// D4b: `Selection.sticky_display_col` travels with the selection; resets
/// when its line is touched by an edit; survives `translate` on
/// untouched lines.
#[test]
fn d4b_sticky_display_col_is_per_selection() {
    use hume_editing::changeset::ChangeSetBuilder;
    use hume_editing::edit::TextChange;
    use hume_editing::selection::{EditView, StickyDisplayCol};
    use hume_editing::text::BufferText;
    use hume_rope::column::BufferLineCol;

    // "abc\ndef\n": two lines.
    let text = BufferText::from("abc\ndef\n");

    // Selection on line 1 (char offset 4 = 'd'), sticky_display_col = 0.
    // Variant is incidental to this test (`translate`'s invalidation
    // doesn't look at it); `BufferLine` is as good as `DisplayLine` here.
    let sticky = StickyDisplayCol::BufferLine {
        display_col: BufferLineCol::new(0),
    };
    let sel = test_fixtures::testing::cursor(&text, 4).with_sticky(sticky);
    let mut sels = test_fixtures::testing::single(&text, sel);

    // CS that inserts at the start of line 0 only: "abc\n" → "Xabc\n"
    // This touches line 0 but not line 1, so sticky_display_col on line-1
    // head should survive.
    let mut b = ChangeSetBuilder::new(text.end());
    b.insert("X"); // insert at start
    let cs = b.finish();

    let text_post = cs.apply(&text).expect("cs built for text");
    sels.translate(&TextChange::new(&text, &text_post, &cs));
    // Head moved from 4 to 5 (past the inserted 'X'), sticky_display_col
    // preserved.
    let view = EditView::bind(&text_post, &sels);
    assert_eq!(
        view.primary().head().offset(),
        co(5),
        "head mapped past insert"
    );
    assert_eq!(
        view.primary().selection().sticky_display_col(),
        Some(sticky),
        "sticky_display_col preserved on untouched line"
    );

    // Now a CS that touches line 1 (inserts at position of 'd'):
    // sticky_display_col should reset. Re-build sels with the updated head
    // but set sticky_display_col back to show it was latched.
    // "Xabc\ndef\n" (after first edit): "d" is now at char 5 (line 1).
    let text2 = BufferText::from("Xabc\ndef\n");
    let sel2 = test_fixtures::testing::cursor(&text2, 5).with_sticky(sticky);
    let mut sels2 = test_fixtures::testing::single(&text2, sel2);

    // Insert at char 5 (start of "def").
    let mut b2 = ChangeSetBuilder::new(text2.end());
    b2.retain_to(co(5)); // skip "Xabc\n"
    b2.insert("Y"); // insert at line 1
    let cs2 = b2.finish();

    let text2_post = cs2.apply(&text2).expect("cs2 built for text2");
    sels2.translate(&TextChange::new(&text2, &text2_post, &cs2));
    // Head moved past insert; sticky_display_col must be reset because line
    // 1 was touched.
    assert_eq!(
        EditView::bind(&text2_post, &sels2)
            .primary()
            .selection()
            .sticky_display_col(),
        None,
        "sticky_display_col reset when head's line is touched"
    );
}

/// An Insert session is scoped to the pane that opened it: sequential
/// sessions on different panes each produce their own revision.  Two
/// separate i…Esc sessions each produce one revision.
#[test]
fn d5_insert_session_is_pane_buffer_scoped() {
    let mut ed = editor_from("-[a]>bc\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focus.id();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();

    // Pane A insert session: type 'X' at the start.
    ed.switch_focused_pane(pid_a);
    assert!(ed.state.active_session.is_none(), "no session before i");
    ed.handle_key(key('i'));
    assert!(
        ed.state
            .active_session
            .as_ref()
            .is_some_and(|s| s.pane() == pid_a),
        "session open on A after i"
    );
    ed.handle_key(key('X'));
    ed.handle_key(key_esc());
    assert!(
        ed.state.active_session.is_none(),
        "session committed on Esc"
    );

    let rev_after_a = ed.doc().revision_id();

    // Pane B insert session: type 'Y'.
    ed.switch_focused_pane(pid_b);
    assert!(
        ed.state.active_session.is_none(),
        "pane B starts with no session"
    );
    ed.handle_key(key('i'));
    assert!(
        ed.state
            .active_session
            .as_ref()
            .is_some_and(|s| s.pane() == pid_b),
        "session opens on B"
    );
    ed.handle_key(key('Y'));
    ed.handle_key(key_esc());
    assert!(
        ed.state.active_session.is_none(),
        "pane B session committed"
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

/// Cancelling a search whose stash lives on a pane that is no longer
/// focused (a mouse click into another pane always falls through under a
/// minibuf-mode layer, per `minibuf_input`'s own `Mouse` arm, so Search stays
/// open across the click) must still restore that pane's selection and
/// clear *that pane's buffer's* search state, not whatever the click just
/// focused.
///
/// The click moves focus to B. A restore/clear that read `state.focus.id()`
/// or `ed.focused_buffer_id()` would restore nothing on A (B has no stash)
/// and clear B's search state, leaving A's matches highlighted and its
/// cursor wherever live-search's last preview left it.
#[test]
fn d6_search_cancel_targets_the_originating_pane_not_the_focused_one() {
    let tmp = tempfile::tempdir().unwrap();
    let path_b = tmp.path().join("b.txt");
    std::fs::write(&path_b, "xyz\n").unwrap();

    let mut ed =
        editor_from("-[0]>123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz\n");
    let pid_a = ed.state.focus.id();
    let bid_a = ed.focused_buffer_id();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        bid_a,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();
    ed.switch_focused_pane(pid_b);
    ed.execute_typed("e", Some(path_b.to_str().unwrap()))
        .unwrap();
    assert_ne!(
        ed.focused_buffer_id(),
        bid_a,
        "setup: pane B views a distinct buffer"
    );
    ed.switch_focused_pane(pid_a);

    // `/1` on pane A: live preview jumps the cursor to the '1' at char 0
    // (already there), so use a pattern further in so the preview actually
    // moves the cursor and arms `search_pattern`.
    ed.feed_key(key('/'));
    ed.feed_key(key('7'));
    assert!(
        ed.state.buffers.get(bid_a).search_pattern.is_some(),
        "sanity: live search armed on A"
    );
    let head_during_preview = ed.state.panes.state[pid_a][bid_a]
        .view(ed.state.buffers.get(bid_a).text())
        .primary()
        .head()
        .offset();
    assert_ne!(
        head_during_preview,
        co(0),
        "sanity: preview moved the cursor"
    );

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(100, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    // Click into pane B's half of the split (right half of a 100-wide
    // horizontal, i.e. left/right, split). Falls through under the
    // still-open Search layer.
    ed.handle_input(mouse_left_down(70, 0));
    assert_eq!(ed.state.focus.id(), pid_b, "sanity: click moved focus to B");
    assert_eq!(
        ed.state.mode(),
        hume_engine::types::EditorMode::Search,
        "sanity: Search stays open across the click"
    );

    ed.feed_key(key_esc());

    assert_eq!(
        ed.state.panes.state[pid_a][bid_a]
            .view(ed.state.buffers.get(bid_a).text())
            .primary()
            .head()
            .offset(),
        co(0),
        "pane A's pre-search position must be restored"
    );
    assert!(
        ed.state.buffers.get(bid_a).search_pattern.is_none(),
        "pane A's buffer search state must be cleared, not pane B's"
    );
}

/// An edit in the focused pane translates non-acting pane selections via the CS.
///
/// Pane A deletes char 0; pane B's cursor at position 9 must slide to 8.
#[test]
fn d2_edit_in_pane_a_translates_pane_b_selections() {
    // "abcdefghij\n" (11 chars including trailing \n); cursor on 'a'.
    let mut ed = editor_from("-[a]>bcdefghij\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focus.id();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();

    // Position pane B's cursor at char 9 ('j').
    ed.switch_focused_pane(pid_b);
    select(&mut ed, &[(9, 9)], 0);

    // Switch to pane A and delete char 0 ('a').
    ed.switch_focused_pane(pid_a);
    ed.handle_key(key('d')); // delete selection (covers 'a')

    // Pane A's cursor is now at 0 (post-delete); pane B's should be at 8.
    assert_eq!(
        EditView::bind(
            ed.state.buffers.get(bid).text(),
            ed.selections_for(pid_b, bid).unwrap(),
        )
        .primary()
        .head()
        .offset(),
        co(8),
        "pane B selection translated by forward CS"
    );
}

/// Undo in the focused pane propagates the inverse CS to non-acting panes.
///
/// After a delete of 'a' in the focused pane, undo restores 'a'; pane B's
/// cursor at 8 must ride the inverse CS back to 9.
#[test]
fn d3_undo_restores_acting_pane_and_translates_others() {
    let mut ed = editor_from("-[a]>bcdefghij\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focus.id();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();

    // Position pane B at char 9.
    ed.switch_focused_pane(pid_b);
    select(&mut ed, &[(9, 9)], 0);

    // Pane A: delete 'a', then undo.
    ed.switch_focused_pane(pid_a);
    ed.handle_key(key('d'));
    // After delete: pane B at 8. Undo restores 'a'.
    ed.handle_key(key('u'));

    // Pane A's cursor is restored to pre-delete position.
    assert_eq!(
        ed.current_view().primary().head().offset(),
        co(0),
        "pane A cursor restored by undo"
    );
    // Pane B's cursor is translated back to 9 by the inverse CS.
    assert_eq!(
        EditView::bind(
            ed.state.buffers.get(bid).text(),
            ed.selections_for(pid_b, bid).unwrap(),
        )
        .primary()
        .head()
        .offset(),
        co(9),
        "pane B selection translated by inverse CS (undo)"
    );
}

/// A sibling pane's plain cursor at a rewritten line's column 0 clamps past
/// the new indent, via the same `ChangeSet` `>` uses to remap the acting
/// pane's own selections. `SelectionSet::translate` (every command's sibling-pane
/// path) always maps with `Assoc::After`, and `>`'s own remap uses that same
/// association for anything but a linewise selection's start. Both paths only
/// agree because the `ChangeSet` puts the new indent's `Insert` op before the
/// `Delete`: with the opposite order, a position exactly at the line start
/// falls straight into the `Delete` regardless of `Assoc` and collapses to
/// the *old* line start instead.
#[test]
fn indent_sibling_pane_cursor_at_line_start_clamps_past_new_indent() {
    // "  foo\n": two spaces of existing indent.
    let mut ed = editor_from("  -[f]>oo\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focus.id();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();

    // Pane B's cursor sits at column 0, an ordinary cursor that merely
    // happens to be at the line start, not a linewise selection.
    ed.switch_focused_pane(pid_b);
    select(&mut ed, &[(0, 0)], 0);

    // Pane A indents the line. Default settings: tab-style=hard, tab-width=4;
    // the existing 2-space indent (width 2) shifts to width 6, rendered as
    // one tab plus 2 spaces ("\t  ", 3 chars).
    ed.switch_focused_pane(pid_a);
    ed.handle_key(key('>'));

    assert_eq!(
        EditView::bind(
            ed.state.buffers.get(bid).text(),
            ed.selections_for(pid_b, bid).unwrap(),
        )
        .primary()
        .head()
        .offset(),
        co(3),
        "pane B's cursor clamps past the new indent, not back to the old line start"
    );
}

/// Multi-cursor propagation: a deletion that spans two selections in pane B
/// merges them into one (`SelectionSet::translate` merges what it folds together).
#[test]
fn propagate_cs_merges_collapsed_non_acting_pane_selections() {
    // "abcde\n": 6 chars.
    let mut ed = editor_from("-[a]>bcde\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focus.id();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();

    // Pane B: two cursors at positions 2 ('c') and 4 ('e').
    ed.switch_focused_pane(pid_b);
    select(&mut ed, &[(2, 2), (4, 4)], 0);

    // Pane A: select chars 1–4 ("bcde") and delete.
    // First put pane A's selection on 'b'-'e'.
    ed.switch_focused_pane(pid_a);
    // Select 'a' then extend to 'e': use 'v' to enter Select then motion.
    // Simplest: directly set selections and do a delete.
    select(&mut ed, &[(1, 4)], 0);
    ed.handle_key(key('d'));

    // After deleting chars 1-4, pane B's two cursors at 2 and 4 both map to
    // the deletion point (1); they must merge into a single cursor at 1.
    let pane_b_sels = EditView::bind(
        ed.state.buffers.get(bid).text(),
        ed.selections_for(pid_b, bid).unwrap(),
    );
    assert_eq!(
        pane_b_sels.len(),
        1,
        "collapsed selections must merge after propagation"
    );
    assert_eq!(
        pane_b_sels.primary().head().offset(),
        co(1),
        "merged cursor at deletion point"
    );
}

/// Non-focused pane engine mirror is updated by `sync_all_pane_mirrors` after
/// an edit translates the pane's authoritative `SelectionSet`.
///
/// The edit carries pane B's selections but leaves its engine mirror to the
/// per-frame sync: the mirror must stay consistent with `pane_state` when
/// synced via that path.
#[test]
fn pane_engine_mirror_synced_for_non_focused_pane_after_edit() {
    // "abcdefghij\n", cursor on 'a'.
    let mut ed = editor_from("-[a]>bcdefghij\n");
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focus.id();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();

    // Position pane B's cursor at char 5 ('f').
    ed.switch_focused_pane(pid_b);
    select(&mut ed, &[(5, 5)], 0);

    // Switch to pane A and delete char 0 ('a'); the edit carries pane B's
    // authoritative SelectionSet but does not write its engine mirror.
    ed.switch_focused_pane(pid_a);
    ed.handle_key(key('d'));

    // Authoritative selection in pane_state must be at 4 (translated by CS).
    assert_eq!(
        EditView::bind(
            ed.state.buffers.get(bid).text(),
            ed.selections_for(pid_b, bid).unwrap(),
        )
        .primary()
        .head()
        .offset(),
        co(4),
        "pane B pane_state selection translated to 4"
    );

    // Simulate the per-frame sync; this is what write the engine mirror.
    ed.sync_all_pane_mirrors(&ed.view.active_pane_ids());

    // Engine mirror for pane B must now reflect the translated position.
    let mirror_head = ed.view.panes[pid_b]
        .selections
        .as_ref()
        .expect("a written mirror")
        .primary()
        .cursor
        .offset();
    assert_eq!(
        mirror_head,
        co(4),
        "pane B engine mirror head reflects translated position"
    );
}

// ── ensure() contract tests ────────────────────────────────────────────────────

/// ensure() is idempotent: calling it twice on the same (pid, bid) does not
/// overwrite existing state (e.g. selections moved away from initial).
#[test]
fn ensure_is_idempotent() {
    use crate::editor::pane_state;

    let mut ed = editor_from("-[h]>ello\n");
    let pid = ed.state.focus.id();
    let bid = ed.focused_buffer_id();

    // Move the cursor away from its initial position.
    select(&mut ed, &[(3, 3)], 0);

    // ensure() on an already-seeded entry must not reset to initial_sels.
    pane_state::ensure(
        &mut ed.state.panes.state,
        &ed.state.buffers,
        &ed.view.panes,
        pid,
        bid,
    );
    assert_eq!(
        ed.current_view().primary().head().offset(),
        co(3),
        "ensure must not overwrite existing pane_state entry",
    );
}

/// ensure()'s trusted-mint contract: a closed pane's `pid` must panic, not
/// silently seed a ghost `pane_state` entry. See `pane_state::try_ensure`'s
/// own doc for why liveness is checked against the engine's `PanePool`
/// rather than inferred from `pane_state` itself.
#[test]
#[should_panic(expected = "pid must be a live PaneId")]
fn ensure_panics_on_a_closed_pane() {
    use crate::editor::pane_state;

    let mut ed = editor_from("-[a]>aaa\n");
    let (pid_a, bid) = close_pane_leaving_slot_vacant(&mut ed);

    pane_state::ensure(
        &mut ed.state.panes.state,
        &ed.state.buffers,
        &ed.view.panes,
        pid_a,
        bid,
    );
}

/// ensure() on a new (pid, bid) pair seeds the entry with the buffer's initial
/// selections, matching the same value that fresh_from_buf() would produce.
#[test]
fn ensure_seeds_new_entry_with_initial_sels() {
    use crate::editor::pane_state;

    let mut ed = editor_from("-[h]>ello\n");
    let pid = ed.state.focus.id();

    // Open a second buffer; the focused pane has never viewed it.
    let doc2 = Buffer::scratch();
    let expected_sels = doc2.initial_sels();
    let bid2 =
        crate::editor::buffer::lifecycle::open_buffer(&mut ed.view, &mut ed.state.buffers, doc2, 0);

    // The focused pane has never viewed `bid2`, so this is its first-visit seed.
    let state = pane_state::ensure(
        &mut ed.state.panes.state,
        &ed.state.buffers,
        &ed.view.panes,
        pid,
        bid2,
    );
    let text = ed.state.buffers.get(bid2).text();
    assert_eq!(
        test_fixtures::testing::serialize_state(text, state.selections()),
        test_fixtures::testing::serialize_state(text, &expected_sels),
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
    let pid_a = ed.state.focus.id();

    ed.execute_typed("split", None).unwrap();

    assert_eq!(ed.view.panes.len(), 2, "split creates exactly one new pane");
    let pid_b = ed.state.focus.id();
    assert_ne!(pid_b, pid_a, "focus moves to the new pane");
    assert_eq!(
        ed.view.panes[pid_b].buffer_id, bid,
        "new pane views the same buffer"
    );

    match ed.view.layout() {
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

/// `:vsplit` places the new pane side by side with the focused one:
/// `Direction::Horizontal` in the engine (it divides width).
#[test]
fn vsplit_places_pane_side_by_side() {
    use hume_engine::pipeline::{Direction, LayoutTree};

    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("vsplit", None).unwrap();

    match ed.view.layout() {
        LayoutTree::Split { direction, .. } => {
            assert_eq!(*direction, Direction::Horizontal, ":vsplit is side-by-side")
        }
        other => panic!("expected Split layout, got {other:?}"),
    }
}

/// End-to-end regression guard: `:split` typed through the real command-mode
/// dispatch path (`type_cmd`, not `execute_typed`'s shortcut) must still
/// move focus to the new pane. `execute_typed`-based tests above skip the
/// minibuffer entirely, so they wouldn't catch a regression specific to
/// dispatch through an open `Command` layer.
#[test]
fn split_via_command_mode_moves_focus() {
    use hume_engine::pipeline::LayoutTree;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focus.id();

    type_cmd(&mut ed, ":split");

    let pid_b = ed.state.focus.id();
    assert_ne!(
        pid_b, pid_a,
        "focus moves to the new pane through the real command-mode path"
    );
    assert!(
        matches!(*ed.view.layout(), LayoutTree::Split { .. }),
        "layout is a Split"
    );
}

// ── T3: Multi-pane render / prepare_frame ──────────────────────────────────────

/// After `:vsplit`, `prepare_frame` must size both panes from the layout tree,
/// not just the focused one. The sibling pane must tile its half of the
/// terminal rather than keep its `Pane::new` default viewport.
#[test]
fn vsplit_sizes_both_panes_from_layout() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focus.id();
    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.state.focus.id();
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
    let pid_a = ed.state.focus.id();
    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focus.id();

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

/// A third `:vsplit` must resize all three panes to equal widths, not just
/// halve the space of whichever pane was split.
#[test]
fn vsplit_twice_sizes_three_panes_equally() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("vsplit", None).unwrap();
    ed.execute_typed("vsplit", None).unwrap();

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(100, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let widths: Vec<u16> = ed
        .view
        .panes
        .every_pane_across_all_tabs()
        .map(|(_, p)| p.viewport.width)
        .collect();
    assert_eq!(widths.len(), 3);
    let min = *widths.iter().min().unwrap();
    let max = *widths.iter().max().unwrap();
    assert!(
        max - min <= 1,
        "widths {widths:?} not within 1 of each other"
    );
    assert_eq!(
        widths.iter().sum::<u16>() + 2,
        100,
        "three panes plus two 1-column seams must tile the full terminal width"
    );
}

/// The stacked counterpart of [`vsplit_twice_sizes_three_panes_equally`]:
/// a third `:split` must resize all three panes to equal heights.
#[test]
fn split_twice_sizes_three_panes_equally() {
    let mut ed = editor_from("-[h]>ello\n");
    ed.execute_typed("split", None).unwrap();
    ed.execute_typed("split", None).unwrap();

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(80, 41);
    ed.settle();
    ed.prepare_frame(&mut ctx); // 41 rows → 40 usable after statusline

    let heights: Vec<u16> = ed
        .view
        .panes
        .every_pane_across_all_tabs()
        .map(|(_, p)| p.viewport.height)
        .collect();
    assert_eq!(heights.len(), 3);
    let min = *heights.iter().min().unwrap();
    let max = *heights.iter().max().unwrap();
    assert!(
        max - min <= 1,
        "heights {heights:?} not within 1 of each other"
    );
    assert_eq!(
        heights.iter().sum::<u16>() + 2,
        40,
        "three panes plus two 1-row seams must tile the full usable height"
    );
}

/// Closing the middle pane of three equal-width panes must redistribute its
/// space equally between the survivors, not hand it all to one neighbour.
#[test]
fn close_pane_redistributes_space_equally() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focus.id();
    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.state.focus.id();
    ed.execute_typed("vsplit", None).unwrap();
    let pid_c = ed.state.focus.id();

    ed.state.focus.set_for_test(pid_b);
    ed.execute_typed("quit", None).unwrap();
    assert_eq!(ed.view.panes.len(), 2);

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(100, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    let wa = ed.view.panes[pid_a].viewport.width;
    let wc = ed.view.panes[pid_c].viewport.width;
    assert!(
        wa.abs_diff(wc) <= 1,
        "widths {wa} vs {wc} not within 1 of each other after close"
    );
}

// ── T4: Split-too-small guard ────────────────────────────────────────────────

/// `:vsplit` on a pane too narrow to fit two minimum-width panes plus the
/// seam divider refuses with an `Err`, not a degraded split.
#[test]
fn vsplit_too_narrow_is_noop_with_warning() {
    use hume_engine::pipeline::LayoutTree;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focus.id();

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(20, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx); // width 20 < 2*MIN_PANE_WIDTH(10)+1 = 21

    let err = ed
        .execute_typed("vsplit", None)
        .expect_err("a too-narrow pane must refuse the split");
    assert_eq!(err.severity(), Severity::Info);
    assert_eq!(err.message(), "pane too small to split");

    assert_eq!(
        ed.state.focus.id(),
        pid_a,
        "focus does not move: split was rejected"
    );
    assert!(
        matches!(*ed.view.layout(), LayoutTree::Leaf(_)),
        "layout is unchanged"
    );
    assert_eq!(
        ed.state.status_msg.as_deref(),
        Some("pane too small to split")
    );
}

/// `:split` on a pane too short likewise refuses with an `Err`.
#[test]
fn split_too_short_is_noop_with_warning() {
    use hume_engine::pipeline::LayoutTree;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focus.id();

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(80, 7);
    ed.settle();
    ed.prepare_frame(&mut ctx); // 7 rows -> 6 usable after statusline < 2*MIN_PANE_HEIGHT(3)+1 = 7

    let err = ed
        .execute_typed("split", None)
        .expect_err("a too-short pane must refuse the split");
    assert_eq!(err.severity(), Severity::Info);
    assert_eq!(err.message(), "pane too small to split");

    assert_eq!(
        ed.state.focus.id(),
        pid_a,
        "focus does not move: split was rejected"
    );
    assert!(
        matches!(*ed.view.layout(), LayoutTree::Leaf(_)),
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
    let pid_a = ed.state.focus.id();

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(21, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx); // exactly 2*MIN_PANE_WIDTH(10)+1

    ed.execute_typed("vsplit", None).unwrap();

    assert_ne!(
        ed.state.focus.id(),
        pid_a,
        "split succeeds at the threshold"
    );
}

/// Symmetric threshold check for `:split`'s height axis.
#[test]
fn split_at_minimum_height_still_splits() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focus.id();

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(80, 8);
    ed.settle();
    ed.prepare_frame(&mut ctx); // 8 rows -> 7 usable = exactly 2*MIN_PANE_HEIGHT(3)+1

    ed.execute_typed("split", None).unwrap();

    assert_ne!(
        ed.state.focus.id(),
        pid_a,
        "split succeeds at the threshold"
    );
}

/// `fits_split` must judge the *post-equalize* size a split would produce,
/// not the pre-split rect of whichever pane is being split, since
/// `equalize` resizes every pane sharing the axis, a stack of several
/// equal-height panes can still have room for one more even though halving
/// any single pane's current height would not fit.
///
/// 24 rows -> 23 usable after the statusline. 4 stacked panes (3 splits) at
/// 3 seams: (23-3)/4 ≈ 5 rows each, well above `MIN_PANE_HEIGHT`(3), so a
/// naive "halve this pane's current rect" guard (5 > 2*3 is false) would
/// wrongly reject a 5th pane; the actual post-equalize height once there are
/// 5 panes is (23-4)/5 ≈ 3.8 -> 3, which clears the minimum.
#[test]
fn fifth_pane_split_succeeds_when_post_equalize_size_still_fits() {
    let mut ed = editor_from("-[h]>ello\n");

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(80, 24);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    for _ in 0..3 {
        ed.execute_typed("split", None).unwrap();
    }
    assert_eq!(ed.view.panes.len(), 4, "setup: 4 stacked panes");

    ed.execute_typed("split", None).unwrap();

    assert_eq!(
        ed.view.panes.len(),
        5,
        "the split producing a 5th pane must succeed: post-equalize height still fits"
    );
    assert_ne!(
        ed.state.status_msg.as_deref(),
        Some("pane too small to split"),
    );
}

/// A zero-height render area (e.g. a terminal reporting height 0 on early
/// startup) must not panic. `EngineView::render` must guard the statusline
/// (and tab bar) provider's synthesized `Rect` on `area.height`. An
/// unguarded `Rect` claiming a row regardless of `area.height` would send
/// the provider's `Buffer::set_string` calls out-of-bounds and panic, unlike
/// the background fill which clamps.
#[test]
fn zero_height_render_does_not_panic() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    let rect = Rect::new(0, 0, 20, 0);
    assert_eq!(render_to_styled_string(&mut ed, rect), "");
}

/// After `:vsplit`, `render_into` must draw both panes at their own rects,
/// not just the focused one. Both panes view the same buffer here, so the
/// same content must appear in both halves of the styled-frame snapshot.
#[test]
fn vsplit_renders_content_in_both_halves() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = crate::testing::build_snapshot_theme();
    ed.execute_typed("vsplit", None).unwrap();

    let rect = Rect::new(0, 0, 20, 4);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

/// The other half of the test above: with `cursor-shape-insert=block` the
/// focused pane's primary head is *painted* from `ui.cursor.primary.insert`
/// rather than left bare for the real terminal cursor to occupy. Both panes
/// show a themed head here, where the default `bar` leaves only the unfocused
/// one painted.
#[test]
fn insert_block_shape_paints_the_head_in_both_panes() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = crate::testing::build_snapshot_theme();
    ed.state.settings.cursor_shape_insert = crate::editor::settings::CursorShape::Block;
    ed.execute_typed("vsplit", None).unwrap();

    ed.feed_key(key('i'));
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: entered Insert mode");

    let rect = Rect::new(0, 0, 20, 4);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

/// A theme that gives `ui.window` only a `bg` (the common upstream Helix
/// shape: Helix's own border code leaves an unset fg as whatever the
/// terminal already shows) must not leave the seam glyph's foreground
/// unthemed in HUME: it falls back to `ui.text`'s color, the same base every
/// other undecorated surface (an ordinary content line, a virtual line)
/// already falls back to. See `EngineView::render`'s seam block in
/// `hume-engine/src/pipeline/mod.rs`.
#[test]
fn seam_divider_falls_back_to_ui_text_when_window_has_no_fg() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = hume_engine::theme::loader::parse_theme(
        r##"
        "ui.text" = { fg = "#abcdef" }
        "ui.background" = { bg = "#000000" }
        "ui.window" = { bg = "#123456" }
        "##,
    )
    .expect("inline test theme must parse")
    .theme;
    ed.execute_typed("vsplit", None).unwrap();

    let rect = Rect::new(0, 0, 20, 4);
    let frame = render_to_styled_string(&mut ed, rect);
    assert!(
        frame.contains("fg=#abcdef,bg=#123456"),
        "seam glyph must use ui.text's color when ui.window has no fg of its own, got: {frame}"
    );
}

/// Where a horizontal seam meets a vertical seam, the crossing cell must get
/// a proper junction glyph (`┬`), not whichever straight glyph drew last.
/// `:split` stacks A/B, then `:vsplit` on B splits it into B|C; the seam
/// below A meets the seam between B and C in a T shape.
#[test]
fn split_then_vsplit_renders_t_junction_glyph() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = crate::testing::build_snapshot_theme();
    ed.execute_typed("split", None).unwrap();
    ed.execute_typed("vsplit", None).unwrap();

    let rect = Rect::new(0, 0, 20, 8);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

/// A 2×2 grid of panes (both rows split at the same ratio, so their vertical
/// seams align in the same column) must render a full cross (`┼`) where the
/// horizontal and vertical seams meet, not two overlapping straight lines.
/// Same grid shape as `quit_in_grid_promotes_correct_sibling`.
#[test]
fn grid_of_four_panes_renders_cross_junction_glyph() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = crate::testing::build_snapshot_theme();
    let pid_a = ed.state.focus.id();

    ed.execute_typed("split", None).unwrap(); // A/B stacked.
    let pid_b = ed.state.focus.id();

    ed.switch_focused_pane(pid_a);
    ed.execute_typed("vsplit", None).unwrap(); // A/D side by side, top row.

    ed.switch_focused_pane(pid_b);
    ed.execute_typed("vsplit", None).unwrap(); // B/C side by side, bottom row.

    let rect = Rect::new(0, 0, 20, 8);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

/// Entering Insert mode must hide the fake block cursor only in the focused
/// pane (which real terminal bar cursor overlays), not in every pane.
/// `resolve_pane_settings` (frame.rs) forces a block-cursor mode for
/// unfocused panes regardless of the editor's global mode; this locks that
/// per-pane behavior at the render level. `:vsplit` moves focus to the new
/// (right) pane, so the left pane's cursor cell must keep its block style
/// after `i`, while the right pane's cursor cell goes transparent.
#[test]
fn insert_mode_hides_cursor_only_in_focused_pane() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = crate::testing::build_snapshot_theme();
    ed.execute_typed("vsplit", None).unwrap();
    assert_eq!(ed.state.mode(), Mode::Normal, "sanity: starts in Normal");

    ed.feed_key(key('i'));
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: entered Insert mode");

    let rect = Rect::new(0, 0, 20, 4);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

/// A pane created via `open_pane` (the shared core of `:split`/`:vsplit` and the
/// keymap-bound `pane-split`/`pane-vsplit`) must get the same gutter column as
/// the initial pane, not the empty `ProviderSet` `Pane::new` alone would give
/// it. Uses the real `Editor::open` constructor (not the bare-pane `for_testing`
/// harness used elsewhere in this file) so the initial pane reflects actual
/// production setup. It compares the split pane's
/// `gutter_columns().count()` against the pre-existing initial pane's, rather
/// than asserting a hardcoded count that could pass even if both were wrongly
/// empty.
#[test]
fn split_pane_gets_gutter_column() {
    let mut ed = open_headless(None);
    let pid_a = ed.state.focus.id();
    let initial_gutter_column_count = ed.view.panes[pid_a].providers.gutter_columns().count();
    assert!(
        initial_gutter_column_count > 0,
        "sanity: the initial pane must itself have a gutter column"
    );

    let bid = ed.focused_buffer_id();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();

    assert_eq!(
        ed.view.panes[pid_b].providers.gutter_columns().count(),
        initial_gutter_column_count,
        "split pane must have the same gutter columns as the initial pane"
    );
}

// ── T5: `:q` pane-awareness + close-pane semantics ──────────────────────────
//
// `close_focused_pane` backs both `:q` (multi-pane branch, exercised here)
// and the keymap-bound `pane-close` (`Ctrl-p c`, exercised in kitty.rs).

/// With multiple panes open, `:q` closes the focused pane instead of the
/// editor, and moves focus to the promoted sibling.
#[test]
fn quit_with_multiple_panes_closes_focused_pane_not_editor() {
    use hume_engine::pipeline::LayoutTree;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focus.id();
    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focus.id();
    assert_ne!(pid_a, pid_b);

    ed.execute_typed("quit", None).unwrap();

    assert!(
        !ed.state.should_quit,
        ":q with panes open must not quit the editor"
    );
    assert_eq!(ed.view.panes.len(), 1, "the focused pane is closed");
    assert_eq!(
        ed.state.focus.id(),
        pid_a,
        "focus returns to the surviving pane"
    );
    assert!(
        matches!(*ed.view.layout(), LayoutTree::Leaf(id) if id == pid_a),
        "layout collapses back to a single leaf"
    );
}

/// The multi-pane `:q` branch skips the dirty check entirely: closing a pane
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
    assert!(focused_unsaved(&ed), "sanity: buffer is dirty");

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
    let pid_a = ed.state.focus.id();
    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focus.id();
    assert_ne!(pid_a, pid_b);

    // Dirty the buffer both panes are viewing.
    ed.handle_key(key('i'));
    ed.handle_key(key('x'));
    ed.handle_key(key_esc());
    assert!(focused_unsaved(&ed), "sanity: buffer is dirty");

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
        ed.state.focus.id(),
        pid_a,
        "focus returns to the surviving pane"
    );
    assert!(!focused_unsaved(&ed), "the write must have happened");
    assert_eq!(
        std::fs::read_to_string(&tmp).unwrap(),
        expected_content,
        "the edit must have been written to disk before the pane closed"
    );
}

/// `viewport_debounce`/`last_visible_range`/`virtual_lines_synced` live on
/// `Editor` rather than `EditorState.panes`, so `drop_pane_state` can't clear
/// them directly. `prepare_frame`'s `prune_closed_pane_caches` sweep is the
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
    let pid_b = ed.state.focus.id();
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    assert!(
        ed.last_visible_range.contains_key(&pid_b),
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
        !ed.last_visible_range.contains_key(&pid_b),
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
/// just `decorations.generation()`. A generation-only key would keep a pane
/// mirroring its *previous* buffer's virtual lines after a switch, since
/// switching a buffer doesn't bump the generation.
#[test]
fn switching_a_panes_buffer_rebuilds_its_virtual_lines() {
    use hume_decorations::VirtualLineEntry;

    let mut ed = editor_from("-[h]>ello\n");
    let bid_a = ed.focused_buffer_id();
    // `editor_from`'s bootstrap pane is built via `Pane::new` directly, with
    // no `panes.render` entry (see `Editor::for_testing`'s comment); only
    // `open_pane` seeds one, so this test opens a second pane rather than
    // using the bootstrap one.
    let pid_a = ed.state.focus.id();
    let pid = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        bid_a,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();
    ed.switch_focused_pane(pid);

    let scope = ed.view.registry.intern("ui.virtual");
    ed.state.config.decorations.set_virtual_lines(
        "test".to_string(),
        bid_a,
        vec![VirtualLineEntry {
            pos: co(0),
            text: "deleted".to_string(),
            before: false,
            scope,
            segments: Vec::new(),
        }],
    );

    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut hume_engine::pipeline::RenderContext::new());

    assert!(
        ed.state.panes.render[pid]
            .virtual_lines()
            .contains_key(&hume_rope::line::ContentLine::new(0)),
        "sanity: pane A mirrors buffer A's virtual line at line 0"
    );

    // Switch the same pane to a fresh buffer with no virtual lines set.
    let bid_b = ed.open_buffer(Buffer::scratch());
    ed.switch_to_buffer_with_jump(FocusedPane::current(&ed.state), bid_b);
    ed.sync_viewport_dims(80, 25);
    ed.settle();
    ed.prepare_frame(&mut hume_engine::pipeline::RenderContext::new());

    assert!(
        ed.state.panes.render[pid].virtual_lines().is_empty(),
        "after switching to buffer B, the pane must no longer mirror buffer \
         A's virtual lines; a generation-only sync gate would leave line 0 \
         populated with A's stale entry"
    );
}

/// Closing one leaf of a 2×2 grid promotes the correct sibling and leaves the
/// other three panes untouched. The expected post-close
/// shape (`Split{Leaf(A), Split{B, C}}`) is derived from how the grid was
/// built, not by re-running the close logic.
#[test]
fn quit_in_grid_promotes_correct_sibling() {
    use hume_engine::pipeline::LayoutTree;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focus.id();

    // A/B stacked.
    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focus.id();

    // A/D side by side, top row.
    ed.switch_focused_pane(pid_a);
    ed.execute_typed("vsplit", None).unwrap();
    let pid_d = ed.state.focus.id();

    // B/C side by side, bottom row. Grid is now: top (A, D), bottom (B, C).
    ed.switch_focused_pane(pid_b);
    ed.execute_typed("vsplit", None).unwrap();
    let pid_c = ed.state.focus.id();

    assert_eq!(ed.view.panes.len(), 4, "sanity: four panes in the grid");

    // Close D (top-right): its sibling A is promoted, collapsing the top row
    // to a single leaf; the bottom row (B, C) is untouched.
    ed.switch_focused_pane(pid_d);
    ed.execute_typed("quit", None).unwrap();

    assert_eq!(ed.view.panes.len(), 3, "one pane closed");
    assert_eq!(
        ed.state.focus.id(),
        pid_a,
        "A is promoted as D's surviving sibling"
    );
    assert!(!ed.view.panes.contains_key(pid_d), "D was closed");

    match ed.view.layout() {
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

// ── wrap_mode: pane override → buffer override → global ───────────────────────

/// A same-buffer split (`:split` with no path) inherits the source pane's
/// live override, not the global default. This lets a `:wrap`-toggled
/// pane pass its mode on to a split of itself.
#[test]
fn same_buffer_split_inherits_source_panes_wrap_override() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focus.id();
    ed.view.panes[pid_a].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Soft { width: 40 }),
        saved: None,
    });
    // Global default differs, to prove it is NOT the source.
    ed.state.settings.wrap_mode = hume_engine::pane::WrapMode::None;

    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focus.id();

    assert_eq!(
        ed.view.panes[pid_b].wrap().mode,
        Some(hume_engine::pane::WrapMode::Soft { width: 40 }),
        "same-buffer split inherits the source pane's live override"
    );
}

/// The other half of the split-inheritance contract: a source pane with *no*
/// pane-level override (still inheriting from the buffer/global setting)
/// splits into a pane that is likewise unpinned, not one frozen at
/// whichever mode the source happened to resolve to. The new pane keeps
/// following later `:set buffer`/`:set global wrap-mode=…` changes, same as
/// the pane it split from.
#[test]
fn same_buffer_split_of_an_unpinned_pane_stays_unpinned() {
    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focus.id();
    assert_eq!(ed.view.panes[pid_a].wrap().mode, None, "sanity: unpinned");

    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focus.id();

    assert_eq!(
        ed.view.panes[pid_b].wrap().mode,
        None,
        "split of an unpinned pane is itself unpinned, not frozen at the \
         resolved mode"
    );
}

/// `:wrap` toggles only the focused pane's override; a sibling pane on the
/// same buffer is untouched. The override lives on `Pane`, not on the
/// buffer, so two panes viewing the same buffer can wrap independently once
/// one is pinned.
#[test]
fn wrap_toggle_affects_only_focused_pane() {
    let mut ed = editor_from("-[h]>ello\n");
    // Global is resolved lazily on every read, so this reaches pid_a's
    // effective mode retroactively; no pane pin needed for A to start off.
    ed.state.settings.wrap_mode = hume_engine::pane::WrapMode::None;
    let pid_a = ed.state.focus.id();

    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focus.id();
    assert_ne!(pid_a, pid_b);
    assert_eq!(
        ed.view.panes[pid_a].buffer_id, ed.view.panes[pid_b].buffer_id,
        "sanity: both panes view the same buffer"
    );

    // Focus is on B (the new pane) after :split; toggle wrap there.
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
// `EngineView` must not cache a `pane_rects` snapshot across frames. Every
// consumer (pane-focus commands, `fits_split`, the bar-cursor lookup) must
// recompute from the live layout tree plus the terminal area cached by the
// last `prepare_frame`. A cache would let a close/split earlier in the same
// macro-replay batch invalidate geometry that a later command in the same
// batch would otherwise still trust.

/// Closing a pane and then focusing the next one, with no `prepare_frame` in
/// between (exactly what a macro-replay batch does: several commands run
/// per frame), must land on the surviving pane. A cached rect list would
/// still list the just-closed pane, handing focus to a dead `PaneId`.
#[test]
fn close_then_focus_next_without_reframe_lands_on_live_pane() {
    use crate::editor::commands::{FocusedPane, cmd_pane_focus_next};
    use hume_ops::MotionMode;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focus.id();
    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focus.id();
    assert_ne!(pid_a, pid_b);

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(100, 51);
    ed.settle();
    ed.prepare_frame(&mut ctx); // establish terminal geometry once

    // Close B, then immediately focus-next, no `prepare_frame` in between.
    ed.execute_typed("quit", None).unwrap();
    assert_eq!(ed.view.panes.len(), 1, "sanity: B is closed");
    let fp = FocusedPane::current(&ed.state);
    cmd_pane_focus_next(&mut ed.state, &mut ed.view, fp, 1, MotionMode::Move).unwrap();

    assert_eq!(
        ed.state.focus.id(),
        pid_a,
        "focus-next after a same-batch close must land on the live survivor"
    );
    assert!(ed.view.panes.contains_key(ed.state.focus.id()));
}

/// Splitting and then focusing directionally, with no `prepare_frame` in
/// between, must reach the freshly created pane. A cached rect list from
/// before the split would still show only the original pane, so the focus
/// command would silently no-op instead of moving to the new pane.
#[test]
fn split_then_focus_left_without_reframe_reaches_new_pane() {
    use crate::editor::commands::{FocusedPane, cmd_pane_focus_left};
    use hume_ops::MotionMode;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focus.id();

    let mut ctx = hume_engine::pipeline::RenderContext::new();
    ed.sync_viewport_dims(100, 51);
    ed.settle();
    ed.prepare_frame(&mut ctx); // geometry established with one pane

    // :vsplit puts the new pane on the right and moves focus to it, with no
    // `prepare_frame` in between.
    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.state.focus.id();
    assert_ne!(pid_a, pid_b);

    let fp = FocusedPane::current(&ed.state);
    cmd_pane_focus_left(&mut ed.state, &mut ed.view, fp, 1, MotionMode::Move).unwrap();
    assert_eq!(
        ed.state.focus.id(),
        pid_a,
        "focus-left from the freshly split pane must reach the original pane"
    );
}

/// A bare split (same buffer as the source pane) inherits the source pane's
/// cursor and scroll position, rather than jumping to the top of the file.
#[test]
fn split_inherits_focused_panes_selection_and_scroll() {
    let content: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let text = BufferText::from(content.as_str());
    let sels = sels_at(&text, &[(0, 0)], 0);
    let mut ed = Editor::for_testing_with(test_fixtures::testing::state(text, sels));
    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focus.id();

    // Move A's cursor and scroll well away from the top of the file.
    let cursor_pos = ed
        .doc()
        .text()
        .line_to_char(hume_rope::line::RopeyLine::new(150));
    set_cursor(&mut ed, cursor_pos.index());
    ed.view.panes[pid_a].viewport.seed_top_for_test(
        hume_engine::display_lines::DisplayLinePos::new(hume_rope::line::ContentLine::new(140), 0),
    );

    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.state.focus.id();
    assert_ne!(pid_a, pid_b);

    assert_eq!(
        ed.state.panes.state[pid_b][bid].selections(),
        ed.state.panes.state[pid_a][bid].selections(),
        "new pane inherits the source pane's selection"
    );
    assert_eq!(
        ed.view.panes[pid_b].viewport.top().line,
        hume_rope::line::ContentLine::new(140),
        "new pane inherits the source pane's scroll position"
    );
}

/// A same-buffer split inherits the source pane's `saved_scrolls`: its
/// memory of where it was in buffers visited *before* the split. Without
/// this, the new pane would reset such a buffer to the top on first visit
/// instead of recalling where the source pane last left it.
#[test]
fn same_buffer_split_inherits_saved_scrolls() {
    use crate::editor::buffer::lifecycle::open_buffer;
    use hume_engine::pane::ScrollPosition;

    let mut ed = editor_from("-[h]>ello\n");
    let pid_a = ed.state.focus.id();

    // A second buffer the source pane visited (and scrolled) before the
    // split, then switched away from. This is what populates
    // `saved_scrolls` in real usage (see `remember_scroll`).
    let bid2 = open_buffer(&mut ed.view, &mut ed.state.buffers, Buffer::scratch(), 0);
    ed.view.panes[pid_a].saved_scrolls.insert(
        bid2,
        ScrollPosition {
            top: hume_engine::display_lines::DisplayLinePos::new(
                hume_rope::line::ContentLine::new(42),
                0,
            ),
            horizontal_offset: hume_rope::column::DisplayLineCol::new(0),
        },
    );

    ed.execute_typed("split", None).unwrap();
    let pid_b = ed.state.focus.id();
    assert_ne!(pid_a, pid_b);

    assert_eq!(
        ed.view.panes[pid_b].saved_scrolls.get(bid2),
        ed.view.panes[pid_a].saved_scrolls.get(bid2),
        "new pane inherits the source pane's saved_scrolls history"
    );
}

/// Before the first `prepare_frame`, there is no real terminal geometry to
/// check a split against, so `fits_split` must allow it; the next
/// `prepare_frame` sizes the result correctly regardless.
#[test]
fn fits_split_allows_before_first_frame() {
    use hume_engine::pipeline::Direction;

    let ed = editor_from("-[h]>ello\n");
    let fp = FocusedPane::current(&ed.state);
    assert!(crate::editor::commands::fits_split(
        &ed.view,
        fp,
        Direction::Vertical
    ));
    assert!(crate::editor::commands::fits_split(
        &ed.view,
        fp,
        Direction::Horizontal
    ));
}

/// If the focused pane isn't reachable in the active layout tree (an
/// invariant violation that should never happen), `split_pane_onto` must
/// refuse before creating anything: `open_pane_in_layout` checks
/// `contains_leaf` up front, so there is never a speculatively created pane
/// to roll back, and no window where one exists with no layout leaf (which
/// would later violate `close_focused_pane`'s precondition on
/// `remove_leaf`).
#[test]
fn split_pane_onto_refuses_when_focused_pane_missing_from_layout() {
    use hume_engine::pipeline::Direction;

    let mut ed = editor_from("-[h]>ello\n");
    let bid = ed.focused_buffer_id();

    // Fabricate the desync directly, but through a real pane: open
    // a second tab (a real, properly-attached pane, just attached to *that*
    // tab's own layout, not the active one), then reuse its id as the active
    // tab's `focus`. That reproduces the same condition
    // `contains_leaf` must defend against (a focused pane the active layout
    // doesn't reach) without ever constructing a pane that was never
    // attached to any tab at all.
    ed.execute_typed("tabnew", None).unwrap();
    let other_tab_pid = ed.state.focus.id();
    ed.execute_typed("tabprev", None).unwrap();
    ed.state.focus.set_for_test(other_tab_pid);
    let fp = FocusedPane::current(&ed.state);
    let panes_before = ed.view.panes.len();

    let result = crate::editor::commands::split_pane_onto(
        &mut ed.state,
        &mut ed.view,
        fp,
        bid,
        Direction::Vertical,
    );

    assert!(
        result.is_err(),
        "a missing split target must surface as an error"
    );
    assert_eq!(
        ed.view.panes.len(),
        panes_before,
        "no pane is created at all when the split target is unreachable"
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
/// column between them), and the non-focused pane is still visibly dimmed:
/// dimming is the focus cue, independent of the divider glyph.
#[test]
fn vsplit_dividers_off_tiles_edge_to_edge_and_still_dims() {
    use super::render_snapshot::render_to_styled_string;

    let mut ed = editor_from("-[a]>bc\n");
    ed.view.theme = crate::testing::build_snapshot_theme();
    ed.state.settings.pane_dividers = false;
    ed.execute_typed("vsplit", None).unwrap();

    let rect = Rect::new(0, 0, 20, 4);
    insta::assert_snapshot!(render_to_styled_string(&mut ed, rect));
}

/// A same-buffer split inherits the source pane's jump history so the new
/// pane can Ctrl-o back to positions visited before the split. The two lists
/// then diverge: a new jump in either pane does not affect the other.
#[test]
fn split_same_buffer_clones_jump_list_then_diverges() {
    let mut ed = jump_editor(10);
    let pid_a = ed.state.focus.id();

    // `gg` (goto-first-line) is a jump command: records the pre-jump position.
    ed.handle_key(key('g'));
    ed.handle_key(key('g'));
    assert_eq!(
        ed.state.panes.jumps[pid_a].len(),
        1,
        "source pane has one jump entry after gg"
    );

    // Same-buffer split: new pane inherits the source pane's jump history.
    ed.execute_typed("vsplit", None).unwrap();
    let pid_b = ed.state.focus.id();
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
// state read by every pane's `ScopedHighlighter`; each pane owns its own
// highlight buffers (`PaneHighlights`), computed from that pane's own buffer
// and viewport. Global, focused-buffer-only state would render the focused
// pane's highlight bytes (bracket/search matches) onto every other pane's
// unrelated text, including one viewing a different buffer or the same
// buffer scrolled elsewhere.

/// A search match that spans a `\n` must produce one highlight span per line
/// it touches, each clipped to that line's own content, not a single span
/// computed by converting the match's absolute end offset through whichever
/// line the *start* happened to be on (which gives a corrupt or inverted
/// span whenever a match crosses a line boundary).
///
/// Byte offsets are hand-computed from the known ASCII
/// content ("abc\ndef\n"), not derived by calling the code under test.
#[test]
fn multiline_search_match_splits_into_per_line_highlight_spans() {
    let mut ed = open_headless(None);
    let pid = ed.state.focus.id();

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

    // Every span shares the one search-match scope (`ScopedHighlighter`
    // carries it per-span now, not fixed on the provider), dropped here
    // since this test is about span geometry, not scope resolution.
    let matches: Vec<(usize, usize, usize)> = pane_highlights(&ed, pid, HighlightTier::SearchMatch)
        .into_iter()
        .map(|(line, start, end, _)| (line.index(), start.index(), end.index()))
        .collect();
    assert_eq!(
        matches,
        vec![(0, 2, 3), (1, 0, 3)],
        "line 0 gets 'c' clipped before its own '\\n' (byte 2..3); \
         line 1 gets 'def' from its own start (byte 0..3)"
    );
}

/// Opens a split on the same buffer, focuses the new pane, feeds `keys`
/// there to open a minibuffer session, then closes that pane through the
/// command path a hook or timer would use. Returns the surviving pane.
fn close_pane_under_open_session(ed: &mut Editor, keys: &str) -> PaneId {
    use hume_scripting::PaneHandle;
    use hume_scripting::host::CommandHost;

    let bid = ed.focused_buffer_id();
    let pid_a = ed.state.focus.id();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .expect("split must succeed");
    ed.switch_focused_pane(pid_b);
    for ch in keys.chars() {
        ed.feed_key(key(ch));
    }
    live_host!(ed)
        .run_command_sync(
            "pane-close",
            PaneHandle::with_pane(bid, pid_b),
            None,
            false,
            None,
        )
        .expect("pane-close must succeed");
    ed.settle();
    pid_a
}

#[test]
fn closing_the_pane_of_an_open_search_ends_the_search() {
    let mut ed = editor_from("-[a]>bc abc\n");
    let pid_a = close_pane_under_open_session(&mut ed, "/b");
    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Normal);
    ed.feed_key(key_esc());
    ed.feed_key(key('l'));
    assert_eq!(ed.state.focus.id(), pid_a);
    assert_eq!(state(&ed), "a-[b]>c abc\n");
    assert!(
        ed.state
            .buffers
            .get(ed.focused_buffer_id())
            .search_pattern
            .is_none(),
        "the closed pane's live search is cleared from the buffer"
    );
}

#[test]
fn closing_the_pane_of_an_open_sift_ends_the_sift() {
    let mut ed = editor_from("-[a]>bc abc\n");
    let pid_a = close_pane_under_open_session(&mut ed, "%sb");
    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Normal);
    ed.feed_key(key_esc());
    ed.feed_key(key('l'));
    assert_eq!(ed.state.focus.id(), pid_a);
    assert_eq!(state(&ed), "a-[b]>c abc\n");
}
