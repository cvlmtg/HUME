//! Shared box-drawing + row rendering for the two popup overlays
//! (`CompletionOverlay`, `PopupOverlay`). Each overlay owns its own
//! placement (bottom-anchored vs. cursor-anchored floating); once a
//! position and outer size are resolved, painting the frame, scroll
//! window, and rows is identical — that shared part lives here.

use ratatui::buffer::Buffer as ScreenBuf;
use ratatui::layout::Rect;
use ratatui::style::Style;

use hume_engine::render::fill_rect_bg;

/// Maximum number of visible rows inside a menu/popup box (excluding the
/// 1-cell frame). Both overlays scroll past this using [`visible_window`].
pub(crate) const MAX_MENU_ROWS: u16 = 10;

/// Widest row's display width — stable across scrolling, so the box doesn't
/// resize as the visible window changes.
pub(crate) fn menu_inner_width(rows: &[String]) -> u16 {
    rows.iter()
        .map(|r| unicode_width::UnicodeWidthStr::width(r.as_str()))
        .max()
        .unwrap_or(0) as u16
}

/// Outer footprint (including the 1-cell frame) for a box showing `rows`,
/// windowed to at most `row_cap` visible rows. Shared by every write side
/// that positions a menu/popup box via `resolve_popup_geometry` and by
/// `CompletionOverlay`, which computes it inline against its own bottom-
/// anchored placement.
pub(crate) fn outer_dims(rows: &[String], row_cap: u16) -> (u16, u16) {
    let outer_w = menu_inner_width(rows) + 2;
    let outer_h = (rows.len() as u16).min(row_cap) + 2;
    (outer_w, outer_h)
}

/// Return `(scroll_offset, visible_slice)` such that `selected` is inside
/// the visible window of `max_height` entries.
fn visible_window(rows: &[String], selected: usize, max_height: usize) -> (usize, &[String]) {
    let total = rows.len();
    if total <= max_height {
        return (0, rows);
    }
    // Keep `selected` visible by anchoring the window.
    let start = selected
        .saturating_sub(max_height / 2)
        .min(total - max_height);
    (start, &rows[start..start + max_height])
}

/// Paint a menu/popup box into `outer` (the full footprint, including the
/// 1-cell frame). Windows `rows` to fit `outer`'s inner height, keeping
/// `selected` (an absolute index into `rows`) visible.
///
/// `border`: when `true`, overdraws the 1-cell frame with box-drawing
/// glyphs; when `false`, the frame stays a plain background-filled margin
/// (still 1 cell wide — only the glyphs are suppressed).
pub(crate) fn draw_menu_box(
    buf: &mut ScreenBuf,
    outer: Rect,
    rows: &[String],
    selected: Option<usize>,
    border: bool,
    menu_style: Style,
    selected_style: Style,
) {
    if rows.is_empty() || outer.height < 3 || outer.width < 3 {
        return;
    }

    let inner_h = (outer.height - 2) as usize;
    let (scroll_offset, visible_rows) = visible_window(rows, selected.unwrap_or(0), inner_h);

    // 1. Fill the entire outer rectangle with the popup background. This
    //    gives a solid, opaque backdrop — no buffer content bleeds through.
    //    For border=false it also acts as the visible 1-cell margin.
    fill_rect_bg(buf, outer, menu_style);

    // 2. Optionally overdraw the 1-cell frame with box-drawing characters.
    if border {
        let right = outer.x + outer.width - 1;
        let bottom = outer.y + outer.height - 1;
        let fill_w = (outer.width - 2) as usize;
        let horiz: String = "─".repeat(fill_w);

        buf.set_string(outer.x, outer.y, "┌", menu_style);
        buf.set_string(outer.x + 1, outer.y, &horiz, menu_style);
        buf.set_string(right, outer.y, "┐", menu_style);
        buf.set_string(outer.x, bottom, "└", menu_style);
        buf.set_string(outer.x + 1, bottom, &horiz, menu_style);
        buf.set_string(right, bottom, "┘", menu_style);

        for row in 1..outer.height - 1 {
            buf.set_string(outer.x, outer.y + row, "│", menu_style);
            buf.set_string(right, outer.y + row, "│", menu_style);
        }
    }

    // 3. Draw content rows inside the frame (offset +1 for top/left border/padding).
    let text_x = outer.x + 1;
    for (i, row_text) in visible_rows.iter().enumerate() {
        let y = outer.y + 1 + i as u16;
        let row_idx = scroll_offset + i;

        if selected == Some(row_idx) {
            // Highlight the full inner width so the selection bar is uniform.
            let inner_rect = Rect::new(text_x, y, outer.width.saturating_sub(2), 1);
            fill_rect_bg(buf, inner_rect, selected_style);
            buf.set_string(text_x, y, row_text, selected_style);
        } else {
            buf.set_string(text_x, y, row_text, menu_style);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
}
