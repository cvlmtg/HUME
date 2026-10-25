//! Producing a frame: preparing it (pane-mirror sync, scroll, render
//! plumbing), drawing it to the terminal, and pacing how often one is drawn.
//!
//! `resize_terminal` (geometry) → `Editor::settle` (advance state) →
//! `prepare_frame` (render prep) is the sequence every frame producer
//! (`Editor::run`'s loop, `render_to_buf`) calls in that order; everything
//! else here is a step `prepare_frame` drives or a helper those steps share.

use std::io;
use std::time::{Duration, Instant};

use hume_grid::Position;
use hume_platform::screen::Screen;
use hume_platform::terminal::SharedTerm;

use hume_grid::{Grid, Rect};

use hume_engine::pane::Pane;
use hume_engine::pipeline::{
    EngineView, PaneId, PaneRenderSettings, PaneSize, RenderContext, ScreenLayout,
};
use hume_engine::types::EditorMode;

use super::buffer::{Buffer, BufferFile};
use super::{Editor, EditorState};

/// Project a pane's selections into its engine mirror, in the model's start
/// order. That is also the cursor order the mirror requires: selections never
/// overlap, so every cursor lies before the next selection's start.
pub(in crate::editor::frame) fn write_pane_mirror(
    pane: &mut hume_engine::pane::Pane,
    text: &hume_editing::text::BufferText,
    sels: &hume_editing::selection::SelectionSet,
) {
    use hume_editing::selection::EditView;
    use hume_engine::types::{PaintedSelection, PaintedSelections};
    let view = EditView::bind(text, sels);
    let items = view.iter().map(|s| PaintedSelection {
        first: s.start(),
        last: s.last(),
        cursor: s.head(),
    });
    match &mut pane.selections {
        Some(mirror) => mirror.rewrite(items, view.primary().index()),
        None => {
            pane.selections = Some(PaintedSelections::new(
                items.collect(),
                view.primary().index(),
            ))
        }
    }
}

/// The shortest time between two frames not drawn for input, so wakes in
/// quick succession (processes finishing, a picker source streaming lines)
/// draw one frame, not one each.
pub(in crate::editor) const FRAME_GAP: Duration = Duration::from_millis(16);

/// When a loop may draw: at once after input, otherwise no sooner than
/// [`FRAME_GAP`] after the last frame.
#[derive(Debug, Default)]
pub(in crate::editor) struct FramePacer {
    /// The earliest a frame not drawn for input may draw, `None` for at once.
    next_due: Option<Instant>,
    /// Input arrived since the last frame.
    input: bool,
}

impl FramePacer {
    /// A pacer whose first frame not drawn for input is due at `at`.
    pub(in crate::editor) fn not_before(at: Instant) -> Self {
        Self {
            next_due: Some(at),
            input: false,
        }
    }

    /// Input arrived: the next frame is due at once.
    pub(in crate::editor) fn input(&mut self) {
        self.input = true;
    }

    /// How long until the next frame is due, `None` when it is due at `now`.
    pub(in crate::editor) fn wait(&self, now: Instant) -> Option<Duration> {
        if self.input {
            return None;
        }
        let due = self.next_due?;
        (due > now).then(|| due - now)
    }

    /// A frame was drawn at `now`.
    pub(in crate::editor) fn drew(&mut self, now: Instant) {
        self.next_due = Some(now + FRAME_GAP);
        self.input = false;
    }
}

/// What `Editor::run`'s loop draws frames with, kept from one frame to the
/// next.
pub(in crate::editor) struct FrameTarget<'a> {
    term: &'a SharedTerm,
    screen: &'a mut Screen,
    /// Outside `Editor` so it can be borrowed mutably alongside the `&mut
    /// self` that `prepare_frame`/`render_into` each take.
    ctx: RenderContext,
    /// Whether the terminal cursor was last set black, `None` before the
    /// first frame.
    cursor_black: Option<bool>,
}

impl<'a> FrameTarget<'a> {
    pub(in crate::editor) fn new(term: &'a SharedTerm, screen: &'a mut Screen) -> Self {
        Self {
            term,
            screen,
            ctx: RenderContext::new(),
            cursor_black: None,
        }
    }

    pub(in crate::editor) fn screen(&mut self) -> &mut Screen {
        self.screen
    }
}

