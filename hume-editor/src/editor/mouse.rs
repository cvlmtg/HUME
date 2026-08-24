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
use ratatui::layout::{Position, Rect};
use termina::event::{MouseButton, MouseEvent, MouseEventKind};

use super::commands::pane_row_map_mut;
use super::cursor;
use super::scroll;
use super::visual_move::{VerticalUnit, apply_visual_vertical};
use hume_editing::selection::{Selection, SelectionSet};
use hume_ops::MotionMode;

use super::{Editor, Mode};

impl Editor {
    /// Dispatch a [`MouseEvent`] to the appropriate handler.
    ///
    /// Hook draining happens in the caller (`handle_input`) — this method only
    /// performs the dispatch.
    pub(super) fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // column-name-safe: termina's MouseEvent::column is a terminal-absolute x
                self.mouse_left_down(mouse.column, mouse.row)
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                // column-name-safe: termina's MouseEvent::column is a terminal-absolute x
                self.mouse_left_drag(mouse.column, mouse.row)
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.state.mouse_drag_anchor = None;
            }
            MouseEventKind::ScrollUp => self.mouse_scroll(false),
            MouseEventKind::ScrollDown => self.mouse_scroll(true),
            _ => {}
        }
        // A click can exit Insert (`mouse_left_down`'s `end_insert_session`)
        // — dismiss a completion session synchronously, same as `handle_key`.
        self.take_pending_lsp_completion_dismiss();
    }

    // ── Click ─────────────────────────────────────────────────────────────────

    fn mouse_left_down(&mut self, x: u16, y: u16) {
        // Hit-test before anything else: a miss (statusline, tabline, a
        // divider seam — anything outside a pane's own rect) is a no-op, and
        // a hit's pane-relative coordinates are what every step below needs.
        let Some((pid, pane_x, pane_y)) = self.pane_at_screen_pos(x, y) else {
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

        if let Some(char_off) = self.click_to_char(pid, pane_x, pane_y) {
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

    fn mouse_left_drag(&mut self, x: u16, y: u16) {
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
        let Some((pane_x, pane_y)) = self
            .view
            .pane_rect(pid)
            .and_then(|rect| rect_relative(rect, x, y))
        else {
            return;
        };

        if let Some(head) = self.click_to_char(pid, pane_x, pane_y) {
            let sel = Selection::new(anchor, head);
            self.set_current_selections(SelectionSet::single(sel));
        }
    }

    // ── Scroll ────────────────────────────────────────────────────────────────

    fn mouse_scroll(&mut self, down: bool) {
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
            if down {
                scroll_viewport_down(viewport, &mut rm, scroll_lines);
            } else {
                scroll_viewport_up(viewport, &mut rm, scroll_lines);
            }
        }
        let vp_after = {
            let vp = &self.view.panes[self.state.focused_pane_id].viewport;
            (vp.top_line, vp.top_row_offset)
        };
        // Only move cursors if the viewport actually moved (file may already be
        // at the top, or may fit entirely in the pane).
        if vp_before != vp_after {
            apply_visual_vertical(
                &mut self.state,
                &mut self.view,
                scroll_lines,
                down,
                MotionMode::Move,
                VerticalUnit::ScreenRow,
            );
        }
    }

    // ── Coordinate conversion ─────────────────────────────────────────────────

    /// Which pane `(x, y)` (terminal-absolute) falls in, and its
    /// position translated into that pane's own rect-relative coordinates —
    /// what `click_to_char` and `screen_to_char_offset` expect. `None` for a
    /// click outside every pane's rect (statusline, tabline, a divider seam).
    fn pane_at_screen_pos(&self, x: u16, y: u16) -> Option<(PaneId, u16, u16)> {
        let (pid, rect) = self.view.layout.find_containing(
            Position::new(x, y),
            self.view.last_pane_area,
            self.view.reserve_seam,
        )?;
        let (pane_x, pane_y) = rect_relative(rect, x, y)?;
        Some((pid, pane_x, pane_y))
    }

    /// Resolve a pane-relative `(x, y)` click in pane `pid` to a buffer
    /// char offset.
    fn click_to_char(&mut self, pid: PaneId, x: u16, y: u16) -> Option<usize> {
        let buf_id = self.view.panes[pid].buffer_id;
        let gutter_w = {
            let pane = &self.view.panes[pid];
            cursor::gutter_width(
                pane.providers.gutter_columns(),
                self.state.buffers.get(buf_id).text().last_ropey_line(),
            )
        };
        let (mut rm, viewport) = pane_row_map_mut(
            self.state.buffers.get(buf_id),
            &self.state.settings,
            &mut self.view.panes[pid],
            &mut self.state.motion_format_scratch,
        );
        cursor::screen_to_char_offset(x, y, gutter_w, viewport, &mut rm)
    }
}

// ---------------------------------------------------------------------------
// Coordinate helpers
// ---------------------------------------------------------------------------

/// Translate terminal-absolute `(x, y)` into `rect`'s own frame — the space
/// `click_to_char` and `screen_to_char_offset` expect — or `None` when the
/// position falls outside `rect`. The `contains` guard is what keeps the
/// `u16` subtraction from underflowing on a position left of/above the rect
/// origin.
fn rect_relative(rect: Rect, x: u16, y: u16) -> Option<(u16, u16)> {
    rect.contains(Position::new(x, y))
        .then(|| (x - rect.x, y - rect.y))
}

/// Translate a pane-content-relative `(content_x, row)` cell — relative to
/// the pane's content area, past the `gutter_w`-wide gutter — into an
/// absolute terminal cell. The inverse of [`rect_relative`], for the two
/// call sites (the popup/menu anchor, the Insert-mode bar cursor) that need
/// to go the other way: a content-relative position `pane_row_map`'s cursor
/// walk already resolved, placed onto the screen.
pub(super) fn content_pos_to_screen(
    content_x: u16,
    row: u16,
    gutter_w: u16,
    pane_rect: Rect,
) -> (u16, u16) {
    (content_x + gutter_w + pane_rect.x, row + pane_rect.y)
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
