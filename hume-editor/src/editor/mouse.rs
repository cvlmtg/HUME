//! Mouse event handling.
//!
//! Mouse events are delivered when normal tracking (mode 1000) is enabled.
//! Button-event tracking (mode 1002) is only enabled when `editor.mouse_select`
//! is true, so `MouseEventKind::Drag` events are received only in that case.
//!
//! Click-to-position converts the terminal-absolute `(column, row)` from the
//! mouse event into a buffer char offset via `screen_to_char_offset`
//! (`editor/src/editor/cursor.rs`).
//!
//! Scroll wheel events move both the viewport and all cursors by the configured
//! number of lines (Vim-style). Moving the cursor with the viewport prevents
//! `ensure_cursor_visible` from snapping the viewport back on the next frame.

use hume_engine::format::{FormatScratch, display_rows_for_line};
use hume_engine::pane::WrapMode;
use hume_engine::providers::ProviderSet;
use termina::event::{MouseButton, MouseEvent, MouseEventKind};

use super::cursor;
use super::visual_move::{VerticalUnit, apply_visual_vertical};
use crate::ops::MotionMode;
use hume_editing::selection::{Selection, SelectionSet};

use super::{Editor, Mode};

impl Editor {
    /// Dispatch a [`MouseEvent`] to the appropriate handler.
    ///
    /// Hook draining happens in the caller (`handle_event`) — this method only
    /// performs the dispatch.
    pub(super) fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.mouse_left_down(mouse.column, mouse.row)
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.mouse_left_drag(mouse.column, mouse.row)
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.state.mouse_drag_anchor = None;
            }
            MouseEventKind::ScrollUp => self.mouse_scroll_up(),
            MouseEventKind::ScrollDown => self.mouse_scroll_down(),
            _ => {}
        }
        // A click can exit Insert (`mouse_left_down`'s `end_insert_session`)
        // — dismiss a completion session synchronously, same as `handle_key`.
        self.take_pending_lsp_completion_dismiss();
    }

    // ── Click ─────────────────────────────────────────────────────────────────

    fn mouse_left_down(&mut self, col: u16, row: u16) {
        // Clicks in the statusline (last terminal row) are ignored.
        let vp_height = self.view.panes[self.state.focused_pane_id].viewport.height;
        if row >= vp_height {
            return;
        }

        // Move to Normal mode on click, regardless of current mode — BEFORE
        // resolving the click's char offset. `end_insert_session` can shrink
        // the buffer (the blank-line indent trim), so computing `click_to_char`
        // first would resolve against a buffer length the exit is about to
        // invalidate: the offset could land past the new end, or simply on
        // the wrong char once positions shift.
        if self.state.mode() == Mode::Insert {
            self.end_insert_session();
        }

        if let Some(char_off) = self.click_to_char(col, row) {
            // Collapse the primary selection to the clicked position.
            let sel = Selection::collapsed(char_off);
            self.set_current_selections(SelectionSet::single(sel));
            // Record anchor for potential drag-select.
            self.state.mouse_drag_anchor = Some(char_off);
            // Clear any pending key sequence so the click is a clean state reset.
            self.state.pending_keys.clear();
            self.state.count = None;
            self.state.status_msg = None;
        }
    }

    // ── Drag ──────────────────────────────────────────────────────────────────

    fn mouse_left_drag(&mut self, col: u16, row: u16) {
        // Drag events are only received when `mouse_select = true` (mode 1002).
        let Some(anchor) = self.state.mouse_drag_anchor else {
            return;
        };

        let vp_height = self.view.panes[self.state.focused_pane_id].viewport.height;
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
        let scroll_lines = self.state.settings.mouse_scroll_lines;
        let vp_before = {
            let vp = &self.view.panes[self.state.focused_pane_id].viewport;
            (vp.top_line, vp.top_row_offset)
        };
        {
            let buf_id = self.focused_buffer_id();
            let raw_wrap = self.focused_wrap_mode();
            let len_lines = self.state.buffers.get(buf_id).text().len_lines();
            let tab_width = self.doc().overrides.tab_width(&self.state.settings);
            let whitespace = self.doc().overrides.whitespace(&self.state.settings);
            let rope = self.state.buffers.get(buf_id).text().rope();
            let content_width = {
                let pane = &self.view.panes[self.state.focused_pane_id];
                pane.content_width(len_lines)
            };
            let wrap_mode = raw_wrap.resolve(content_width);
            let pane = &mut self.view.panes[self.state.focused_pane_id];
            scroll_viewport_up(
                &mut pane.viewport,
                rope,
                &wrap_mode,
                tab_width,
                &whitespace,
                scroll_lines,
                &pane.providers,
                content_width,
                &mut self.state.motion_format_scratch,
            );
        }
        let vp_after = {
            let vp = &self.view.panes[self.state.focused_pane_id].viewport;
            (vp.top_line, vp.top_row_offset)
        };
        // Only move cursors if the viewport actually moved (file may already be at top).
        if vp_before != vp_after {
            apply_visual_vertical(
                &mut self.state,
                &mut self.view,
                scroll_lines,
                false,
                MotionMode::Move,
                VerticalUnit::ScreenRow,
            );
        }
    }

    fn mouse_scroll_down(&mut self) {
        let scroll_lines = self.state.settings.mouse_scroll_lines;
        let vp_before = {
            let vp = &self.view.panes[self.state.focused_pane_id].viewport;
            (vp.top_line, vp.top_row_offset)
        };
        {
            let buf_id = self.focused_buffer_id();
            let raw_wrap = self.focused_wrap_mode();
            let tab_width = self.doc().overrides.tab_width(&self.state.settings);
            let whitespace = self.doc().overrides.whitespace(&self.state.settings);
            let rope = self.state.buffers.get(buf_id).text().rope();
            let total_lines = rope.len_lines();
            let content_width = {
                let pane = &self.view.panes[self.state.focused_pane_id];
                pane.content_width(total_lines)
            };
            let wrap_mode = raw_wrap.resolve(content_width);
            let pane = &mut self.view.panes[self.state.focused_pane_id];
            scroll_viewport_down(
                &mut pane.viewport,
                rope,
                &wrap_mode,
                tab_width,
                &whitespace,
                total_lines,
                scroll_lines,
                &pane.providers,
                content_width,
                &mut self.state.motion_format_scratch,
            );
        }
        let vp_after = {
            let vp = &self.view.panes[self.state.focused_pane_id].viewport;
            (vp.top_line, vp.top_row_offset)
        };
        // Only move cursors if the viewport actually moved (file may fit entirely in the pane).
        if vp_before != vp_after {
            apply_visual_vertical(
                &mut self.state,
                &mut self.view,
                scroll_lines,
                true,
                MotionMode::Move,
                VerticalUnit::ScreenRow,
            );
        }
    }

    // ── Coordinate conversion ─────────────────────────────────────────────────

    fn click_to_char(&mut self, col: u16, row: u16) -> Option<usize> {
        let buf_id = self.focused_buffer_id();
        let (vp, gutter_w) = {
            let pane = &self.view.panes[self.state.focused_pane_id];
            let gw = cursor::gutter_width(
                pane.providers.gutter_columns(),
                self.state.buffers.get(buf_id).text().len_lines(),
            );
            (pane.viewport.clone(), gw)
        };
        let content_width = vp.width.saturating_sub(gutter_w).max(1);
        let wrap_mode = self.focused_wrap_mode().resolve(content_width);
        let tab_width = self.doc().overrides.tab_width(&self.state.settings);
        let whitespace = self.doc().overrides.whitespace(&self.state.settings);
        let rope = self.state.buffers.get(buf_id).text().rope();
        cursor::screen_to_char_offset(
            col,
            row,
            gutter_w,
            &vp,
            rope,
            &wrap_mode,
            tab_width,
            &whitespace,
            &mut self.state.motion_format_scratch,
            &self.view.panes[self.state.focused_pane_id].providers,
            content_width,
        )
    }
}

