//! A resize, a wrap-mode change, a virtual-line block appearing above the
//! cursor, an inlay hint or EOL text wrapping an earlier line, a buffer
//! switch, a tab-width or scrolloff change, or a gutter that widens for a
//! longer line count must still replace the caret even though the cursor
//! itself is unmoved between the two frames each test drives. None of these
//! fixtures move the primary head, so `PaneBufferState::reveal_pending`
//! alone would stay `false` for every one of them — each instead shows up
//! as some field of `EditorState::layout_key(pane)` differing from
//! `PaneBufferState::last_layout_key`, which `frame.rs`'s scroll step
//! compares every frame. These tests pin that every layout input the
//! mechanism is supposed to cover actually reaches it, plus (the last test)
//! that a revisit with nothing changed does *not* — a parked view stays
//! parked rather than snapping back onto the cursor on every buffer switch.

use super::*;
use crate::editor::commands::open_pane_in_layout;

/// A resize with the cursor unmoved must still replace the caret: a pane's
/// `height` is part of `EditorState::layout_key`, so a shrunk viewport
/// differs from `PaneBufferState::last_layout_key` and `frame.rs`'s scroll
/// step runs `Viewport::reveal`, scrolling the (unchanged) viewport top far
/// enough for the shrunk height to still cover the cursor, rather than
/// trusting a stale top left over from the taller frame.
#[test]
fn a_height_change_replaces_the_caret_even_when_the_cursor_has_not_moved() {
    let content: String = numbered_lines(60);
    let mut ed = unwrapped_editor(&content, 0);
    seek_to_line(&mut ed, 40);

    assert!(
        frame(&mut ed, 80, 50).cursor_content_pos.is_some(),
        "sanity: the cursor is visible in the tall viewport"
    );

    assert!(
        frame(&mut ed, 80, 10).cursor_content_pos.is_some(),
        "the caret must still be placed after a resize, even though the \
         cursor itself did not move"
    );
}

/// A wrap-mode change with the cursor unmoved must still replace the caret:
/// `wrap_mode` (resolved through `commands::effective_wrap_mode`, the same
/// resolver `EditorState::layout_key` calls) is part of that key, so an
/// actual mode change differs from `PaneBufferState::last_layout_key` and
/// `frame.rs`'s scroll step re-places the caret against wrapping's
/// now-different display-line layout rather than trusting the unwrapped
/// frame's screen position.
#[test]
fn a_wrap_mode_change_replaces_the_caret_even_when_the_cursor_has_not_moved() {
    let content: String = (0..60)
        .map(|i| format!("{i} {}\n", "x".repeat(120)))
        .collect();
    let mut ed = unwrapped_editor(&content, 0);
    seek_to_line(&mut ed, 20);

    assert!(
        frame(&mut ed, 80, 15).cursor_content_pos.is_some(),
        "sanity: the cursor is visible unwrapped"
    );

    // Through the real production path (`:set pane wrap-mode=…`), not a raw
    // `Pane::set_wrap` poke: `layout_key` reads the pane's live, resolved
    // wrap mode regardless of which path set it, so a poke would exercise
    // the same derivation either way — this still drives the production
    // path because that's what a real `:wrap`/`:set` change looks like, not
    // because a poke would bypass anything.
    run_set(&mut ed, "pane wrap-mode=soft:0").expect(":set pane wrap-mode=soft:0 failed");

    assert!(
        frame(&mut ed, 80, 15).cursor_content_pos.is_some(),
        "the caret must still be placed after a wrap-mode change, even \
         though the cursor's DisplayLinePos is unchanged"
    );
}