impl Editor {
    /// Draw one frame into `target`: prepare it, render it, then set the
    /// terminal cursor's shape and colour. Every caller has settled first,
    /// or has run nothing a settle would drain since it last did.
    pub(in crate::editor) fn draw_frame(&mut self, target: &mut FrameTarget<'_>) -> io::Result<()> {
        let FrameTarget {
            term,
            screen,
            ctx,
            cursor_black,
        } = target;
        let frame_layout = self.prepare_frame(ctx);

        // Compute terminal cursor position before the draw closure to avoid
        // split-borrow conflicts: pane borrows and rope borrows must end
        // before `&mut self.view` is captured by the closure.
        let cursor_screen = if let Some(mb) = self.state.minibuf() {
            // Minibuf active (Command / Search / Select): place the terminal
            // cursor in the statusline at the minibuf edit position. Always a
            // bar: HUME's prompt modes have no cursor-shape option of their
            // own, unlike the document's own Insert-mode cursor below.
            let statusline_row = frame_layout.statusline_area().y;
            Some(Position::new(mb.statusline_cursor_x(), statusline_row))
        } else {
            // Reads `cursor_shape()` directly rather than through
            // `resolve_pane_settings`: the focused pane's own
            // `cursor_is_block` is defined as this same comparison there,
            // and resolving full pane settings (a `FormatKey` build
            // included) just to throw away everything but this one bool
            // would repeat, for the focused pane specifically, work
            // `render_into` already does for every pane this same frame.
            if self.state.cursor_shape() == crate::editor::settings::CursorShape::Block {
                None
            } else {
                // Non-block shape (only Insert can be, via
                // `insert-cursor-shape`): place the terminal cursor at the
                // document head, where `prepare_frame`'s scroll step already
                // resolved it. The display-line map that decided *where to
                // scroll* had to answer this question anyway, so re-deriving
                // it here would walk the same display lines a second time.
                ctx.cursor_content_pos.map(|(content_x, row)| {
                    // The focused pane's origin in the frame's layout, so the
                    // cursor lands inside the pane, not at the origin.
                    let layout = self.view.focused_layout();
                    let pane_rect = frame_layout.focused_rect(layout);
                    let gutter_w = self
                        .state
                        .pane_geometry(
                            &self.view,
                            self.view.focused_pane(),
                            frame_layout.focused_size(layout),
                        )
                        .gutter_width;
                    let (x, y) =
                        super::mouse::content_pos_to_screen(content_x, row, gutter_w, pane_rect);
                    Position::new(x, y)
                })
            }
        };

        // Open the synchronized-output envelope so the terminal defers
        // display until after every byte of this frame has been written.
        // Terminals that don't support DEC 2026 ignore the
        // sequence, hence `let _ =` rather than `?`.
        let _ = hume_platform::terminal::begin_synchronized_update(term);
        let area = self.view.terminal_area();
        let grid = screen.frame(area.width, area.height);
        self.render_into(grid, ctx, frame_layout);
        screen.present(cursor_screen)?;

        // Emitted *after* the frame so it's the last escape sequence the
        // terminal sees before we block: the show-cursor sequence closing
        // a frame can otherwise reset the shape on some terminals.
        //
        // Minibuf active forces a bar unconditionally, same as the position
        // branch above: HUME's prompt modes have no shape option of their
        // own. Otherwise the live mode's resolved shape
        // (`EditorState::cursor_shape`) is the single source both this and
        // `resolve_pane_settings`' `cursor_is_block` read, so the real
        // terminal cursor and the grid's painted heads can never disagree
        // about which shape is in effect.
        //
        // Re-asserted every frame rather than only on a change, unlike
        // the cursor *colour* below, which is gated on one. That's the
        // point of emitting it here: the show-cursor sequence closing a
        // frame resets the shape on some terminals, so a shape sent once
        // when the mode changed would be undone by the next
        // frame. One SGR-sized write per frame is not worth tracking.
        //
        // The write is best-effort: a terminal that doesn't understand
        // DECSCUSR ignores it, and a genuine I/O failure surfaces from the
        // frame's one flush below.
        let shape = if self.state.minibuf().is_some() {
            crate::editor::settings::CursorShape::Bar
        } else {
            self.state.cursor_shape()
        };
        let cursor_style = match shape {
            crate::editor::settings::CursorShape::Block => {
                hume_platform::terminal::CursorStyle::SteadyBlock
            }
            crate::editor::settings::CursorShape::Bar => {
                hume_platform::terminal::CursorStyle::SteadyBar
            }
            crate::editor::settings::CursorShape::Underline => {
                hume_platform::terminal::CursorStyle::SteadyUnderline
            }
        };
        let _ = hume_platform::terminal::set_cursor_shape(term, cursor_style);
        // A minibuffer puts the cursor on the statusline's background; black
        // keeps it visible there. Everywhere else the default colour returns.
        let black = self.state.minibuf().is_some();
        if *cursor_black != Some(black) {
            let _ = hume_platform::terminal::set_cursor_color(term, black);
            *cursor_black = Some(black);
        }
        // Close the synchronized-output envelope: the terminal now atomically
        // paints the complete frame: clear + cells + cursor shape in one shot.
        // This is the frame's only flush, so its error is the frame's.
        hume_platform::terminal::end_synchronized_update(term)
    }

