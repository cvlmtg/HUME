use super::*;
use crate::test_support::{cursor_mirror, mirror, painted};

#[test]
fn viewport_defaults() {
    let vp = Viewport::new(80, 24);
    assert_eq!(vp.top(), DisplayLinePos::default());
    assert_eq!(vp.horizontal_offset, DisplayLineCol::new(0));
    assert_eq!(vp.width, 80);
    assert_eq!(vp.height, 24);
}

#[test]
fn wrap_mode_wrap_width() {
    assert_eq!(WrapMode::None.wrap_width(), None);
    assert_eq!(WrapMode::Soft { width: 80 }.wrap_width(), Some(80));
    assert_eq!(WrapMode::Word { width: 40 }.wrap_width(), Some(40));
    assert_eq!(WrapMode::Indent { width: 60 }.wrap_width(), Some(60));
}

#[test]
fn wrap_mode_resolve() {
    // Sentinel → concrete.
    assert_eq!(
        WrapMode::Soft { width: 0 }.resolve(80),
        WrapMode::Soft { width: 80 }
    );
    assert_eq!(
        WrapMode::Word { width: 0 }.resolve(80),
        WrapMode::Word { width: 80 }
    );
    assert_eq!(
        WrapMode::Indent { width: 0 }.resolve(80),
        WrapMode::Indent { width: 80 }
    );
    // Concrete and None pass through unchanged.
    assert_eq!(
        WrapMode::Soft { width: 40 }.resolve(80),
        WrapMode::Soft { width: 40 }
    );
    assert_eq!(WrapMode::None.resolve(80), WrapMode::None);
}

#[test]
fn wrap_mode_is_wrapping() {
    assert!(!WrapMode::None.is_wrapping());
    assert!(WrapMode::Soft { width: 80 }.is_wrapping());
    assert!(WrapMode::Word { width: 80 }.is_wrapping());
    assert!(WrapMode::Indent { width: 80 }.is_wrapping());
    // Sentinel (width: 0 = terminal width) must still report is_wrapping()
    // = true; it must not be conflated with WrapMode::None.
    assert!(WrapMode::Indent { width: 0 }.is_wrapping());
    assert!(WrapMode::Soft { width: 0 }.is_wrapping());
}

// ── Pane::new / wrap-mode override seeding ───────────────────────────────

#[test]
fn pane_new_has_no_wrap_override_and_nothing_to_restore() {
    // A fresh pane inherits the buffer/global setting (no pane-level pin)
    // and has never been toggled off, so there is no `:wrap` restore target
    // yet; see `hume-editor`'s `pane_state::toggle_focused_wrap`.
    let pane = Pane::new(BufferId::default());
    let wrap = pane.wrap();
    assert_eq!(wrap.mode, None);
    assert_eq!(wrap.saved, None);
}

// ── remember_scroll / recall_scroll ─────────────────────────────────────

/// A real (non-null) `BufferId`. `BufferId::default()` is slotmap's null
/// key, which `SecondaryMap::insert` silently no-ops on, so `saved_scrolls`
/// (a `SecondaryMap`) needs a minted key for `remember_scroll` to actually
/// persist anything.
fn fresh_buffer_id() -> BufferId {
    let mut sm: slotmap::SlotMap<BufferId, ()> = slotmap::SlotMap::with_key();
    sm.insert(())
}

#[test]
fn recall_scroll_clamps_top_line_to_the_buffers_current_last_content_line() {
    let bid = fresh_buffer_id();
    let mut pane = Pane::new(bid);

    // Save a scroll position deep into a buffer that was, at the time, tall.
    pane.viewport
        .seed_top_for_test(DisplayLinePos::new(ContentLine::new(100), 0));
    pane.remember_scroll();

    // The pane moves elsewhere, then recalls the same buffer, which has
    // since shrunk to a last content line of 3 (e.g. edited by another pane
    // in the meantime).
    pane.viewport
        .seed_top_for_test(DisplayLinePos::new(ContentLine::new(0), 0));
    pane.recall_scroll(bid, ContentLine::new(3));

    assert_eq!(pane.viewport.top().line.index(), 3);
}

