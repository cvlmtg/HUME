//! Shared box-drawing + row rendering for `PopupOverlay` and `PickerOverlay`.
//! Placement (bottom-anchored minibuffer completion, cursor-anchored popup/
//! menu, full-modal picker) is resolved per write side, each against a
//! different anchor; once a position and outer size are settled, painting the
//! frame, scroll window, and rows is identical — that shared part lives here.

use hume_engine::render::Canvas;
use hume_engine::theme::Theme;
use hume_engine::theme::ui_scopes;
use hume_engine::types::ResolvedStyle;
use hume_engine::types::Scope;
use hume_grid::Rect;
use hume_grid::box_glyphs::{
    BOTTOM_LEFT, BOTTOM_RIGHT, HORIZONTAL, THICK_VERTICAL, TOP_LEFT, TOP_RIGHT, VERTICAL,
};

use super::popup::StyledRow;
use super::width::text_width;

/// Theme styles a bordered box paints with — grouped into one struct rather
/// than three positional `ResolvedStyle` arguments on `draw_menu_box`, which needs
/// `#[allow(clippy::too_many_arguments)]` regardless given its other params
/// (canvas, rect, rows, selection, scroll, border).
#[derive(Clone, Copy)]
pub(crate) struct MenuBoxStyles {
    /// Fill, border, and unstyled rows.
    pub base: ResolvedStyle,
    /// The highlighted row (menus only — plain popups never set `selected`).
    pub selected: ResolvedStyle,
    /// The scrollbar thumb.
    pub scroll: ResolvedStyle,
}

impl MenuBoxStyles {
    /// Resolve all three styles from one root scope (`"ui.popup"` or
    /// `"ui.menu"`) — the single place that pairs each root with its
    /// `.selected`/`.scroll` leaves, so the three every popup/menu overlay
    /// paints with can't drift out of sync with each other. Leaf names are
    /// paired here rather than built with `format!` because `Scope` requires
    /// a `&'static str`, which a runtime-joined `String` can't provide.
    ///
    /// A root with no `.selected` leaf (`"ui.popup"` — hover popups never
    /// highlight a row) falls back to `base`, matching what an undefined
    /// theme scope would already resolve to via `Theme::resolve_by_name`'s
    /// own dot-notation chain.
    pub(crate) fn resolve(theme: &Theme, scope: &'static str) -> Self {
        let (selected_scope, scroll_scope): (Option<&'static str>, &'static str) = match scope {
            ui_scopes::MENU => (Some(ui_scopes::MENU_SELECTED), ui_scopes::MENU_SCROLL),
            ui_scopes::POPUP => (None, ui_scopes::POPUP_SCROLL),
            _ => (None, scope),
        };
        let base = theme.resolve_by_name(Scope(scope));
        let selected = selected_scope
            .map(|s| theme.resolve_by_name(Scope(s)))
            .unwrap_or(base);
        let scroll = theme.resolve_by_name(Scope(scroll_scope));
        Self {
            base,
            selected,
            scroll,
        }
    }
}

/// Maximum number of visible rows inside a menu/popup box (excluding the
/// 1-cell frame). Both overlays scroll past this using [`window_range`].
pub(crate) const MAX_MENU_ROWS: u16 = 10;

/// Widest row's display width — used only where a caller still has the full,
/// unwindowed row list in hand (`resolve_popup`'s wrapped content); a menu's
/// width is measured from its visible window alone (`resolve_menu`), never
/// from this.
pub(crate) fn menu_inner_width(rows: &[String]) -> u16 {
    rows.iter().map(|r| text_width(r)).max().unwrap_or(0) as u16
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
    let outer_w = inner_width + 2;
    let outer_h = (row_count as u16).min(row_cap) + 2;
    (outer_w, outer_h)
}

