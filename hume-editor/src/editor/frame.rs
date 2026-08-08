//! Per-frame preparation: pane-mirror sync, scroll, and render plumbing.
//!
//! `sync_viewport_dims` (geometry) → `Editor::settle` (advance state) →
//! `prepare_frame` (render prep) is the sequence every frame producer —
//! `Editor::run`'s loop, `render_to_buf` — calls in that order; everything
//! else here is a step `prepare_frame` drives or a helper those steps share.

use std::ops::Range;

use hume_engine::pane::Pane;
use hume_engine::pipeline::{BufferId, PaneId, PaneRenderSettings, RenderContext};
use hume_engine::types::EditorMode;

use super::Editor;
use super::buffer::Buffer;
use crate::settings::EditorSettings;

/// Project a `SelectionSet` into an engine pane's head-sorted selection mirror.
///
/// `SelectionSet` stores selections in `start()` order; the engine asserts they
/// are sorted by `head` (see `populate_sorted_sels`).  The two orderings differ
/// whenever a selection is backward (`anchor > head`).  `primary_idx` is
/// re-located after the sort by matching the primary's unique head value.
pub(super) fn write_pane_mirror(
    pane: &mut hume_engine::pane::Pane,
    sels: &hume_editing::selection::SelectionSet,
) {
    use hume_engine::types::Selection as EngineSelection;
    let primary_head = sels.primary().head();
    pane.selections.clear();
    pane.selections.extend(
        sels.iter_head_sorted()
            .into_iter()
            .map(|s| EngineSelection {
                anchor: s.anchor(),
                head: s.head(),
            }),
    );
    pane.primary_idx = pane
        .selections
        .iter()
        .position(|s| s.head == primary_head)
        .unwrap_or(0);
}

impl Editor {
    /// Resolve any pane's render settings and gutter width.
    ///
    /// Returns `(PaneRenderSettings, gutter_w)`. Single source of truth for
    /// wrap_mode / tab_width / whitespace settings across all render paths.
    /// `tab_width`, `whitespace`, and `wrap_mode` all resolve from that
    /// pane's buffer overrides against the global settings — `wrap_mode`
    /// additionally checks the pane's own override first (see
    /// `commands::effective_wrap_mode`), since two panes on the same buffer
    /// may wrap differently once `:wrap`/`:set pane wrap-mode=…` pins one.
    /// `mode` is a per-focus fact: only the focused pane owns the real
    /// terminal cursor, so it alone gets the live editor mode; other panes
    /// are forced to a block-cursor mode so their fake cursor stays visible
    /// instead of turning transparent.
    pub(super) fn resolve_pane_settings(&self, pid: PaneId) -> (PaneRenderSettings, u16) {
        let pane = &self.view.panes[pid];
        let doc = self.state.buffers.get(pane.buffer_id);
        let last_line_idx = doc.text().last_ropey_line();
        let gutter_w = super::cursor::gutter_width(pane.providers.gutter_columns(), last_line_idx);
        let wrap_mode = super::commands::effective_wrap_mode(doc, &self.state.settings, pane)
            .resolve(pane.content_width(last_line_idx));
        let tab_width = doc.overrides.tab_width(&self.state.settings);
        let whitespace = doc.overrides.whitespace(&self.state.settings);
        let show_indent_guides = doc.overrides.show_indent_guides(&self.state.settings);
        let mode = if pid == self.state.focused_pane_id {
            self.state.mode()
        } else {
            EditorMode::Normal
        };
        (
            PaneRenderSettings {
                mode,
                wrap_mode,
                tab_width,
                whitespace,
                show_indent_guides,
            },
            gutter_w,
        )
    }

    /// Render one frame into `buf`. Single home for the rope / syntax /
    /// pane-settings closures shared by the event loop and `render_to_buf`.
    pub(super) fn render_into(
        &self,
        area: ratatui::layout::Rect,
        buf: &mut ratatui::buffer::Buffer,
        ctx: &mut RenderContext,
    ) {
        // The statusline provider borrows `self` immutably; it's built here so
        // its lifetime is tied to this call, not stored across the draw closure.
        let statusline = crate::ui::statusline::HumeStatusline { editor: self };
        self.view.render(
            area,
            buf,
            |bid| self.state.buffers.try_get(bid).map(|b| b.text().rope()),
            |bid| {
                self.state
                    .buffers
                    .try_get(bid)
                    .and_then(|b| b.syntax.as_ref())
                    .map(|s| s as &dyn hume_engine::providers::SyntaxSpans)
            },
            |pid| self.resolve_pane_settings(pid).0,
            &statusline,
            self.state.focused_pane_id,
            self.state.settings.pane_dividers,
            ctx,
        );
    }

