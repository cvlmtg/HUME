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
//! Scroll wheel events move both the viewport and all cursors by the configured
//! number of lines (Vim-style). Moving the cursor with the viewport prevents
//! `ensure_cursor_visible` from snapping the viewport back on the next frame.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use engine::format::{FormatScratch, count_visual_rows};
use engine::pane::WrapMode;

use crate::core::selection::{Selection, SelectionSet};
use crate::cursor;
use crate::ops::MotionMode;
use super::visual_move::{cmd_visual_move_down, cmd_visual_move_up};

use super::{Editor, Mode};


impl Editor {
    /// Dispatch a crossterm [`MouseEvent`] to the appropriate handler.
    pub(super) fn handle_mouse(&mut self, mouse: MouseEvent) {
        // Scratch view (`:messages`) is not interactive — ignore all mouse events.
        if self.scratch_view.is_some() {
            return;
        }
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
            self.set_current_selections(SelectionSet::single(sel));
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
            self.set_current_selections(SelectionSet::single(sel));
        }
    }

    // ── Scroll ────────────────────────────────────────────────────────────────

    fn mouse_scroll_up(&mut self) {
        let scroll_lines = self.settings.mouse_scroll_lines;
        let vp_before = {
            let vp = &self.engine_view.panes[self.pane_id].viewport;
            (vp.top_line, vp.top_row_offset)
        };
        {
            let pane = &mut self.engine_view.panes[self.pane_id];
            let rope = self.doc.text().rope();
            scroll_viewport_up(&mut pane.viewport, rope, &pane.wrap_mode, pane.tab_width, &pane.whitespace, scroll_lines, &mut self.motion_format_scratch);
        }
        let vp_after = {
            let vp = &self.engine_view.panes[self.pane_id].viewport;
            (vp.top_line, vp.top_row_offset)
        };
        // Only move cursors if the viewport actually moved (file may already be at top).
        if vp_before != vp_after {
            let _ = cmd_visual_move_up(self, scroll_lines, MotionMode::Move);
        }
    }

    fn mouse_scroll_down(&mut self) {
        let scroll_lines = self.settings.mouse_scroll_lines;
        let vp_before = {
            let vp = &self.engine_view.panes[self.pane_id].viewport;
            (vp.top_line, vp.top_row_offset)
        };
        {
            let pane = &mut self.engine_view.panes[self.pane_id];
            let rope = self.doc.text().rope();
            let total_lines = rope.len_lines();
            scroll_viewport_down(&mut pane.viewport, rope, &pane.wrap_mode, pane.tab_width, &pane.whitespace, total_lines, scroll_lines, &mut self.motion_format_scratch);
        }
        let vp_after = {
            let vp = &self.engine_view.panes[self.pane_id].viewport;
            (vp.top_line, vp.top_row_offset)
        };
        // Only move cursors if the viewport actually moved (file may fit entirely in the pane).
        if vp_before != vp_after {
            let _ = cmd_visual_move_down(self, scroll_lines, MotionMode::Move);
        }
    }

    // ── Coordinate conversion ─────────────────────────────────────────────────

    fn click_to_char(&mut self, col: u16, row: u16) -> Option<usize> {
        let pane = &self.engine_view.panes[self.pane_id];
        let gutter_w = cursor::gutter_width(pane.providers.gutter_columns(), self.doc.text().len_lines());
        let (vp, wrap_mode, tab_width, whitespace) = (
            pane.viewport.clone(),
            pane.wrap_mode.clone(),
            pane.tab_width,
            pane.whitespace.clone(),
        );
        let rope = self.doc.text().rope();
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
    scroll_lines: usize,
    scratch: &mut FormatScratch,
) {
    if wrap_mode.is_wrapping() {
        scratch.clear();
        // Decrement by `scroll_lines` display rows, respecting sub-row offsets.
        let mut rows_left = scroll_lines;
        while rows_left > 0 {
            if viewport.top_row_offset > 0 {
                let dec = rows_left.min(viewport.top_row_offset as usize);
                viewport.top_row_offset -= dec as u16;
                rows_left -= dec;
            } else if viewport.top_line > 0 {
                viewport.top_line -= 1;
                let rows = count_visual_rows(rope, viewport.top_line, tab_width, whitespace, wrap_mode, scratch);
                // Jump to the last sub-row of the new top line.
                let sub = rows.saturating_sub(1);
                viewport.top_row_offset = sub as u16;
                rows_left = rows_left.saturating_sub(1);
            } else {
                break;
            }
        }
    } else {
        viewport.top_line = viewport.top_line.saturating_sub(scroll_lines);
    }
}

