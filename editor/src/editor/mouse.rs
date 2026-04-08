//! Mouse event handling.
//!
//! Crossterm delivers mouse events when normal tracking (mode 1000) is enabled.
//! Button-event tracking (mode 1002) is only enabled when `editor.mouse_select`
//! is true, so `MouseEventKind::Drag` events are received only in that case.
//!
//! Click-to-position uses [`crate::cursor::screen_to_char_offset`] to convert
//! the terminal-absolute `(column, row)` from the mouse event into a buffer
//! char offset.
//!
//! Scroll wheel events adjust `viewport.top_line` directly, without moving
//! the cursor. The scroll amount is [`SCROLL_LINES`] buffer lines per notch.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use engine::format::{FormatScratch, count_visual_rows};
use engine::pane::WrapMode;

use crate::core::selection::{Selection, SelectionSet};
use crate::cursor;

use super::{Editor, Mode};


/// Number of buffer lines (no-wrap) or display rows (wrap) to scroll per
/// scroll-wheel notch.
const SCROLL_LINES: usize = 3;

impl Editor {
    /// Dispatch a crossterm [`MouseEvent`] to the appropriate handler.
    pub(super) fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => self.mouse_left_down(mouse.column, mouse.row),
            MouseEventKind::Drag(MouseButton::Left) => self.mouse_left_drag(mouse.column, mouse.row),
            MouseEventKind::Up(MouseButton::Left)   => { self.mouse_drag_anchor = None; }
            MouseEventKind::ScrollUp                => self.mouse_scroll_up(),
            MouseEventKind::ScrollDown              => self.mouse_scroll_down(),
            _ => {}
        }
    }

    // ── Click ─────────────────────────────────────────────────────────────────

    fn mouse_left_down(&mut self, col: u16, row: u16) {
        // Clicks in the statusline (last terminal row) are ignored.
        let vp_height = self.engine_view.panes[self.pane_id].viewport.height;
        if row >= vp_height {
            return;
        }

        if let Some(char_off) = self.click_to_char(col, row) {
            // Move to Normal mode on click, regardless of current mode.
            if self.mode == Mode::Insert {
                self.end_insert_session();
                self.set_mode(Mode::Normal);
            }
            // Collapse the primary selection to the clicked position.
            let sel = Selection::collapsed(char_off);
            self.doc.set_selections(SelectionSet::single(sel));
            // Record anchor for potential drag-select.
            self.mouse_drag_anchor = Some(char_off);
            // Clear any pending key sequence so the click is a clean state reset.
            self.pending_keys.clear();
            self.count = None;
            self.status_msg = None;
        }
    }

    // ── Drag ──────────────────────────────────────────────────────────────────

    fn mouse_left_drag(&mut self, col: u16, row: u16) {
        // Drag events are only received when `mouse_select = true` (mode 1002).
        let Some(anchor) = self.mouse_drag_anchor else { return };

        let vp_height = self.engine_view.panes[self.pane_id].viewport.height;
        if row >= vp_height {
            return;
        }

        if let Some(head) = self.click_to_char(col, row) {
            let sel = Selection::new(anchor, head);
            self.doc.set_selections(SelectionSet::single(sel));
        }
    }

    // ── Scroll ────────────────────────────────────────────────────────────────

    fn mouse_scroll_up(&mut self) {
        let pane = &mut self.engine_view.panes[self.pane_id];
        let rope = self.doc.buf().rope();
        scroll_viewport_up(&mut pane.viewport, rope, &pane.wrap_mode, pane.tab_width, &pane.whitespace);
    }

    fn mouse_scroll_down(&mut self) {
        let pane = &mut self.engine_view.panes[self.pane_id];
        let rope = self.doc.buf().rope();
        let total_lines = rope.len_lines();
        scroll_viewport_down(&mut pane.viewport, rope, &pane.wrap_mode, pane.tab_width, &pane.whitespace, total_lines);
    }

    // ── Coordinate conversion ─────────────────────────────────────────────────

    fn click_to_char(&mut self, col: u16, row: u16) -> Option<usize> {
        let pane = &self.engine_view.panes[self.pane_id];
        let gutter_w = cursor::gutter_width(
            &pane.viewport,
            pane.providers.gutter_columns(),
            self.doc.buf().len_lines(),
        );
        let (vp, wrap_mode, tab_width, whitespace) = (
            pane.viewport.clone(),
            pane.wrap_mode.clone(),
            pane.tab_width,
            pane.whitespace.clone(),
        );
        let rope = self.doc.buf().rope();
        cursor::screen_to_char_offset(
            col,
            row,
            gutter_w,
            &vp,
            rope,
            &wrap_mode,
            tab_width,
            &whitespace,
            &mut self.motion_format_scratch,
        )
    }
}

// ---------------------------------------------------------------------------
// Viewport scroll helpers (no cursor movement)
// ---------------------------------------------------------------------------

fn scroll_viewport_up(
    viewport: &mut engine::pane::ViewportState,
    rope: &ropey::Rope,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &engine::pane::WhitespaceConfig,
) {
    let mut scratch = FormatScratch::new();
    if wrap_mode.is_wrapping() {
        // Decrement by SCROLL_LINES display rows, respecting sub-row offsets.
        let mut rows_left = SCROLL_LINES;
        while rows_left > 0 {
            if viewport.top_row_offset > 0 {
                let dec = rows_left.min(viewport.top_row_offset as usize);
                viewport.top_row_offset -= dec as u16;
                rows_left -= dec;
            } else if viewport.top_line > 0 {
                viewport.top_line -= 1;
                let rows = count_visual_rows(rope, viewport.top_line, tab_width, whitespace, wrap_mode, &mut scratch);
                // Jump to the last sub-row of the new top line.
                let sub = rows.saturating_sub(1);
                viewport.top_row_offset = sub as u16;
                rows_left = rows_left.saturating_sub(1);
            } else {
                break;
            }
        }
    } else {
        viewport.top_line = viewport.top_line.saturating_sub(SCROLL_LINES);
    }
}

fn scroll_viewport_down(
    viewport: &mut engine::pane::ViewportState,
    rope: &ropey::Rope,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &engine::pane::WhitespaceConfig,
    total_lines: usize,
) {
    // Do not scroll past the last real line (total_lines - 2 for the sentinel '\n').
    let last_line = total_lines.saturating_sub(2);

    let mut scratch = FormatScratch::new();
    if wrap_mode.is_wrapping() {
        let mut rows_left = SCROLL_LINES;
        while rows_left > 0 {
            if viewport.top_line > last_line {
                break;
            }
            let rows = count_visual_rows(rope, viewport.top_line, tab_width, whitespace, wrap_mode, &mut scratch);
            let remaining_in_line = rows.saturating_sub(1 + viewport.top_row_offset as usize);
            if rows_left <= remaining_in_line {
                viewport.top_row_offset += rows_left as u16;
                break;
            }
            // Consume the rest of this line.
            rows_left -= remaining_in_line + 1;
            viewport.top_row_offset = 0;
            if viewport.top_line < last_line {
                viewport.top_line += 1;
            } else {
                break;
            }
        }
    } else {
        viewport.top_line = (viewport.top_line + SCROLL_LINES).min(last_line);
    }
}
