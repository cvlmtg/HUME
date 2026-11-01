//! Shared box-drawing + row rendering for `PopupOverlay`, `MenuOverlay`,
//! `PickerOverlay` and `DrawerWidget`.
//! Placement (bottom-anchored minibuffer completion, cursor-anchored popup/
//! menu, full-modal picker, the drawer's bottom band) is resolved per write
//! side, each against a different anchor; once a position and outer size are
//! settled, painting the frame, scroll window, and rows is identical. That
//! shared part lives here, along with the band arithmetic the drawer and the
//! docked popup share.

use hume_engine::render::Canvas;
use hume_engine::theme::Theme;
use hume_engine::theme::ui_scopes;
use hume_engine::types::ResolvedStyle;
use hume_engine::types::Scope;
use hume_grid::Rect;
use hume_grid::box_glyphs::{
    BOTTOM_LEFT, BOTTOM_RIGHT, HORIZONTAL, THICK_VERTICAL, TOP_LEFT, TOP_RIGHT, VERTICAL,
};
use hume_rope::width::chrome_width;

use super::list_row::{ListRow, draw_row};
use super::popup::StyledRow;

/// Theme styles a bordered box paints with, grouped into one struct rather
/// than three positional `ResolvedStyle` arguments on `draw_menu_box`, which needs
/// `#[allow(clippy::too_many_arguments)]` regardless given its other params
/// (canvas, rect, rows, selection, scroll, border).
#[derive(Clone, Copy)]
pub(crate) struct MenuBoxStyles {
    /// Fill, border, and unstyled rows.
    pub base: ResolvedStyle,
    /// The highlighted row (menus only; a popup never selects a row).
    pub selected: ResolvedStyle,
    /// The scrollbar thumb.
    pub scroll: ResolvedStyle,
}

impl MenuBoxStyles {
    /// A menu's styles: `ui.menu`, `ui.menu.selected` and `ui.menu.scroll`.
    pub(crate) fn menu(theme: &Theme) -> Self {
        let by = |name| theme.resolve_by_name(Scope(name));
        Self {
            base: by(ui_scopes::MENU),
            selected: by(ui_scopes::MENU_SELECTED),
            scroll: by(ui_scopes::MENU_SCROLL),
        }
    }

    /// A popup's styles: `ui.popup` and `ui.popup.scroll`. A popup never
    /// selects a row, so its selected style is its base.
    pub(crate) fn popup(theme: &Theme) -> Self {
        let base = theme.resolve_by_name(Scope(ui_scopes::POPUP));
        Self {
            base,
            selected: base,
            scroll: theme.resolve_by_name(Scope(ui_scopes::POPUP_SCROLL)),
        }
    }
}

/// Maximum number of visible rows inside a menu/popup box (excluding the
/// 1-cell frame). Both overlays scroll past this using [`window_range`].
pub(crate) const MAX_MENU_ROWS: u16 = 10;

/// Cells a box's frame takes at each of its edges. Reserved even with
/// `popup-border` off: a plain background margin takes the same cells (see
/// [`draw_menu_box`]'s `border`).
pub(crate) const FRAME_EDGE: u16 = 1;

/// Cells the frame takes across a box, both edges together, on either axis.
pub(crate) const FRAME_CELLS: u16 = 2 * FRAME_EDGE;

/// Widest of an arbitrary run of texts' display widths, or `0` for an empty
/// run.
pub(crate) fn widest<'a>(texts: impl Iterator<Item = &'a str>) -> u16 {
    texts.map(chrome_width).max().unwrap_or(0) as u16
}

/// Outer row count (including the 1-cell frame's top/bottom) for a box
/// windowing `row_count` rows to at most `row_cap` visible ones: the height
/// half of [`outer_dims_from_width`], split out because `resolve_menu`
/// resolves its height before it knows its width (see that function's own
/// comment on the ordering) and so has no `inner_width` to hand that
/// function yet.
pub(crate) fn outer_rows(row_count: usize, row_cap: u16) -> u16 {
    // Clamp in `usize` before casting down: `(row_count as u16)` truncates
    // first and can wrap a huge `row_count` (a large `buffer-words` index on
    // an empty prefix) to a value *smaller* than `row_cap`, which then wins
    // the min and produces a near-zero menu height. Same fix as
    // `outer_dims_from_width`'s own sibling below.
    (row_count.min(row_cap as usize) + usize::from(FRAME_CELLS)) as u16
}

