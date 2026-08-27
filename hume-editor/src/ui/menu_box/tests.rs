use super::super::symbols_in;
use super::*;
use hume_engine::theme::Theme;
use hume_engine::types::ResolvedStyle;
use hume_grid::{Grid, Rect};

fn rows(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("item{i}")).collect()
}

fn style() -> ResolvedStyle {
    ResolvedStyle::default()
}

fn styles() -> MenuBoxStyles {
    MenuBoxStyles {
        base: style(),
        selected: style(),
        scroll: style(),
    }
}

#[test]
fn styled_runs_stay_adjacent_when_a_run_holds_an_undrawable_grapheme() {
    // A zero-width space draws as nothing, so writing it into the cell
    // reserved for it would leave the terminal's cursor where it was and
    // slide the rest of the row one column left. It renders as its codepoint
    // instead — the same substitution buffer text gets — and the next run
    // begins after the whole placeholder.
    let mut buf = Grid::new(20, 3);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    let runs: StyledRow = vec![
        ("a\u{200B}b".to_string(), style()),
        ("cd".to_string(), style()),
    ];
    paint_styled_row(&mut canvas, 0, 1, &runs, 20);

    assert_eq!(
        symbols_in(&buf, Rect::new(0, 1, 12, 1)),
        "a<200b>bcd",
        "the placeholder spans the columns reserved for it, and the second \
         run starts in the cell right after it"
    );
}

#[test]
fn styled_runs_stop_at_the_right_edge() {
    // Runs are bounded by the caller's edge, not by the terminal buffer's: a
    // row wider than its box must be clipped at the border rather than
    // written over it. A cluster that would straddle the edge is dropped
    // whole, never half-drawn.
    let mut buf = Grid::new(20, 3);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    let runs: StyledRow = vec![
        ("abc".to_string(), style()),
        ("\u{6F22}z".to_string(), style()),
    ];
    // Edge at 4: "abc" fills 0..3, and 漢 would need cells 3 and 4, so it is
    // dropped — leaving 'z' nowhere to start from either.
    paint_styled_row(&mut canvas, 0, 1, &runs, 4);

    assert_eq!(symbols_in(&buf, Rect::new(0, 1, 8, 1)), "abc");
}

#[test]
fn a_row_wider_than_the_box_is_clipped_at_the_border() {
    // Rows reach `draw_menu_box` untruncated — the box was sized to the
    // widest of them, then clamped to the pane it has to fit inside (see
    // `completion_overlay`), so a long LSP label on a narrow terminal is
    // wider than the box it lands in. It must stop at the inner edge: the
    // right border has to survive, and nothing may be written past it.
    let mut buf = Grid::new(20, 5);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    let outer = Rect::new(2, 0, 8, 3); // inner text spans x 3..9
    let long = vec!["abcdefghij".to_string()];
    draw_menu_box(&mut canvas, outer, &long, Some(0), 0, true, styles(), None);

    assert_eq!(
        symbols_in(&buf, Rect::new(0, 1, 20, 1)),
        "  │abcdef│",
        "the row fills the inner width and stops; the border stands and the \
         cells beyond it are untouched"
    );
}

#[test]
fn draw_menu_box_border_frame_snapshot() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    let outer = Rect::new(2, 3, 8, 4);
    draw_menu_box(
        &mut canvas,
        outer,
        &rows(2),
        Some(0),
        0,
        true,
        styles(),
        None,
    );

    insta::assert_snapshot!(symbols_in(&buf, outer), @"
    ┌──────┐
    │item0 │
    │item1 │
    └──────┘
    ");
}

#[test]
fn draw_menu_box_no_border_leaves_plain_margin() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    let outer = Rect::new(2, 3, 8, 4);
    draw_menu_box(
        &mut canvas,
        outer,
        &rows(2),
        Some(0),
        0,
        false,
        styles(),
        None,
    );

    // Corners stay background-filled space, never a box-drawing glyph.
    insta::assert_snapshot!(symbols_in(&buf, outer), @"

    item0
    item1
    ");
}

#[test]
fn draw_menu_box_scrolls_to_keep_selected_visible() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    // Inner height 3 (outer height 5), 10 rows total, selected near the end.
    let outer = Rect::new(0, 0, 10, 5);
    let data = rows(10);
    draw_menu_box(&mut canvas, outer, &data, Some(9), 0, true, styles(), None);

    // Window of size 3 anchored so index 9 is visible: start = 9 - 1 = 8,
    // clamped to total-max = 7 → window [7, 10) = item7,item8,item9.
    let row0: String = (1..=5).map(|x| buf[(x, 1)].text().to_string()).collect();
    assert_eq!(row0, "item7");
    let row2: String = (1..=5).map(|x| buf[(x, 3)].text().to_string()).collect();
    assert_eq!(row2, "item9");
}

