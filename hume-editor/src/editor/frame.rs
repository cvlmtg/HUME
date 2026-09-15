//! Per-frame preparation: pane-mirror sync, scroll, and render plumbing.
//!
//! `sync_viewport_dims` (geometry) → `Editor::settle` (advance state) →
//! `prepare_frame` (render prep) is the sequence every frame producer —
//! `Editor::run`'s loop, `render_to_buf` — calls in that order; everything
//! else here is a step `prepare_frame` drives or a helper those steps share.

use hume_grid::{Grid, Rect};

use hume_engine::pane::Pane;
use hume_engine::pipeline::{PaneId, PaneRenderSettings, RenderContext};
use hume_engine::types::EditorMode;

use super::Editor;
use super::buffer::Buffer;

/// Project a `SelectionSet` into an engine pane's head-sorted selection mirror.
///
/// `SelectionSet` stores selections in `start()` order; the engine asserts they
/// are sorted by `head` (see `populate_sorted_sels`).  The two orderings differ
/// whenever a selection is backward (`anchor > head`).  `primary_idx` is
/// re-located after the sort by matching the primary's unique head value.
pub(in crate::editor::frame) fn write_pane_mirror(
    pane: &mut hume_engine::pane::Pane,
    sels: &hume_editing::selection::SelectionSet,
) {
    use hume_engine::types::Selection as EngineSelection;
    let primary_head = sels.primary().head();
    // Sorted after the mirror is filled rather than through a scratch `Vec` of
    // references, so the pane's own storage — reused across frames — is the
    // only buffer involved.
    pane.selections.clear();
    pane.selections
        .extend(sels.iter_sorted().map(|s| EngineSelection {
            anchor: s.anchor(),
            head: s.head(),
        }));
    pane.selections.sort_by_key(|s| s.head);
    pane.primary_idx = pane
        .selections
        .iter()
        .position(|s| s.head == primary_head)
        .unwrap_or(0);
}