/// Outer footprint (including the 1-cell frame) for a box showing `row_count`
/// rows measuring `inner_width` wide, windowed to at most `row_cap` visible
/// rows. `resolve_popup`'s wrapped content already has the width in hand
/// from a cached measurement, so this takes it directly rather than
/// re-measuring `rows` itself.
pub(crate) fn outer_dims_from_width(
    inner_width: u16,
    row_count: usize,
    row_cap: u16,
) -> (u16, u16) {
    (inner_width + FRAME_CELLS, outer_rows(row_count, row_cap))
}

/// The row ceiling of every bottom band: 35% of the terminal's height.
/// Widened to `u32` for the multiply so a `u16::MAX`-tall terminal can't
/// overflow before the divide narrows it back.
fn band_ceiling(terminal_height: u16) -> u16 {
    (u32::from(terminal_height) * 35 / 100) as u16
}

/// Outer row count for a bottom band showing `content_rows` rows plus
/// `chrome_rows` of fixed frame (a drawer's 1-row top frame, a docked
/// popup's 2-row top/bottom border) on a terminal `terminal_height` rows
/// tall, capped at the band ceiling. The single source of
/// truth for this arithmetic, shared by the drawer's and docked popup's
/// `band_height` (the rows the layout reserves) and `band_visible_rows`
/// (what the write side pages against). Kept in one place so the laid-out
/// band and the scroll clamp can never disagree.
///
/// Adds in `usize` and clamps once, rather than `content_rows as u16 +
/// chrome_rows`: a `+ chrome_rows` on a `u16`-truncated row count can wrap
/// (debug-panic, or paint an empty band with no error in release) for a
/// references drawer at or above 65535 rows.
pub(crate) fn band_capacity(content_rows: usize, chrome_rows: u16, terminal_height: u16) -> u16 {
    content_rows
        .saturating_add(chrome_rows as usize)
        .min(band_ceiling(terminal_height) as usize) as u16
}

/// Cells inside a bottom band's side frame on a terminal `terminal_width`
/// cells wide: the band spans the terminal. Shared by the drawer's rows and
/// the docked popup's wrap width.
pub(crate) fn band_inner_width(terminal_width: u16) -> u16 {
    terminal_width.saturating_sub(FRAME_CELLS)
}

/// Rows a bottom band shows at once, given `content_rows` rows and
/// `chrome_rows` of fixed frame on a terminal `terminal_height` rows tall:
/// the number the write side pages against, agreeing with the rows the
/// layout reserves (both derive from [`band_capacity`]).
pub(crate) fn band_visible_rows(
    content_rows: usize,
    chrome_rows: u16,
    terminal_height: u16,
) -> usize {
    band_capacity(content_rows, chrome_rows, terminal_height).saturating_sub(chrome_rows) as usize
}

/// The `[start, end)` window of `max_height` entries out of `total`,
/// starting as close to `desired_start` as the total allows, clamped so the
/// window never runs past the end. Resolved on the write side, not at paint time:
/// a menu passes `selected.saturating_sub(max_height / 2)` to keep the
/// selected row anchored near the window's center; a plain popup passes its
/// own `scroll` directly, so the window starts at the scroll position.
pub(crate) fn window_range(
    total: usize,
    desired_start: usize,
    max_height: usize,
) -> std::ops::Range<usize> {
    if total <= max_height {
        return 0..total;
    }
    let start = desired_start.min(total - max_height);
    start..start + max_height
}

