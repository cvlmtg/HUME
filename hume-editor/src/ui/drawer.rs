//! Class B bottom drawer (`show-drawer-list!`) — a generic, display-only
//! scrolling list rendered in a chrome band above the statusline.
//!
//! Unlike the popup/menu widgets, the drawer has no per-frame geometry to
//! resolve: its position is fixed (bottom band) and its content only
//! changes on discrete events (open, selection move, scroll, close) — so
//! the shared view is written directly at each of those event sites
//! (`sync_drawer_view`), never per frame from `prepare_frame`. Highlight
//! *styles* still resolve fresh every `render` call, though (see
//! `DrawerModel::syntax`), so a `:theme` switch repaints correctly without
//! needing its own sync event.
//!
//! Rows are pre-formatted display strings — the drawer is a generic list
//! picker, not a location list; Rust never interprets row content beyond an
//! optional `#:lang` syntax highlight pass. The caller's `on-select`
//! callback does the jump itself (e.g. via `goto-location!`), and may fire
//! more than once: unlike the popup/menu, the drawer stays open across
//! `Enter` (Helix-style browse).

use std::sync::{Arc, RwLock};

use ratatui::buffer::Buffer as ScreenBuf;
use ratatui::layout::Rect;

use hume_engine::providers::DrawerProvider;
use hume_engine::theme::Theme;
use hume_engine::types::Scope;

use super::menu_box::paint_styled_row;
use super::popup::MarkupSyntax;

/// `(show-drawer-list! items on-select)`'s raw state, including the
/// not-yet-exhausted Steel callback — cleared by `Esc` or `close-drawer!`,
/// not by `Enter` (the drawer stays open across selections).
pub(crate) struct DrawerModel {
    pub(crate) items: Vec<String>,
    pub(crate) selected: usize,
    /// Index of the first visible row — clamped to keep `selected` in view
    /// whenever the selection moves (`Editor::clamp_drawer_scroll`).
    pub(crate) scroll: usize,
    pub(crate) callback: steel::rvals::SteelVal,
    /// `#:lang` — parsed once from `items.join("\n")` at `show-drawer-list!`
    /// time (one tree-sitter line per row), `None` when no such grammar is
    /// registered or `#:lang` wasn't requested. `Arc`'d (unlike the popup's
    /// bare `MarkupSyntax`) because `sync_drawer_view` clones this into
    /// `DrawerViewState` on every discrete drawer event, not just once.
    pub(crate) syntax: Option<Arc<MarkupSyntax>>,
}

/// Read-side snapshot for [`DrawerWidget`] — the same shape as
/// [`DrawerModel`] minus the callback, which the render side never needs.
pub(crate) struct DrawerViewState {
    pub(crate) rows: Vec<String>,
    pub(crate) selected: usize,
    pub(crate) scroll: usize,
    pub(crate) syntax: Option<Arc<MarkupSyntax>>,
}

/// Engine-facing drawer provider — one instance, owned by `EngineView`
/// (chrome, not per-pane), constructed once at `Editor::open` time.
pub(crate) struct DrawerWidget {
    pub(crate) data: Arc<RwLock<Option<DrawerViewState>>>,
}

impl DrawerProvider for DrawerWidget {
    fn height(&self, max: u16) -> u16 {
        let guard = self.data.read().expect("RwLock not poisoned");
        guard
            .as_ref()
            .map_or(0, |s| (s.rows.len() as u16 + 1).min(max))
    }

    fn render(&self, area: Rect, theme: &Theme, buf: &mut ScreenBuf) {
        if area.height == 0 {
            return;
        }
        let guard = self.data.read().expect("RwLock not poisoned");
        let Some(state) = guard.as_ref() else { return };

        let style = theme.resolve_by_name(Scope("ui.drawer")).into();
        let selected_style = theme.resolve_by_name(Scope("ui.menu.selected")).into();
        hume_engine::render::fill_rect_bg(buf, area, style);

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
            if row_idx == state.selected {
                // Highlight bar always wins, same as `draw_menu_box` — a
                // per-span-styled row still gets the flat selected style.
                hume_engine::render::fill_rect_bg(
                    buf,
                    Rect::new(area.x, y, area.width, 1),
                    selected_style,
                );
                buf.set_string(area.x, y, item, selected_style);
            } else if let Some(syntax) = state.syntax.as_ref() {
                let runs = syntax.styled_row(row_idx, item, theme, style);
                paint_styled_row(buf, area.x, y, &runs);
            } else {
                buf.set_string(area.x, y, item, style);
            }
        }
    }
}