/// A virtual-line block appearing above the cursor between two frames, with
/// no input in between, must still replace the caret: `EditorState::layout_key`'s
/// `buffer_tag` carries `decorations.generation(bid)`, which
/// `update_virtual_line_providers` bumps by adding the block, so `frame.rs`'s
/// scroll step re-scrolls the viewport to follow the cursor's now-lower
/// display row rather than trusting `content_pos`'s plain re-lookup against
/// the unchanged top — which, five rows below the settled scrolloff target
/// in a 10-row viewport, would find the cursor's row past the bottom edge
/// and answer `None`.
#[test]
fn a_virtual_line_block_above_the_cursor_replaces_the_caret_even_when_the_cursor_has_not_moved() {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    pin_no_wrap(&mut ed);
    type_text(&mut ed, &numbered_lines(40));
    seek_to_line(&mut ed, 20);
    let bid = ed.focused_buffer_id();

    assert!(
        frame(&mut ed, 40, 10).cursor_content_pos.is_some(),
        "sanity: the cursor is visible before any virtual lines exist"
    );

    // A 5-row `Before` block anchored on the cursor's own line pushes its
    // display row down by 5 — past the 10-row viewport's settled bottom
    // margin (scrolloff 3, target row 6) if the viewport doesn't re-scroll
    // to follow it.
    let pos = hume_rope::lines::line_start_char(
        ed.doc().text().rope(),
        hume_rope::line::RopeyLine::new(20),
    );
    let scope = ed.view.registry.intern("diagnostic.warning");
    ed.state.config.decorations.set_virtual_lines(
        "linter".to_string(),
        bid,
        (0..5)
            .map(|_| hume_decorations::VirtualLineEntry {
                pos,
                text: "V".to_string(),
                before: true,
                scope,
                segments: Vec::new(),
            })
            .collect(),
    );

    assert!(
        frame(&mut ed, 40, 10).cursor_content_pos.is_some(),
        "the caret must still be placed once a virtual-line block appears \
         above the cursor, even though the cursor's own CharOffset never moved"
    );
}

/// Switching a pane to another buffer between two frames must still replace
/// the caret, even in the case a purely selection-driven signal would miss:
/// the recalled selection in the new buffer happens to have the exact same
/// primary head `CharOffset` as the outgoing buffer's, so a head-comparison
/// alone would see no change. `EditorState::layout_key`'s `buffer_tag` names
/// the buffer, so switching to a different one always differs from
/// `PaneBufferState::last_layout_key` — this pane has never viewed the
/// second buffer before, so that field reads `None` for it, which differs
/// from anything. `Pane::recall_scroll` resets the viewport's own top to
/// the document's first line on a pane's first visit to a buffer, so the
/// matching head — deep in a 60-line buffer here — is left far outside it
/// unless something re-scrolls to find it.
#[test]
fn a_buffer_switch_replaces_the_caret_even_when_the_recalled_head_matches() {
    let content: String = numbered_lines(60);
    let mut ed = unwrapped_editor(&content, 0);
    seek_to_line(&mut ed, 30);
    let shared_head = ed.current_selections().primary().head();

    assert!(
        frame(&mut ed, 80, 15).cursor_content_pos.is_some(),
        "sanity: the cursor is visible in the first buffer"
    );

    // A second buffer with identical content, so `shared_head` names the
    // same line in it too, and an initial selection collapsed there — this
    // pane has never visited it, so its own primary head numerically
    // matches the outgoing buffer's right from the switch.
    let second_text = BufferText::from(content.as_str());
    let second_sels =
        SelectionSet::single(hume_editing::selection::Selection::collapsed(shared_head));
    let second_bid = ed.open_buffer(Buffer::new(second_text, second_sels));

    ed.switch_to_buffer_without_jump(second_bid);

    assert!(
        frame(&mut ed, 80, 15).cursor_content_pos.is_some(),
        "the caret must still be placed against the new buffer's own \
         reset-to-top viewport, even though the recalled head's CharOffset \
         numerically matches the outgoing buffer's"
    );
}