/// Outer row count for a bottom band showing `content_rows` rows plus
/// `chrome_rows` of fixed frame (a drawer's 1-row padding gap, a docked
/// popup's 2-row top/bottom border), capped at `max` — the single source of
/// truth for this arithmetic, shared by `DrawerWidget`/`PopupBandWidget`'s
/// own `height` (what the engine paints against) and `band_visible_rows`
/// (what the write side pages against). Kept in one place so the painted
/// band and the scroll clamp can never silently disagree.
///
/// Adds in `usize` and clamps once, rather than `content_rows as u16 +
/// chrome_rows`: a `+ chrome_rows` on a `u16`-truncated row count can wrap
/// (debug-panic, or silently paint an empty band in release) for a
/// references drawer at or above 65535 rows.
pub(crate) fn band_capacity(content_rows: usize, chrome_rows: u16, max: u16) -> u16 {
    content_rows
        .saturating_add(chrome_rows as usize)
        .min(max as usize) as u16
}

/// Rows a bottom band shows at once, given `content_rows` rows, `chrome_rows`
/// of fixed frame, and the band's row ceiling `max` — the number the write
/// side pages against, agreeing with what the engine will next paint by
/// construction (both derive from [`band_capacity`]).
pub(crate) fn band_visible_rows(content_rows: usize, chrome_rows: u16, max: u16) -> usize {
    band_capacity(content_rows, chrome_rows, max).saturating_sub(chrome_rows) as usize
}

/// The `[start, end)` window of `max_height` entries out of `total`,
/// starting as close to `desired_start` as the total allows — clamped so the
/// window never runs past the end. Resolved on the *write* side now (every
/// `resolve_popup`/`resolve_band`/`resolve_menu` caller), not at paint time:
/// a menu passes `selected.saturating_sub(max_height / 2)` to keep the
/// selected row anchored near the window's center; a plain popup passes its
/// own `scroll` directly, so the window start is exactly the scroll
/// position.
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
/// scrolling by the minimum needed in either direction. Shared by
/// `PickerSession::move_selection` and `Editor::clamp_drawer_scroll`, whose
/// scroll models otherwise differ (edge-anchored vs centered) but converge on
/// this one "keep the selection on screen" formula. A no-op (returns `scroll`
/// unchanged) when `visible_rows` is `0` — nothing fits, so there's no window
/// to clamp into.
///
/// This alone doesn't bound `scroll` to the list's own length — a caller
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

/// Whether `outer` fits entirely inside `pane_rect`. Shared by every overlay
/// that positions itself against a pane rect resolved earlier in the frame
/// (`PopupOverlay`, `PickerOverlay`) as a defensive backstop: the write side
/// already computed `outer` against this same rect this same frame, so this
/// should never return `false` — but painting outside the pane is worse than
/// a dropped frame of content.
pub(crate) fn fits_inside(outer: Rect, pane_rect: Rect) -> bool {
    outer.left() >= pane_rect.left()
        && outer.top() >= pane_rect.top()
        && outer.right() <= pane_rect.right()
        && outer.bottom() <= pane_rect.bottom()
}

