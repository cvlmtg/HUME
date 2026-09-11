//! Bottom drawer (`show-drawer-list!`) — a generic, display-only
//! scrolling pick-list rendered in a chrome band above the statusline.
//!
//! Unlike the popup/menu widgets, the drawer has no per-frame geometry to
//! resolve: its position is fixed (bottom band) and its content only
//! changes on discrete events (open, selection move, scroll, close) — so the
//! shared view is written directly at each of those event sites
//! (`sync_drawer_view`) for zero-lag immediacy. It is *also* re-synced
//! unconditionally every frame from `prepare_frame`, like the popup/menu/
//! picker views, as a self-healing backstop: a direct model mutation that
//! bypasses the normal open/close builtins (`Editor::reset_config_state`'s
//! `:reload-config` reset) can otherwise leave a stale view painting a
//! closed drawer for however long it takes the next frame to arrive.
//!
//! Rows are pre-formatted display strings — the drawer is a generic list
//! picker, not a location list; Rust never interprets row content. The
//! caller's `on-select` callback does the jump itself (e.g. via
//! `goto-location!`), and may fire more than once: unlike the popup/menu,
//! the drawer stays open across `Enter` (Helix-style browse).
//!
//! Long-form text (e.g. hover overflow) is not a drawer use case — that's
//! the docked popup (`show-popup! #:anchor 'bottom`, `ui::popup`), a
//! separate bottom band that keeps popup scroll/dismiss semantics instead of
//! pick-list selection.

use hume_grid::Rect;
use std::sync::{Arc, RwLock};

use hume_engine::lock::LockExt;

use hume_engine::providers::BottomBandProvider;
use hume_engine::render::Canvas;
use hume_engine::theme::Theme;
use hume_engine::types::Scope;

/// Read-side snapshot for [`DrawerWidget`] — the same shape as
/// `DrawerModel` (`hume-editor`'s raw `(show-drawer-list! …)` state) minus
/// the callback, which the render side never needs.
pub struct DrawerViewState {
    pub rows: Arc<Vec<String>>,
    pub selected: usize,
    pub scroll: usize,
}

/// Engine-facing drawer provider — one instance, owned by `EngineView`
/// (chrome, not per-pane), constructed once at `Editor::open` time.
pub struct DrawerWidget {
    pub data: Arc<RwLock<Option<DrawerViewState>>>,
}

/// Outer row count for a drawer showing `items` rows, capped at `max` — the
/// single source of truth for this arithmetic, shared by
/// [`DrawerWidget::height`] (what the engine paints against) and
/// [`visible_rows`] (what `Editor::drawer_visible_rows` pages against),
/// mirroring how `ui::popup`'s own `band_capacity` is shared by its band
/// widget and [`super::popup::band_visible_rows`].
///
/// `+1` reserves row 0, the blank padding row (visual gap from the pane
/// above) — always present regardless of item count. Adds in `usize` and
/// clamps once, rather than `items as u16 + 1`: a `+ 1` on a `u16`-truncated
/// item count can wrap (debug-panic, or silently paint an empty band in
/// release) for a references drawer at or above 65535 rows.
fn band_capacity(items: usize, max: u16) -> u16 {
    items.saturating_add(1).min(max as usize) as u16
}

/// Rows a drawer shows at once, given `items` rows and the band's row
/// ceiling `max` (half the last-rendered *terminal* height, mirroring
/// [`DrawerWidget::height`]'s own `max`) — the number `Editor::
/// drawer_visible_rows` pages against, agreeing with what the engine will
/// next paint by construction (both derive from `band_capacity`).
pub fn visible_rows(items: usize, max: u16) -> usize {
    band_capacity(items, max).saturating_sub(1) as usize
}

impl BottomBandProvider for DrawerWidget {
    fn height(&self, max: u16) -> u16 {
        let guard = self.data.read_or_panic();
        guard
            .as_ref()
            .map_or(0, |s| band_capacity(s.rows.len(), max))
    }

    fn render(&self, area: Rect, theme: &Theme, canvas: &mut Canvas) {
        if area.height == 0 {
            return;
        }
        let guard = self.data.read_or_panic();
        let Some(state) = guard.as_ref() else { return };

        let style = theme.resolve_by_name(Scope("ui.drawer"));
        let selected_style = theme.resolve_by_name(Scope("ui.menu.selected"));
        canvas.fill_rect_bg(area, style);

        // Row 0 is a blank padding row (visual gap from the pane above);
        // rows 1.. show the scroll-adjusted, visible slice of items.
        let visible_rows = area.height.saturating_sub(1) as usize;
        for (i, item) in state
            .rows
            .iter()
            .skip(state.scroll)
            .take(visible_rows)
            .enumerate()
        {
            let row_idx = state.scroll + i;
            let y = area.y + 1 + i as u16;
            super::menu_box::draw_list_row(
                canvas,
                area.x,
                y,
                area.width,
                area.right(),
                item,
                row_idx == state.selected,
                selected_style,
                style,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::band_capacity;

    /// `items as u16 + 1` overflows at exactly `u16::MAX` (65535 rows,
    /// e.g. a references drawer) and truncates silently above it — both
    /// would previously wrap to a near-zero `u16` instead of clamping to
    /// `max`.
    #[test]
    fn band_capacity_clamps_instead_of_overflowing_u16() {
        assert_eq!(band_capacity(65_535, 20), 20);
        assert_eq!(band_capacity(usize::MAX, 20), 20);
    }

    #[test]
    fn band_capacity_reserves_one_row_below_the_cap() {
        assert_eq!(band_capacity(3, 20), 4);
    }
}
