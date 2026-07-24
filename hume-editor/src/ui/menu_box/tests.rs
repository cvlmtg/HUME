use super::*;

fn rows(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("item{i}")).collect()
}

fn style() -> Style {
    Style::default()
}

/// Rows of `area` as plain symbols, trailing spaces trimmed per row.
fn symbols_in(buf: &ScreenBuf, area: Rect) -> String {
    (area.y..area.y + area.height)
        .map(|y| {
            let row: String = (area.x..area.x + area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect();
            row.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn draw_menu_box_border_frame_snapshot() {
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    let outer = Rect::new(2, 3, 8, 4);
    draw_menu_box(
        &mut buf,
        outer,
        &rows(2),
        Some(0),
        0,
        true,
        style(),
        style(),
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
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    let outer = Rect::new(2, 3, 8, 4);
    draw_menu_box(
        &mut buf,
        outer,
        &rows(2),
        Some(0),
        0,
        false,
        style(),
        style(),
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
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    // Inner height 3 (outer height 5), 10 rows total, selected near the end.
    let outer = Rect::new(0, 0, 10, 5);
    let data = rows(10);
    draw_menu_box(
        &mut buf,
        outer,
        &data,
        Some(9),
        0,
        true,
        style(),
        style(),
        None,
    );

    // Window of size 3 anchored so index 9 is visible: start = 9 - 1 = 8,
    // clamped to total-max = 7 → window [7, 10) = item7,item8,item9.
    let row0: String = (1..=5).map(|x| buf[(x, 1)].symbol().to_string()).collect();
    assert_eq!(row0, "item7");
    let row2: String = (1..=5).map(|x| buf[(x, 3)].symbol().to_string()).collect();
    assert_eq!(row2, "item9");
}

/// Unlike a menu (which windows around `selected`), a plain popup
/// (`selected: None`) windows from `scroll` directly — this is what makes
/// `Ctrl+u`/`Ctrl+d` page a scrollable hover popup instead of the window
/// always anchoring back to row 0.
#[test]
fn draw_menu_box_scroll_windows_from_offset_when_no_selection() {
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    // Inner height 3 (outer height 5), 10 rows total, scrolled to row 4.
    let outer = Rect::new(0, 0, 10, 5);
    let data = rows(10);
    draw_menu_box(
        &mut buf,
        outer,
        &data,
        None,
        4,
        true,
        style(),
        style(),
        None,
    );

    let row0: String = (1..=5).map(|x| buf[(x, 1)].symbol().to_string()).collect();
    assert_eq!(row0, "item4");
    let row2: String = (1..=5).map(|x| buf[(x, 3)].symbol().to_string()).collect();
    assert_eq!(row2, "item6");
}

#[test]
fn draw_menu_box_shows_down_arrow_when_scrolled_to_top() {
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    // Inner height 3, 10 rows total, scroll = 0: more below, nothing above.
    let outer = Rect::new(0, 0, 10, 5);
    draw_menu_box(
        &mut buf,
        outer,
        &rows(10),
        None,
        0,
        true,
        style(),
        style(),
        None,
    );

    insta::assert_snapshot!(symbols_in(&buf, outer), @"
    ┌────────┐
    │item0   │
    │item1   │
    │item2   ▼
    └────────┘
    ");
}

#[test]
fn draw_menu_box_shows_both_arrows_when_scrolled_to_middle() {
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    // Inner height 3, 10 rows total, scroll = 4: more above and below.
    let outer = Rect::new(0, 0, 10, 5);
    draw_menu_box(
        &mut buf,
        outer,
        &rows(10),
        None,
        4,
        true,
        style(),
        style(),
        None,
    );

    insta::assert_snapshot!(symbols_in(&buf, outer), @"
    ┌────────┐
    │item4   ▲
    │item5   │
    │item6   ▼
    └────────┘
    ");
}

#[test]
fn draw_menu_box_shows_up_arrow_when_scrolled_to_bottom() {
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    // Inner height 3, 10 rows total, scroll = 7 (max_scroll): more above only.
    let outer = Rect::new(0, 0, 10, 5);
    draw_menu_box(
        &mut buf,
        outer,
        &rows(10),
        None,
        7,
        true,
        style(),
        style(),
        None,
    );

    insta::assert_snapshot!(symbols_in(&buf, outer), @"
    ┌────────┐
    │item7   ▲
    │item8   │
    │item9   │
    └────────┘
    ");
}

/// `inner_h == 1` (`outer.height == 3`): both arrow targets are the same
/// cell. The down arrow is drawn after the up arrow, so it wins the
/// collision — pins the ordering documented at the `more_below` paint site.
#[test]
fn draw_menu_box_single_row_window_collision_down_arrow_wins() {
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    // Inner height 1, 10 rows total, scroll = 4: more above and below both true.
    let outer = Rect::new(0, 0, 10, 3);
    draw_menu_box(
        &mut buf,
        outer,
        &rows(10),
        None,
        4,
        true,
        style(),
        style(),
        None,
    );

    insta::assert_snapshot!(symbols_in(&buf, outer), @"
    ┌────────┐
    │item4   ▼
    └────────┘
    ");
}

#[test]
fn draw_menu_box_no_overflow_shows_no_arrows() {
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    // 2 rows fit entirely inside inner height 3 — nothing to scroll.
    let outer = Rect::new(0, 0, 10, 5);
    draw_menu_box(
        &mut buf,
        outer,
        &rows(2),
        None,
        0,
        true,
        style(),
        style(),
        None,
    );

    insta::assert_snapshot!(symbols_in(&buf, outer), @"
    ┌────────┐
    │item0   │
    │item1   │
    │        │
    └────────┘
    ");
}

#[test]
fn draw_menu_box_selected_menu_never_shows_arrows() {
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    // Same overflowing case as the middle-scroll test above, but with a
    // selection (a menu) — arrows must stay suppressed regardless of scroll.
    let outer = Rect::new(0, 0, 10, 5);
    draw_menu_box(
        &mut buf,
        outer,
        &rows(10),
        Some(5),
        0,
        true,
        style(),
        style(),
        None,
    );

    let symbols = symbols_in(&buf, outer);
    assert!(!symbols.contains('▲'));
    assert!(!symbols.contains('▼'));
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
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    let before = buf.clone();
    draw_menu_box(
        &mut buf,
        Rect::new(0, 0, 2, 2),
        &rows(1),
        Some(0),
        0,
        true,
        style(),
        style(),
        None,
    );
    assert_eq!(buf, before, "sub-3x3 outer must not panic or paint");
}
