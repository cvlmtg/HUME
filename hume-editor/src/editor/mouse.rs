//! Mouse event handling.
//!
//! Mouse events are delivered when normal tracking (mode 1000) is enabled.
//! Button-event tracking (mode 1002) is only enabled when `editor.mouse_select`
//! is true, so `MouseEventKind::Drag` events are received only in that case.
//!
//! A click's `(column, row)` is terminal-absolute, but every pane's own
//! coordinate space starts at its rect's origin — a split puts more than one
//! pane on screen at once, so a click is first hit-tested against
//! `EngineView::pane_rects()` ([`Editor::pane_at_screen_pos`]) to find which
//! pane it landed in and to translate the coordinate into that pane's frame
//! before `screen_to_char_offset` (`editor/src/editor/cursor.rs`) resolves it
//! to a buffer char offset.
//!
//! Scroll wheel events move both the viewport and all cursors by the configured
//! number of lines (Vim-style). Moving the cursor with the viewport prevents
//! `ensure_cursor_visible` from snapping the viewport back on the next frame.

use hume_engine::pane::ViewportState;
use hume_engine::pipeline::PaneId;
use hume_engine::rows::RowMap;
use ratatui::layout::Position;
use termina::event::{MouseButton, MouseEvent, MouseEventKind};

use super::commands::pane_row_map_mut;
use super::cursor;
use super::scroll;
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
        // Hit-test before anything else: a miss (statusline, tabline, a
        // divider seam — anything outside a pane's own rect) is a no-op, and
        // a hit's pane-relative coordinates are what every step below needs.
        let Some((pid, rel_col, rel_row)) = self.pane_at_screen_pos(col, row) else {
            return;
        };

        // Move to Normal mode on click, regardless of current mode — BEFORE
        // resolving the click's char offset, and while the *previously*
        // focused pane is still current. `end_insert_session` can shrink
        // that pane's buffer (the blank-line indent trim), so computing
        // `click_to_char` first would resolve against a buffer length the
        // exit is about to invalidate: the offset could land past the new
        // end, or simply on the wrong char once positions shift.
        if self.state.mode() == Mode::Insert {
            self.end_insert_session();
        }

        // Click-to-focus: a click in another pane (a `:split`/`:vsplit`)
        // moves focus there, the same plain assignment `cmd_pane_focus_*`
        // uses — no jump-list push, matching those.
        self.state.focused_pane_id = pid;

        if let Some(char_off) = self.click_to_char(pid, rel_col, rel_row) {
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

        // A drag never moves focus mid-gesture — it extends the selection in
        // the pane the click that started it already focused. Hit-test only
        // that pane's own rect, so a drag that leaves it (as a fast mouse
        // move easily can) is ignored rather than resolving against the
        // wrong pane.
        let pid = self.state.focused_pane_id;
        let Some(rect) = self.view.pane_rect(pid) else {
            return;
        };
        if !rect.contains(Position::new(col, row)) {
            return;
        }

        if let Some(head) = self.click_to_char(pid, col - rect.x, row - rect.y) {
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
            let (mut rm, viewport) = pane_row_map_mut(
                self.state.buffers.get(buf_id),
                &self.state.settings,
                &mut self.view.panes[self.state.focused_pane_id],
                &mut self.state.motion_format_scratch,
            );
            scroll_viewport_up(viewport, &mut rm, scroll_lines);
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
            let (mut rm, viewport) = pane_row_map_mut(
                self.state.buffers.get(buf_id),
                &self.state.settings,
                &mut self.view.panes[self.state.focused_pane_id],
                &mut self.state.motion_format_scratch,
            );
            scroll_viewport_down(viewport, &mut rm, scroll_lines);
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

    /// Which pane `(col, row)` (terminal-absolute) falls in, and its
    /// position translated into that pane's own rect-relative coordinates —
    /// what `click_to_char` and `screen_to_char_offset` expect. `None` for a
    /// click outside every pane's rect (statusline, tabline, a divider seam).
    fn pane_at_screen_pos(&self, col: u16, row: u16) -> Option<(PaneId, u16, u16)> {
        let pos = Position::new(col, row);
        self.view
            .pane_rects()
            .into_iter()
            .find(|(_, rect)| rect.contains(pos))
            .map(|(pid, rect)| (pid, col - rect.x, row - rect.y))
    }

    /// Resolve a pane-relative `(col, row)` click in pane `pid` to a buffer
    /// char offset.
    fn click_to_char(&mut self, pid: PaneId, col: u16, row: u16) -> Option<usize> {
        let buf_id = self.view.panes[pid].buffer_id;
        let gutter_w = {
            let pane = &self.view.panes[pid];
            cursor::gutter_width(
                pane.providers.gutter_columns(),
                self.state.buffers.get(buf_id).text().len_lines(),
            )
        };
        let (mut rm, viewport) = pane_row_map_mut(
            self.state.buffers.get(buf_id),
            &self.state.settings,
            &mut self.view.panes[pid],
            &mut self.state.motion_format_scratch,
        );
        cursor::screen_to_char_offset(col, row, gutter_w, viewport, &mut rm)
    }
}

// ---------------------------------------------------------------------------
// Viewport scroll helpers (no cursor movement)
// ---------------------------------------------------------------------------

/// Scroll the viewport up by `rows` display rows, saturating at the top of the
/// document.
fn scroll_viewport_up(viewport: &mut ViewportState, rm: &mut RowMap<'_>, rows: usize) {
    let top = scroll::top_pos(viewport);
    scroll::set_top(viewport, rm.advance(top, -(rows as isize)));
}

/// Scroll the viewport down by `rows` display rows.
///
/// Saturates at the document's last display row rather than at "last line on
/// the last screen row": scrolling past EOF is allowed (the vim/Helix
/// convention `scroll_cursor_to_row` already follows), and an
/// `After(last_line)` virtual block would otherwise be permanently unreachable.
fn scroll_viewport_down(viewport: &mut ViewportState, rm: &mut RowMap<'_>, rows: usize) {
    // Nothing to scroll when the whole document — virtual rows included —
    // already fits on screen.
    if rm.fits_in(viewport.height) {
        return;
    }
    let top = scroll::top_pos(viewport);
    scroll::set_top(viewport, rm.advance(top, rows as isize));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