#[allow(clippy::too_many_arguments)]
fn scroll_viewport_down(
    viewport: &mut engine::pane::ViewportState,
    rope: &ropey::Rope,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &engine::pane::WhitespaceConfig,
    total_lines: usize,
    scroll_lines: usize,
    scratch: &mut FormatScratch,
) {
    scratch.clear();
    // Content lines = all lines minus the structural trailing '\n' sentinel.
    let content_lines = total_lines.saturating_sub(1);
    let height = viewport.height as usize;
    if wrap_mode.is_wrapping() {
        // For wrapping, guard: if total display rows fit in the viewport, nothing to scroll.
        let mut total_rows = 0usize;
        for i in 0..content_lines {
            total_rows += count_visual_rows(rope, i, tab_width, whitespace, wrap_mode, scratch);
            if total_rows > height {
                break;
            }
        }
        if total_rows <= height {
            return;
        }

        // Maximum top_line is the last content line index.
        let last_line = content_lines.saturating_sub(1);
        let mut rows_left = scroll_lines;
        while rows_left > 0 {
            if viewport.top_line > last_line {
                break;
            }
            let rows = count_visual_rows(rope, viewport.top_line, tab_width, whitespace, wrap_mode, scratch);
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
        // Max top_line is the farthest position where the last content line is still visible.
        let max_top = content_lines.saturating_sub(height);
        viewport.top_line = (viewport.top_line + scroll_lines).min(max_top);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use engine::format::FormatScratch;
    use engine::pane::{ViewportState, WhitespaceConfig, WrapMode};
    use ropey::Rope;

    fn ws() -> WhitespaceConfig { WhitespaceConfig::default() }
    const SCROLL_LINES: usize = 3; // default from EditorSettings

    // Build a rope with `n` content lines (each "line\n"), plus the structural trailing '\n'.
    // total_lines() == n + 1 (ropey's phantom line).
    fn rope_with_lines(n: usize) -> Rope {
        let mut s = String::new();
        for i in 0..n { s.push_str(&format!("line{}\n", i)); }
        Rope::from_str(&s)
    }

    // ── scroll_viewport_down (no-wrap) ──────────────────────────────────────

    #[test]
    fn down_no_wrap_clamps_at_max_top() {
        // 10 content lines, viewport height 5 → max_top = 10 - 5 = 5.
        let rope = rope_with_lines(10);
        let total = rope.len_lines(); // 11 (10 content + phantom)
        let mut vp = ViewportState::new(80, 5);
        vp.top_line = 0;
        let mut scratch = FormatScratch::new();

        // Scroll far enough to hit the cap.
        for _ in 0..20 {
            scroll_viewport_down(&mut vp, &rope, &WrapMode::None, 4, &ws(), total, SCROLL_LINES, &mut scratch);
        }
        assert_eq!(vp.top_line, 5, "top_line must not exceed max_top=5");
    }

    #[test]
    fn down_no_wrap_file_fits_no_movement() {
        // 3 content lines, viewport height 10 → max_top = 0 → no movement.
        let rope = rope_with_lines(3);
        let total = rope.len_lines();
        let mut vp = ViewportState::new(80, 10);
        let mut scratch = FormatScratch::new();

        scroll_viewport_down(&mut vp, &rope, &WrapMode::None, 4, &ws(), total, SCROLL_LINES, &mut scratch);
        assert_eq!(vp.top_line, 0, "viewport must not move when file fits");
    }

    #[test]
    fn down_no_wrap_advances_by_scroll_lines() {
        let rope = rope_with_lines(20);
        let total = rope.len_lines();
        let mut vp = ViewportState::new(80, 5);
        let mut scratch = FormatScratch::new();

        scroll_viewport_down(&mut vp, &rope, &WrapMode::None, 4, &ws(), total, SCROLL_LINES, &mut scratch);
        assert_eq!(vp.top_line, SCROLL_LINES, "first scroll advances by SCROLL_LINES");
    }

    // ── scroll_viewport_up (no-wrap) ────────────────────────────────────────

    #[test]
    fn up_no_wrap_clamps_at_zero() {
        let rope = rope_with_lines(10);
        let mut vp = ViewportState::new(80, 5);
        vp.top_line = 1; // only 1 above top
        let mut scratch = FormatScratch::new();

        scroll_viewport_up(&mut vp, &rope, &WrapMode::None, 4, &ws(), SCROLL_LINES, &mut scratch);
        assert_eq!(vp.top_line, 0, "saturating_sub must not underflow");
    }

    #[test]
    fn up_no_wrap_decrements_by_scroll_lines() {
        let rope = rope_with_lines(20);
        let mut vp = ViewportState::new(80, 5);
        vp.top_line = 10;
        let mut scratch = FormatScratch::new();

        scroll_viewport_up(&mut vp, &rope, &WrapMode::None, 4, &ws(), SCROLL_LINES, &mut scratch);
        assert_eq!(vp.top_line, 10 - SCROLL_LINES);
    }

    #[test]
    fn up_at_top_is_no_op() {
        let rope = rope_with_lines(10);
        let mut vp = ViewportState::new(80, 5);
        vp.top_line = 0;
        let mut scratch = FormatScratch::new();

        scroll_viewport_up(&mut vp, &rope, &WrapMode::None, 4, &ws(), SCROLL_LINES, &mut scratch);
        assert_eq!(vp.top_line, 0);
        assert_eq!(vp.top_row_offset, 0);
    }

    // ── scroll_viewport_down (wrap) ─────────────────────────────────────────

    #[test]
    fn down_wrap_file_fits_no_movement() {
        // 2 short lines in a wide viewport → all rows fit → no scroll.
        let rope = rope_with_lines(2);
        let total = rope.len_lines();
        let mut vp = ViewportState::new(80, 10);
        let wrap = WrapMode::Soft { width: 80 };
        let mut scratch = FormatScratch::new();

        scroll_viewport_down(&mut vp, &rope, &wrap, 4, &ws(), total, SCROLL_LINES, &mut scratch);
        assert_eq!(vp.top_line, 0, "no scroll when file fits in viewport");
        assert_eq!(vp.top_row_offset, 0);
    }
}
