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
//! before `screen_to_char_offset` (`hume-editor/src/editor/cursor.rs`) resolves it
//! to a buffer char offset.
//!
//! Scroll wheel events are `commands::scroll_view` with `count =
//! mouse-scroll-lines` — the same viewport-plus-cursor scroll `Ctrl-d`/`Ctrl-u`
//! and `PageDown`/`PageUp` use, just with a smaller count. Unlike those
//! keyboard commands (always the focused pane), the wheel hit-tests its own
//! event coordinates through `pane_at_screen_pos` to scroll whichever pane
//! the pointer is over, falling back to the focused pane on a miss — but,
//! unlike a click, never moves focus there.
//!
//! Every mouse event dismisses an open `Scrollable` popup first — matching
//! `handle_key`'s any-key dismissal (`editor/mappings/mod.rs`). Past that
//! pre-step, [`Editor::handle_mouse`] walks the input-layer stack exactly
//! like a key or a paste (`InputEvent::Mouse` — `input_stack.rs`);
//! [`Editor::base_mouse`] is `Base`'s own policy, the `match mouse.kind`
//! this module used to dispatch unconditionally before the stack could gate
//! it.

use hume_engine::pipeline::PaneId;
use hume_grid::{Position, Rect};
use termina::event::{MouseButton, MouseEvent, MouseEventKind};

use super::commands::{self, pane_display_lines};
use super::cursor;
use super::input_stack::InputEvent;
use hume_editing::selection::{Selection, SelectionSet};
use hume_ops::MotionMode;

use super::Editor;

/// Whether `kind` is a *fresh* user action rather than the tail of a gesture
/// that began before the current layer existed — a press or a wheel notch is
/// the user acting now; a release, a drag, or a pointer move belongs to a
/// gesture already in flight. A layer that retires on stray input (`Confirm`,
/// `Menu`) must not retire on the latter: the click that opens the
/// disk-change confirm (click-to-focus → `OnBufferEnter` → the disk check at
/// the next `settle()`) sends its own `Up` one loop iteration afterwards,
/// which would dismiss the prompt before it was ever answered.
pub(super) fn is_fresh_gesture(kind: MouseEventKind) -> bool {
    matches!(
        kind,
        MouseEventKind::Down(_) | MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
    )
}

impl Editor {
    /// Dispatch a [`MouseEvent`] into the input-layer stack.
    ///
    /// Hook draining happens in the caller (`handle_input`) — this method only
    /// performs the dispatch.
    pub(super) fn handle_mouse(&mut self, mouse: MouseEvent) {
        // Any mouse event dismisses a scrollable popup, same as any key —
        // see `ConfigState::dismiss_scrollable_popup`. Before the dispatch
        // below, so the event still performs its own action (a wheel notch
        // still scrolls, a click still moves the cursor). Stays a pre-step
        // here (mirroring `handle_key`'s own) until step 6 folds the popup
        // into the stack.
        self.state.config.dismiss_scrollable_popup();
        self.dispatch_input(InputEvent::Mouse(mouse));
    }