// ---------------------------------------------------------------------------
// Viewport scroll helpers (no cursor movement)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
/// Decrement by `scroll_lines` display rows — one row of `top_line`'s whole
/// visual block (`before` + content + `after`) per unit, same invariant as
/// `top_row_offset` itself (see `ViewportState`'s doc). Wrap-mode-agnostic:
/// `display_rows_for_line` returns `content: 1` for `WrapMode::None`.
fn scroll_viewport_up(
    viewport: &mut hume_engine::pane::ViewportState,
    rope: &ropey::Rope,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &hume_engine::pane::WhitespaceConfig,
    scroll_lines: usize,
    providers: &ProviderSet,
    content_width: u16,
    scratch: &mut FormatScratch,
) {
    let mut rows_left = scroll_lines;
    while rows_left > 0 {
        if viewport.top_row_offset > 0 {
            let dec = rows_left.min(viewport.top_row_offset as usize);
            viewport.top_row_offset -= dec as u16;
            rows_left -= dec;
        } else if viewport.top_line > 0 {
            viewport.top_line -= 1;
            let rows = display_rows_for_line(
                rope,
                viewport.top_line,
                tab_width,
                whitespace,
                wrap_mode,
                providers,
                content_width,
                scratch,
            )
            .total();
            // Jump to the last row of the new top line's block.
            let sub = rows.saturating_sub(1);
            viewport.top_row_offset = sub as u16;
            rows_left = rows_left.saturating_sub(1);
        } else {
            break;
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// Wrap-mode-agnostic mirror of `scroll_viewport_up` — see its doc.
fn scroll_viewport_down(
    viewport: &mut hume_engine::pane::ViewportState,
    rope: &ropey::Rope,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &hume_engine::pane::WhitespaceConfig,
    total_lines: usize,
    scroll_lines: usize,
    providers: &ProviderSet,
    content_width: u16,
    scratch: &mut FormatScratch,
) {
    // Content lines = all lines minus the structural trailing '\n' sentinel.
    let content_lines = total_lines.saturating_sub(1);
    let height = viewport.height as usize;

    // If everything (real content + any virtual rows) already fits in the
    // viewport, there's nothing to scroll.
    let mut total_rows = 0usize;
    for i in 0..content_lines {
        total_rows += display_rows_for_line(
            rope,
            i,
            tab_width,
            whitespace,
            wrap_mode,
            providers,
            content_width,
            scratch,
        )
        .total();
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
        let rows = display_rows_for_line(
            rope,
            viewport.top_line,
            tab_width,
            whitespace,
            wrap_mode,
            providers,
            content_width,
            scratch,
        )
        .total();
        let remaining_in_line = rows.saturating_sub(1 + viewport.top_row_offset as usize);
        if rows_left <= remaining_in_line {
            viewport.top_row_offset += rows_left as u16;
            break;
        }
        if viewport.top_line < last_line {
            // Consume the rest of this line, advance to the next.
            rows_left -= remaining_in_line + 1;
            viewport.top_row_offset = 0;
            viewport.top_line += 1;
        } else {
            // Already the last line — nothing further to scroll to. Clamp
            // to its final row (e.g. the last `After` row) instead of
            // resetting to 0, which would snap back to the top of this
            // line's block and undo whatever forward progress this notch
            // already made.
            viewport.top_row_offset = rows.saturating_sub(1) as u16;
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
