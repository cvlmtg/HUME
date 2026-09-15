//! A resize, a wrap-mode change, a virtual-line block appearing above the
//! cursor, an inlay hint or EOL text wrapping an earlier line, or a buffer
//! switch must still replace the caret even though the cursor itself is
//! unmoved between the two frames each test drives. None of these fixtures
//! move the primary head, so `PaneBufferState::reveal_pending` would stay
//! `false` if each didn't raise it explicitly — these tests pin that all six
//! do, closing exactly the gap a purely selection-driven reveal signal would
//! otherwise have.

use super::*;

/// A resize with the cursor unmoved must still replace the caret:
/// `sync_viewport_dims` raises `reveal_pending` itself when a pane's height
/// actually changes, so `frame.rs`'s scroll step runs `Viewport::reveal` and
/// scrolls the (unchanged) viewport top far enough for the shrunk height to
/// still cover the cursor, rather than trusting a stale top left over from
/// the taller frame.
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
/// `set_focused_wrap_override`/`toggle_focused_wrap` raise `reveal_pending`
/// themselves on an actual mode change, so `frame.rs`'s scroll step
/// re-places the caret against wrapping's now-different display-line layout
/// rather than trusting the unwrapped frame's screen position.
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
    // `Pane::set_wrap` poke: only `set_focused_wrap_override`/
    // `toggle_focused_wrap` ever change wrap mode in a running editor, and
    // both raise `PaneBufferState::reveal_pending` — a poke that bypasses
    // them bypasses the signal too, which is not a gap this test should
    // exercise.
    run_set(&mut ed, "pane wrap-mode=soft:0").expect(":set pane wrap-mode=soft:0 failed");

    assert!(
        frame(&mut ed, 80, 15).cursor_content_pos.is_some(),
        "the caret must still be placed after a wrap-mode change, even \
         though the cursor's DisplayLinePos is unchanged"
    );
}

/// A virtual-line block appearing above the cursor between two frames, with
/// no input in between, must still replace the caret: `update_virtual_line_providers`
/// raises `reveal_pending` itself on a decoration-generation change, so
/// `frame.rs`'s scroll step re-scrolls the viewport to follow the cursor's
/// now-lower display row rather than trusting `content_pos`'s plain
/// re-lookup against the unchanged top — which, five rows below the settled
/// scrolloff target in a 10-row viewport, would find the cursor's row past
/// the bottom edge and answer `None`.
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
/// alone would see no change. `switch_pane_to_buffer` raises `reveal_pending`
/// unconditionally on every switch (`buffer/lifecycle.rs`) instead — a
/// different buffer's cursor/viewport pairing needs re-settling regardless of
/// whether the head value happens to match: `Pane::recall_scroll` resets the
/// viewport's own top to the document's first line on a pane's first visit
/// to a buffer, so the matching head — deep in a 60-line buffer here — is
/// left far outside it unless something re-scrolls to find it.
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
/// `update_inlay_hint_providers` raises `reveal_pending` itself on a
/// decoration-generation change, the same signal virtual-line blocks use,
/// so `frame.rs`'s scroll step re-scrolls to follow rather than trusting
/// `content_pos`'s plain re-lookup against the unchanged top.
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
/// inlay hint can — `update_eol_text_providers` raises `reveal_pending`
/// itself on a decoration-generation change, the same signal inlay hints and
/// virtual-line blocks use.
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
