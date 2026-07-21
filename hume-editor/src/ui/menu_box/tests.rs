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
    draw_menu_box(&mut buf, outer, &rows(2), Some(0), true, style(), style());

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
    draw_menu_box(&mut buf, outer, &rows(2), Some(0), false, style(), style());

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
    draw_menu_box(&mut buf, outer, &data, Some(9), true, style(), style());

    // Window of size 3 anchored so index 9 is visible: start = 9 - 1 = 8,
    // clamped to total-max = 7 → window [7, 10) = item7,item8,item9.
    let row0: String = (1..=5).map(|x| buf[(x, 1)].symbol().to_string()).collect();
    assert_eq!(row0, "item7");
    let row2: String = (1..=5).map(|x| buf[(x, 3)].symbol().to_string()).collect();
    assert_eq!(row2, "item9");
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
        true,
        style(),
        style(),
    );
    assert_eq!(buf, before, "sub-3x3 outer must not panic or paint");
}