    /// [`Self::draw_frame`] after resizing to the screen's current size.
    pub(in crate::editor) fn draw_resized_frame(
        &mut self,
        target: &mut FrameTarget<'_>,
    ) -> io::Result<()> {
        let (width, height) = target.screen.size()?;
        self.resize_terminal(width, height);
        self.draw_frame(target)
    }
}

impl Editor {
    /// Resolve any pane's render settings.
    ///
    /// `format` is [`EditorState::format_key`](super::EditorState::format_key),
    /// the single source of truth for wrap_mode / tab_width / whitespace
    /// across all render paths, so this and the scroll pass
    /// (`commands::pane_display_lines`) resolve a bit-identical key for the same
    /// pane. `mode` is a per-focus fact: only the focused pane owns the real
    /// terminal cursor, so it alone gets the live editor mode; other panes are
    /// forced to `Normal` so they don't take Insert's or Extend's cursor
    /// colours. `cursor_is_block` is the separate per-focus resolution that
    /// decides whether either selection head is painted at all: always for
    /// an unfocused pane (no real cursor sits there to stand in for one), and
    /// for the focused pane only when its mode's resolved shape is `Block`.
    ///
    /// `pid` must name a live, active-tab pane. Every caller reads it from
    /// `active_pane_ids()`, which can never contain a stale id (see
    /// `EngineView::layout`'s privacy: no whole-tree write can install a leaf
    /// the pool doesn't back).
    pub(super) fn resolve_pane_settings(&self, pid: PaneId) -> PaneRenderSettings {
        let pane = &self.view.panes[pid];
        let doc = self.state.buffers.get(pane.buffer_id());
        let indent_guides = doc.overrides.indent_guides(&self.state.settings);
        let is_focused = pid == self.view.focused_pane();
        let mode = if is_focused {
            self.state.mode()
        } else {
            EditorMode::Normal
        };
        let cursor_is_block =
            !is_focused || self.state.cursor_shape() == crate::editor::settings::CursorShape::Block;
        PaneRenderSettings {
            mode,
            format: self.state.format_key(pane),
            indent_guides,
            cursor_is_block,
            gutter: self.state.gutter_inputs(&self.view, pid),
        }
    }

    /// Render one frame into `grid` as laid out by `layout`, the one
    /// `prepare_frame` returned, whose terminal area is `grid`'s size. Single
    /// home for the rope and syntax lookups shared by the event loop and
    /// `render_to_buf`.
    pub(super) fn render_into(
        &mut self,
        grid: &mut Grid,
        ctx: &mut RenderContext,
        layout: ScreenLayout,
    ) {
        // Resolved for every active pane up front: `resolve_pane_settings`
        // reads the pane it is asked about, and `render`'s pane loop borrows
        // `self.view.panes` mutably, so it cannot run from inside that loop.
        // `render` itself is layout-scoped (it walks `view.layout`, same as
        // `active_pane_ids`), so this stays the exact set it needs, never
        // more.
        let active = self.view.active_pane_ids();
        ctx.set_pane_settings(
            active
                .iter()
                .map(|&pid| (pid, self.resolve_pane_settings(pid))),
        );
        let focused_pid = self.view.focused_pane();
        let focused_bid = self.focused_buffer_id();

        // Split explicitly rather than leaning on closure-capture precision:
        // `view` is borrowed mutably below, so everything else the call needs
        // has to come from a provably disjoint field.
        let Editor {
            state,
            view,
            kitty_enabled,
            ..
        } = self;
        let focused_state = state.panes.entries.shown(&view.panes, focused_pid);
        let statusline = crate::statusline::HumeStatusline {
            state,
            kitty_enabled: *kitty_enabled,
            focused_bid,
            focused_view: focused_state.view(state.buffers.get(focused_bid).text()),
            focused_search_cursor: &focused_state.search_cursor,
        };
        let buffers = &state.buffers;
        view.render(
            grid,
            hume_engine::pipeline::FrameInputs {
                get_rope: |bid| buffers.try_get(bid).map(|b| b.text().rope()),
                get_syntax: |bid| {
                    buffers
                        .try_get(bid)
                        .and_then(|b| b.syntax.as_ref())
                        .map(|s| s as &dyn hume_engine::providers::SyntaxSpans)
                },
                statusline: &statusline,
                layout,
            },
            ctx,
        );
    }