/// Fixture shared by the inlay-hint and EOL-text wrap tests below: 40 lines,
/// wrap at a fixed 10-column width (independent of pane/gutter width), line
/// 19 exactly 9 columns — fits one display row alone, wraps to two once a
/// 2-column decoration pushes it past the width-10 wrap boundary. `scrolloff`
/// zeroed so the scrolloff *margin* can't itself explain a `None` — a bare
/// 1-row push must already exceed the viewport's raw height to prove the
/// caret would otherwise vanish, not just drift out of the margin band.
/// Cursor seeks to line 20, one line *after* the wrapping line — between the
/// settled top and the cursor, so the extra row actually falls in the span
/// `content_pos`'s `top`-to-`cursor` walk crosses.
fn wrap_earlier_line_fixture() -> (Editor, BufferId, hume_rope::offset::CharOffset) {
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.state.settings.scrolloff = 0;
    ed.view.panes[ed.state.focus.id()].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Soft { width: 10 }),
        saved: None,
    });
    let mut content = String::new();
    for i in 0..40 {
        if i == 19 {
            content.push_str("abcdefghi\n");
        } else {
            content.push_str(&format!("{i}\n"));
        }
    }
    type_text(&mut ed, &content);
    let bid = ed.focused_buffer_id();
    seek_to_line(&mut ed, 20);
    let line19_start = hume_rope::lines::line_start_char(
        ed.doc().text().rope(),
        hume_rope::line::RopeyLine::new(19),
    );
    (ed, bid, line19_start)
}

/// An inlay hint appearing between two frames, with no input in between,
/// must still replace the caret: a hint that wraps an earlier line (between
/// the settled top and the cursor) shifts the cursor's own row down by one —
/// `EditorState::layout_key`'s `buffer_tag` carries `decorations.generation(bid)`,
/// the same field virtual-line blocks change, so `frame.rs`'s scroll step
/// re-scrolls to follow rather than trusting `content_pos`'s plain
/// re-lookup against the unchanged top.
#[test]
fn an_inlay_hint_that_wraps_an_earlier_line_replaces_the_caret_even_when_the_cursor_has_not_moved()
{
    let (mut ed, bid, line19_start) = wrap_earlier_line_fixture();

    assert!(
        frame(&mut ed, 40, 10).cursor_content_pos.is_some(),
        "sanity: the cursor is visible before the hint exists"
    );

    ed.state.config.decorations.set_inlay_hints(
        "test".to_string(),
        bid,
        vec![hume_decorations::InlayHintEntry {
            pos: line19_start,
            text: "XY".to_string(),
            before: true,
        }],
    );

    assert!(
        frame(&mut ed, 40, 10).cursor_content_pos.is_some(),
        "the caret must still be placed once an inlay hint wraps an earlier \
         line, even though the cursor's own CharOffset never moved"
    );
}

/// EOL text appearing between two frames, with no input in between, must
/// still replace the caret: text appended past an earlier line's own content
/// (between the settled top and the cursor) can wrap it the same way an
/// inlay hint can — the same `decorations.generation(bid)` field of
/// `EditorState::layout_key` inlay hints and virtual-line blocks change.
#[test]
fn eol_text_that_wraps_an_earlier_line_replaces_the_caret_even_when_the_cursor_has_not_moved() {
    let (mut ed, bid, line19_start) = wrap_earlier_line_fixture();

    assert!(
        frame(&mut ed, 40, 10).cursor_content_pos.is_some(),
        "sanity: the cursor is visible before the EOL text exists"
    );

    let scope = ed.view.registry.intern("diagnostic.warning");
    ed.state.config.decorations.set_eol_text(
        "test".to_string(),
        bid,
        vec![hume_decorations::EolTextEntry {
            pos: line19_start,
            text: "XY".to_string(),
            scope,
        }],
    );

    assert!(
        frame(&mut ed, 40, 10).cursor_content_pos.is_some(),
        "the caret must still be placed once EOL text wraps an earlier \
         line, even though the cursor's own CharOffset never moved"
    );
}

