//! Bottom drawer (`show-drawer-list!`): a generic, display-only
//! scrolling pick-list rendered in a chrome band above the statusline.
//!
//! Its position is fixed (bottom band). The write side resolves the view
//! once per frame (`Editor::sync_drawer_view`, from `prepare_frame`, which
//! runs before every paint), like the popup/menu/picker views, so a model
//! change by any path, `:reload-config`'s reset included, shows in the
//! frame it happens.
//!
//! Rows are the visible window of pre-formatted [`ListRow`]s, already cut to
//! [`row_width`], whose spans may name theme scopes for parts of them: the
//! drawer is a generic list picker, not a location list, and Rust interprets
//! row content only to style it. The caller's `on-select` callback does the
//! jump itself (e.g. via `goto-location!`), and may fire more than once:
//! unlike the popup/menu, the drawer stays open across `Enter` (Helix-style
//! browse).
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

use crate::list_row::{ListRow, draw_row};

/// Read-side snapshot for `DrawerWidget`: the rows of `hume-editor`'s
/// `DrawerLayer` that the band shows this frame, and which one is selected.
pub struct DrawerViewState {
    /// The rows the band shows, top first, each cut to [`row_width`]. A row
    /// a render proc has not made yet is `None` and draws blank.
    pub rows: Vec<Option<Arc<ListRow>>>,
    /// The selected row's index in `rows`, `None` when the band shows no
    /// row.
    pub selected_row: Option<usize>,
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

/// Cells one row gets in a drawer on a terminal `terminal_width` cells
/// wide: the width the write side cuts rows to and the one the drawer
/// paints them in.
pub fn row_width(terminal_width: u16) -> usize {
    super::menu_box::band_inner_width(terminal_width) as usize
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
            x: area.x + super::menu_box::FRAME_EDGE,
            y: area.y + DRAWER_FRAME_ROWS,
            width: row_width(area.width) as u16,
            height: area.height - DRAWER_FRAME_ROWS,
        };
        for (i, item) in state.rows.iter().take(inner.height as usize).enumerate() {
            draw_row(
                canvas,
                Rect::new(inner.x, inner.y + i as u16, inner.width, 1),
                item.as_deref(),
                theme,
                state.selected_row == Some(i),
                selected_style,
                style,
            );
        }
    }
}

#[cfg(test)]
mod tests;
