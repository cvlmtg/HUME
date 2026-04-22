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
use ratatui::style::{Color, Modifier, Style};

use engine::providers::OverlayProvider;
use engine::theme::Theme;

// ── Popup palette ─────────────────────────────────────────────────────────────

// TODO: replace with `theme.ui.menu.bg` / `theme.ui.menu.border` once the
// engine theme system exposes ui.menu / ui.menu.selected / ui.menu.border scopes.
const POPUP_BG: Color = Color::Rgb(40, 40, 50);
const BORDER_FG: Color = Color::Rgb(90, 90, 110);

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

    fn render(&self, pane_area: Rect, _theme: &Theme, buf: &mut ScreenBuf) {
        let guard = self.data.read().expect("RwLock not poisoned");
        let Some(view) = guard.as_ref() else { return };

        if view.rows.is_empty() { return; }

        let inner_rows = (view.rows.len() as u16).min(MAX_POPUP_ROWS);

        // Compute the visible slice (scroll window to keep `selected` visible).
        let selected = view.selected.min(view.rows.len().saturating_sub(1));
        let (scroll_offset, visible_rows) = visible_window(&view.rows, selected, inner_rows as usize);

        // Content width = widest candidate string.
        let inner_w = visible_rows.iter()
            .map(|r| unicode_display_width(r))
            .max()
            .unwrap_or(0) as u16;

        // Outer dimensions include a 1-cell frame on all sides.
        let outer_h = (inner_rows + 2).min(pane_area.height);
        let outer_w = (inner_w + 2).min(pane_area.width);

        // Need room for at least one border row on each side plus one content row.
        if outer_h < 3 || outer_w < 3 { return; }

        // Position: just above the statusline.
        // Shift left by 1 so the text column aligns under the token in the input.
        let popup_y = pane_area.y + pane_area.height - outer_h;
        let popup_x = view.anchor_col
            .saturating_sub(1)
            .min(pane_area.x + pane_area.width.saturating_sub(outer_w));

        let bg_style       = Style::default().bg(POPUP_BG);
        let border_style   = Style::default().fg(BORDER_FG).bg(POPUP_BG);
        let selected_style = Style::default().bg(POPUP_BG).add_modifier(Modifier::REVERSED);

        // 1. Fill the entire outer rectangle with the popup background.
        //    This gives a solid, opaque backdrop — no buffer content bleeds through.
        //    For border=false it also acts as the visible 1-cell margin.
        buf.set_style(Rect::new(popup_x, popup_y, outer_w, outer_h), bg_style);

        // 2. Optionally overdraw the 1-cell frame with box-drawing characters.
        if view.border {
            let right  = popup_x + outer_w - 1;
            let bottom = popup_y + outer_h - 1;
            // Number of ─ characters to fill between the two corner columns.
            let fill_w = (outer_w - 2) as usize;
            let horiz: String = "─".repeat(fill_w);

            // Top and bottom edges.
            buf.set_string(popup_x, popup_y, "┌", border_style);
            buf.set_string(popup_x + 1, popup_y, &horiz, border_style);
            buf.set_string(right, popup_y, "┐", border_style);
            buf.set_string(popup_x, bottom, "└", border_style);
            buf.set_string(popup_x + 1, bottom, &horiz, border_style);
            buf.set_string(right, bottom, "┘", border_style);

            // Left and right sides.
            for row in 1..outer_h - 1 {
                buf.set_string(popup_x, popup_y + row, "│", border_style);
                buf.set_string(right, popup_y + row, "│", border_style);
            }
        }

        // 3. Draw content rows inside the frame (y offset +1 for top border/padding).
        let text_x = popup_x + 1;
        for (i, row_text) in visible_rows.iter().enumerate() {
            let y = popup_y + 1 + i as u16;
            let row_idx = scroll_offset + i;

            if row_idx == selected {
                // Highlight the full inner width so the reversed bar is uniform.
                let inner_rect = Rect::new(text_x, y, outer_w.saturating_sub(2), 1);
                buf.set_style(inner_rect, selected_style);
                buf.set_string(text_x, y, row_text, selected_style);
            } else {
                buf.set_string(text_x, y, row_text, bg_style);
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Maximum number of visible popup rows (excluding the 1-cell frame).
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