    /// The `Base` layer's own mouse policy — routed here by `dispatch_at`
    /// once every overlay above it has had a chance to swallow or fall
    /// through the event.
    pub(super) fn base_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // termina's MouseEvent::column is a terminal-absolute x
                self.mouse_left_down(mouse.column, mouse.row)
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                // termina's MouseEvent::column is a terminal-absolute x
                self.mouse_left_drag(mouse.column, mouse.row)
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.state.mouse_drag_anchor = None;
            }
            MouseEventKind::ScrollUp => self.mouse_scroll(mouse.column, mouse.row, false),
            MouseEventKind::ScrollDown => self.mouse_scroll(mouse.column, mouse.row, true),
            _ => {}
        }
    }

    // ── Click ─────────────────────────────────────────────────────────────────

    fn mouse_left_down(&mut self, x: u16, y: u16) {
        // A tabline click is handled separately (and unconditionally, even
        // when it doesn't land on an actual tab) — it's outside every
        // pane's rect, so `pane_at_screen_pos` below would just treat it as
        // a miss anyway, but routing it first avoids relying on that.
        if self.tabline_click(x, y) {
            return;
        }

        // Hit-test before anything else: a miss (statusline, a divider seam
        // — anything outside a pane's own rect) is a no-op, and a hit's
        // pane-relative coordinates are what every step below needs.
        let Some((pid, pane_x, pane_y)) = self.pane_at_screen_pos(x, y) else {
            return;
        };

        // Click-to-focus: a click in another pane (a `:split`/`:vsplit`)
        // moves focus there, the same `focus_pane` chokepoint
        // `cmd_pane_focus_*` uses — no jump-list push, matching those.
        // `focus_pane` exits Insert (if active) BEFORE resolving the click's
        // char offset below, and while the *previously* focused pane is
        // still current — `end_insert_session` can shrink that pane's
        // buffer (the blank-line indent trim), so computing `click_to_char`
        // first would resolve against a buffer length the exit is about to
        // invalidate: the offset could land past the new end, or simply on
        // the wrong char once positions shift.
        super::focus::focus_pane(&mut self.state, &self.view, pid);

        if let Some(char_off) = self.click_to_char(pid, pane_x, pane_y) {
            // Collapse the primary selection to the clicked position.
            let sel = Selection::collapsed(char_off);
            self.set_current_selections(SelectionSet::single(sel));
            self.clear_pending_input();
            // Record anchor for potential drag-select.
            self.state.mouse_drag_anchor = Some(char_off);
        }
    }

    /// Reset transient input state every click starts fresh from: any
    /// half-typed key sequence (`pending_keys`/`count`) and the status line.
    /// Shared by `mouse_left_down`'s pane path and `tabline_click` — both
    /// are "the user just clicked somewhere new". Leaves `mouse_drag_anchor`
    /// alone — `mouse_left_down` sets its own right after calling this, and
    /// `tabline_click` clears it explicitly, since a tab switch is exactly
    /// the case where a stale anchor would extend a drag against a buffer
    /// that isn't even focused anymore.
    fn clear_pending_input(&mut self) {
        self.state.pending_keys.clear();
        self.state.count = None;
        self.state.status_msg = None;
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
        let pid = self.state.focus.id();
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

    fn mouse_scroll(&mut self, x: u16, y: u16, down: bool) {
        // Unlike a click, a wheel notch never moves focus — `focus_pane`
        // exits Insert mode, and a stray notch over another pane must not be
        // able to do that. A miss (statusline, tabline, a divider seam) has
        // no pane to prefer over the focused one, so it falls back there
        // instead of being a no-op like a click's own miss.
        let pid = self
            .pane_at_screen_pos(x, y)
            .map_or(self.state.focus.id(), |(pid, _, _)| pid);
        let scroll_lines = self.state.settings.mouse_scroll_lines;
        commands::scroll_view(
            &mut self.state,
            &mut self.view,
            pid,
            scroll_lines,
            down,
            MotionMode::Move,
        );
    }

    // ── Coordinate conversion ─────────────────────────────────────────────────

    /// Handle a click at terminal-absolute `(x, y)` landing in the tab
    /// bar's row, if it does. Returns `true` when it was — whether or not
    /// it landed on an actual tab, so a click on the row's blank tail is a
    /// no-op but still doesn't fall through to `pane_at_screen_pos` (which
    /// would just miss anyway, since the tabline sits outside every pane's
    /// rect). Uses the same `tab_extents` layout `TablineWidget::render`
    /// paints from, so a click always lands on the tab it visually appears
    /// to.
    ///
    /// Hit-tests against `EngineView::tabbar_area` rather than
    /// `last_pane_area.y`: `pane_area`'s degenerate branch (terminal too
    /// small to fit chrome + content) leaves `last_pane_area.y` at the
    /// terminal's own `y`, which would make every row look like the tab bar
    /// — `tabbar_area` is the one rect that always reports where the bar
    /// itself is, whether or not there's room left for panes.
    fn tabline_click(&mut self, x: u16, y: u16) -> bool {
        let bar = self.view.tabbar_area(self.view.last_terminal_area);
        if !bar.contains(Position::new(x, y)) {
            return false;
        }
        // No separate `guard.visible` check needed: `bar.height` (above) came
        // from `TabBarProvider::height()`, which reads that same flag off
        // this same shared slot — a nonzero height already means it was
        // `true` moments ago, and nothing mutates the view between then and
        // this read.
        let guard = self.state.tabline_view.read();
        let extents = crate::tabline::tab_extents(&guard.tabs, guard.scroll, bar.x, bar.width);
        let target = extents
            .ranges
            .iter()
            .position(|&(start, end)| x >= start && x < end)
            .map(|i| guard.tabs[guard.scroll + i].id);
        drop(guard);
        if let Some(id) = target {
            // `switch_to_tab` -> `install_live` -> `focus_pane` exits Insert
            // (if active) against the outgoing tab's own pane, still focused
            // at this point, before moving focus to the new tab's pane — see
            // `focus_pane`'s own doc for why that order matters.
            self.clear_pending_input();
            self.state.mouse_drag_anchor = None;
            crate::editor::tab::switch_to_tab(&mut self.state, &mut self.view, id);
        }
        true
    }

    /// Which pane `(x, y)` (terminal-absolute) falls in, and its
    /// position translated into that pane's own rect-relative coordinates —
    /// what `click_to_char` and `screen_to_char_offset` expect. `None` for a
    /// click outside every pane's rect (statusline, tabline, a divider seam).
    fn pane_at_screen_pos(&self, x: u16, y: u16) -> Option<(PaneId, u16, u16)> {
        let (pid, rect) = self.view.layout().find_containing(
            Position::new(x, y),
            self.view.last_pane_area,
            self.view.reserve_seam,
        )?;
        let (pane_x, pane_y) = rect_relative(rect, x, y)?;
        Some((pid, pane_x, pane_y))
    }

    /// Resolve a pane-relative `(x, y)` click in pane `pid` to a buffer
    /// char offset.
    fn click_to_char(
        &mut self,
        pid: PaneId,
        x: u16,
        y: u16,
    ) -> Option<hume_rope::offset::CharOffset> {
        let buf_id = self.view.panes[pid].buffer_id;
        let gutter_w = {
            let pane = &self.view.panes[pid];
            cursor::gutter_width(
                pane.providers.gutter_columns(),
                self.state.buffers.get(buf_id).text().last_ropey_line(),
            )
        };
        let key = self.state.format_key(&self.view.panes[pid]);
        let (mut dlm, viewport) = pane_display_lines(
            self.state.buffers.get(buf_id),
            &mut self.view.panes[pid],
            key,
        );
        cursor::screen_to_char_offset(x, y, gutter_w, viewport, &mut dlm)
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
/// to go the other way: a content-relative position `pane_display_lines`'s cursor
/// walk already resolved, placed onto the screen.
pub(super) fn content_pos_to_screen(
    content_x: u16,
    row: u16,
    gutter_w: u16,
    pane_rect: Rect,
) -> (u16, u16) {
    (content_x + gutter_w + pane_rect.x, row + pane_rect.y)
}