    /// The statusline provider over this editor: the fixture the element
    /// tests need, since `HumeStatusline`'s fields are assembled from
    /// disjoint borrows that a test holding a whole `&Editor` doesn't have to
    /// bother splitting.
    #[cfg(test)]
    pub(crate) fn statusline(&self) -> crate::statusline::HumeStatusline<'_> {
        let focused_bid = self.focused_buffer_id();
        let focused_state = self
            .state
            .panes
            .entries
            .shown(&self.view.panes, self.view.focused_pane());
        crate::statusline::HumeStatusline {
            state: &self.state,
            kitty_enabled: self.kitty_enabled,
            focused_bid,
            focused_view: focused_state.view(self.state.buffers.get(focused_bid).text()),
            focused_search_cursor: &focused_state.search_cursor,
        }
    }

    /// Render the current frame into a [`Grid`] without a live terminal.
    ///
    /// Calls `resize_terminal` + `settle` + `prepare_frame` (the same
    /// three-step sequence `Editor::run`'s loop uses), so pane mirrors are
    /// synced and parse trees are up to date before rendering.
    #[cfg(test)]
    pub(in crate::editor) fn render_to_buf(&mut self, rect: Rect) -> Grid {
        let mut buf = Grid::new(rect.width, rect.height);
        let mut ctx = RenderContext::new();
        self.resize_terminal(rect.width, rect.height);
        self.settle();
        let layout = self.prepare_frame(&mut ctx);
        self.render_into(&mut buf, &mut ctx, layout);
        buf
    }

    /// Drop `viewport_debounce`/`last_visible_range`/`virtual_lines_synced`
    /// entries whose pane no longer exists in `self.view.panes`. A pending
    /// debounce timer is cancelled outright (its `TimerPayload` no-ops via
    /// `queue_viewport_change`'s own liveness check anyway, but there is no
    /// reason to let it sit in the wheel until it fires).
    ///
    /// A pane's line store needs no entry here: it lives on the pane and
    /// dies with it, as do `PaneBufferState::reveal_pending` and
    /// `PaneBufferState::last_layout_key`, which go with the closed pane's
    /// `SecondaryMap` entries.
    fn prune_closed_pane_caches(&mut self) {
        let panes = &self.view.panes;
        self.last_visible_range
            .retain(|pid, _| panes.contains_key(*pid));
        self.virtual_lines_synced
            .retain(|pid, _| panes.contains_key(*pid));
        let timers = &mut self.timers;
        self.viewport_debounce.retain(|pid, id| {
            let live = panes.contains_key(*pid);
            if !live {
                timers.cancel(*id);
            }
            live
        });
    }

    /// Re-apply the terminal's mouse-tracking mode if `mouse`/
    /// `mouse-select` changed since the last time this ran. A no-op when
    /// nothing changed (the common case, checked every frame) and when no
    /// terminal is attached (tests, headless `run_keys`).
    ///
    /// The comparison-and-update itself doesn't require a live terminal, so
    /// it stays outside the `if let Some(term)` below. This keeps
    /// `applied_mouse_mode` in sync with `state.settings` even headless,
    /// which is what makes the change-detection unit-testable without a
    /// real `SharedTerm`.
    pub(in crate::editor::frame) fn resync_mouse_mode(&mut self) {
        let desired = (self.state.settings.mouse, self.state.settings.mouse_select);
        if desired == self.applied_mouse_mode {
            return;
        }
        if let Some(term) = self.tui.terminal() {
            let _ = hume_platform::terminal::set_mouse_mode(term, desired.0, desired.1);
        }
        self.applied_mouse_mode = desired;
    }

    /// Report `state.cwd` to the terminal when it differs from what the
    /// terminal was last told. A cwd with no URL clears the report so the
    /// terminal does not keep a stale directory.
    ///
    /// As in `resync_mouse_mode`, the comparison and update sit outside the
    /// terminal check so they are testable headless.
    fn resync_cwd(&mut self) {
        if self.applied_cwd.as_ref() == Some(&self.state.cwd) {
            return;
        }
        if let Some(term) = self.tui.terminal() {
            let _ = hume_platform::terminal::set_working_directory(term, &self.state.cwd);
        }
        self.applied_cwd = Some(self.state.cwd.clone());
    }

    /// Send the clipboard text queued for OSC 52, if any. A no-op without a
    /// terminal (tests, headless `run_keys`), which leaves the text queued.
    pub(super) fn flush_osc52(&mut self) {
        let Some(term) = self.tui.terminal() else {
            return;
        };
        if let Some(text) = self.state.clipboard.take_pending_osc52() {
            let _ = hume_platform::terminal::set_clipboard(term, &text);
        }
    }

    /// Record a new terminal size: the one step that needs the raw
    /// `(width, height)`, so it's split out from `prepare_frame` and called
    /// separately, *before* `Editor::settle()`.
    ///
    /// Must run before `settle()`: `drain_due_timers` fires `OnViewportChange`
    /// with each pane's size (`timer_bridge.rs`), which is laid out against
    /// this terminal, so a resize has to land before that drain runs.
    pub(super) fn resize_terminal(&mut self, terminal_width: u16, terminal_height: u16) {
        self.view
            .resize(Rect::new(0, 0, terminal_width, terminal_height));
    }

    /// Sync all editor-owned state into the engine view, once per frame,
    /// right before `render()`.
    ///
    /// Every caller runs `Editor::settle()` immediately before this, or has
    /// run nothing since its last settle: a pipe run's repaints, where only
    /// the terminal size and the run's progress change. A settled drain can
    /// switch a pane's buffer (picker accept, LSP goto-definition) or move
    /// its selections, so everything here must read post-settle state.
    /// Syncing earlier would pair a stale selection head with the pane's new
    /// rope.
    ///
    /// Order matters twice. Providers that change display-line counts
    /// (signs, inlay hints, virtual lines, EOL text) sync before the scroll
    /// pass, so the cursor is positioned against the layout render will
    /// draw. Cursor-anchored overlays sync after it, because they need the
    /// cursor's final screen cell.
    ///
    /// Returns the layout this frame's scroll pass and overlays were resolved
    /// against, for `render_into` to draw.
    pub(super) fn prepare_frame(&mut self, ctx: &mut RenderContext) -> ScreenLayout {
        // `ctx` is reused across frames; the scroll pass refills this.
        ctx.cursor_content_pos = None;
        // Required for correctness, see `EngineView::begin_frame`.
        self.view.begin_frame();

        // These caches live on `Editor`, not `EditorState.panes`, so
        // `drop_pane_state` can't clear them when a pane closes.
        self.prune_closed_pane_caches();

        // Mouse modes are terminal state applied once at startup; resyncing
        // here makes `:set global mouse=…` take effect immediately.
        self.resync_mouse_mode();
        self.resync_cwd();
        self.flush_osc52();

        // The drawer's model (the `:ls` rows, the scroll clamp, the text of
        // its newly visible rows), then the layout, for a band a settled drain
        // or the drawer's render proc opened or closed, then the band views
        // this frame draws.
        let focused_bid = self.focused_buffer_id();
        self.state.refresh_buffer_list_drawer(focused_bid);
        self.state
            .clamp_drawer_scroll_to_terminal(self.view.terminal_area().height);
        self.fill_drawer_window();
        let screen = self.state.screen_layout(&self.view);
        self.state.sync_band_views(&mut self.view, &screen);

        // Nothing below changes which panes the active tab shows.
        let active = self.view.active_pane_ids();
        let sizes: Vec<(PaneId, PaneSize)> = active
            .iter()
            .map(|&pid| {
                let size = screen
                    .pane_size(self.view.focused_layout(), pid)
                    .expect("an active pane is a leaf of the focused layout");
                (pid, size)
            })
            .collect();

        self.sync_all_pane_mirrors(&active);

        // Everything that changes display-line counts or columns, synced
        // before scrolling. Signs set the gutter width and so the wrap
        // column; the other three are `DisplayLineMap` providers. The
        // snapshot predates the scroll, so a line exposed by this frame's
        // scroll gets its hints one frame late. Syncing after the scroll
        // would instead let scroll and render disagree on layout.
        let panes = self.decorated_panes(&sizes);
        self.update_sign_providers(&panes);
        self.update_inlay_hint_providers(&panes);
        self.update_virtual_line_providers(&panes);
        self.update_eol_text_providers(&panes);

        // Scroll each active pane so its primary cursor stays visible.
        //
        // A pane whose tab went to the background keeps no key, so its
        // next visible frame always counts as a viewport change.
        self.last_visible_range
            .retain(|pid, _| active.contains(pid));

        for &(pid, size) in &sizes {
            let buf_id = self.view.panes[pid].buffer_id();
            let layout_key = self.state.layout_key(&self.view, pid, size);
            // Per (pane, buffer) state: a pane that just switched buffers
            // reads a fresh entry whose `last_layout_key` is `None`.
            let pbs = self.state.panes.entries.shown_mut(&self.view.panes, pid);
            let cursor = pbs
                .view(self.state.buffers.get(buf_id).text())
                .primary()
                .head();
            let layout_changed = pbs.last_layout_key.replace(layout_key) != Some(layout_key);
            // A layout change reveals only a pane that isn't parked: one
            // parked behind an unfollowable scroll must not snap back onto
            // its cursor just because something changed elsewhere in the
            // buffer it's viewing. The pane's own action (a head move, or an
            // edit it made) always reveals, via `reveal_pending`.
            let reveal = std::mem::take(&mut pbs.reveal_pending) || (layout_changed && !pbs.parked);
            let outcome = scroll_into_view(
                self.state.buffers.get(buf_id),
                &mut self.view.panes[pid],
                cursor,
                layout_key,
                reveal,
            );
            self.state
                .panes
                .entries
                .shown_mut(&self.view.panes, pid)
                .parked = outcome.parked;
            if pid == self.view.focused_pane() {
                ctx.cursor_content_pos = outcome.cursor_screen;
            }

            // A visible-range change arms the `OnViewportChange` debounce
            // timer; the hook fires later from the timer drain.
            let content_lines = self.state.buffers.get(buf_id).text().content_line_count();
            let range =
                super::geometry::pane_visible_range(&self.view.panes[pid], size, content_lines);
            if self.last_visible_range.insert(pid, (buf_id, range)) != Some((buf_id, range)) {
                self.debounce_viewport_change(pid);
            }
        }

        // Highlights and line tints only affect painting, so they sync after
        // the scroll from a fresh snapshot of the final viewport.
        let panes = self.decorated_panes(&sizes);
        self.update_highlight_providers(&panes);
        self.update_line_bg_providers(&panes);

        // Overlays resolve their geometry now, so render only paints. The
        // cursor-anchored ones need the scroll result above.
        self.sync_minibuf_completion_view(&screen);
        self.sync_popup_view(&screen, ctx.cursor_content_pos);
        self.sync_menu_view(&screen, ctx.cursor_content_pos);
        self.sync_completion_menu_view(&screen, ctx.cursor_content_pos);
        self.sync_picker_view(&screen);

        // Bake scopes interned during this frame (extra highlights, inline
        // diagnostics, a new grammar's captures) before render resolves them.
        self.view.theme.bake_if_stale(&self.view.registry);
        screen
    }

    /// Sync every active-tab pane's selection mirror from the authoritative
    /// `pane_state`.
    ///
    /// This is the **single sync point**: no other code path writes
    /// `pane.selections`.
    ///
    /// Called once per frame from `prepare_frame`, after the async/Steel
    /// drains and before `render()`, passing the same `active_pane_ids()`
    /// snapshot `prepare_frame` already computed for its other steps rather
    /// than recomputing it here too.
    pub(in crate::editor) fn sync_all_pane_mirrors(&mut self, active: &[PaneId]) {
        let state = &self.state;
        let view = &mut self.view;
        for &pid in active {
            let selections = state.panes.entries.shown(&view.panes, pid).selections();
            let pane = &mut view.panes[pid];
            let bid = pane.buffer_id();
            write_pane_mirror(pane, state.buffers.get(bid).text(), selections);
        }
    }

    // ── Engine accessors ──────────────────────────────────────────────────────

    #[cfg(test)]
    pub(in crate::editor) fn viewport(&self) -> &hume_engine::pane::Viewport {
        &self.view.panes[self.view.focused_pane()].viewport
    }

    /// How many times `claim_terminal` has actually entered the
    /// inline-output terminal bracket (alt-screen toggle + "press any key")
    /// on this `Editor`. Off the event loop this must stay `0` for every
    /// `#:inline-output #t` command dispatched, output or not; see `tui` on
    /// `Editor`. Also lets a test pin an exact count through nested `call!`s
    /// (a re-entry bug shows up as `2`, not just "entered").
    #[cfg(test)]
    pub(in crate::editor) fn inline_output_enter_count(&self) -> usize {
        self.state.inline_output.enter_count()
    }

    /// The screen's layout as the current models have it.
    #[cfg(test)]
    pub(in crate::editor) fn screen_layout(&self) -> ScreenLayout {
        self.state.screen_layout(&self.view)
    }

    /// Pane `pid`'s size as the current models lay it out.
    #[cfg(test)]
    pub(in crate::editor) fn pane_size(&self, pid: PaneId) -> PaneSize {
        self.state.pane_size(&self.view, pid)
    }

    #[cfg(test)]
    pub(in crate::editor) fn viewport_mut(&mut self) -> &mut hume_engine::pane::Viewport {
        let focused_pid = self.view.focused_pane();
        &mut self.view.panes[focused_pid].viewport
    }
}