/// A `:set buffer tab-width=` change with the cursor unmoved must still
/// replace the caret: a tab-indented line between the settled top and the
/// cursor that fits one display row at the default width can exceed a fixed
/// wrap boundary once tabs render wider, pushing the cursor's own row down —
/// the same shape as the inlay-hint/EOL-text cases above, but for a setting
/// that has no raise site of its own: `tab_width` is part of
/// `EditorState::layout_key`, so the derived comparison covers it without one.
#[test]
fn a_tab_width_change_replaces_the_caret_even_when_the_cursor_has_not_moved() {
    let mut content = String::new();
    for i in 0..40 {
        if i == 19 {
            // Tab + 5 chars: 9 display columns at the default tab-width 4
            // (tab 0->4, then 5 more) — one column of headroom left on the
            // width-10 row for the line's own EOL sentinel cell, so it fits
            // one display row, same margin `wrap_earlier_line_fixture`
            // leaves with its plain 9-column line. 13 columns at tab-width
            // 8 (tab 0->8), overflowing the row on content alone.
            content.push_str("\tabcde\n");
        } else {
            content.push_str(&format!("{i}\n"));
        }
    }
    let text = BufferText::from(content.as_str());
    let sels = SelectionSet::single(hume_editing::selection::Selection::collapsed(co(0)));
    let mut ed = Editor::for_testing(Buffer::new(text, sels));
    ed.state.settings.scrolloff = 0;
    ed.view.panes[ed.state.focus.id()].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Soft { width: 10 }),
        saved: None,
    });
    seek_to_line(&mut ed, 20);

    assert!(
        frame(&mut ed, 40, 10).cursor_content_pos.is_some(),
        "sanity: the cursor is visible before the tab-width change"
    );

    run_set(&mut ed, "buffer tab-width=8").expect(":set buffer tab-width=8 failed");

    assert!(
        frame(&mut ed, 40, 10).cursor_content_pos.is_some(),
        "the caret must still be placed after a tab-width change, even \
         though the cursor's own CharOffset never moved"
    );
}

/// A gutter that widens for a 3-digit line count narrows every pane's
/// `content_width` (gutter subtracted from viewport width) without touching
/// any per-pane state directly — a gap none of the six original raise sites
/// covered, closed for free by `content_width` being part of
/// `EditorState::layout_key`. The growth is driven through a *second* pane
/// on the same buffer so pane A's own cursor and selections are never
/// touched, isolating the layout-derived comparison from the selection
/// funnel: typing in the focused pane would move its own head and the
/// funnel would explain the caret surviving on its own, proving nothing
/// about the gutter.
#[test]
fn a_gutter_growth_replaces_the_caret_even_when_the_cursor_has_not_moved() {
    // `Editor::open` (not `for_testing`, which builds a bare `Pane::new`
    // with no gutter columns at all) — this test is specifically about the
    // line-number gutter, so it needs the real pane-construction path that
    // registers one (`pane_state.rs`'s `new_pane`).
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.state.settings.scrolloff = 0;
    let pid_a = ed.state.focus.id();
    ed.view.panes[pid_a].set_wrap(hume_engine::pane::WrapOverride {
        mode: Some(hume_engine::pane::WrapMode::Soft { width: 0 }),
        saved: None,
    });
    let mut content = String::new();
    for i in 0..97 {
        if i == 19 {
            // 24 columns — one short of `content_width` at the initial
            // gutter (30-column pane; a 2-digit line-number column plus the
            // default 2-column sign column `hume_decorations::build_providers`
            // also registers, together 5, leaving content_width 25). The
            // spare column is for the line's own EOL sentinel cell — the
            // same margin `wrap_earlier_line_fixture` leaves with its plain
            // 9-column line against a width-10 row; an exact fit still
            // needs a column for the sentinel, so it wraps on its own. Once
            // the gutter grows by one digit, content_width drops to 24 — an
            // exact fit with no spare column, wrapping to two display rows.
            content.push_str(&"x".repeat(24));
            content.push('\n');
        } else {
            content.push_str(&format!("{i}\n"));
        }
    }
    type_text(&mut ed, &content);
    let bid = ed.focused_buffer_id();
    seek_to_line(&mut ed, 20);

    // 97 typed lines land the buffer at 98 ropey lines (97 content lines
    // plus the trailing empty line the last `Enter` leaves) — `last_ropey_line`
    // (the phantom trailing line) is index 98, so the line-number gutter
    // sizes for `digit_count(99)` = 2 digits, width 3; content_width = 30 -
    // 3 (line numbers) - 2 (sign column) = 25.
    assert!(
        frame(&mut ed, 30, 10).cursor_content_pos.is_some(),
        "sanity: the cursor is visible before the gutter grows"
    );

    let pid_b = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        pid_a,
        bid,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();
    ed.switch_focused_pane(pid_b);
    seek_to_line(&mut ed, 96);
    ed.feed_key(key('o'));
    ed.feed_key(key('9'));
    ed.feed_key(key_esc());
    ed.execute_typed("quit", None).unwrap();
    assert_eq!(
        ed.state.focus.id(),
        pid_a,
        "sanity: focus returns to pane A"
    );

    // 99 ropey lines now: `last_ropey_line` is index 99, so the gutter
    // sizes for `digit_count(100)` = 3 digits, width 4; content_width = 30
    // - 4 - 2 = 24 — an exact fit with no spare column for line 19's own
    // EOL sentinel, wrapping it to two display rows and pushing the
    // cursor's own row past the 10-row viewport.
    assert!(
        frame(&mut ed, 30, 10).cursor_content_pos.is_some(),
        "the caret must still be placed once the gutter widens for a \
         3-digit line count, even though the cursor's own CharOffset never \
         moved"
    );
}

