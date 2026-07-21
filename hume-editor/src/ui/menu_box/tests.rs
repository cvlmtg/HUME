use super::*;

fn rows(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("item{i}")).collect()
}

fn style() -> Style {
    Style::default()
}

#[test]
fn draw_menu_box_border_draws_corner_glyphs() {
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    let outer = Rect::new(2, 3, 8, 4);
    draw_menu_box(&mut buf, outer, &rows(2), Some(0), true, style(), style());

    assert_eq!(buf[(2, 3)].symbol(), "┌");
    assert_eq!(buf[(9, 3)].symbol(), "┐");
    assert_eq!(buf[(2, 6)].symbol(), "└");
    assert_eq!(buf[(9, 6)].symbol(), "┘");
    assert_eq!(buf[(5, 3)].symbol(), "─");
    assert_eq!(buf[(2, 4)].symbol(), "│");
}

#[test]
fn draw_menu_box_no_border_leaves_plain_margin() {
    let mut buf = ScreenBuf::empty(Rect::new(0, 0, 20, 20));
    let outer = Rect::new(2, 3, 8, 4);
    draw_menu_box(&mut buf, outer, &rows(2), Some(0), false, style(), style());

    // Corners stay background-filled space, never a box-drawing glyph.
    assert_eq!(buf[(2, 3)].symbol(), " ");
    assert_eq!(buf[(9, 3)].symbol(), " ");
    assert_eq!(buf[(2, 6)].symbol(), " ");
    assert_eq!(buf[(9, 6)].symbol(), " ");
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
