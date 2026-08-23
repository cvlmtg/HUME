//! Shared box-drawing + row rendering for the two popup overlays
//! (`MinibufCompletionOverlay`, `PopupOverlay`). Each overlay owns its own
//! placement (bottom-anchored vs. cursor-anchored floating); once a
//! position and outer size are resolved, painting the frame, scroll
//! window, and rows is identical — that shared part lives here.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::symbols::line;

use hume_engine::render::Canvas;

use super::popup::StyledRow;
use super::width::text_width;

/// Theme styles a bordered box paints with — grouped into one struct rather
/// than three positional `Style` arguments on `draw_menu_box`, which needs
/// `#[allow(clippy::too_many_arguments)]` regardless given its other params
/// (buffer, rect, rows, selection, scroll, border).
#[derive(Clone, Copy)]
pub(crate) struct MenuBoxStyles {
    /// Fill, border, and unstyled rows.
    pub(crate) base: Style,
    /// The highlighted row (menus only — plain popups never set `selected`).
    pub(crate) selected: Style,
    /// The scrollbar thumb.
    pub(crate) scroll: Style,
}

/// Maximum number of visible rows inside a menu/popup box (excluding the
/// 1-cell frame). Both overlays scroll past this using [`visible_window`].
pub(crate) const MAX_MENU_ROWS: u16 = 10;

/// Widest row's display width — stable across scrolling, so the box doesn't
/// resize as the visible window changes.
pub(crate) fn menu_inner_width(rows: &[String]) -> u16 {
    rows.iter().map(|r| text_width(r)).max().unwrap_or(0) as u16
}

/// Outer footprint (including the 1-cell frame) for a box showing `rows`,
/// windowed to at most `row_cap` visible rows. Shared by every write side
/// that positions a menu/popup box via `resolve_popup_geometry` and by
/// `MinibufCompletionOverlay`, which computes it inline against its own bottom-
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

/// Return `(scroll_offset, visible_slice)` for a plain popup (no selected
/// row) windowed from `scroll` — unlike [`visible_window`], the window start
/// is exactly `scroll`, clamped so it never runs past the end of `rows`.
fn scroll_window(rows: &[String], scroll: usize, max_height: usize) -> (usize, &[String]) {
    let total = rows.len();
    if total <= max_height {
        return (0, rows);
    }
    let start = scroll.min(total - max_height);
    (start, &rows[start..start + max_height])
}

/// Overdraws `outer`'s 1-cell frame with box-drawing glyphs (`┌─┐└┘│`).
/// Shared by every bordered box overlay — [`draw_menu_box`] and
/// `super::picker_panel::draw_picker_panel` — so the frame glyphs stay
/// identical without a copy per caller.
pub(crate) fn draw_box_border(canvas: &mut Canvas, outer: Rect, style: Style) {
    let right = outer.x + outer.width - 1;
    let bottom = outer.y + outer.height - 1;
    let fill_w = (outer.width - 2) as usize;
    let horiz: String = "─".repeat(fill_w);

    // Border glyphs are constants a cell wide, so they need none of
    // `write_text_run`'s substitution — but they go through it anyway rather
    // than carry an exemption from the one-writer rule for no benefit, and
    // the bound keeps a mis-sized box from drawing past its own footprint.
    let edge = outer.x + outer.width;
    canvas.write_text_run(outer.x, outer.y, "┌", style, edge);
    canvas.write_text_run(outer.x + 1, outer.y, &horiz, style, edge);
    canvas.write_text_run(right, outer.y, "┐", style, edge);
    canvas.write_text_run(outer.x, bottom, "└", style, edge);
    canvas.write_text_run(outer.x + 1, bottom, &horiz, style, edge);
    canvas.write_text_run(right, bottom, "┘", style, edge);

    for row in 1..outer.height - 1 {
        canvas.write_text_run(outer.x, outer.y + row, line::VERTICAL, style, edge);
        canvas.write_text_run(right, outer.y + row, line::VERTICAL, style, edge);
    }
}

/// Track-relative `(start, len)` of the scrollbar thumb for a `view`-row
/// window into `total` rows scrolled to `scroll`, or `None` when everything
/// fits (nothing to scroll, so no thumb to draw).
///
/// `len` is proportional to the visible fraction (`view / total`), clamped to
/// `1..=(view - 1).max(1)` so the thumb never grows to fill the whole track —
/// a full track conveys no position at all — except at `view == 1`, where
/// there's no shorter length to clamp to and the single-cell track is always
/// a full-length thumb. `start` places the thumb so it sits flush against the
/// top edge exactly when `scroll == 0` and flush against the bottom edge
/// exactly when `scroll == max_scroll`; floor division alone reaches the
/// bottom edge but not the top (a `scroll` of 1 out of a large `max_scroll`
/// floors to 0), so a scrolled-at-all window is nudged one cell off the top
/// to keep the two edges symmetric. That nudge can push `start` past `slack`
/// when `view == 1` (`slack == 0`), so the final `.min(slack)` clamps it back
/// onto the track.
///
/// A proportional thumb, not arrow glyphs: an arrow can tell you there's more
/// to scroll, not how much more, and both menus and popups need that at a
/// glance.
fn scrollbar_thumb(view: usize, total: usize, scroll: usize) -> Option<(usize, usize)> {
    if view == 0 || total <= view {
        return None;
    }
    let len = (view * view).div_ceil(total).clamp(1, (view - 1).max(1));
    let slack = view - len;
    let max_scroll = total - view;
    let start = scroll * slack / max_scroll;
    let start = if scroll > 0 { start.max(1) } else { start };
    Some((start.min(slack), len))
}