impl EditorState {
    /// The document-mode cursor shape for the live mode: how the document's
    /// selection heads are painted, and (outside a prompt) the real terminal
    /// cursor's shape.
    ///
    /// Only Insert is configurable (`insert-cursor-shape`): a terminal has one
    /// hardware cursor, so HUME offers no shape choice for Normal/Extend
    /// (always `Block`, matching Helix's own default), and Command/Search/
    /// Sift also resolve to `Block` here (their document heads render like
    /// Normal mode while a prompt is open), but their *real* terminal cursor
    /// never reflects this value: it lives in the minibuf, placed by a
    /// separate branch in `draw_frame`'s `cursor_screen` that is
    /// unconditionally a bar regardless of what this method returns. This is
    /// the single source both that branch's sibling (the non-minibuf case)
    /// and `resolve_pane_settings`' `cursor_is_block` read, so the real
    /// terminal cursor and the grid's painted heads can never disagree about
    /// which shape is in effect outside a prompt.
    pub(in crate::editor) fn cursor_shape(&self) -> crate::editor::settings::CursorShape {
        if self.mode() == super::Mode::Insert {
            self.settings.insert_cursor_shape
        } else {
            crate::editor::settings::CursorShape::Block
        }
    }

    /// Bring the band views (drawer, docked popup, tab bar) up to their
    /// models for this frame to paint, the tab bar at its place in `screen`.
    fn sync_band_views(&mut self, view: &mut EngineView, screen: &ScreenLayout) {
        self.sync_drawer_view();
        self.sync_popup_band_view(view);
        self.sync_tabline_view(view, screen);
    }

