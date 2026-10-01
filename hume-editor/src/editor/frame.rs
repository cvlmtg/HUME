//! Per-frame preparation: pane-mirror sync, scroll, and render plumbing.
//!
//! `sync_viewport_dims` (geometry) → `Editor::settle` (advance state) →
//! `prepare_frame` (render prep) is the sequence every frame producer
//! (`Editor::run`'s loop, `render_to_buf`) calls in that order; everything
//! else here is a step `prepare_frame` drives or a helper those steps share.

use hume_grid::{Grid, Rect};

use hume_engine::pane::Pane;
use hume_engine::pipeline::{PaneId, PaneRenderSettings, RenderContext};
use hume_engine::types::EditorMode;

use super::Editor;
use super::buffer::Buffer;

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
    /// Split from gutter width ([`Self::pane_gutter_width`]) because every
    /// caller wants one or the other, never reliably both.
    ///
    /// `pid` must name a live, active-tab pane. Every caller reads it from
    /// `active_pane_ids()`, which can never contain a stale id (see
    /// `EngineView::layout`'s privacy: no whole-tree write can install a leaf
    /// the pool doesn't back).
    pub(super) fn resolve_pane_settings(&self, pid: PaneId) -> PaneRenderSettings {
        let pane = &self.view.panes[pid];
        let doc = self.state.buffers.get(pane.buffer_id);
        let indent_guides = doc.overrides.indent_guides(&self.state.settings);
        let is_focused = pid == self.state.focus.id();
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
        }
    }

    /// The gutter width a pane's own providers currently occupy, used to
    /// offset the terminal cursor column past line numbers and other gutter
    /// providers. See [`Self::resolve_pane_settings`] for why this is split
    /// out rather than returned alongside it.
    pub(super) fn pane_gutter_width(&self, pid: PaneId) -> u16 {
        let pane = &self.view.panes[pid];
        let doc = self.state.buffers.get(pane.buffer_id);
        let last_line_idx = doc.text().last_ropey_line();
        super::cursor::gutter_width(pane.providers.gutter_columns(), last_line_idx)
    }

    /// Render one frame into `grid`. Single home for the rope and syntax
    /// lookups shared by the event loop and `render_to_buf`.
    pub(super) fn render_into(&mut self, area: Rect, grid: &mut Grid, ctx: &mut RenderContext) {
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
        let focused_pane_id = self.state.focus.id();
        let draw_dividers = self.state.settings.pane_dividers;
        let focused_bid = self.focused_buffer_id();

        // Split explicitly rather than leaning on closure-capture precision:
        // `view` is borrowed mutably below, so everything else the call needs
        // has to come from a provably disjoint field.
        let Editor {
            state,
            view,
            lsp,
            kitty_enabled,
            ..
        } = self;
        let statusline = crate::statusline::HumeStatusline {
            state,
            lsp,
            kitty_enabled: *kitty_enabled,
            focused_bid,
        };
        let buffers = &state.buffers;
        view.render(
            area,
            grid,
            |bid| buffers.try_get(bid).map(|b| b.text().rope()),
            |bid| {
                buffers
                    .try_get(bid)
                    .and_then(|b| b.syntax.as_ref())
                    .map(|s| s as &dyn hume_engine::providers::SyntaxSpans)
            },
            &statusline,
            focused_pane_id,
            draw_dividers,
            ctx,
        );
    }

    /// The statusline provider over this editor: the fixture the element
    /// tests need, since `HumeStatusline`'s fields are assembled from
    /// disjoint borrows that a test holding a whole `&Editor` doesn't have to
    /// bother splitting.
    #[cfg(test)]
    pub(crate) fn statusline(&self) -> crate::statusline::HumeStatusline<'_> {
        crate::statusline::HumeStatusline {
            state: &self.state,
            lsp: &self.lsp,
            kitty_enabled: self.kitty_enabled,
            focused_bid: self.focused_buffer_id(),
        }
    }

    /// Render the current frame into a [`Grid`] without a live terminal.
    ///
    /// Calls `sync_viewport_dims` + `settle` + `prepare_frame` (the same
    /// three-step sequence `Editor::run`'s loop uses), so pane mirrors are
    /// synced and parse trees are up to date before rendering.
    #[cfg(test)]
    pub(in crate::editor) fn render_to_buf(&mut self, rect: Rect) -> Grid {
        let mut buf = Grid::new(rect.width, rect.height);
        let mut ctx = RenderContext::new();
        self.sync_viewport_dims(rect.width, rect.height);
        self.settle();
        self.prepare_frame(&mut ctx);
        self.render_into(rect, &mut buf, &mut ctx);
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
        let wheel = &mut self.timer_wheel;
        let payloads = &mut self.timer_payloads;
        self.viewport_debounce.retain(|pid, id| {
            let live = panes.contains_key(*pid);
            if !live {
                wheel.cancel(*id);
                payloads.remove(id);
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

    /// Send the clipboard text queued for OSC 52, if any. A no-op without a
    /// terminal (tests, headless `run_keys`), which leaves the text queued.
    pub(in crate::editor::frame) fn flush_osc52(&mut self) {
        let Some(term) = self.tui.terminal() else {
            return;
        };
        if let Some(text) = self.state.clipboard.take_pending_osc52() {
            let _ = hume_platform::terminal::set_clipboard(term, &text);
        }
    }

    /// Sync every pane's viewport dimensions and the frame's geometry
    /// snapshot from the terminal size: the one step that needs the raw
    /// `(width, height)`, so it's split out from `prepare_frame` and called
    /// separately, *before* `Editor::settle()`.
    ///
    /// Must run before `settle()`: `drain_due_timers` fires `OnViewportChange`
    /// off each pane's *current* bounds (`timer_bridge.rs`), so the bounds
    /// have to be current before that drain runs, not after.
    ///
    /// `prepare_frame`'s bottom-band re-partition calls this a second time, from the stored
    /// `last_terminal_area`, after re-syncing the bottom-band views, so a
    /// band-height change `settle()` made (a hook-driven `close-popup!`, a
    /// settle-drained `close-drawer!`) is re-partitioned into `viewport`
    /// before this same frame renders, instead of lagging a frame behind.
    pub(super) fn sync_viewport_dims(&mut self, terminal_width: u16, terminal_height: u16) {
        // Partitioned through the same `EngineView::pane_area` that `render`
        // uses, so viewport dims and drawn rects never disagree even when a
        // tab bar is present.
        let terminal_area = Rect {
            x: 0,
            y: 0,
            width: terminal_width,
            height: terminal_height,
        };
        let pane_area = self.view.pane_area(terminal_area);
        let reserve_seam = self.state.settings.pane_dividers;
        // Compared before the write below overwrites them: an inactive
        // tab's tree never changes on its own (splits/closes only ever
        // touch the active one), so its own panes only need re-partitioning
        // when this same partition actually moved, not on every frame this
        // runs on.
        let geometry_changed =
            self.view.last_pane_area != pane_area || self.view.reserve_seam != reserve_seam;

        // Stored before the write below runs. `resync_viewport_dims` reads
        // these three fields to do the actual per-pane partition and write,
        // the same partition `EngineView::pane_rects`/`pane_rect` recompute
        // from for pane-focus/split commands with no terminal handle between
        // frames. One partition, one write loop, shared with
        // `resync_viewport_dims`'s other caller (`tab::install_live`).
        self.view.last_pane_area = pane_area;
        self.view.last_terminal_area = terminal_area;
        self.view.reserve_seam = reserve_seam;

        // A resize can move the cursor's own display line relative to the
        // viewport (a shorter pane can push it out of view, a narrower one
        // can rewrap it) without the selection itself moving at all. Both
        // `height` and (through the wrap column) `content_width` are
        // geometry facts `EditorState::layout_key` carries, so `frame.rs`'s
        // scroll step derives the reveal from the pane's new dimensions
        // directly rather than this function raising it.
        self.view.resync_viewport_dims();
        // Keeps every background tab's own panes sized to the current
        // terminal too; see `TabStore::inactive_layouts`'s own doc.
        if geometry_changed {
            for layout in self.state.tabs.inactive_layouts() {
                self.view.resync_viewport_dims_for(layout);
            }
        }
    }

    /// Hash of everything [`Self::sync_tabline_view`]'s rebuild depends on:
    /// whether the tab bar is shown at all, and (while it is) the tab
    /// count/order, each tab's `(id, pane, buffer, dirty, path)`, and the
    /// bar's own geometry (a resize must still trigger a rebuild even when
    /// the tab list itself is unchanged, since `scroll` depends on width
    /// too). `buf.path()` is hashed rather than `buf.display_name()`:
    /// `display_name()` allocates a `String`, defeating the point of a
    /// cheap signature, and the two agree on every rename/attach that
    /// actually changes what's drawn (a buffer's dirty marker is covered
    /// separately by `is_dirty()`). Excludes anything that
    /// doesn't change what a rebuild would produce: buffer *content*, for
    /// instance, since neither the label nor `scroll` reads it.
    fn tabline_signature(&self, visible: bool) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = rustc_hash::FxHasher::default();
        visible.hash(&mut hasher);
        if visible {
            let current = self.state.tabs.current();
            current.hash(&mut hasher);
            let bar = self.view.tabbar_area(self.view.last_terminal_area);
            bar.x.hash(&mut hasher);
            bar.width.hash(&mut hasher);
            for &id in self.state.tabs.order() {
                id.hash(&mut hasher);
                let pid = if id == current {
                    self.state.focus.id()
                } else {
                    self.state.tabs.stashed_focus(id)
                };
                pid.hash(&mut hasher);
                let bid = self.view.panes[pid].buffer_id;
                bid.hash(&mut hasher);
                let buf = self.state.buffers.get(bid);
                buf.is_dirty().hash(&mut hasher);
                buf.path().hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    /// Re-sync the tab-bar view from `state.tabs` + each tab's focused
    /// pane's buffer, self-healing every frame, same rationale as
    /// `EditorState::sync_drawer_view`'s own doc: a direct mutation that
    /// bypasses the normal `:tabnew`/`:tabclose` builtins would otherwise
    /// leave a stale row painting for however long it takes the next frame.
    /// Gated on [`Self::tabline_signature`]: an unchanged signature means
    /// the previous frame's `TablineViewState` is already correct, so the
    /// rebuild below (one allocating `display_name()` call per tab, plus
    /// the scroll probe) is skipped on every steady-state frame, which is
    /// almost all of them.
    ///
    /// Needs both `state` (tab order, settings, buffers) and `view` (which
    /// buffer a stashed tab's pane was viewing), unlike `sync_drawer_view`.
    /// That's why this lives on `Editor` rather than on `EditorState`.
    fn sync_tabline_view(&mut self) {
        let visible = match self.state.settings.tabline {
            crate::editor::settings::TablineVisibility::Always => true,
            crate::editor::settings::TablineVisibility::Never => false,
            crate::editor::settings::TablineVisibility::Dynamic => self.state.tabs.len() > 1,
        };

        let signature = self.tabline_signature(visible);
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
            self.state
                .tabline_view
                .set(crate::tabline::TablineViewState::default());
            return;
        }

        let order = self.state.tabs.order();
        let current = self.state.tabs.current();
        let active_index = self.state.tabs.current_pos();

        let tabs: Vec<crate::tabline::TabEntry> = order
            .iter()
            .map(|&id| {
                let pid = if id == current {
                    self.state.focus.id()
                } else {
                    self.state.tabs.stashed_focus(id)
                };
                let bid = self.view.panes[pid].buffer_id;
                let buf = self.state.buffers.get(bid);
                let mut label = buf.display_name();
                if buf.is_dirty() {
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
        // Reads geometry from `tabbar_area`, same as `render`/`tabline_click`:
        // `tab_extents`' own doc explains why all three must agree. Probes
        // via `tab_extents_into` rather than `tab_extents`: one reused
        // `ranges` buffer across every candidate this loop tries, instead of
        // a fresh allocation per candidate.
        let bar = self.view.tabbar_area(self.view.last_terminal_area);
        let mut probe_ranges = Vec::new();
        let mut scroll = 0;
        while scroll < active_index {
            crate::tabline::tab_extents_into(&tabs, scroll, bar.x, bar.width, &mut probe_ranges);
            if active_index < scroll + probe_ranges.len() {
                break;
            }
            scroll += 1;
        }

        self.state
            .tabline_view
            .set(crate::tabline::TablineViewState {
                tabs,
                active_index,
                scroll,
                visible,
            });
    }

    /// Sync all editor-owned state into the engine view, once per frame,
    /// right before `render()`.
    ///
    /// Every caller runs `Editor::settle()` immediately before this. A
    /// settled drain can switch a pane's buffer (picker accept, LSP
    /// goto-definition) or move its selections, so everything here must
    /// read post-settle state. Syncing earlier would pair a stale selection
    /// head with the pane's new rope.
    ///
    /// Order matters twice. Providers that change display-line counts
    /// (signs, inlay hints, virtual lines, EOL text) sync before the scroll
    /// pass, so the cursor is positioned against the layout render will
    /// draw. Cursor-anchored overlays sync after it, because they need the
    /// cursor's final screen cell.
    pub(super) fn prepare_frame(&mut self, ctx: &mut RenderContext) {
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
        self.flush_osc52();

        // Bake scopes interned since the last frame. It must run before the
        // bottom-band sync: `show-popup!` interns its grammar's scopes at
        // dispatch time, and the band resolves them to styles right away.
        // Scopes interned later in this function are baked by the second
        // call at the end.
        self.view.theme.bake_if_stale(&self.view.registry);

        // A settled drain can resize a bottom band (docked popup, drawer)
        // after the pre-settle `sync_viewport_dims`. Re-partition from the
        // settled views so the pane doesn't paint short with blank rows
        // left where the band was. Skipped without terminal geometry
        // (headless callers).
        self.sync_popup_band_view();
        self.state
            .clamp_drawer_scroll_to_terminal(self.view.last_terminal_area.height);
        self.state.sync_drawer_view();
        self.sync_tabline_view();
        let area = self.view.last_terminal_area;
        if area.width > 0 && area.height > 0 {
            self.sync_viewport_dims(area.width, area.height);
        }

        // Nothing below changes which panes the active tab shows.
        let active = self.view.active_pane_ids();

        // Line-number style depends on each pane's current buffer overrides.
        for &pid in &active {
            let buf_id = self.view.panes[pid].buffer_id;
            let ln_style = self
                .state
                .buffers
                .get(buf_id)
                .overrides
                .line_number_style(&self.state.settings);
            self.view.panes[pid]
                .providers
                .sync_line_number_style(ln_style);
        }

        self.sync_all_pane_mirrors(&active);

        // Everything that changes display-line counts or columns, synced
        // before scrolling. Signs set the gutter width and so the wrap
        // column; the other three are `DisplayLineMap` providers. The
        // snapshot predates the scroll, so a line exposed by this frame's
        // scroll gets its hints one frame late. Syncing after the scroll
        // would instead let scroll and render disagree on layout.
        let panes = self.decorated_panes(&active);
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

        let scroll_margin = self.state.settings.scroll_margin;
        for &pid in &active {
            let buf_id = self.view.panes[pid].buffer_id;
            let layout_key = self.state.layout_key(&self.view.panes[pid]);
            let format_key = layout_key.format_key();
            // Per (pane, buffer) state: a pane that just switched buffers
            // reads a fresh entry whose `last_layout_key` is `None`.
            let pbs = &mut self.state.panes.state[pid][buf_id];
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
                format_key,
                scroll_margin,
                reveal,
            );
            self.state.panes.state[pid][buf_id].parked = outcome.parked;
            if pid == self.state.focus.id() {
                ctx.cursor_content_pos = outcome.cursor_screen;
            }

            // A visible-range change arms the `OnViewportChange` debounce
            // timer; the hook fires later from the timer drain.
            let content_lines = self.state.buffers.get(buf_id).text().content_line_count();
            let range =
                super::lsp::introspect::pane_visible_range(&self.view.panes[pid], content_lines);
            if self.last_visible_range.insert(pid, (buf_id, range)) != Some((buf_id, range)) {
                self.debounce_viewport_change(pid);
            }
        }

        // Highlights and line tints only affect painting, so they sync after
        // the scroll from a fresh snapshot of the final viewport.
        let panes = self.decorated_panes(&active);
        self.update_highlight_providers(&panes);
        self.update_line_bg_providers(&panes);

        // Overlays resolve their geometry now, so render only paints. The
        // cursor-anchored ones need the scroll result above.
        self.sync_minibuf_completion_view();
        self.sync_popup_view(ctx);
        self.sync_menu_view(ctx);
        self.sync_completion_menu_view(ctx);
        self.sync_picker_view();

        // Bake scopes interned during this frame (extra highlights, inline
        // diagnostics, a new grammar's captures) before render resolves them.
        self.view.theme.bake_if_stale(&self.view.registry);
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
            let pane = &mut view.panes[pid];
            let bid = pane.buffer_id;
            write_pane_mirror(
                pane,
                state.buffers.get(bid).text(),
                state.panes.state[pid][bid].selections(),
            );
        }
    }

    // ── Engine accessors ──────────────────────────────────────────────────────

    #[cfg(test)]
    pub(in crate::editor) fn viewport(&self) -> &hume_engine::pane::Viewport {
        &self.view.panes[self.state.focus.id()].viewport
    }

    /// How many times `ensure_inline_output_screen` has actually entered the
    /// inline-output terminal bracket (alt-screen toggle + "press any key")
    /// on this `Editor`. Off the event loop this must stay `0` for every
    /// `#:inline-output #t` command dispatched, output or not; see `tui` on
    /// `Editor`. Also lets a test pin an exact count through nested `call!`s
    /// (a re-entry bug shows up as `2`, not just "entered").
    #[cfg(test)]
    pub(in crate::editor) fn inline_output_enter_count(&self) -> usize {
        self.state.inline_output.enter_count()
    }

    #[cfg(test)]
    pub(in crate::editor) fn viewport_mut(&mut self) -> &mut hume_engine::pane::Viewport {
        &mut self.view.panes[self.state.focus.id()].viewport
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
    /// See [`PaneBufferState::parked`]'s own doc.
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
    format_key: hume_engine::display_lines::line_store::FormatKey,
    scroll_margin: usize,
    reveal: bool,
) -> ScrollOutcome {
    // Whatever this pass formats deciding where to scroll, the render pass
    // finds already done: both work through this pane's one store.
    let (mut dlm, viewport) = super::commands::pane_display_lines(doc, pane, format_key);
    // A collapsed split has nothing to scroll and nowhere to put a cursor.
    // Checked before `locate`, which would otherwise format the cursor's
    // line for an answer no one can use, and before `geometry`, which
    // returns `None` for exactly this case.
    let Some(geo) = viewport.geometry(scroll_margin) else {
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
            .distance(top, cursor_pos, geo.height - 1)
            .map(|screen_row| super::cursor::place(viewport, cursor_display_col, screen_row));
        ScrollOutcome {
            cursor_screen,
            parked: true,
        }
    }
}
