//! Mouse event handling.
//!
//! Events arrive whenever the `mouse` setting is on. Drag motion arrives with
//! `mouse-select` on, or from a press on a divider until its release (see
//! [`EditorState::mouse_reporting`]). A drag on a divider, drawn with
//! `pane-dividers` on, moves it. A drag in text extends the selection only
//! with `mouse-select` on.
//!
//! Click coordinates are terminal-absolute, so `EngineView::pane_hit` first
//! finds the pane under the pointer, in its own cells. The pane's rows then
//! say what the hit row shows, and `cursor::cell_to_cluster` resolves the
//! buffer cluster under it (`header_to_cluster` for a scope header row). A
//! press on a fold caret in the gutter toggles that fold instead.
//! Every hit test reads the pane rects the last frame drew, so a click lands in the pane
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

use hume_core::PaneId;
use hume_engine::builtins::fold_column::FoldStarts;
use hume_engine::display_lines::{BlockSlot, DisplayLineMap};
use hume_engine::pane::{Pane, Viewport};
use hume_engine::pipeline::{PaneHit, SeamHandle};
use hume_engine::providers::FoldMark;
use hume_engine::scope_headers::{HeaderLayout, PaneRow};
use hume_grid::{Position, Rect};
use hume_platform::terminal::MouseReporting;
use hume_rope::line::ContentLine;
use hume_rope::offset::InclusiveRange;
use termina::event::{MouseButton, MouseEvent, MouseEventKind};

use super::commands;
use super::cursor;
use hume_ops::MotionMode;

use super::buffer::Buffer;
use super::geometry::LayoutKey;
use super::selection_history::Record;
use super::{Editor, EditorState};
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

/// What a left-button drag in progress moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MouseDrag {
    /// The head of the primary selection of the pane the drag started in.
    /// The anchor is that selection's own, read live.
    Select(PaneId),
    /// The divider the drag started on.
    Seam(SeamHandle),
}

impl EditorState {
    /// What the terminal should report of the mouse. Drags are reported
    /// with `mouse-select` on, and otherwise only while a divider drag is in
    /// progress: the press that grabs the divider arrives as a click and
    /// turns motion reporting on until the release.
    pub(crate) fn mouse_reporting(&self) -> Option<MouseReporting> {
        if !self.settings.mouse {
            return None;
        }
        let drags =
            self.settings.mouse_select || matches!(self.mouse_drag, Some(MouseDrag::Seam(_)));
        Some(if drags {
            MouseReporting::Drags
        } else {
            MouseReporting::Clicks
        })
    }
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
        // A press starts fresh: it ends whatever drag the previous one left,
        // released or not, and drops a half-typed key sequence
        // (`pending`/`count`) and the status line.
        self.state.mouse_drag = None;
        self.state.discard_pending_keys();
        self.state.status_msg.clear();

        // A tabline click is handled separately (and unconditionally, even
        // when it doesn't land on an actual tab): it's outside every
        // pane's rect, so the pane hit test below would just treat it as a
        // miss anyway, but routing it first avoids relying on that.
        if self.tabline_click(x, y) {
            return;
        }

        // A divider is outside every pane's rect. Grabbing one moves neither
        // focus nor a selection.
        if let Some(seam) = self.view.seam_hit(Position::new(x, y)) {
            self.state.mouse_drag = Some(MouseDrag::Seam(seam.handle));
            return;
        }

        // Hit-test before anything else: a miss (statusline or anything else
        // outside a pane's own rect) does nothing more, and a hit's
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

        if let Some(line) = self.fold_caret_at(hit) {
            let toggled = commands::apply_fold(
                &mut self.state,
                &mut self.view,
                hit.pane(),
                [line],
                commands::FoldOp::Toggle,
            );
            if let Err(e) = toggled {
                self.report_error(&e);
            }
            return;
        }