    /// Render the current frame into a ratatui `Buffer` without a live terminal.
    ///
    /// Calls `sync_viewport_dims` + `settle` + `prepare_frame` — the same
    /// three-step sequence `Editor::run`'s loop uses — so pane mirrors are
    /// synced and parse trees are up to date before rendering. Used by
    /// snapshot tests to lock down styled output without a live terminal.
    #[cfg(test)]
    pub(crate) fn render_to_buf(&mut self, rect: ratatui::layout::Rect) -> ratatui::buffer::Buffer {
        let mut buf = ratatui::buffer::Buffer::empty(rect);
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
    /// `queue_viewport_change`'s own liveness check anyway, but there is
    /// no reason to let it sit in the wheel until it fires).
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
    pub(super) fn resync_mouse_mode(&mut self) {
        let desired = (
            self.state.settings.mouse_enabled,
            self.state.settings.mouse_select,
        );
        if desired == self.applied_mouse_mode {
            return;
        }
        if let Some(term) = &self.terminal {
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
    pub(super) fn sync_viewport_dims(&mut self, terminal_width: u16, terminal_height: u16) {
        // Shared rect list every per-pane step below drives off — partitioned
        // through the same `EngineView::pane_area` that `render` uses, so
        // viewport dims and drawn rects never disagree even when a tab bar is
        // present.
        let terminal_area = ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: terminal_width,
            height: terminal_height,
        };
        let pane_area = self.view.pane_area(terminal_area);
        let reserve_seam = self.state.settings.pane_dividers;
        let mut rects = Vec::new();
        self.view
            .layout
            .collect_rects_into(pane_area, reserve_seam, &mut rects);

        for &(pid, rect) in &rects {
            let vp = &mut self.view.panes[pid].viewport;
            vp.width = rect.width;
            vp.height = rect.height;
        }

        // Stored so pane-focus/split commands, which have no terminal handle
        // between frames, can recompute geometry from these via
        // `EngineView::pane_rects`/`pane_rect` rather than trusting a stored
        // rect list.
        self.view.last_pane_area = pane_area;
        self.view.last_terminal_area = terminal_area;
        self.view.reserve_seam = reserve_seam;
    }

    /// Prepare the engine pane for rendering by syncing all editor-authoritative
    /// state in one place, once per frame.
    ///
    /// `sync_all_pane_mirrors` is the **single sync point** for `pane.selections`
    /// and `pane.primary_idx` — it covers every pane in one pass.  No other code
    /// path writes those fields.  It, and the scroll pass right after it, run
    /// *after* `Editor::settle()` (called by every caller of this function,
    /// immediately before it — see `settle`'s doc) since a settled drain can
    /// switch a pane's `buffer_id` (picker accept, LSP goto-definition) or
    /// move its selections (timer/LSP callbacks) — syncing or scrolling any
    /// earlier would use a stale selection head against the pane's new
    /// buffer, which can be out of bounds for that rope, or leave the new
    /// buffer's cursor unvalidated against the viewport for a frame.
    /// Highlight and statusline shared buffers are also written here,
    /// immediately before every `render()` call. Mode and display settings
    /// are resolved lazily via the `get_pane_settings` closure passed to
    /// `render()`.
    pub(super) fn prepare_frame(&mut self, ctx: &mut RenderContext) {
        // A `RenderContext` is allocated once and reused for every frame, so
        // last frame's cursor cell would otherwise be indistinguishable from
        // one step 4 resolved this frame. Cleared here, filled there.
        ctx.cursor_screen = None;

        // Reclaim viewport-debounce/scroll-key/virtual-line-sync cache
        // entries for panes closed since the last frame. These three live on
        // `Editor` rather than `EditorState.panes` (unlike `jumps`/`render`/
        // `transient`/`state`, which `drop_pane_state` clears directly), so
        // this per-frame sweep is where they get reclaimed instead.
        self.prune_closed_pane_caches();

        // `mouse-enabled`/`mouse-select` are terminal modes, not per-frame
        // render state — `init` (hume-editor/src/lib.rs) only applies them
        // once at startup. This is the per-frame chokepoint that makes a
        // later `:set global mouse-enabled=…` take effect immediately
        // instead of silently doing nothing until restart (see L2 in
        // docs/LESSONS.md: resync self-triggers at the one place the value
        // is consumed, not at every write site).
        self.resync_mouse_mode();

        // Re-bake the theme if any scope was interned since the last bake —
        // catches up on interning from the *previous* frame, from command
        // dispatch between frames (e.g. `:theme`), or from the `settle()`
        // call every caller makes immediately before this one. This frame's
        // own steps (3, 5 below) can themselves intern new scopes — extra
        // highlights, inline diagnostics, virtual lines, a newly attached
        // grammar's capture names — so a second `bake_if_stale` runs at the
        // very end of this function, right before `render_into` gets to
        // resolve anything. Without it, a scope interned mid-frame and
        // resolved by that same frame's render is past the end of `baked`.
        self.view.theme.bake_if_stale(&self.view.registry);

        // 1. Sync line-number style provider for every pane (depends on that
        //    pane's own buffer overrides). Must run after `settle()`: a
        //    settled drain can switch a pane's `buffer_id` (picker accept,
        //    LSP goto-definition), so syncing any earlier would apply the
        //    just-left buffer's style to the pane's new buffer for a frame.
        //    Iterates a fresh pane-id snapshot (not a frame-start rect list)
        //    since a drained callback may have closed a pane.
        for pid in self.view.panes.keys().collect::<Vec<_>>() {
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

        // 2. Sync selection mirrors for every pane. Must run after
        //    `settle()`: a settled drain can switch a pane's `buffer_id`
        //    (picker accept, LSP goto-definition) or move its selections
        //    (timer/LSP callbacks), and render (right after this function
        //    returns) reads this mirror against the pane's *current* buffer.
        self.sync_all_pane_mirrors();

        // 3. Sync everything that decides row counts/columns for step 4's
        //    `RowMap`-driven scroll, in this order because none of them
        //    depends on this frame's viewport (a gutter/decoration change
        //    must be visible to the scroll math that positions the cursor
        //    against it, not just to the renderer one step later):
        //      3a. gutter sign data (diagnostics + plugin signs) — decides
        //          gutter width, which decides `Pane::content_width`, which
        //          decides the wrap column.
        //      3b/3c/3d. inlay hints / virtual lines / EOL text — each a
        //          `RowMap` provider
        //          (`inline_decorations` or `virtual_lines`) that
        //          `RowMap::format_line`/`block` reads, so they change wrap
        //          row counts and columns the moment they appear.
        //    Tradeoff: 3a/3b/3d scope their own work to
        //    `visible_char_range`/`visible_line_range`, which read
        //    `viewport.top_line` — so a same-frame scroll (step 4) can leave
        //    a newly-exposed line's hints/signs unsynced until next frame.
        //    That's a one-frame cosmetic lag that self-corrects; syncing
        //    after scroll instead would let step 4's `RowMap` see row
        //    counts/columns the providers haven't caught up to yet — the
        //    scroll/render/caret disagreement this ordering avoids.
        //    `update_virtual_line_providers` has no viewport dependency, so
        //    its position here is unconditional either way.
        self.update_sign_providers();
        self.update_inlay_hint_providers();
        self.update_virtual_line_providers();
        self.update_eol_text_providers();

        // 4. Scroll every pane so its primary cursor stays visible. Must run
        //    after `settle()`: a settled drain can switch a pane's `buffer_id`
        //    mid-frame (picker accept, LSP goto-definition), and this reads
        //    buffer_id/rope/cursor together from SSOT, so it always scrolls
        //    the pane's *current* buffer instead of leaving a just-switched-to
        //    buffer's cursor unvalidated against the viewport for a frame.
        //    Iterates a fresh pane-id snapshot (not `sync_viewport_dims`'
        //    frame-start rect list) since a drained callback may have closed
        //    a pane.
        let scrolloff = self.state.settings.scrolloff;
        let pane_ids: Vec<PaneId> = self.view.panes.keys().collect();
        for pid in pane_ids {
            let buf_id = self.view.panes[pid].buffer_id;
            let cursor_char = self.state.panes.state[pid][buf_id]
                .selections
                .primary()
                .head();
            let cursor_screen = scroll_into_view(
                self.state.buffers.get(buf_id),
                &self.state.settings,
                &mut self.view.panes[pid],
                cursor_char,
                &mut ctx.cursor_format,
                scrolloff,
            );
            if pid == self.state.focused_pane_id {
                ctx.cursor_screen = cursor_screen;
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
            let viewport = &self.view.panes[pid].viewport;
            let key = (viewport.top_line, viewport.height);
            if self.last_viewport_key.insert(pid, key) != Some(key) {
                self.debounce_viewport_change(pid);
            }
        }

        // 5. Sync highlight data (search matches, bracket matches, diagnostic
        //    underlines, extra highlights) and line-background tints to
        //    shared Arc buffers read by the highlight/line-bg providers
        //    during rendering. Render-only — no `RowMap` consumer reads
        //    either one, only the paint stage.
        self.update_highlight_providers();
        self.update_line_bg_providers();

        // 6. Sync completion-popup view to the shared Arc for `MinibufCompletionOverlay`.
        self.sync_minibuf_completion_view();

        // 7. Sync the popup-, menu-, LSP-completion-menu-, and
        //    picker-overlay views. Their geometry needs the focused pane's
        //    current-frame rect via `EngineView::pane_rect` (popup/menu/
        //    completion) or `last_pane_area` directly (picker) — both
        //    written by `sync_viewport_dims`, called by every caller of
        //    this function before it.
        self.sync_popup_view(ctx);
        self.sync_popup_band_view();
        self.sync_menu_view(ctx);
        self.sync_completion_menu_view(ctx);
        self.sync_picker_view();
        // The drawer has no cursor-relative geometry, so it doesn't need
        // step 7's ordering — but it's synced here unconditionally anyway
        // (self-healing), on top of every direct mutation-site call, so the
        // view can never drift from `state.config.drawer` for a frame. See
        // `EditorState::sync_drawer_view`'s doc.
        self.state.sync_drawer_view();

        // Second bake — see the comment on the early call above. Cheap when
        // nothing changed (one `usize` comparison); catches every scope this
        // frame's own steps interned, so `render_into` never resolves against
        // a `ScopeId` past the end of `baked`.
        self.view.theme.bake_if_stale(&self.view.registry);
    }

    /// Sync every engine pane's selection mirror from the authoritative `pane_state`.
    ///
    /// The engine requires `pane.selections` sorted by `head` (not by `start()` as
    /// `SelectionSet` stores internally); `primary_idx` is re-located by matching
    /// the primary's head value after the sort.  This is the **single sync point** —
    /// no other code path writes `pane.selections` or `pane.primary_idx`.
    ///
    /// Called once per frame from `prepare_frame`, after the async/Steel
    /// drains and before `render()`.
    pub(crate) fn sync_all_pane_mirrors(&mut self) {
        let state = &mut self.state;
        let view = &mut self.view;
        for (pid, pane) in view.panes.iter_mut() {
            if let Some(pbs) = state
                .panes
                .state
                .get(pid)
                .and_then(|m| m.get(pane.buffer_id))
            {
                write_pane_mirror(pane, &pbs.selections);
            }
        }
    }

    // ── Engine accessors ──────────────────────────────────────────────────────

    #[cfg(test)]
    pub(crate) fn viewport(&self) -> &hume_engine::pane::ViewportState {
        &self.view.panes[self.state.focused_pane_id].viewport
    }

    /// `true` if `ensure_inline_output_screen` has ever actually entered the
    /// inline-output terminal bracket (alt-screen toggle + "press any key")
    /// on this `Editor`. Off the event loop this must stay `false` for every
    /// `#:inline-output #t` command dispatched, output or not — see
    /// `tui_active` on `Editor`.
    #[cfg(all(test, unix))]
    pub(crate) fn inline_output_entered(&self) -> bool {
        self.state.inline_output_entered
    }

    pub(crate) fn viewport_mut(&mut self) -> &mut hume_engine::pane::ViewportState {
        &mut self.view.panes[self.state.focused_pane_id].viewport
    }

    /// Pane `pid`'s visible viewport as a line range, before any clamping to
    /// the buffer's actual line count — the shared basis for
    /// [`Self::visible_char_range`] and [`Self::visible_line_range`]. These
    /// render-side helpers deliberately return a one-row *superset* of it
    /// (its end plus one more row) — cheap over-fetch beats a wrap-aware
    /// exact bound for a bulk store slice. Both this range and
    /// `lsp::introspect::pane_visible_range` are end-exclusive, so the two
    /// conventions differ only in that one-row slack, not in
    /// inclusive-vs-exclusive — they still don't share an implementation,
    /// since one clamps to `content_lines` and the other to the ropey-domain
    /// line count.
    fn visible_line_bounds(&self, pid: PaneId) -> Range<usize> {
        let vp = &self.view.panes[pid].viewport;
        let top_line = vp.top_line;
        top_line..(top_line + vp.height as usize)
    }

    /// The char range of `bid`'s content currently visible in pane `pid` —
    /// shared by every per-frame write side that pulls a bounded slice from a
    /// Rust-side store (diagnostics, decorations) instead of the whole buffer.
    pub(super) fn visible_char_range(&self, pid: PaneId, bid: BufferId) -> Range<usize> {
        let bounds = self.visible_line_bounds(pid);
        let text = self.state.buffers.get(bid).text();
        let top_char = text.line_to_char(bounds.start.min(text.last_ropey_line()));
        let end_char = hume_editing::lines::line_end_exclusive(text, bounds.end);
        top_char..end_char
    }

    /// The line range of `bid`'s content currently visible in pane `pid`
    /// (end-exclusive) — used by line-indexed stores (gutter signs) instead
    /// of the char-offset stores' [`Self::visible_char_range`].
    pub(super) fn visible_line_range(&self, pid: PaneId, bid: BufferId) -> Range<usize> {
        let bounds = self.visible_line_bounds(pid);
        let text = self.state.buffers.get(bid).text();
        bounds.start.min(text.last_ropey_line())..(bounds.end + 1).min(text.ropey_line_count())
    }
}

/// Scroll the pane viewport so `cursor_char` stays within the visible area, and
/// report where the cursor ended up on screen (pane-relative, before the
/// gutter). `None` for a viewport with no rows to place it in.
///
/// Calls the clamp and both the vertical and horizontal `ensure_cursor_visible`
/// helpers in one shot, over a single row map — so the three agree on the row
/// list by construction, and a line's format is reused across them. The cursor
/// is resolved exactly once here, for all three plus the terminal-cursor
/// placement: scrolling only ever *writes* the viewport, and the row map holds
/// no viewport, so no arm below can change what `locate` already answered.
fn scroll_into_view(
    doc: &Buffer,
    settings: &EditorSettings,
    pane: &mut Pane,
    cursor_char: usize,
    scratch: &mut hume_engine::format::FormatScratch,
    scrolloff: usize,
) -> Option<(u16, u16)> {
    use super::scroll;
    let (mut rm, viewport) = super::commands::pane_row_map_mut(doc, settings, pane, scratch);
    // Self-heal a viewport top left stale by a write site that doesn't
    // validate it (`recall_scroll`, an LSP jump) before the cursor-follow
    // logic below reads it — see `clamp_viewport_top`'s doc.
    scroll::clamp_viewport_top(viewport, &mut rm);
    // A collapsed split has nothing to scroll and nowhere to put a cursor.
    // Checked before `locate`, which would otherwise scan the cursor's line
    // for an answer no one can use.
    if viewport.height == 0 {
        return None;
    }
    let (cursor_pos, cursor_col) = rm.locate(cursor_char);
    let screen_row = scroll::ensure_cursor_visible(viewport, &mut rm, cursor_pos, scrolloff);
    scroll::ensure_cursor_visible_horizontal(viewport, &mut rm, cursor_col);
    screen_row.map(|row| super::cursor::place(viewport, cursor_col, row))
}
