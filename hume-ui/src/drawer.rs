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
//! the docked popup (`show-popup! #:anchor 'bottom`, [`super::popup`]), a
//! separate bottom band that keeps popup scroll/dismiss semantics instead of
//! pick-list selection.

use hume_grid::Rect;
use std::sync::Arc;

use hume_engine::lock::SharedSlot;

use hume_engine::providers::BottomBandProvider;
use hume_engine::render::Canvas;
use hume_engine::theme::Theme;
use hume_engine::types::Scope;

/// Read-side snapshot for `DrawerWidget` — the same shape as
/// `DrawerModel` (`hume-editor`'s raw `(show-drawer-list! …)` state) minus
/// the callback, which the render side never needs.
pub struct DrawerViewState {
    pub rows: Arc<Vec<String>>,
    pub selected: usize,
    pub scroll: usize,
}

/// Engine-facing drawer provider — one instance, owned by `EngineView`
/// (chrome, not per-pane), constructed once at `Editor::open` time.
pub(crate) struct DrawerWidget {
    pub(crate) data: SharedSlot<Option<DrawerViewState>>,
}

/// Row 0 is a blank padding row (visual gap from the pane above) — always
/// present regardless of item count.
const DRAWER_PAD_ROWS: u16 = 1;

/// Rows a drawer shows at once, given `items` rows and the band's row
/// ceiling `max` (half the last-rendered *terminal* height, mirroring
/// `DrawerWidget::height`'s own `max`) — the number `Editor::
/// drawer_visible_rows` pages against, agreeing with what the engine will
/// next paint by construction (both derive from
/// `super::menu_box::band_capacity`).
pub fn visible_rows(items: usize, max: u16) -> usize {
    super::menu_box::band_visible_rows(items, DRAWER_PAD_ROWS, max)
}

impl BottomBandProvider for DrawerWidget {
    fn height(&self, max: u16) -> u16 {
        let guard = self.data.read();
        guard.as_ref().map_or(0, |s| {
            super::menu_box::band_capacity(s.rows.len(), DRAWER_PAD_ROWS, max)
        })
    }

    fn render(&self, area: Rect, theme: &Theme, canvas: &mut Canvas) {
        if area.height == 0 {
            return;
        }
        let guard = self.data.read();
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
    use super::visible_rows;

    /// The overflow-safety guard on the shared arithmetic itself lives in
    /// `menu_box::tests` (`band_capacity_clamps_instead_of_overflowing_u16`)
    /// — this pins the drawer's own 1-row pad against it instead of
    /// re-testing the arithmetic.
    #[test]
    fn visible_rows_reserves_the_pad_row_below_the_cap() {
        assert_eq!(
            visible_rows(3, 20),
            3,
            "well under the cap: all 3 items fit"
        );
    }
}