    /// Hash of everything [`Self::sync_tabline_view`]'s rebuild depends on:
    /// whether the tab bar is shown at all, and (while it is) the tab
    /// count/order, each tab's `(id, pane, buffer, dirty, file name)`, and
    /// the bar's own geometry (a resize must still trigger a rebuild even
    /// when the tab list itself is unchanged, since `scroll` depends on
    /// width too). The file name is hashed rather than `buf.display_name()`:
    /// `display_name()` allocates a `String`, defeating the point of a
    /// cheap signature, and the two agree on every rename/attach that
    /// actually changes what's drawn (a buffer's dirty marker is covered
    /// separately by `has_unsaved_changes`). Excludes anything that
    /// doesn't change what a rebuild would produce: buffer *content*, for
    /// instance, since neither the label nor `scroll` reads it.
    fn tabline_signature(&self, view: &EngineView, screen: &ScreenLayout, visible: bool) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = rustc_hash::FxHasher::default();
        visible.hash(&mut hasher);
        if visible {
            let current = self.tabs.current();
            current.hash(&mut hasher);
            let bar = screen.tabbar_area();
            bar.x.hash(&mut hasher);
            bar.width.hash(&mut hasher);
            for (id, pid) in self.tabs.focused_panes(view.focused_pane()) {
                id.hash(&mut hasher);
                pid.hash(&mut hasher);
                let bid = view.panes[pid].buffer_id();
                bid.hash(&mut hasher);
                let buf = self.buffers.get(bid);
                self.has_unsaved_changes(bid).hash(&mut hasher);
                buf.file().map(BufferFile::file_name).hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    /// Re-sync the tab-bar view from `self.tabs` + each tab's focused
    /// pane's buffer, self-healing every frame: a direct mutation that
    /// bypasses the normal `:tabnew`/`:tabclose` builtins would otherwise
    /// leave a stale row painting.
    /// Gated on [`Self::tabline_signature`]: an unchanged signature means
    /// the previous frame's `TablineViewState` is already correct, so the
    /// rebuild below (one allocating `display_name()` call per tab, plus
    /// the scroll probe) is skipped on every steady-state frame, which is
    /// almost all of them.
    fn sync_tabline_view(&mut self, view: &EngineView, screen: &ScreenLayout) {
        let visible = self.tabline_visible();

        let signature = self.tabline_signature(view, screen, visible);
        if self.last_tabline_signature == Some(signature) {
            return;
        }
        self.last_tabline_signature = Some(signature);

        if !visible {
            // The common case (the `dynamic` default with one tab open, i.e.
            // a normal session): skip every per-tab step below entirely.
            // `TabEntry::label` is an owned `String`, so building the full
            // `Vec` just to immediately hide it would cost one allocation
            // and a `display_name()` call per tab, every frame, for a row
            // that's never drawn.
            self.tabline_view
                .set(crate::tabline::TablineViewState::default());
            return;
        }

        let active_index = self.tabs.current_pos();

        let tabs: Vec<crate::tabline::TabEntry> = self
            .tabs
            .focused_panes(view.focused_pane())
            .map(|(id, pid)| {
                let bid = view.panes[pid].buffer_id();
                let buf = self.buffers.get(bid);
                let mut label = buf.display_name();
                if self.has_unsaved_changes(bid) {
                    label.push('+');
                }
                crate::tabline::TabEntry::new(id, label)
            })
            .collect();

        // The smallest `scroll` that still keeps `active_index` inside the
        // packed window, computed fresh every frame rather than clamped
        // from last frame's value, which this loop's own result never
        // actually depends on: packing from an earlier `scroll` can only
        // reach the same window end or earlier (an earlier start must first
        // fit the tab(s) before it), so "does `active_index` fit starting at
        // `scroll`" is monotone in `scroll` and the smallest passing value
        // is a pure function of `tabs`/`active_index`/`width`. Bounded by
        // `scroll < active_index` even in the degenerate `width == 0` case
        // (startup, before the first real terminal size arrives), where no
        // tab ever fits and the window never grows.
        // Reads geometry from `tabbar_area`, same as `render`/`tabline_click`,
        // so painting, hit-testing and this scroll agree on where each tab
        // starts and ends. Probes
        // via `tab_extents_into` rather than `tab_extents`: one reused
        // `ranges` buffer across every candidate this loop tries, instead of
        // a fresh allocation per candidate.
        let bar = screen.tabbar_area();
        let mut probe_ranges = Vec::new();
        let mut scroll = 0;
        while scroll < active_index {
            crate::tabline::tab_extents_into(&tabs, scroll, bar.x, bar.width, &mut probe_ranges);
            if active_index < scroll + probe_ranges.len() {
                break;
            }
            scroll += 1;
        }

        self.tabline_view.set(crate::tabline::TablineViewState {
            tabs,
            active_index,
            scroll,
        });
    }
}

/// [`scroll_into_view`]'s result: where the cursor ended up on screen, and
/// whether this pane came out of the pass parked (its cursor outside the
/// scroll-margin band). `PaneBufferState::parked`'s only writer is the caller,
/// which stores this field back onto it.
struct ScrollOutcome {
    /// Pane-relative, before the gutter. `None` for a viewport with no
    /// display lines to place it in, or when the cursor has scrolled out of
    /// view: a legitimate state, not a bug. The cursor can only occupy
    /// content display lines, so a pure view scroll into a virtual-line
    /// block can carry the viewport further than the cursor can follow. The
    /// terminal caret is simply hidden until an ordinary cursor motion
    /// resyncs the view.
    cursor_screen: Option<(u16, u16)>,
    /// See [`PaneBufferState::parked`](crate::editor::pane_state::PaneBufferState::parked)'s own doc.
    parked: bool,
}

/// Scroll the pane viewport so `cursor` stays within the visible area.
///
/// Calls both the vertical (`reveal`) and horizontal (`reveal_horizontal`)
/// verbs in one shot, over a single display-line map, so the two agree on
/// the same display-line list, and a line's format is reused
/// across them. The cursor is resolved exactly once here, for both plus the
/// terminal-cursor placement: scrolling only ever *writes* the viewport, and
/// the display-line map holds no viewport, so no arm below can change what
/// `locate` already answered.
///
/// `reveal` is `PaneBufferState::reveal_pending`'s value for this frame
/// (already taken by the caller) or'd with an unparked layout change; see
/// that field's own doc. When `true`, `Viewport::reveal` runs and the pane
/// comes out unparked. When `false`, `Viewport::settled_row`
/// answers whether the cursor is already where `reveal` would have left it:
/// `Some` places the cursor there directly (an unparked pane, same walk
/// `reveal` itself would need); `None` means the cursor is outside the band,
/// so this pass falls back to a plain forward walk from `top` (resolved via
/// `Viewport::top_at`) and reports the pane parked.
fn scroll_into_view(
    doc: &Buffer,
    pane: &mut Pane,
    cursor: hume_rope::cluster::ClusterStart,
    layout_key: super::geometry::LayoutKey,
    reveal: bool,
) -> ScrollOutcome {
    // Whatever this pass formats deciding where to scroll, the render pass
    // finds already done: both work through this pane's one store.
    let (mut dlm, viewport) = super::commands::pane_display_lines(
        doc,
        pane,
        layout_key.format_key(),
        layout_key.content_width(),
    );
    // A collapsed split has nothing to scroll and nowhere to put a cursor.
    // Checked before `locate`, which would otherwise format the cursor's
    // line for an answer no one can use, and before `geometry`, which
    // returns `None` for exactly this case.
    let Some(geo) = layout_key.geometry() else {
        return ScrollOutcome {
            cursor_screen: None,
            parked: false,
        };
    };
    let (cursor_pos, cursor_display_col) = dlm.locate(cursor);
    // Horizontal scroll is its own axis (a fixed margin, no `scroll-margin`, no
    // document-edge special-casing; see `reveal_horizontal`'s own doc) and
    // has no snap-back to guard against, so it always runs: a
    // same-display-line cursor move (`l` on a long unwrapped line) changes
    // the column without changing `cursor_pos`, and gating this on the same
    // `reveal` the vertical arm below reads would leave it stale for
    // exactly that case.
    viewport.reveal_horizontal(&mut dlm, cursor_display_col);
    if reveal {
        let screen_row = viewport.reveal(&mut dlm, geo, cursor_pos);
        return ScrollOutcome {
            cursor_screen: Some(super::cursor::place(
                viewport,
                cursor_display_col,
                screen_row,
            )),
            parked: false,
        };
    }
    if let Some(screen_row) = viewport.settled_row(&mut dlm, geo, cursor_pos) {
        ScrollOutcome {
            cursor_screen: Some(super::cursor::place(
                viewport,
                cursor_display_col,
                screen_row,
            )),
            parked: false,
        }
    } else {
        // `reveal_horizontal` just guaranteed `cursor_display_col >=
        // horizontal_offset`, so this is `cursor::content_pos` minus the two
        // checks it exists to make for a caller that hasn't already done
        // them. Only its forward walk is left to redo.
        let top = viewport.top_at(&mut dlm);
        let cursor_screen = dlm
            .distance(top, cursor_pos, geo.height() - 1)
            .map(|screen_row| super::cursor::place(viewport, cursor_display_col, screen_row));
        ScrollOutcome {
            cursor_screen,
            parked: true,
        }
    }
}
