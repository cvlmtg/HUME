//! Completion popup overlay — renders the Tab-completion candidate list above
//! the statusline while a completion session is active.
//!
//! The overlay reads a `MinibufCompletionView` snapshot from an `Arc<RwLock<_>>` that
//! `Editor` writes once per frame (in `prepare_frame`) before `EngineView::render`
//! is called.  The snapshot pattern (same as `ScopedHighlighter`) avoids any
//! borrow-checker conflicts between the editor and the render pipeline.

use hume_grid::Rect;
use std::sync::{Arc, RwLock};

use crate::lock_ext::LockExt;

use hume_engine::providers::OverlayProvider;
use hume_engine::render::Canvas;
use hume_engine::theme::Theme;

use super::menu_box::{MAX_MENU_ROWS, MenuBoxStyles, draw_menu_box, outer_dims};
use super::popup::{clamp_size_to_pane, clamp_x_to_pane};

// ── Public types ──────────────────────────────────────────────────────────────

/// Frame-stable snapshot of the completion popup content.
///
/// Computed from `Editor.minibuf_completion` in `prepare_frame`; stored in an
/// `Arc<RwLock<_>>` shared with `MinibufCompletionOverlay`.
pub(crate) struct MinibufCompletionView {
    /// Candidate display strings (one per row, already sorted).
    pub rows: Vec<String>,
    /// Index of the currently-selected row.
    pub selected: usize,
    /// Absolute terminal column where the popup's left edge begins — see
    /// `MiniBuffer::cursor_x_at` for the formula (computed at the
    /// completion span's start, not the edit cursor).
    pub anchor_x: u16,
    /// Whether to draw box-drawing border characters around the popup.
    /// When `false`, a 1-cell bg-filled frame is still drawn on all sides;
    /// only the box-drawing glyphs are suppressed.
    pub border: bool,
}

/// Overlay that paints the completion popup on top of pane content.
pub(crate) struct MinibufCompletionOverlay {
    pub data: Arc<RwLock<Option<MinibufCompletionView>>>,
}

impl OverlayProvider for MinibufCompletionOverlay {
    fn is_active(&self) -> bool {
        self.data.read_or_panic().is_some()
    }

    fn render(&self, pane_area: Rect, theme: &Theme, canvas: &mut Canvas) {
        let guard = self.data.read_or_panic();
        let Some(view) = guard.as_ref() else { return };

        if view.rows.is_empty() {
            return;
        }

        let selected = view.selected.min(view.rows.len().saturating_sub(1));
        let (outer_w, outer_h) = outer_dims(&view.rows, MAX_MENU_ROWS);
        let (outer_w, outer_h) = clamp_size_to_pane(outer_w, outer_h, pane_area);

        // Position: just above the statusline.
        // Shift left by 1 so the text column aligns under the token in the input.
        let popup_y = pane_area.bottom() - outer_h;
        let popup_x = clamp_x_to_pane(view.anchor_x.saturating_sub(1), outer_w, pane_area);
        draw_menu_box(
            canvas,
            Rect::new(popup_x, popup_y, outer_w, outer_h),
            &view.rows,
            Some(selected),
            0,
            view.border,
            MenuBoxStyles::resolve(theme, "ui.menu"),
            None,
        );
    }
}
