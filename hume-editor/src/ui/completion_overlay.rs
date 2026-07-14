//! Completion popup overlay — renders the Tab-completion candidate list above
//! the statusline while a completion session is active.
//!
//! The overlay reads a `CompletionView` snapshot from an `Arc<RwLock<_>>` that
//! `Editor` writes once per frame (in `prepare_frame`) before `EngineView::render`
//! is called.  The snapshot pattern (same as `SharedHighlighter`) avoids any
//! borrow-checker conflicts between the editor and the render pipeline.

use std::sync::{Arc, RwLock};

use ratatui::buffer::Buffer as ScreenBuf;
use ratatui::layout::Rect;
use ratatui::style::Style;

use hume_engine::providers::OverlayProvider;
use hume_engine::theme::Theme;
use hume_engine::types::Scope;

use super::menu_box::{MAX_MENU_ROWS, draw_menu_box, outer_dims};

// ── Public types ──────────────────────────────────────────────────────────────

/// Frame-stable snapshot of the completion popup content.
///
/// Computed from `Editor.completion` in `prepare_frame`; stored in an
/// `Arc<RwLock<_>>` shared with `CompletionOverlay`.
pub(crate) struct CompletionView {
    /// Candidate display strings (one per row, already sorted).
    pub rows: Vec<String>,
    /// Index of the currently-selected row.
    pub selected: usize,
    /// Absolute terminal column where the popup's left edge begins.
    /// Equals: `pad(1) + prompt_w(1) + display_width(input[..span_start])`.
    pub anchor_col: u16,
    /// Whether to draw box-drawing border characters around the popup.
    /// When `false`, a 1-cell bg-filled frame is still drawn on all sides;
    /// only the box-drawing glyphs are suppressed.
    pub border: bool,
}

/// Overlay that paints the completion popup on top of pane content.
pub(crate) struct CompletionOverlay {
    pub data: Arc<RwLock<Option<CompletionView>>>,
}

impl OverlayProvider for CompletionOverlay {
    fn is_active(&self) -> bool {
        self.data.read().expect("RwLock not poisoned").is_some()
    }

    fn render(&self, pane_area: Rect, theme: &Theme, buf: &mut ScreenBuf) {
        let guard = self.data.read().expect("RwLock not poisoned");
        let Some(view) = guard.as_ref() else { return };

        if view.rows.is_empty() {
            return;
        }

        let selected = view.selected.min(view.rows.len().saturating_sub(1));
        let (outer_w, outer_h) = outer_dims(&view.rows, MAX_MENU_ROWS);
        let outer_w = outer_w.min(pane_area.width);
        let outer_h = outer_h.min(pane_area.height);

        // Position: just above the statusline.
        // Shift left by 1 so the text column aligns under the token in the input.
        let popup_y = pane_area.y + pane_area.height - outer_h;
        let popup_x = view
            .anchor_col
            .saturating_sub(1)
            .min(pane_area.x + pane_area.width.saturating_sub(outer_w));

        let menu_style: Style = theme.resolve_by_name(Scope("ui.menu")).into();
        let selected_style: Style = theme.resolve_by_name(Scope("ui.menu.selected")).into();

        draw_menu_box(
            buf,
            Rect::new(popup_x, popup_y, outer_w, outer_h),
            &view.rows,
            Some(selected),
            view.border,
            menu_style,
            selected_style,
        );
    }
}