/// Clamps `scroll` so `selected` stays inside a `visible_rows`-tall window,
/// scrolling by the minimum needed in either direction. A no-op (returns `scroll`
/// unchanged) when `visible_rows` is `0`: nothing fits, so there's no window
/// to clamp into.
///
/// This alone doesn't bound `scroll` to the list's own length. A caller
/// whose list just shrank under an unrelated `scroll` (a diagnostics refresh
/// dropping most of the rows) needs that bound applied separately, at read
/// time; see `EditorState::clamp_drawer_scroll_to_terminal`'s own doc for
/// why the two are split rather than folded into one function here.
pub fn clamp_scroll_to_window(selected: usize, scroll: usize, visible_rows: usize) -> usize {
    if visible_rows == 0 {
        return scroll;
    }
    if selected >= scroll + visible_rows {
        selected + 1 - visible_rows
    } else if selected < scroll {
        selected
    } else {
        scroll
    }
}

/// Overdraws `outer`'s 1-cell frame with box-drawing glyphs (`┌─┐└┘│`).
pub(crate) fn draw_box_border(canvas: &mut Canvas, outer: Rect, style: ResolvedStyle) {
    draw_open_box_border(canvas, outer, style);
    let inner = outer.inset(FRAME_EDGE, 0);
    let bottom = outer.bottom() - 1;
    let edge = outer.right();
    canvas.write_text_run(outer.x, bottom, BOTTOM_LEFT, style, edge);
    canvas.fill_glyph_run(inner.x, bottom, HORIZONTAL, inner.width, style, edge);
    canvas.write_text_run(inner.right(), bottom, BOTTOM_RIGHT, style, edge);
}

/// Overdraws `outer`'s top row and its left and right columns with
/// box-drawing glyphs (`┌─┐│`), leaving the bottom open.
pub(crate) fn draw_open_box_border(canvas: &mut Canvas, outer: Rect, style: ResolvedStyle) {
    // `inner.right()` is the right border's own column: `outer.right()` is
    // the exclusive bound one past it.
    let inner = outer.inset(FRAME_EDGE, 0);
    let right = inner.right();

    // Border glyphs are constants a cell wide, so they need none of
    // `write_text_run`'s substitution, but the corners go through it
    // anyway rather than carry an exemption from the one-writer rule for no
    // benefit, and the bound keeps a mis-sized box from drawing past its
    // own footprint. The top edge is a run of the same glyph repeated, so
    // it goes through `fill_glyph_run` instead: same writer, no `String`
    // built just to hand a grapheme walker a cluster it already knows is
    // one repeated character.
    let edge = outer.right();
    canvas.write_text_run(outer.x, outer.y, TOP_LEFT, style, edge);
    canvas.fill_glyph_run(inner.x, outer.y, HORIZONTAL, inner.width, style, edge);
    canvas.write_text_run(right, outer.y, TOP_RIGHT, style, edge);

    for row in 1..outer.height {
        canvas.write_text_run(outer.x, outer.y + row, VERTICAL, style, edge);
        canvas.write_text_run(right, outer.y + row, VERTICAL, style, edge);
    }
}

/// Track-relative `(start, len)` of the scrollbar thumb for a `view`-row
/// window into `total` rows scrolled to `scroll`, or `None` when everything
/// fits (nothing to scroll, so no thumb to draw).
///
/// `len` is proportional to the visible fraction (`view / total`), clamped to
/// `1..=(view - 1).max(1)` so the thumb never grows to fill the whole track
/// (a full track conveys no position at all), except at `view == 1`, where
/// there's no shorter length to clamp to and the single-cell track is always
/// a full-length thumb. `start` places the thumb so it sits flush against the
/// top edge when, and only when, `scroll == 0` and flush against the bottom
/// edge when, and only when, `scroll == max_scroll`; floor division alone reaches the
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

/// The rows a framed box paints, already windowed to what is visible.
pub(crate) enum BoxRows<'a> {
    /// A popup's wrapped text, each line in the box's base style. No row is
    /// selected.
    Plain(&'a [String]),
    /// A styled popup's wrapped text (a markdown popup with a `markdown`
    /// grammar registered): each line is its runs, each in its own absolute
    /// style. No row is selected.
    Styled(&'a [StyledRow]),
    /// A menu's rows, `selected` (an index into `rows`) highlighted. A row's
    /// spans name theme scopes, layered over the row's own style.
    List {
        rows: &'a [ListRow],
        selected: Option<usize>,
        theme: &'a Theme,
    },
}

impl BoxRows<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Plain(lines) => lines.len(),
            Self::Styled(lines) => lines.len(),
            Self::List { rows, .. } => rows.len(),
        }
    }
}

