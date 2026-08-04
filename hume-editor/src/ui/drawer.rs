//! Class B bottom drawer (`show-drawer-list!`) — a generic, display-only
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

use std::sync::{Arc, RwLock};

use crate::lock_ext::LockExt;

use ratatui::buffer::Buffer as ScreenBuf;
use ratatui::layout::Rect;

use hume_engine::providers::BottomBandProvider;
use hume_engine::theme::Theme;
use hume_engine::types::Scope;

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
}

/// Read-side snapshot for [`DrawerWidget`] — the same shape as
/// [`DrawerModel`] minus the callback, which the render side never needs.
pub(crate) struct DrawerViewState {
    pub(crate) rows: Vec<String>,
    pub(crate) selected: usize,
    pub(crate) scroll: usize,
}

/// Engine-facing drawer provider — one instance, owned by `EngineView`
/// (chrome, not per-pane), constructed once at `Editor::open` time.
pub(crate) struct DrawerWidget {
    pub(crate) data: Arc<RwLock<Option<DrawerViewState>>>,
}

impl BottomBandProvider for DrawerWidget {
    fn height(&self, max: u16) -> u16 {
        let guard = self.data.read_or_panic();
        guard
            .as_ref()
            .map_or(0, |s| (s.rows.len() as u16 + 1).min(max))
    }

    fn render(&self, area: Rect, theme: &Theme, buf: &mut ScreenBuf) {
        if area.height == 0 {
            return;
        }
        let guard = self.data.read_or_panic();
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
                // Highlight bar always wins, same as `draw_menu_box`.
                hume_engine::render::fill_rect_bg(
                    buf,
                    Rect::new(area.x, y, area.width, 1),
                    selected_style,
                );
                buf.set_string(area.x, y, item, selected_style);
            } else {
                buf.set_string(area.x, y, item, style);
            }
        }
    }
}