/// Unlike a menu (which windows around `selected`), a plain popup
/// (`selected: None`) windows from `scroll` directly — this is what makes
/// `Ctrl+u`/`Ctrl+d` page a scrollable hover popup instead of the window
/// always anchoring back to row 0.
#[test]
fn draw_menu_box_scroll_windows_from_offset_when_no_selection() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    // Inner height 3 (outer height 5), 10 rows total, scrolled to row 4.
    let outer = Rect::new(0, 0, 10, 5);
    let data = rows(10);
    draw_menu_box(&mut canvas, outer, &data, None, 4, true, styles(), None);

    let row0: String = (1..=5).map(|x| buf[(x, 1)].text().to_string()).collect();
    assert_eq!(row0, "item4");
    let row2: String = (1..=5).map(|x| buf[(x, 3)].text().to_string()).collect();
    assert_eq!(row2, "item6");
}

#[test]
fn draw_menu_box_shows_scrollbar_thumb_at_top_when_scrolled_to_top() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    // Inner height 3, 10 rows total, scroll = 0: thumb flush at the top.
    let outer = Rect::new(0, 0, 10, 5);
    draw_menu_box(&mut canvas, outer, &rows(10), None, 0, true, styles(), None);

    insta::assert_snapshot!(symbols_in(&buf, outer), @"
    ┌────────┐
    │item0   ┃
    │item1   │
    │item2   │
    └────────┘
    ");
}

#[test]
fn draw_menu_box_shows_scrollbar_thumb_in_the_middle_when_scrolled_to_the_middle() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    // Inner height 3, 10 rows total, scroll = 4: thumb centered.
    let outer = Rect::new(0, 0, 10, 5);
    draw_menu_box(&mut canvas, outer, &rows(10), None, 4, true, styles(), None);

    insta::assert_snapshot!(symbols_in(&buf, outer), @"
    ┌────────┐
    │item4   │
    │item5   ┃
    │item6   │
    └────────┘
    ");
}

#[test]
fn draw_menu_box_shows_scrollbar_thumb_at_bottom_when_scrolled_to_bottom() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    // Inner height 3, 10 rows total, scroll = 7 (max_scroll): thumb flush at the bottom.
    let outer = Rect::new(0, 0, 10, 5);
    draw_menu_box(&mut canvas, outer, &rows(10), None, 7, true, styles(), None);

    insta::assert_snapshot!(symbols_in(&buf, outer), @"
    ┌────────┐
    │item7   │
    │item8   │
    │item9   ┃
    └────────┘
    ");
}

/// `inner_h == 1` (`outer.height == 3`): the thumb has nowhere to move —
/// `scrollbar_thumb` degenerates to a single full-height thumb regardless of
/// `scroll`, still distinguishing "more to scroll" from "nothing to scroll".
#[test]
fn draw_menu_box_single_row_window_shows_a_solid_thumb() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    // Inner height 1, 10 rows total, scroll = 4 (mid-range).
    let outer = Rect::new(0, 0, 10, 3);
    draw_menu_box(&mut canvas, outer, &rows(10), None, 4, true, styles(), None);

    insta::assert_snapshot!(symbols_in(&buf, outer), @"
    ┌────────┐
    │item4   ┃
    └────────┘
    ");
}

#[test]
fn draw_menu_box_no_overflow_shows_no_scrollbar() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    // 2 rows fit entirely inside inner height 3 — nothing to scroll.
    let outer = Rect::new(0, 0, 10, 5);
    draw_menu_box(&mut canvas, outer, &rows(2), None, 0, true, styles(), None);

    insta::assert_snapshot!(symbols_in(&buf, outer), @"
    ┌────────┐
    │item0   │
    │item1   │
    │        │
    └────────┘
    ");
}

#[test]
fn draw_menu_box_scrolled_menu_shows_scrollbar_thumb() {
    let mut buf = Grid::new(20, 20);
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    // Same overflowing case as the middle-scroll test above, but with a
    // selection (a menu) — the highlight signals *which* row, the thumb
    // signals how much more there is to scroll past; both show together.
    let outer = Rect::new(0, 0, 10, 5);
    draw_menu_box(
        &mut canvas,
        outer,
        &rows(10),
        Some(5),
        0,
        true,
        styles(),
        None,
    );

    assert!(symbols_in(&buf, outer).contains('┃'));
}