        if let Some(pos) = self.click_to_cluster(hit) {
            // Collapse the selections to one cursor on the clicked cluster.
            self.select_only(Selection::cursor(pos));
            // The cursor is the anchor of a drag that follows.
            if self.state.settings.mouse_select {
                self.state.mouse_drag = Some(MouseDrag::Select(hit.pane()));
            }
        }
    }

    /// Replace the focused pane's selections with `sel`, a selection on its
    /// buffer's current text.
    fn select_only(&mut self, sel: Selection) {
        let text = self.doc().text();
        let only = SelectionSet::new(text, vec![sel], 0);
        self.set_current_selections(only);
    }

    // ── Drag ──────────────────────────────────────────────────────────────────

    fn mouse_left_drag(&mut self, x: u16, y: u16) {
        match self.state.mouse_drag {
            Some(MouseDrag::Seam(handle)) => self.drag_seam(handle, x, y),
            Some(MouseDrag::Select(pid)) => self.drag_selection(pid, x, y),
            None => {}
        }
    }

    /// Move the divider `handle` names under the pointer, the way
    /// `Ctrl-p >` and its siblings move one. A handle that names no divider
    /// of the layout ends the drag.
    fn drag_seam(&mut self, handle: SeamHandle, x: u16, y: u16) {
        let screen = self.state.screen_layout(&self.view);
        if self
            .view
            .move_seam(&screen, handle, Position::new(x, y))
            .is_err()
        {
            self.state.mouse_drag = None;
        }
    }

    fn drag_selection(&mut self, pid: PaneId, x: u16, y: u16) {
        // A drag never moves focus mid-gesture. It extends the selection in
        // `pid`, the pane the click that started it focused, and only while
        // that pane still has focus. A drag that leaves it (as a fast mouse
        // move easily can) is ignored rather than resolving against the
        // wrong pane.
        if pid != self.view.focused_pane() {
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
            let t = commands::FocusedPane::current(&self.view).pane();
            // A drag grows the selection like an Extend-mode motion, so the
            // whole gesture is one history entry.
            let recording = self
                .state
                .begin_selection_record(&self.view, t, Record::Continue);
            self.select_only(Selection::new(anchor, head));
            recording.finish(&mut self.state, &self.view, t);
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
            crate::editor::tab::switch_to_tab(&mut self.state, &mut self.view, id);
        }
        true
    }

    /// The line whose fold caret `hit` lands on: a cell of the pane's fold
    /// column, on a row drawing a line's first display line or a scope
    /// header, beside a caret. `None` anywhere else.
    fn fold_caret_at(&mut self, hit: PaneHit) -> Option<ContentLine> {
        let pid = hit.pane();
        let lane = self.state.panes.entries[pid].fold_lane?;
        let doc = self.state.buffers.get(self.view.panes[pid].buffer_id());
        let rope = doc.text().rope();
        let inputs = self.state.gutter_inputs(&self.view, pid);
        let last_line = hume_rope::lines::last_ropey_line(rope);
        if self.view.panes[pid]
            .providers
            .gutter_lane_at(last_line, inputs, hit.x())
            != Some(lane)
        {
            return None;
        }
        let layout_key = self.state.layout_key(&self.view, pid, hit.size());
        let (row, mut dlm, _) = row_at(doc, &mut self.view.panes[pid], &layout_key, hit.y())?;
        let line = match row {
            PaneRow::Header(line) => line,
            PaneRow::Body(pos) => (dlm.slot(pos) == BlockSlot::Content(0)).then_some(pos.line)?,
        };
        let mut starts = FoldStarts::default();
        starts.reset(dlm.hidden().clone());
        if let Some(syntax) = doc.syntax_source() {
            starts.read([InclusiveRange::new(line, line)], syntax, rope);
        }
        (starts.mark(line) != FoldMark::None).then_some(line)
    }

    /// Resolve `hit` to the buffer cluster under it, at its pane's drawn size
    /// with the pane's current gutter. A press on a scope header row resolves
    /// to its line's cluster, laid out as the row draws it, not to the hidden
    /// line under it.
    fn click_to_cluster(&mut self, hit: PaneHit) -> Option<hume_rope::cluster::ClusterStart> {
        let pid = hit.pane();
        let buf_id = self.view.panes[pid].buffer_id();
        let gutter_width = self
            .state
            .pane_geometry(&self.view, pid, hit.size())
            .gutter_width;
        let layout_key = self.state.layout_key(&self.view, pid, hit.size());
        let doc = self.state.buffers.get(buf_id);
        let (row, mut dlm, viewport) =
            row_at(doc, &mut self.view.panes[pid], &layout_key, hit.y())?;
        match row {
            PaneRow::Body(pos) => cursor::cell_to_cluster(
                hit.x(),
                gutter_width,
                viewport.horizontal_offset(),
                &mut dlm,
                pos,
            ),
            PaneRow::Header(line) => {
                let mut header_layout = HeaderLayout::default();
                let mut header_dlm =
                    header_layout.display_map(doc.text().rope(), &layout_key.inputs());
                cursor::header_to_cluster(hit.x(), gutter_width, &mut header_dlm, line)
            }
        }
    }
}

/// The row `pane` draws at pane row `y`, laid out by `layout_key` over
/// `doc`, with the display lines and viewport it was read through. `None`
/// for a pane with no rows.
#[allow(clippy::disallowed_methods)] // a click aims at the view on screen
fn row_at<'a>(
    doc: &'a Buffer,
    pane: &'a mut Pane,
    layout_key: &LayoutKey,
    y: u16,
) -> Option<(PaneRow, DisplayLineMap<'a>, &'a mut Viewport)> {
    let (mut dlm, viewport) =
        pane.display_lines_and_viewport(doc.text().rope(), layout_key.inputs());
    let geo = layout_key.geometry(doc.syntax_source())?;
    let row = geo.rows(&mut dlm, viewport).row_at(&mut dlm, y);
    Some((row, dlm, viewport))
}

// Coordinate helpers

/// Translate a pane-content-relative `(content_x, row)` cell (relative to
/// the pane's content area, past the `gutter_w`-wide gutter) into an
/// absolute terminal cell: a content-relative position a display-line walk
/// already resolved, placed onto the screen.
pub(super) fn content_pos_to_screen(
    content_x: u16,
    row: u16,
    gutter_w: u16,
    pane_rect: Rect,
) -> (u16, u16) {
    (content_x + gutter_w + pane_rect.x, row + pane_rect.y)
}
