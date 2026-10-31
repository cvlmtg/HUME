//! Mouse event handling.
//!
//! Events arrive when normal tracking (mode 1000) is enabled. Button-event
//! tracking (mode 1002, which delivers `MouseEventKind::Drag`) is enabled only
//! when `editor.mouse_select` is true.
//!
//! Click coordinates are terminal-absolute, so `EngineView::pane_hit` first
//! finds the pane under the pointer, in its own cells, before
//! `screen_to_cluster` resolves the buffer cluster it hits. Every hit test
//! reads the pane rects the last frame drew, so a click lands in the pane
//! that was on screen under it even after a band or a pane changed. The
//! gutter and content widths are the pane's current ones at its drawn size.
//!
//! The wheel runs `commands::scroll_view` with `count = mouse-scroll-lines`,
//! like `Ctrl-d`/`Ctrl-u`. It scrolls the pane under the pointer (the focused
//! pane on a miss) without moving focus.
//!
//! Mouse events walk the input-layer stack like keys (`InputEvent::Mouse`); a
//! `Popup` dismisses on any of them. [`Editor::base_mouse`] is the `Base`
//! layer's policy, reached only after the stack has let the event through.

use hume_core::Count;
use hume_engine::pipeline::PaneHit;
use hume_grid::{Position, Rect};
use termina::event::{MouseButton, MouseEvent, MouseEventKind};

use super::commands::{self, pane_display_lines};
use super::cursor;
use hume_ops::MotionMode;

use super::Editor;
use hume_editing::selection::{EditView, Selection, SelectionSet};