/// Overdraws `outer`'s 1-cell frame with box-drawing glyphs (`┌─┐└┘│`).
/// Shared by every bordered box overlay — [`draw_menu_box`] and
/// `super::picker_panel::draw_picker_panel` — so the frame glyphs stay
/// identical without a copy per caller.
pub(crate) fn draw_box_border(canvas: &mut Canvas, outer: Rect, style: ResolvedStyle) {
    let inner = outer.inset(1, 1);
    // The right/bottom border's own column/row — `outer.right()`/`.bottom()`
    // is the exclusive bound one past it; `inner.right()`/`.bottom()` is
    // this same column/row, already computed by `inset` above.
    let right = inner.right();
    let bottom = inner.bottom();

    // Border glyphs are constants a cell wide, so they need none of
    // `write_text_run`'s substitution — but the corners go through it
    // anyway rather than carry an exemption from the one-writer rule for no
    // benefit, and the bound keeps a mis-sized box from drawing past its
    // own footprint. The two edges are a run of the same glyph repeated,
    // so they go through `fill_glyph_run` instead — same writer, no `String`
    // built just to hand a grapheme walker a cluster it already knows is
    // one repeated character.
    let edge = outer.right();
    canvas.write_text_run(outer.x, outer.y, TOP_LEFT, style, edge);
    canvas.fill_glyph_run(inner.x, outer.y, HORIZONTAL, inner.width, style, edge);
    canvas.write_text_run(right, outer.y, TOP_RIGHT, style, edge);
    canvas.write_text_run(outer.x, bottom, BOTTOM_LEFT, style, edge);
    canvas.fill_glyph_run(inner.x, bottom, HORIZONTAL, inner.width, style, edge);
    canvas.write_text_run(right, bottom, BOTTOM_RIGHT, style, edge);

    for row in 1..outer.height - 1 {
        canvas.write_text_run(outer.x, outer.y + row, VERTICAL, style, edge);
        canvas.write_text_run(right, outer.y + row, VERTICAL, style, edge);
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
/// 1-cell frame). `rows` arrives *already windowed* to what's visible — the
/// write side (`resolve_popup`/`resolve_band`/`resolve_menu`) resolves the
/// window, this only paints it; `total_rows` and `scroll` (the window's own
/// start within the full, unwindowed list) exist here solely to size and
/// place the scrollbar thumb.
///
/// `selected`: the highlighted row, already window-relative (an index into
/// `rows`, not into the full list) — `None` for a plain popup, which never
/// highlights a row.
///
/// `border`: when `true`, overdraws the 1-cell frame with box-drawing
/// glyphs; when `false`, the frame stays a plain background-filled margin
/// (still 1 cell wide — only the glyphs are suppressed).
///
/// `styled`: per-row style runs, same length (and same window) as `rows` — a
/// markdown popup with a `markdown` grammar registered. `None` for every
/// other caller (plain popups, menus), which paint each row in one style.
/// Ignored for a row that has `selected == Some(i)`: the highlight bar
/// always wins.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_menu_box(
    canvas: &mut Canvas,
    outer: Rect,
    rows: &[String],
    selected: Option<usize>,
    total_rows: usize,
    scroll: usize,
    border: bool,
    styles: MenuBoxStyles,
    styled: Option<&[StyledRow]>,
) {
    if rows.is_empty() || outer.height < 3 || outer.width < 3 {
        return;
    }

    let inner = outer.inset(1, 1);

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

    // 3. Draw content rows inside the frame (offset +1 for top/left border/padding).
    let text_x = inner.x;
    // Rows arrive untruncated — `outer` was sized to the widest of them but
    // then clamped to the pane, so a row wider than the pane would otherwise
    // be written straight over the right border and past it. Bounding every
    // row write at the inner edge is what keeps the box a box.
    let text_right = inner.right();
    for (i, row_text) in rows.iter().enumerate() {
        let y = inner.y + i as u16;
        let is_selected = selected == Some(i);

        // Highlight bar always wins, even over a styled row — a selected row
        // never needs per-run markdown styling, just the plain highlight.
        if !is_selected && let Some(runs) = styled.and_then(|rows| rows.get(i)) {
            // The base fill (step 1) already covers the row — runs are
            // contiguous and together span exactly `row_text`, so there are
            // no gaps left for `styles.base` to show through.
            paint_styled_row(canvas, text_x, y, runs, text_right);
        } else {
            draw_list_row(
                canvas,
                text_x,
                y,
                inner.width,
                text_right,
                row_text,
                is_selected,
                styles.selected,
                styles.base,
            );
        }
    }
}

/// Paint one row of a scrolling list: a full-width highlight-bar fill plus
/// its text in `selected_style` when `is_selected`, or just the text in
/// `base_style` otherwise. Shared by every list-style overlay
/// ([`draw_menu_box`], `super::picker_panel::draw_picker_panel`,
/// `super::drawer::DrawerWidget::render`) so the fill-then-write shape can't
/// drift between them — each caller still owns its own row-index bookkeeping
/// and text truncation, which differ in kind, not just in value, between them.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_list_row(
    canvas: &mut Canvas,
    x: u16,
    y: u16,
    highlight_width: u16,
    right_edge: u16,
    text: &str,
    is_selected: bool,
    selected_style: ResolvedStyle,
    base_style: ResolvedStyle,
) {
    if is_selected {
        canvas.fill_rect_bg(Rect::new(x, y, highlight_width, 1), selected_style);
        canvas.write_text_run(x, y, text, selected_style, right_edge);
    } else {
        canvas.write_text_run(x, y, text, base_style, right_edge);
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
