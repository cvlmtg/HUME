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
use ratatui::style::{Modifier, Style};

use engine::providers::OverlayProvider;
use engine::theme::Theme;

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
}

/// Overlay that paints the completion popup on top of pane content.
pub(crate) struct CompletionOverlay {
    pub data: Arc<RwLock<Option<CompletionView>>>,
}

impl OverlayProvider for CompletionOverlay {
    fn is_active(&self) -> bool {
        self.data.read().expect("RwLock not poisoned").is_some()
    }

    fn render(&self, pane_area: Rect, _theme: &Theme, buf: &mut ScreenBuf) {
        let guard = self.data.read().expect("RwLock not poisoned");
        let Some(view) = guard.as_ref() else { return };

        if view.rows.is_empty() { return; }

        let popup_height = (view.rows.len() as u16).min(MAX_POPUP_ROWS).min(pane_area.height);
        if popup_height == 0 { return; }

        // Compute the visible slice (scroll window to keep `selected` visible).
        let selected = view.selected.min(view.rows.len().saturating_sub(1));
        let max_height = popup_height as usize;
        let (scroll_offset, visible_rows) = visible_window(&view.rows, selected, max_height);

        // Width: widest display string + 2 padding columns, clipped to screen.
        let content_w = visible_rows.iter()
            .map(|r| unicode_display_width(r))
            .max()
            .unwrap_or(0) as u16;
        let popup_w = (content_w + 2).min(pane_area.width);
        if popup_w == 0 { return; }

        // Position: just above the statusline (which sits at pane_area.bottom()).
        // Clip x so the popup doesn't overflow the right edge.
        let popup_y = pane_area.y + pane_area.height - popup_height;
        let popup_x = view.anchor_col.min(pane_area.x + pane_area.width.saturating_sub(popup_w));

        let plain    = Style::default();
        let selected_style = Style::default().add_modifier(Modifier::REVERSED);

        for (i, row_text) in visible_rows.iter().enumerate() {
            let y = popup_y + i as u16;
            let row_idx = scroll_offset + i;
            let style = if row_idx == selected { selected_style } else { plain };

            // Paint the full popup-width background first.
            let area = Rect::new(popup_x, y, popup_w, 1);
            buf.set_style(area, style);
            // Then write the text (with 1-column left padding).
            buf.set_string(popup_x + 1, y, row_text, style);
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Maximum number of visible popup rows.
const MAX_POPUP_ROWS: u16 = 10;

/// Return `(scroll_offset, visible_slice)` such that `selected` is inside
/// the visible window of `max_height` entries.
fn visible_window(rows: &[String], selected: usize, max_height: usize) -> (usize, &[String]) {
    let total = rows.len();
    if total <= max_height {
        return (0, rows);
    }
    // Keep `selected` visible by anchoring the window.
    let start = selected.saturating_sub(max_height / 2)
        .min(total - max_height);
    (start, &rows[start..start + max_height])
}

/// Unicode display width (number of terminal columns) of a string.
fn unicode_display_width(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}
