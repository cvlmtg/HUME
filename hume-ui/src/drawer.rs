//! Bottom drawer (`show-drawer-list!`): a generic, display-only
//! scrolling pick-list rendered in a chrome band above the statusline.
//!
//! Unlike the popup/menu widgets, the drawer has no per-frame geometry to
//! resolve: its position is fixed (bottom band) and its content only
//! changes on discrete events (open, selection move, scroll, close), so the
//! shared view is written directly at each of those event sites
//! (`sync_drawer_view`) for zero-lag immediacy. It is *also* re-synced
//! unconditionally every frame from `prepare_frame`, like the popup/menu/
//! picker views, as a self-healing backstop: a direct model mutation that
//! bypasses the normal open/close builtins (`Editor::reset_config_state`'s
//! `:reload-config` reset) can otherwise leave a stale view painting a
//! closed drawer for however long it takes the next frame to arrive.
//!
//! Rows are pre-formatted display strings: the drawer is a generic list
//! picker, not a location list; Rust never interprets row content. The
//! caller's `on-select` callback does the jump itself (e.g. via
//! `goto-location!`), and may fire more than once: unlike the popup/menu,
//! the drawer stays open across `Enter` (Helix-style browse).
//!
//! Long-form text (e.g. hover overflow) is not a drawer use case. That's
//! the docked popup (`show-popup! #:anchor 'bottom`, [`super::popup`]), a
//! separate bottom band that keeps popup scroll/dismiss semantics instead of
//! pick-list selection.

use hume_grid::Rect;
use std::sync::Arc;

use hume_engine::lock::SharedSlot;

use hume_engine::providers::ChromeProvider;
use hume_engine::render::Canvas;
use hume_engine::theme::Theme;
use hume_engine::theme::ui_scopes;
use hume_engine::types::Scope;

/// Read-side snapshot for `DrawerWidget`: the same shape as
/// `DrawerLayer` (`hume-editor`'s raw `(show-drawer-list! …)` state) minus
/// the callback, which the render side never needs. A row is `None` until
/// its text is rendered, and draws blank.
pub struct DrawerViewState {
    pub rows: Arc<Vec<Option<String>>>,
    pub selected: usize,
    pub scroll: usize,
    /// Whether to draw box-drawing glyphs on the frame's top row and side
    /// columns. Fed from the `popup-border` setting.
    pub border: bool,
}

/// Engine-facing drawer provider: one instance, owned by `EngineView`
/// (chrome, not per-pane), constructed once at `Editor::open` time.
pub(crate) struct DrawerWidget {
    pub(crate) data: SharedSlot<Option<DrawerViewState>>,
}

/// Row 0 is the frame's top row, always present regardless of item count.
/// The frame has no bottom row, so the band reads as sliding up from the
/// bottom of the screen.
const DRAWER_FRAME_ROWS: u16 = super::menu_box::FRAME_EDGE;

/// Rows a drawer of `items` rows takes on a terminal `terminal_height` rows
/// tall.
pub fn band_height(items: usize, terminal_height: u16) -> u16 {
    super::menu_box::band_capacity(items, DRAWER_FRAME_ROWS, terminal_height)
}

/// Rows of items a drawer shows at once inside [`band_height`]: the number
/// the drawer pages against.
pub fn band_visible_rows(items: usize, terminal_height: u16) -> usize {
    super::menu_box::band_visible_rows(items, DRAWER_FRAME_ROWS, terminal_height)
}

impl ChromeProvider for DrawerWidget {
    fn render(&self, area: Rect, theme: &Theme, canvas: &mut Canvas) {
        if area.height == 0 {
            return;
        }
        let guard = self.data.read();
        let Some(state) = guard.as_ref() else { return };

        let style = theme.resolve_by_name(Scope(ui_scopes::DRAWER));
        let selected_style = theme.resolve_by_name(Scope(ui_scopes::MENU_SELECTED));
        canvas.fill_rect_bg(area, style);
        // The frame takes the same cells with `popup-border` off, left as
        // plain background, matching the popup's margin.
        if state.border {
            super::menu_box::draw_open_box_border(canvas, area, style);
        }

        let inner = Rect {
            y: area.y + DRAWER_FRAME_ROWS,
            height: area.height - DRAWER_FRAME_ROWS,
            ..area.inset(super::menu_box::FRAME_EDGE, 0)
        };
        for (i, item) in state
            .rows
            .iter()
            .skip(state.scroll)
            .take(inner.height as usize)
            .enumerate()
        {
            let row_idx = state.scroll + i;
            let y = inner.y + i as u16;
            super::menu_box::draw_list_row(
                canvas,
                inner.x,
                y,
                inner.width,
                inner.right(),
                item.as_deref().unwrap_or(""),
                row_idx == state.selected,
                selected_style,
                style,
            );
        }
    }
}

#[cfg(test)]
mod tests;
