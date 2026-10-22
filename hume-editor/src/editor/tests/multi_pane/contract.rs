//! The multi-pane state contract and `ensure`.

use super::*;
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
    let pid_a = ed.view.focused_pane();

    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    );

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
    let pid_a = ed.view.focused_pane();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    );

    // Both panes see Buffer.search_pattern: it's a single field on `doc`.
    // Verify independence of search_cursor: write distinct values per pane.
    ed.state.panes.entries.seeded_mut(pid_a, bid).search_cursor = SearchCursor {
        match_count: Some((1, 3)),
        wrapped: false,
        ..SearchCursor::default()
    };
    ed.state.panes.entries.seeded_mut(pid_b, bid).search_cursor = SearchCursor {
        match_count: Some((2, 3)),
        wrapped: true,
        ..SearchCursor::default()
    };

    // Pane A and pane B see different cursors even though they share the buffer.
    assert_eq!(
        ed.state
            .panes
            .entries
            .seeded(pid_a, bid)
            .search_cursor
            .match_count,
        Some((1, 3))
    );
    assert!(
        !ed.state
            .panes
            .entries
            .seeded(pid_a, bid)
            .search_cursor
            .wrapped
    );

    assert_eq!(
        ed.state
            .panes
            .entries
            .seeded(pid_b, bid)
            .search_cursor
            .match_count,
        Some((2, 3))
    );
    assert!(
        ed.state
            .panes
            .entries
            .seeded(pid_b, bid)
            .search_cursor
            .wrapped
    );
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
    let pid_a = ed.view.focused_pane();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    );

    // Pane A insert session: type 'X' at the start.
    ed.switch_focused_pane(pid_a);
    assert!(ed.state.open_session().is_none(), "no session before i");
    ed.handle_key(key('i'));
    assert!(
        ed.state
            .input
            .insert_session()
            .is_some_and(|s| s.pane() == pid_a),
        "session open on A after i"
    );
    ed.handle_key(key('X'));
    ed.handle_key(key_esc());
    assert!(
        ed.state.open_session().is_none(),
        "session committed on Esc"
    );

    let rev_after_a = ed.doc().revision_id();

    // Pane B insert session: type 'Y'.
    ed.switch_focused_pane(pid_b);
    assert!(
        ed.state.open_session().is_none(),
        "pane B starts with no session"
    );
    ed.handle_key(key('i'));
    assert!(
        ed.state
            .input
            .insert_session()
            .is_some_and(|s| s.pane() == pid_b),
        "session opens on B"
    );
    ed.handle_key(key('Y'));
    ed.handle_key(key_esc());
    assert!(
        ed.state.open_session().is_none(),
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
/// The click moves focus to B. A restore/clear that read `view.focused_pane()`
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
    let pid_a = ed.view.focused_pane();
    let bid_a = ed.focused_buffer_id();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        bid_a,
        hume_engine::pipeline::Direction::Horizontal,
    );
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
    let head_during_preview = ed
        .state
        .panes
        .entries
        .seeded(pid_a, bid_a)
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
    ed.resize_terminal(100, 25);
    ed.settle();
    ed.prepare_frame(&mut ctx);

    // Click into pane B's half of the split (right half of a 100-wide
    // horizontal, i.e. left/right, split). Falls through under the
    // still-open Search layer.
    draw(&mut ed);
    ed.handle_input(mouse_left_down(70, 0));
    assert_eq!(
        ed.view.focused_pane(),
        pid_b,
        "sanity: click moved focus to B"
    );
    assert_eq!(
        ed.state.mode(),
        hume_engine::types::EditorMode::Search,
        "sanity: Search stays open across the click"
    );

    ed.feed_key(key_esc());

    assert_eq!(
        ed.state
            .panes
            .entries
            .seeded(pid_a, bid_a)
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
    let pid_a = ed.view.focused_pane();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    );

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
    let pid_a = ed.view.focused_pane();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    );

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
    let pid_a = ed.view.focused_pane();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    );

    // Pane B's cursor sits at column 0, an ordinary cursor that merely
    // happens to be at the line start, not a linewise selection.
    ed.switch_focused_pane(pid_b);
    select(&mut ed, &[(0, 0)], 0);

    // Pane A indents the line. Default settings: indent-style=hard, indent-width=4;
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
    let pid_a = ed.view.focused_pane();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    );

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
    let pid_a = ed.view.focused_pane();
    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    );

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
    let pid = ed.view.focused_pane();
    let bid = ed.focused_buffer_id();

    // Move the cursor away from its initial position.
    select(&mut ed, &[(3, 3)], 0);

    // ensure() on an already-seeded entry must not reset to initial_sels.
    pane_state::ensure(
        &mut ed.state.panes.entries,
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
        &mut ed.state.panes.entries,
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
    let pid = ed.view.focused_pane();

    // Open a second buffer; the focused pane has never viewed it.
    let doc2 = Buffer::scratch();
    let expected_sels = doc2.initial_sels();
    let bid2 =
        crate::editor::buffer::lifecycle::open_buffer(&mut ed.view, &mut ed.state.buffers, doc2, 0);

    // The focused pane has never viewed `bid2`, so this is its first-visit seed.
    let state = pane_state::ensure(
        &mut ed.state.panes.entries,
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