#[test]
fn menu_inner_width_is_widest_row() {
    assert_eq!(
        menu_inner_width(&["a".into(), "abc".into(), "ab".into()]),
        3
    );
}

#[test]
fn draw_menu_box_too_small_outer_does_nothing() {
    let mut buf = Grid::new(20, 20);
    let before = buf.clone();
    let theme = Theme::default();
    let mut canvas = Canvas::new(&mut buf, &theme, None);
    draw_menu_box(
        &mut canvas,
        Rect::new(0, 0, 2, 2),
        &rows(1),
        Some(0),
        0,
        true,
        styles(),
        None,
    );
    assert_eq!(buf, before, "sub-3x3 outer must not panic or paint");
}

// ---------------------------------------------------------------------------
// scrollbar_thumb — pure geometry, independent of the painter
// ---------------------------------------------------------------------------

#[test]
fn scrollbar_thumb_none_when_content_fits() {
    assert_eq!(scrollbar_thumb(3, 3, 0), None);
    assert_eq!(scrollbar_thumb(5, 3, 0), None);
}

#[test]
fn scrollbar_thumb_some_the_moment_content_overflows() {
    assert!(scrollbar_thumb(3, 4, 0).is_some());
}

#[test]
fn scrollbar_thumb_flush_top_at_scroll_zero() {
    let (start, _) = scrollbar_thumb(3, 10, 0).expect("overflowing");
    assert_eq!(start, 0);
}

#[test]
fn scrollbar_thumb_flush_bottom_at_max_scroll() {
    let (start, len) = scrollbar_thumb(3, 10, 7).expect("overflowing");
    assert_eq!(
        start + len,
        3,
        "thumb's bottom edge must touch the track's bottom edge"
    );
}

/// Without the `scroll > 0` nudge, floor division alone would leave the
/// thumb pinned at the same start as `scroll == 0` here — indistinguishable
/// from "haven't scrolled at all". This is the test that would catch
/// reverting to plain `scroll * slack / max_scroll`.
#[test]
fn scrollbar_thumb_moves_off_the_top_after_one_line_of_scroll() {
    let (start_at_zero, _) = scrollbar_thumb(3, 100, 0).expect("overflowing");
    let (start_at_one, _) = scrollbar_thumb(3, 100, 1).expect("overflowing");
    assert_eq!(start_at_zero, 0);
    assert_ne!(start_at_one, start_at_zero);
}

#[test]
fn scrollbar_thumb_length_is_proportional_and_leaves_track_visible() {
    let (_, len) = scrollbar_thumb(3, 4, 0).expect("overflowing");
    assert_eq!(
        len, 2,
        "3 of 4 rows visible must not fill the whole 3-cell track"
    );
}

/// The `(view*view).div_ceil(total)` proportional term, not just the
/// `(view-1).max(1)` clamp, must actually drive the thumb length. The test
/// above (`view=3,total=4`) only exercises the clamp — the ratio term there
/// (`ceil(9/4)=3`) gets clamped down to 2 regardless, so deleting that term
/// entirely and hardcoding `view-1` would still pass it. Hand-computed
/// (independent-oracle) expected lengths here land below the clamp ceiling,
/// so only the real ratio term can produce them.
#[test]
fn scrollbar_thumb_length_scales_with_visible_fraction_not_just_the_clamp() {
    let (_, len_10_of_100) = scrollbar_thumb(10, 100, 0).expect("overflowing");
    assert_eq!(
        len_10_of_100, 1,
        "10 of 100 rows visible: ceil(10*10/100)=1, near-minimal"
    );

    let (_, len_10_of_20) = scrollbar_thumb(10, 20, 0).expect("overflowing");
    assert_eq!(
        len_10_of_20, 5,
        "10 of 20 rows visible (half): ceil(10*10/20)=5, about half the track"
    );

    assert!(
        len_10_of_20 > len_10_of_100,
        "a larger visible fraction (10/20) must give a longer thumb than a \
         smaller one (10/100) — proves length scales with the fraction \
         rather than being pinned to the view-1 clamp ceiling (which would \
         give 9 for both)"
    );
}

#[test]
fn scrollbar_thumb_single_row_window_is_a_solid_cell() {
    assert_eq!(scrollbar_thumb(1, 10, 0), Some((0, 1)));
    assert_eq!(scrollbar_thumb(1, 10, 9), Some((0, 1)));
}