impl Editor {
    /// Resolve any pane's render settings.
    ///
    /// `format` is [`EditorState::format_key`](super::EditorState::format_key)
    /// — the single source of truth for wrap_mode / tab_width / whitespace
    /// across all render paths, so this and the scroll pass
    /// (`commands::pane_display_lines`) resolve a bit-identical key for the same
    /// pane. `mode` is a per-focus fact: only the focused pane owns the real
    /// terminal cursor, so it alone gets the live editor mode; other panes are
    /// forced to `Normal` so they don't take Insert's or Extend's cursor
    /// colours. `cursor_is_block` is the separate per-focus resolution that
    /// decides whether either selection head is painted at all — always for
    /// an unfocused pane (no real cursor sits there to stand in for one), and
    /// for the focused pane only when its mode's resolved shape is `Block`.
    ///
    /// Split from gutter width ([`Self::pane_gutter_width`]) because every
    /// caller wants one or the other, never reliably both.
    ///
    /// `pid` must name a live, active-tab pane — every caller reads it from
    /// `active_pane_ids()`, which by construction (see `EngineView::layout`'s
    /// privacy — no whole-tree write can install a leaf the pool doesn't
    /// back) can never contain a stale id.
    pub(super) fn resolve_pane_settings(&self, pid: PaneId) -> PaneRenderSettings {
        let pane = &self.view.panes[pid];
        let doc = self.state.buffers.get(pane.buffer_id);
        let show_indent_guides = doc.overrides.show_indent_guides(&self.state.settings);
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
            show_indent_guides,
            cursor_is_block,
        }
    }

    /// The gutter width a pane's own providers currently occupy — used to
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

    /// The statusline provider over this editor — the fixture the element
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
    /// Calls `sync_viewport_dims` + `settle` + `prepare_frame` — the same
    /// three-step sequence `Editor::run`'s loop uses — so pane mirrors are
    /// synced and parse trees are up to date before rendering. Used by
    /// snapshot tests to lock down styled output without a live terminal.
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

    /// Drop `viewport_debounce`/`last_viewport_key`/`virtual_lines_synced`
    /// entries whose pane no longer exists in `self.view.panes`. A pending
    /// debounce timer is cancelled outright (its `TimerPayload` no-ops via
    /// `queue_viewport_change`'s own liveness check anyway, but there is no
    /// reason to let it sit in the wheel until it fires).
    ///
    /// A pane's line store needs no entry here — it lives on the pane and
    /// dies with it, as do `PaneBufferState::reveal_pending` and
    /// `PaneBufferState::last_layout_key`, which go with the closed pane's
    /// `SecondaryMap` entries.
    fn prune_closed_pane_caches(&mut self) {
        let panes = &self.view.panes;
        self.last_viewport_key
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

    /// Re-apply the terminal's mouse-tracking mode if `mouse-enabled`/
    /// `mouse-select` changed since the last time this ran. A no-op when
    /// nothing changed (the common case, checked every frame) and when no
    /// terminal is attached (tests, headless `run_keys`).
    ///
    /// The comparison-and-update itself doesn't require a live terminal, so
    /// it stays outside the `if let Some(term)` below — this keeps
    /// `applied_mouse_mode` in sync with `state.settings` even headless,
    /// which is what makes the change-detection unit-testable without a
    /// real `SharedTerm`.
    pub(in crate::editor::frame) fn resync_mouse_mode(&mut self) {
        let desired = (
            self.state.settings.mouse_enabled,
            self.state.settings.mouse_select,
        );
        if desired == self.applied_mouse_mode {
            return;
        }
        if let Some(term) = self.tui.terminal() {
            let _ = hume_platform::terminal::set_mouse_mode(term, desired.0, desired.1);
        }
        self.applied_mouse_mode = desired;
    }

    /// Sync every pane's viewport dimensions and the frame's geometry
    /// snapshot from the terminal size — the one step that needs the raw
    /// `(width, height)`, so it's split out from `prepare_frame` and called
    /// separately, *before* `Editor::settle()`.
    ///
    /// Must run before `settle()`: `drain_due_timers` fires `OnViewportChange`
    /// off each pane's *current* bounds (`timer_bridge.rs`), so the bounds
    /// have to be current before that drain runs, not after.
    ///
    /// `prepare_frame`'s step 0 calls this a second time, from the stored
    /// `last_terminal_area`, after re-syncing the bottom-band views — so a
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

        // Stored before the write below runs — `resync_viewport_dims` reads
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
        // can rewrap it) without the selection itself moving at all — both
        // `height` and (through the wrap column) `content_width` are
        // geometry facts `EditorState::layout_key` carries, so `frame.rs`'s
        // scroll step derives the reveal from the pane's new dimensions
        // directly rather than this function raising it.
        self.view.resync_viewport_dims();
    }

    /// Hash of everything [`Self::sync_tabline_view`]'s rebuild depends on:
    /// whether the tab bar is shown at all, and — while it is — the tab
    /// count/order, each tab's `(id, pane, buffer, dirty, path)`, and the
    /// bar's own geometry (a resize must still trigger a rebuild even when
    /// the tab list itself is unchanged, since `scroll` depends on width
    /// too). `buf.path()` is hashed rather than `buf.display_name()` —
    /// `display_name()` allocates a `String`, defeating the point of a
    /// cheap signature, and the two agree on every rename/attach that
    /// actually changes what's drawn (a buffer's dirty marker is covered
    /// separately by `is_dirty()`). Deliberately excludes anything that
    /// doesn't change what a rebuild would produce — buffer *content*, for
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
    /// pane's buffer — self-healing every frame, same rationale as
    /// `EditorState::sync_drawer_view`'s own doc: a direct mutation that
    /// bypasses the normal `:tabnew`/`:tabclose` builtins would otherwise
    /// leave a stale row painting for however long it takes the next frame.
    /// Gated on [`Self::tabline_signature`]: an unchanged signature means
    /// the previous frame's `TablineViewState` is already correct, so the
    /// rebuild below — one allocating `display_name()` call per tab, plus
    /// the scroll probe — is skipped on every steady-state frame, which is
    /// almost all of them.
    ///
    /// Needs both `state` (tab order, settings, buffers) and `view` (which
    /// buffer a stashed tab's pane was viewing), unlike `sync_drawer_view`
    /// — that's why this lives on `Editor` rather than on `EditorState`.
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
            // a normal session): skip every per-tab step below entirely —
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
        // packed window — computed fresh every frame rather than clamped
        // from last frame's value, which this loop's own result never
        // actually depends on: packing from an earlier `scroll` can only
        // reach the same window end or earlier (an earlier start must first
        // fit the tab(s) before it), so "does `active_index` fit starting at
        // `scroll`" is monotone in `scroll` and the smallest passing value
        // is a pure function of `tabs`/`active_index`/`width`. Bounded by
        // `scroll < active_index` even in the degenerate `width == 0` case
        // (startup, before the first real terminal size arrives), where no
        // tab ever fits and the window never grows.
        // Reads geometry from `tabbar_area`, same as `render`/`tabline_click`
        // — `tab_extents`' own doc explains why all three must agree. Probes
        // via `tab_extents_into` rather than `tab_extents` — one reused
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

    /// Prepare the engine pane for rendering by syncing all editor-authoritative
    /// state in one place, once per frame.
    ///
    /// `sync_all_pane_mirrors` is the **single sync point** for `pane.selections`
    /// and `pane.primary_idx` — it covers every active pane (see
    /// [`EngineView::active_pane_ids`](hume_engine::pipeline::EngineView::active_pane_ids)),
    /// in one pass. No other code path writes those fields. It, and
    /// *after* `Editor::settle()` (called by every caller of this function,
    /// immediately before it — see `settle`'s doc) since a settled drain can
    /// switch a pane's `buffer_id` (picker accept, LSP goto-definition) or
    /// move its selections (timer/LSP callbacks) — syncing or scrolling any
    /// earlier would use a stale selection head against the pane's new
    /// buffer, which can be out of bounds for that rope, or leave the new
    /// buffer's cursor unvalidated against the viewport for a frame.
    /// Highlight and statusline shared buffers are also written here,
    /// immediately before every `render()` call. Mode and display settings are
    /// resolved by `render_into`, which runs after this.
    pub(super) fn prepare_frame(&mut self, ctx: &mut RenderContext) {
        // A `RenderContext` is allocated once and reused for every frame, so
        // last frame's cursor cell would otherwise be indistinguishable from
        // one step 4 resolved this frame. Cleared here, filled there.
        ctx.cursor_content_pos = None;
        // Load-bearing rather than tidiness, and for a reason step 3 below is
        // what creates — see `EngineView::begin_frame`.
        self.view.begin_frame();

        // Reclaim viewport-debounce/scroll-key/virtual-line-sync entries
        // for panes closed since the last frame. These live on `Editor`
        // rather than `EditorState.panes` (unlike `jumps`/`render`/
        // `transient`/`state`, which `drop_pane_state` clears directly), so
        // this per-frame sweep is where they get reclaimed instead.
        self.prune_closed_pane_caches();

        // `mouse-enabled`/`mouse-select` are terminal modes, not per-frame
        // render state — `init` (hume-editor/src/lib.rs) only applies them
        // once at startup. This is the per-frame chokepoint that makes a
        // later `:set global mouse-enabled=…` take effect immediately
        // instead of silently doing nothing until restart: it resyncs at
        // the one place the value is consumed, not at every write site.
        self.resync_mouse_mode();

        // Re-bake the theme if any scope was interned since the last bake —
        // catches up on interning from the *previous* frame, from command
        // dispatch between frames (e.g. `:theme`), or from the `settle()`
        // call every caller makes immediately before this one. This frame's
        // own steps (0, 3, 5 below) can themselves intern new scopes — extra
        // highlights, inline diagnostics, virtual lines, a newly attached
        // grammar's capture names — so a second `bake_if_stale` runs at the
        // very end of this function, right before `render_into` gets to
        // resolve anything. Without it, a scope interned mid-frame and
        // resolved by that same frame's render is past the end of `baked`.
        //
        // Must run before step 0: a docked popup attaches its syntax (and
        // interns its grammar's capture-name scopes) at `show-popup!`
        // dispatch time, before this frame's `prepare_frame` — step 0's
        // `sync_popup_band_view` resolves those scopes to concrete styles
        // synchronously while building the band's styled rows, so they must
        // already be baked by the time it runs.
        self.view.theme.bake_if_stale(&self.view.registry);

        // 0. Re-sync the bottom-band views (docked popup, drawer) from their
        //    now-settled models, then re-partition viewport dims from them.
        //    Every caller runs `settle()` immediately before this function, and
        //    a settled drain (`close-popup!` from a hook/callback) or the
        //    pre-dispatch dismissal on any key or mouse event
        //    (mappings/mod.rs, mouse.rs) can change a band's height after
        //    the pre-settle `sync_viewport_dims` ran —
        //    leaving `viewport.height` partitioned against a band `render`
        //    will no longer draw, so the pane paints short and the vacated
        //    rows stay blank until the next event wakes the loop.
        //    Re-partitioning here, from the same settled views render's own
        //    `pane_area` reads, keeps them agreeing. `last_terminal_area` is
        //    fresh: the pre-settle sync wrote it from this frame's
        //    `term.size()`. Skipped when no terminal geometry was ever
        //    established (headless callers relying on `Pane::new` defaults).
        self.sync_popup_band_view();
        self.state.sync_drawer_view();
        self.sync_tabline_view();
        let area = self.view.last_terminal_area;
        if area.width > 0 && area.height > 0 {
            self.sync_viewport_dims(area.width, area.height);
        }

        // The active tab's pane set, fixed for the rest of this frame —
        // nothing between here and `render_into` changes which panes
        // `view.layout` reaches, only settled callbacks before this
        // function was entered could, and `settle()` already ran (every
        // caller runs it immediately before this — see this function's own
        // doc). Steps 1/3/4/5 below all read from this instead of
        // `view.panes` directly.
        let active = self.view.active_pane_ids();

        // 1. Sync line-number style provider for every active pane (depends
        //    on that pane's own buffer overrides). Must run after `settle()`:
        //    a settled drain can switch a pane's `buffer_id` (picker accept,
        //    LSP goto-definition), so syncing any earlier would apply the
        //    just-left buffer's style to the pane's new buffer for a frame.
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

        // 2. Sync selection mirrors for every active pane. Must run after
        //    `settle()`: a settled drain can switch a pane's `buffer_id`
        //    (picker accept, LSP goto-definition) or move its selections
        //    (timer/LSP callbacks), and render (right after this function
        //    returns) reads this mirror against the pane's *current* buffer.
        self.sync_all_pane_mirrors(&active);

        // 3. Sync everything that decides display-line counts/columns for
        //    step 4's `DisplayLineMap`-driven scroll, in this order because none of them
        //    depends on this frame's viewport (a gutter/decoration change
        //    must be visible to the scroll math that positions the cursor
        //    against it, not just to the renderer one step later):
        //      3a. gutter sign data (diagnostics + plugin signs) — decides
        //          gutter width, which decides `Pane::content_width`, which
        //          decides the wrap column.
        //      3b/3c/3d. inlay hints / virtual lines / EOL text — each a
        //          `DisplayLineMap` provider
        //          (`inline_decorations` or `virtual_lines`) that
        //          `DisplayLineMap::ensure_formatted`/`block` reads, so they change wrap
        //          display-line counts and columns the moment they appear.
        //    All four read this one `decorated_panes()` snapshot (see its
        //    doc), taken here rather than after step 4 — so a same-frame
        //    scroll can leave a newly-exposed line's hints/signs unsynced
        //    until next frame. That's a one-frame cosmetic lag that
        //    self-corrects; syncing after scroll instead would let step 4's
        //    `DisplayLineMap` see display-line counts/columns the providers
        //    haven't caught up to yet — the scroll/render/caret disagreement
        //    this ordering avoids.
        let panes = self.decorated_panes(&active);
        self.update_sign_providers(&panes);
        self.update_inlay_hint_providers(&panes);
        self.update_virtual_line_providers(&panes);
        self.update_eol_text_providers(&panes);

        // 4. Scroll every active pane so its primary cursor stays visible.
        //    Must run after `settle()`: a settled drain can switch a pane's
        //    `buffer_id` mid-frame (picker accept, LSP goto-definition), and
        //    this reads buffer_id/rope/cursor together from SSOT, so it
        //    always scrolls the pane's *current* buffer instead of leaving a
        //    just-switched-to buffer's cursor unvalidated against the
        //    viewport for a frame.
        // A pane that left the active set (its tab went to the background)
        // is never visited by the loop below, so its stale entry would
        // otherwise survive untouched — and then match on return, even
        // though nothing observed it while it was hidden. Drop it now so
        // the pane's next visible frame always reads as a change.
        self.last_viewport_key.retain(|pid, _| active.contains(pid));

        let scrolloff = self.state.settings.scrolloff;
        for &pid in &active {
            let buf_id = self.view.panes[pid].buffer_id;
            let format_key = self.state.format_key(&self.view.panes[pid]);
            let layout_key = self.state.layout_key(&self.view.panes[pid]);
            // `reveal_pending`/`last_layout_key` live on the current (pane,
            // buffer)'s own `PaneBufferState` — a pane that switched buffers
            // this frame reads a different, freshly-seeded state (`None`
            // for `last_layout_key`, always differing from a fresh key), so
            // no separate buffer-identity filter is needed here.
            let pbs = &mut self.state.panes.state[pid][buf_id];
            let cursor_char = pbs.selections().primary().head();
            let layout_changed = pbs.last_layout_key.replace(layout_key) != Some(layout_key);
            let reveal_pending = std::mem::take(&mut pbs.reveal_pending) || layout_changed;
            let cursor_screen = scroll_into_view(
                self.state.buffers.get(buf_id),
                &mut self.view.panes[pid],
                cursor_char,
                format_key,
                scrolloff,
                reveal_pending,
            );
            if pid == self.state.focus.id() {
                ctx.cursor_content_pos = cursor_screen;
            }

            // A real visible-range change (scroll command, cursor-follow
            // during typing, or a resize that altered height) debounces
            // OnViewportChange. This is bookkeeping over scroll_into_view's
            // *result*, not part of computing what to render — the hook
            // itself never fires from here, only the coalescer timer gets
            // (re)armed; the actual fire happens later via the async-source
            // drain, same as every other timer. Arming here (after
            // `settle()`'s drain) means a change detected this frame is
            // picked up by *next* frame's drain — one frame later than when
            // this ran pre-drain, immaterial for any nonzero debounce interval.
            // The slot is part of the key, not just the line: a view-led
            // scroll (mouse wheel, `Ctrl+D`) can move entirely within one
            // line's virtual block, which the line alone can't see. A
            // zero-height pane's `top` is never resolved (`scroll_into_view`
            // returns before any `top_at` read for one), so its key no
            // longer changes on its own the way it did while a heal ran
            // unconditionally here — a collapsed pane draws nothing, so that
            // fire was spurious.
            let viewport = &self.view.panes[pid].viewport;
            let top = viewport.top();
            let key = (buf_id, top.line, top.slot, viewport.height);
            if self.last_viewport_key.insert(pid, key) != Some(key) {
                self.debounce_viewport_change(pid);
            }
        }

        // 5. Sync highlight data (search matches, bracket matches, diagnostic
        //    underlines, extra highlights) and line-background tints to
        //    shared Arc buffers read by the highlight/line-bg providers
        //    during rendering. Render-only — no `DisplayLineMap` consumer reads
        //    either one, only the paint stage. A fresh `decorated_panes()`
        //    snapshot here (distinct from step 3's) is what gives these two
        //    the *current* viewport, post-scroll.
        let panes = self.decorated_panes(&active);
        self.update_highlight_providers(&panes);
        self.update_line_bg_providers(&panes);

        // 6. Sync completion-popup view to the shared Arc for `MinibufCompletionOverlay`.
        self.sync_minibuf_completion_view();

        // 7. Sync the cursor-anchored popup, menu, LSP-completion-menu, and
        //    picker overlay views. Their geometry needs step 4's scroll
        //    result (`ctx.cursor_content_pos`) or the current-frame
        //    `last_pane_area`/`pane_rect` — both only settled after step 0
        //    re-partitions and step 4 scrolls — which is why these stay
        //    here while the bottom bands (docked popup, drawer) sync in
        //    step 0 instead: those have no cursor-relative geometry, only
        //    the settled model, so they don't need to wait on scroll.
        self.sync_popup_view(ctx);
        self.sync_menu_view(ctx);
        self.sync_completion_menu_view(ctx);
        self.sync_picker_view();

        // Second bake — see the comment on the early call above. Cheap when
        // nothing changed (one `usize` comparison); catches every scope this
        // frame's own steps interned, so `render_into` never resolves against
        // a `ScopeId` past the end of `baked`.
        self.view.theme.bake_if_stale(&self.view.registry);
    }

    /// Sync every active-tab pane's selection mirror from the authoritative
    /// `pane_state`.
    ///
    /// The engine requires `pane.selections` sorted by `head` (not by `start()` as
    /// `SelectionSet` stores internally); `primary_idx` is re-located by matching
    /// the primary's head value after the sort.  This is the **single sync point** —
    /// no other code path writes `pane.selections` or `pane.primary_idx`.
    ///
    /// Called once per frame from `prepare_frame`, after the async/Steel
    /// drains and before `render()`, passing the same `active_pane_ids()`
    /// snapshot `prepare_frame` already computed for its other steps rather
    /// than recomputing it here too. Tests that need the mirror without a
    /// full frame call this directly, passing `ed.view.active_pane_ids()`.
    pub(in crate::editor) fn sync_all_pane_mirrors(&mut self, active: &[PaneId]) {
        let state = &mut self.state;
        let view = &mut self.view;
        for &pid in active {
            let pane = &mut view.panes[pid];
            if let Some(pbs) = state.panes.buffer_state(pid, pane.buffer_id) {
                write_pane_mirror(pane, pbs.selections());
            }
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
    /// `#:inline-output #t` command dispatched, output or not — see `tui` on
    /// `Editor`. Also lets a test pin an exact count through nested `call!`s
    /// (a re-entry bug shows up as `2`, not just "entered").
    #[cfg(test)]
    pub(in crate::editor) fn inline_output_enter_count(&self) -> usize {
        self.state.inline_output.enter_count()
    }

    pub(in crate::editor) fn viewport_mut(&mut self) -> &mut hume_engine::pane::Viewport {
        &mut self.view.panes[self.state.focus.id()].viewport
    }
}

/// Scroll the pane viewport so `cursor_char` stays within the visible area,
/// and report where the cursor ended up on screen (pane-relative, before the
/// gutter). `None` for a viewport with no display lines to place it in, or
/// when the cursor has scrolled out of view (see below) — a legitimate
/// state, not a bug: the cursor can only occupy content display lines, so a
/// pure view scroll into a virtual-line block can carry the viewport
/// further than the cursor can follow. The terminal caret is simply hidden
/// until an ordinary cursor motion resyncs the view.
///
/// Calls both the vertical (`reveal`) and horizontal (`reveal_horizontal`)
/// verbs in one shot, over a single display-line map — so the two agree on
/// the display-line list by construction, and a line's format is reused
/// across them. The cursor is resolved exactly once here, for both plus the
/// terminal-cursor placement: scrolling only ever *writes* the viewport, and
/// the display-line map holds no viewport, so no arm below can change what
/// `locate` already answered.
///
/// `reveal_pending` is `PaneBufferState::reveal_pending`'s value for this
/// frame, already taken by the caller — see that field's own doc for why the
/// vertical `reveal` correction runs only when it's `true`, falling back to
/// a plain forward walk from `top` otherwise (resolved via `Viewport::top_at`
/// either way — `reveal` resolves its own), and what that leaves hidden.
fn scroll_into_view(
    doc: &Buffer,
    pane: &mut Pane,
    cursor_char: hume_rope::offset::CharOffset,
    format_key: hume_engine::display_lines::line_store::FormatKey,
    scrolloff: usize,
    reveal_pending: bool,
) -> Option<(u16, u16)> {
    // Whatever this pass formats deciding where to scroll, the render pass
    // finds already done — both work through this pane's one store.
    let (mut dlm, viewport) = super::commands::pane_display_lines(doc, pane, format_key);
    // A collapsed split has nothing to scroll and nowhere to put a cursor.
    // Checked before `locate`, which would otherwise format the cursor's
    // line for an answer no one can use — and before `geometry`, which
    // returns `None` for exactly this case.
    let geo = viewport.geometry(scrolloff)?;
    let (cursor_pos, cursor_display_col) = dlm.locate(cursor_char);
    // Horizontal scroll is its own axis (a fixed margin, no `scrolloff`, no
    // document-edge special-casing — see `reveal_horizontal`'s own doc) and
    // has no snap-back to guard against, so it always runs: a
    // same-display-line cursor move (`l` on a long unwrapped line) changes
    // the column without changing `cursor_pos`, and gating this on the same
    // `reveal_pending` the vertical arm below reads would leave it stale for
    // exactly that case.
    viewport.reveal_horizontal(&mut dlm, cursor_display_col);
    if reveal_pending {
        let screen_row = viewport.reveal(&mut dlm, geo, cursor_pos);
        Some(super::cursor::place(
            viewport,
            cursor_display_col,
            screen_row,
        ))
    } else {
        // `reveal_horizontal` just guaranteed `cursor_display_col >=
        // horizontal_offset`, so this is `cursor::content_pos` minus the two
        // checks it exists to make for a caller that hasn't already done
        // them — only its forward walk is left to redo.
        let top = viewport.top_at(&mut dlm);
        let screen_row = dlm.distance(top, cursor_pos, geo.height - 1)?;
        Some(super::cursor::place(
            viewport,
            cursor_display_col,
            screen_row,
        ))
    }
}