#[test]
fn recall_scroll_leaves_an_in_range_top_line_untouched() {
    let bid = fresh_buffer_id();
    let mut pane = Pane::new(bid);

    pane.viewport
        .seed_top_for_test(DisplayLinePos::new(ContentLine::new(4), 0));
    pane.remember_scroll();

    pane.viewport
        .seed_top_for_test(DisplayLinePos::new(ContentLine::new(0), 0));
    pane.recall_scroll(bid, ContentLine::new(100));

    assert_eq!(pane.viewport.top().line.index(), 4);
}

#[test]
fn whitespace_config_defaults() {
    let wc = WhitespaceConfig::default();
    assert_eq!(wc.space, WhitespaceRender::None);
    assert_eq!(wc.tab, WhitespaceRender::None);
    assert!(!wc.newline);
    assert_eq!(wc.space_char, "·");
    assert_eq!(wc.tab_char, "→");
    assert_eq!(wc.newline_char, "⏎");
    assert_eq!(wc.nbsp_char, "⍽");
}

fn make_pane_at(rope: &ropey::Rope, head_char: usize) -> Pane {
    Pane {
        selections: Some(cursor_mirror(rope, head_char)),
        ..Pane::new(crate::pipeline::BufferId::default())
    }
}

#[test]
fn primary_head_line_returns_head_line() {
    // "aaa\nbbb\nccc": line 0 is chars 0..3, line 1 is chars 4..7, line 2 is chars 8..11.
    // Char 8 (start of line 2) should resolve to line 2.
    let rope = ropey::Rope::from_str("aaa\nbbb\nccc");
    let pane = make_pane_at(&rope, 8); // first char of line 2
    let sels = pane.selections.as_ref().expect("a written mirror");
    assert_eq!(primary_head_line(sels, &rope).index(), 2);
}

#[test]
fn primary_head_line_follows_the_primary_index() {
    // "aaa\nbbb\nccc": char 0 = line 0, char 8 = line 2.
    let rope = ropey::Rope::from_str("aaa\nbbb\nccc");
    let sels = [(0, 0), (8, 8)];
    assert_eq!(
        primary_head_line(&mirror(&rope, &sels, 1), &rope).index(),
        2
    );
    assert_eq!(
        primary_head_line(&mirror(&rope, &sels, 0), &rope).index(),
        0
    );
}

#[test]
#[should_panic(expected = "a selection mirror holds at least one selection")]
fn a_selection_mirror_refuses_an_empty_list() {
    crate::types::PaintedSelections::new(Vec::new(), 0);
}

#[test]
#[should_panic(expected = "the primary index names a selection of the mirror")]
fn a_selection_mirror_refuses_a_primary_past_its_selections() {
    let rope = ropey::Rope::from_str("aaa\nbbb\n");
    crate::types::PaintedSelections::new(vec![painted(&rope, 0, 0), painted(&rope, 4, 4)], 2);
}

#[test]
#[should_panic(expected = "a selection mirror is in cursor order")]
fn a_selection_mirror_refuses_cursors_out_of_order() {
    let rope = ropey::Rope::from_str("aaa\nbbb\n");
    crate::types::PaintedSelections::new(vec![painted(&rope, 4, 4), painted(&rope, 0, 0)], 0);
}

#[test]
fn a_rewritten_selection_mirror_holds_the_new_selections_and_primary() {
    let rope = ropey::Rope::from_str("aaa\nbbb\n");
    let mut sels = cursor_mirror(&rope, 0);
    sels.rewrite([painted(&rope, 0, 2), painted(&rope, 4, 5)], 1);
    assert_eq!(sels, mirror(&rope, &[(0, 2), (4, 5)], 1));
}

#[test]
#[should_panic(expected = "a selection mirror holds at least one selection")]
fn a_selection_mirror_refuses_a_rewrite_to_an_empty_list() {
    let rope = ropey::Rope::from_str("aaa\nbbb\n");
    cursor_mirror(&rope, 0).rewrite([], 0);
}