/// Whether `kind` is a *fresh* user action rather than the tail of a gesture
/// that began before the current layer existed: a press or a wheel notch is
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
    /// The `Base` layer's own mouse policy, routed here by `dispatch_at`
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
                self.state.mouse_drag = None;
            }
            MouseEventKind::ScrollUp => self.mouse_scroll(mouse.column, mouse.row, false),
            MouseEventKind::ScrollDown => self.mouse_scroll(mouse.column, mouse.row, true),
            _ => {}
        }
    }

    // ── Click ─────────────────────────────────────────────────────────────────

    fn mouse_left_down(&mut self, x: u16, y: u16) {
        // A tabline click is handled separately (and unconditionally, even
        // when it doesn't land on an actual tab): it's outside every
        // pane's rect, so the pane hit test below would just treat it as a
        // miss anyway, but routing it first avoids relying on that.
        if self.tabline_click(x, y) {
            return;
        }

        // Hit-test before anything else: a miss (statusline, a divider seam
        // or anything else outside a pane's own rect) is a no-op, and a hit's
        // pane-relative coordinates are what every step below needs.
        let Some(hit) = self.view.pane_hit(Position::new(x, y)) else {
            return;
        };

        // Click-to-focus: a click in another pane (a `:split`/`:vsplit`)
        // moves focus there, the same `focus_pane` chokepoint
        // `cmd_pane_focus_*` uses. No jump-list push, matching those.
        // `focus_pane` exits Insert (if active) BEFORE resolving the click's
        // char offset below, and while the *previously* focused pane is
        // still current: `end_insert_session` can shrink that pane's
        // buffer (the blank-line indent trim), so computing `click_to_cluster`
        // first would resolve against a buffer the exit is about to change.
        super::focus::focus_pane(&mut self.state, &mut self.view, hit.pane());

        if let Some(pos) = self.click_to_cluster(hit) {
            // Collapse the selections to one cursor on the clicked cluster.
            self.select_only(Selection::cursor(pos));
            self.clear_pending_input();
            // The cursor is the anchor of a drag that follows.
            self.state.mouse_drag = Some(hit.pane());
        }
    }

    /// Replace the focused pane's selections with `sel`, a selection on its
    /// buffer's current text.
    fn select_only(&mut self, sel: Selection) {
        let text = self.doc().text();
        let only = SelectionSet::new(text, vec![sel], 0);
        self.set_current_selections(only);
    }

    /// Reset transient input state every click starts fresh from: any
    /// half-typed key sequence (`pending`/`count`) and the status line.
    /// Leaves `mouse_drag`
    /// alone: `mouse_left_down` sets its own right after calling this, and
    /// `tabline_click` clears it explicitly, since a tab switch is exactly
    /// the case where a stale drag would extend a selection in a pane that
    /// isn't even focused anymore.
    fn clear_pending_input(&mut self) {
        self.state.pending.clear();
        self.state.count = Count::UNTYPED;
        self.state.status_msg = None;
    }

    // ── Drag ──────────────────────────────────────────────────────────────────

    fn mouse_left_drag(&mut self, x: u16, y: u16) {
        // Drag events are only received when `mouse_select = true` (mode 1002).
        // A drag never moves focus mid-gesture. It extends the selection in
        // the pane the click that started it focused, and only while that
        // pane still has focus. A drag that leaves it (as a fast mouse move
        // easily can) is ignored rather than resolving against the wrong pane.
        let pid = self.view.focused_pane();
        if self.state.mouse_drag != Some(pid) {
            return;
        }
        let Some(hit) = self
            .view
            .pane_hit(Position::new(x, y))
            .filter(|hit| hit.pane() == pid)
        else {
            return;
        };

        if let Some(head) = self.click_to_cluster(hit) {
            // The anchor is read from the selection, not remembered from the
            // click, so a key that changes the selection mid-drag moves it.
            let anchor = EditView::bind(self.doc().text(), self.current_selections())
                .primary()
                .anchor();
            self.select_only(Selection::new(anchor, head));
        }
    }

    // ── Scroll ────────────────────────────────────────────────────────────────

    fn mouse_scroll(&mut self, x: u16, y: u16, down: bool) {
        // Unlike a click, a wheel notch never moves focus: `focus_pane`
        // exits Insert mode, and a stray notch over another pane must not be
        // able to do that. A miss (statusline, tabline, a divider seam) has
        // no pane to prefer over the focused one, so it falls back there
        // instead of being a no-op like a click's own miss.
        let pid = self
            .view
            .pane_hit(Position::new(x, y))
            .map_or(self.view.focused_pane(), PaneHit::pane);
        let scroll_lines = self.state.settings.mouse_scroll_lines;
        // The notch scrolls the pane the next frame draws, which a popup the
        // notch retired may have grown.
        let size = self.state.pane_size(&self.view, pid);
        commands::scroll_view(
            &mut self.state,
            &mut self.view,
            pid,
            size,
            scroll_lines,
            down,
            MotionMode::Move,
        );
    }

    // ── Coordinate conversion ─────────────────────────────────────────────────

    /// Handle a click at terminal-absolute `(x, y)` landing in the tab
    /// bar's row, if it does. Returns `true` when it was, whether or not
    /// it landed on an actual tab, so a click on the row's blank tail is a
    /// no-op but still doesn't fall through to the pane hit test (which
    /// would just miss anyway, since the tabline sits outside every pane's
    /// rect). Uses the same `tab_extents` layout `TablineWidget::render`
    /// paints from, so a click always lands on the tab it visually appears
    /// to.
    ///
    /// Hit-tests against the drawn tab bar's own rect rather than the pane
    /// area's `y`: the pane area's degenerate branch (terminal too small to
    /// fit chrome and content) leaves its `y` at the terminal's top, which
    /// would make every row look like the tab bar.
    fn tabline_click(&mut self, x: u16, y: u16) -> bool {
        let bar = self.view.drawn_tabbar_area();
        if !bar.contains(Position::new(x, y)) {
            return false;
        }
        // The tabs the last frame painted into `bar`.
        let guard = self.state.tabline_view.read();
        let extents = crate::tabline::tab_extents(&guard.tabs, guard.scroll, bar.x, bar.width);
        let target = extents
            .ranges
            .iter()
            .position(|&(start, end)| x >= start && x < end)
            .map(|i| guard.tabs[guard.scroll + i].id);
        drop(guard);
        if let Some(id) = target {
            // `switch_to_tab` -> `focus::install_layout` exits Insert (if active) against
            // the outgoing tab's own pane while its tab is still live, before
            // the new tab is installed: the teardown's indent trim edits a
            // buffer the outgoing tab's panes may show.
            self.clear_pending_input();
            self.state.mouse_drag = None;
            crate::editor::tab::switch_to_tab(&mut self.state, &mut self.view, id);
        }
        true
    }

    /// Resolve `hit` to the buffer cluster under it, at its pane's drawn size
    /// with the pane's current gutter.
    fn click_to_cluster(&mut self, hit: PaneHit) -> Option<hume_rope::cluster::ClusterStart> {
        let pid = hit.pane();
        let buf_id = self.view.panes[pid].buffer_id();
        let geometry = self.state.pane_geometry(&self.view, pid, hit.size());
        let key = self.state.format_key(&self.view.panes[pid]);
        let (mut dlm, viewport) = pane_display_lines(
            self.state.buffers.get(buf_id),
            &mut self.view.panes[pid],
            key,
            geometry.content_width,
        );
        cursor::screen_to_cluster(hit.x(), hit.y(), geometry.gutter_width, viewport, &mut dlm)
    }
}

// ---------------------------------------------------------------------------
// Coordinate helpers
// ---------------------------------------------------------------------------

/// Translate a pane-content-relative `(content_x, row)` cell (relative to
/// the pane's content area, past the `gutter_w`-wide gutter) into an
/// absolute terminal cell: a content-relative position `pane_display_lines`'s
/// cursor walk already resolved, placed onto the screen.
pub(super) fn content_pos_to_screen(
    content_x: u16,
    row: u16,
    gutter_w: u16,
    pane_rect: Rect,
) -> (u16, u16) {
    (content_x + gutter_w + pane_rect.x, row + pane_rect.y)
}