/// Paint a menu/popup box into `outer` (the full footprint, including the
/// 1-cell frame). Windows `rows` to fit `outer`'s inner height, keeping
/// `selected` (an absolute index into `rows`) visible.
///
/// `scroll`: for a plain popup (`selected` is `None`), the first visible
/// row — ignored when `selected` is `Some` (a menu windows around the
/// selected row instead).
///
/// `border`: when `true`, overdraws the 1-cell frame with box-drawing
/// glyphs; when `false`, the frame stays a plain background-filled margin
/// (still 1 cell wide — only the glyphs are suppressed).
///
/// `styled`: per-row style runs, same length as `rows` — a markdown popup
/// with a `markdown` grammar registered. `None` for every other caller
/// (plain popups, menus), which paint each row in one style. Ignored for a
/// row that has `selected == Some(row_idx)`: the highlight bar always wins.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_menu_box(
    canvas: &mut Canvas,
    outer: Rect,
    rows: &[String],
    selected: Option<usize>,
    scroll: usize,
    border: bool,
    styles: MenuBoxStyles,
    styled: Option<&[StyledRow]>,
) {
    if rows.is_empty() || outer.height < 3 || outer.width < 3 {
        return;
    }

    let inner_h = (outer.height - 2) as usize;
    let (scroll_offset, visible_rows) = match selected {
        Some(sel) => visible_window(rows, sel, inner_h),
        None => scroll_window(rows, scroll, inner_h),
    };

    // 1. Fill the entire outer rectangle with the popup background. This
    //    gives a solid, opaque backdrop — no buffer content bleeds through.
    //    For border=false it also acts as the visible 1-cell margin.
    canvas.fill_rect_bg(outer, styles.base);

    // 2. Optionally overdraw the 1-cell frame with box-drawing characters.
    if border {
        draw_box_border(canvas, outer, styles.base);
    }

    // 2b. Scrollbar thumb on the right border, overdrawing the track cells
    //     it spans — including for a menu (`selected.is_some()`): the
    //     highlight bar signals *which row*, not how much more there is to
    //     scroll past.
    if border
        && let Some((thumb_start, thumb_len)) = scrollbar_thumb(inner_h, rows.len(), scroll_offset)
    {
        let right = outer.x + outer.width - 1;
        for row in thumb_start..thumb_start + thumb_len {
            canvas.write_text_run(
                right,
                outer.y + 1 + row as u16,
                line::THICK_VERTICAL,
                styles.scroll,
                right + 1,
            );
        }
    }

    // 3. Draw content rows inside the frame (offset +1 for top/left border/padding).
    let text_x = outer.x + 1;
    // Rows arrive untruncated — `outer` was sized to the widest of them but
    // then clamped to the pane, so a row wider than the pane would otherwise
    // be written straight over the right border and past it. Bounding every
    // row write at the inner edge is what keeps the box a box.
    let text_right = outer.x + outer.width.saturating_sub(1);
    for (i, row_text) in visible_rows.iter().enumerate() {
        let y = outer.y + 1 + i as u16;
        let row_idx = scroll_offset + i;

        if selected == Some(row_idx) {
            // Highlight the full inner width so the selection bar is uniform.
            let inner_rect = Rect::new(text_x, y, outer.width.saturating_sub(2), 1);
            canvas.fill_rect_bg(inner_rect, styles.selected);
            canvas.write_text_run(text_x, y, row_text, styles.selected, text_right);
        } else if let Some(runs) = styled.and_then(|rows| rows.get(row_idx)) {
            // The base fill (step 1) already covers the row — runs are
            // contiguous and together span exactly `row_text`, so there are
            // no gaps left for `styles.base` to show through.
            paint_styled_row(canvas, text_x, y, runs, text_right);
        } else {
            canvas.write_text_run(text_x, y, row_text, styles.base, text_right);
        }
    }
}

/// Paint one pre-resolved styled row's runs left-to-right starting at
/// `(x, y)` — [`draw_menu_box`]'s styled-row branch, factored out for
/// readability.
fn paint_styled_row(canvas: &mut Canvas, x: u16, y: u16, runs: &StyledRow, right_edge: u16) {
    let mut cx = x;
    for (run_text, run_style) in runs {
        cx = canvas.write_text_run(cx, y, run_text, *run_style, right_edge);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