/// A `:set global scrolloff=` change with the cursor unmoved must still
/// re-settle the viewport: `cursor_content_pos.is_some()` can't distinguish
/// this case (the false branch's own cap is the raw viewport height, not the
/// scrolloff band — see `frame.rs`'s `scroll_into_view`), so this asserts on
/// `top` moving directly.
#[test]
fn a_scrolloff_change_replaces_the_caret_even_when_the_cursor_has_not_moved() {
    let content: String = numbered_lines(60);
    let mut ed = unwrapped_editor(&content, 0);
    seek_to_line(&mut ed, 40);
    let pid = ed.state.focus.id();

    frame(&mut ed, 80, 20);
    let top_before = ed.view.panes[pid].viewport.top();

    run_set(&mut ed, "global scrolloff=6").expect(":set global scrolloff=6 failed");

    frame(&mut ed, 80, 20);
    let top_after = ed.view.panes[pid].viewport.top();

    assert_ne!(
        top_after, top_before,
        "the viewport must re-settle against the new scrolloff band, even \
         though the cursor's own CharOffset never moved"
    );
}

/// A revisit to a previously-viewed buffer with nothing else changed must
/// leave a genuinely parked view exactly where it was, rather than being
/// snapped back onto the cursor the way `switch_pane_to_buffer`'s old
/// unconditional raise did on every switch. `Viewport::seed_top_for_test`
/// builds the park directly (a test-only escape hatch never reached in
/// production — see its own doc) instead of via a scroll command: every
/// scroll command carries the cursor along with it (`carry`), so none of
/// them produce this state on their own outside a genuine EOF/BOF stall.
#[test]
fn a_revisit_with_nothing_changed_leaves_a_parked_view_parked() {
    let content: String = numbered_lines(60);
    let mut ed = unwrapped_editor(&content, 0);
    seek_to_line(&mut ed, 40);
    let pid = ed.state.focus.id();
    let bid = ed.focused_buffer_id();

    frame(&mut ed, 80, 20);
    ed.view.panes[pid]
        .viewport
        .seed_top_for_test(hume_engine::display_lines::DisplayLinePos::new(
            hume_rope::line::ContentLine::new(0),
            0,
        ));

    assert!(
        frame(&mut ed, 80, 20).cursor_content_pos.is_none(),
        "sanity: the seeded top leaves the cursor genuinely off-screen"
    );
    let top_before = ed.view.panes[pid].viewport.top();

    let second_bid = ed.open_buffer(Buffer::new(
        BufferText::from("x\n"),
        SelectionSet::single(hume_editing::selection::Selection::collapsed(co(0))),
    ));
    ed.switch_to_buffer_without_jump(second_bid);
    frame(&mut ed, 80, 20);
    ed.switch_to_buffer_without_jump(bid);

    assert_eq!(
        frame(&mut ed, 80, 20).cursor_content_pos,
        None,
        "a revisit with nothing changed must leave the parked view exactly \
         where it was, not snap back onto the cursor"
    );
    assert_eq!(
        ed.view.panes[pid].viewport.top(),
        top_before,
        "the viewport itself must not move either"
    );
}