/// Paint a menu/popup box into `outer` (the full footprint, including the
/// 1-cell frame). `rows` arrives *already windowed* to what's visible: the
/// write side resolves the window, this only paints it; `total_rows` and
/// `scroll` (the window's own start within the full, unwindowed list) exist
/// here solely to size and place the scrollbar thumb.
///
/// `border`: when `true`, overdraws the 1-cell frame with box-drawing
/// glyphs; when `false`, the frame stays a plain background-filled margin
/// (still 1 cell wide; only the glyphs are suppressed).
pub(crate) fn draw_menu_box(
    canvas: &mut Canvas,
    outer: Rect,
    rows: BoxRows<'_>,
    total_rows: usize,
    scroll: usize,
    border: bool,
    styles: MenuBoxStyles,
) {
    if rows.len() == 0 || outer.height < 3 || outer.width < 3 {
        return;
    }

    let inner = outer.inset(FRAME_EDGE, FRAME_EDGE);

    // Opaque backdrop so no buffer content shows through. Without a border
    // it also forms the 1-cell margin.
    canvas.fill_rect_bg(outer, styles.base);

    if border {
        draw_box_border(canvas, outer, styles.base);
    }

    // Scrollbar thumb on the right border. Menus get one too: the
    // highlight bar shows which row is selected, not how far there is to
    // scroll.
    if border
        && let Some((thumb_start, thumb_len)) =
            scrollbar_thumb(inner.height as usize, total_rows, scroll)
    {
        let right = inner.right();
        for row in thumb_start..thumb_start + thumb_len {
            canvas.write_text_run(
                right,
                inner.y + row as u16,
                THICK_VERTICAL,
                styles.scroll,
                right + 1,
            );
        }
    }

    let text_x = inner.x;
    // Rows arrive untruncated: `outer` was sized to the widest of them but
    // then clamped to the pane, so a row wider than the pane would otherwise
    // be written straight over the right border and past it. Bounding every
    // row write at the inner edge is what keeps the box a box, horizontally.
    // The `.min` below is that same guarantee for the vertical axis: the
    // write side is trusted to hand in a pre-windowed slice, but this is the
    // one place that could still catch a caller that gets it wrong. One row
    // too many would otherwise overwrite the bottom border, then paint
    // straight past the box.
    let text_right = inner.right();
    for i in 0..rows.len().min(inner.height as usize) {
        let y = inner.y + i as u16;
        match rows {
            BoxRows::Styled(lines) => {
                // The box's base fill already covers the row. Runs are
                // contiguous and together span the whole line, so there are
                // no gaps left for `styles.base` to show through.
                paint_styled_row(
                    canvas,
                    text_x,
                    y,
                    lines[i].iter().map(|(text, style)| (text.as_str(), *style)),
                    text_right,
                );
            }
            BoxRows::Plain(lines) => paint_styled_row(
                canvas,
                text_x,
                y,
                [(lines[i].as_str(), styles.base)],
                text_right,
            ),
            BoxRows::List {
                rows,
                selected,
                theme,
            } => draw_row(
                canvas,
                text_x,
                y,
                inner.width,
                text_right,
                Some(&rows[i]),
                theme,
                selected == Some(i),
                styles.selected,
                styles.base,
            ),
        }
    }
}

/// Paint `runs` left to right from `(x, y)`, each in its own style, stopping
/// at `right_edge`. Every row goes through it: a list's rows
/// ([`draw_row`](crate::list_row::draw_row)) and a popup's lines in
/// [`draw_menu_box`].
pub(crate) fn paint_styled_row<'a>(
    canvas: &mut Canvas,
    x: u16,
    y: u16,
    runs: impl IntoIterator<Item = (&'a str, ResolvedStyle)>,
    right_edge: u16,
) {
    let mut cx = x;
    for (run_text, run_style) in runs {
        cx = canvas.write_text_run(cx, y, run_text, run_style, right_edge);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
