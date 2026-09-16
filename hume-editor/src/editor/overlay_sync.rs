//! Per-frame sync of the popup / docked-popup / menu / LSP-completion-menu /
//! picker overlay views into their shared `Arc` buffers. The docked popup
//! (`sync_popup_band_view`) syncs from `prepare_frame`'s step 0 — it only
//! needs `last_terminal_area`, which the pre-settle `sync_viewport_dims`
//! already wrote this frame. The cursor-anchored overlays (`sync_popup_view`
//! and the rest of this module) need the focused pane's post-scroll rect
//! instead, so they sync later, from step 7.

use hume_engine::pipeline::RenderContext;

use super::Editor;

impl Editor {
    /// Write the current completion state into the shared `MinibufCompletionView`
    /// so `MinibufCompletionOverlay` can render it during this frame.
    ///
    /// Called from `prepare_frame` after highlight data is synced.
    pub(super) fn sync_minibuf_completion_view(&self) {
        // Skip the write-lock when both sides are already None — common case
        // while no popup is open.
        if self.state.minibuf_completion.is_none()
            && self.state.views.minibuf_completion.read().is_none()
        {
            return;
        }
        let view = self.state.minibuf_completion.as_ref().map(|state| {
            let anchor_x = self
                .state
                .minibuf
                .as_ref()
                .map(|mb| mb.cursor_x_at(state.span_start))
                .unwrap_or(0);
            hume_ui::completion_overlay::MinibufCompletionView {
                rows: state.rows.clone(),
                selected: state.selected,
                anchor_x,
                border: self.state.settings.popup_border,
            }
        });
        self.state.views.minibuf_completion.set(view);
    }

    /// The focused pane's primary cursor position — the anchor char for
    /// [`Self::sync_popup_view`] and [`Self::sync_menu_view`] (unlike the
    /// LSP completion menu, which anchors at the session's token-start
    /// char instead, via a separately-computed `anchor_char`).
    fn focused_cursor_char(&self) -> hume_rope::offset::CharOffset {
        let pid = self.state.focus.id();
        self.state.panes.state[pid][self.focused_buffer_id()]
            .selections()
            .primary()
            .head()
    }

    /// Screen anchor (absolute cell) + containing pane + text-column budget
    /// for the focused pane, given an arbitrary buffer char position —
    /// shared by [`Self::sync_popup_view`], [`Self::sync_menu_view`], and
    /// the LSP completion menu (each passes a different `anchor_char`).
    /// `None` when the pane has no rect yet or `anchor_char` isn't
    /// currently visible.
    fn popup_placement(
        &mut self,
        ctx: &mut RenderContext,
        anchor_char: hume_rope::offset::CharOffset,
    ) -> Option<hume_ui::popup::PopupPlacement> {
        let focused = self.state.focus.id();
        let pane_rect = self.view.pane_rect(focused)?;
        let gutter_w = self.pane_gutter_width(focused);
        let content_width = pane_rect.width.saturating_sub(gutter_w);
        // Step 6 (`scroll_into_view`) already resolved the focused cursor's
        // screen cell this frame, via the same locate/distance walk
        // `content_pos` runs below — nothing between steps 6 and 10 moves the
        // cursor or the viewport, so the two callers anchored at the live
        // cursor (`sync_popup_view`, `sync_menu_view`) can reuse it instead of
        // re-walking the display-line list (a full per-line format in wrap mode).
        let (content_x, row) = match ctx.cursor_content_pos {
            Some(cell) if anchor_char == self.focused_cursor_char() => cell,
            _ => {
                // Every read of `self` the map needs resolves before the pane
                // is borrowed mutably; the viewport comes back out of
                // `pane_display_lines`'s own split rather than being held across it.
                let bid = self.focused_buffer_id();
                let key = self.state.format_key(&self.view.panes[focused]);
                let Editor { state, view, .. } = self;
                let (mut dlm, vp) = super::commands::pane_display_lines(
                    state.buffers.get(bid),
                    &mut view.panes[focused],
                    key,
                );
                super::cursor::content_pos(vp, &mut dlm, anchor_char)?
            }
        };
        let anchor = super::mouse::content_pos_to_screen(content_x, row, gutter_w, pane_rect);
        Some(hume_ui::popup::PopupPlacement {
            anchor,
            pane_rect,
            content_width,
        })
    }

    /// Write the current *cursor-anchored* popup content into the shared
    /// `PopupState` Arc so `PopupOverlay` can render it during this frame.
    /// Geometry (wrap width, flip/clamp position) is resolved fresh every
    /// frame against the focused pane's *current* rect — never pre-computed
    /// at `show-popup!` call time — so a resize or scroll never leaves it
    /// stale. A docked popup (`PopupLayout::Docked`) is handled by
    /// [`Self::sync_popup_band_view`] instead — this clears the popup view
    /// slot for that case, same as when no popup is open at all.
    ///
    /// Called from `prepare_frame` after `last_pane_area` is set (step 10):
    /// `EngineView::pane_rect` reads that field, so calling this any earlier
    /// would position against the previous frame's geometry.
    pub(super) fn sync_popup_view(&mut self, ctx: &mut RenderContext) {
        let is_cursor = matches!(
            self.state.config.popup.as_ref().map(|m| &m.layout),
            Some(hume_ui::popup::PopupLayout::Cursor)
        );
        if !is_cursor {
            if self.state.views.popup.read().is_some() {
                self.state.views.popup.set(None);
            }
            return;
        }

        let Some(placement) = self.popup_placement(ctx, self.focused_cursor_char()) else {
            self.state.views.popup.set(None);
            return;
        };
        let border = self.state.settings.popup_border;
        let theme = &self.view.theme;
        let model = self
            .state
            .config
            .popup
            .as_mut()
            .expect("popup present: is_cursor checked above");
        // Read before the `&mut` below — a second `self.state.config.popup`
        // borrow once `content` is live would conflict with it.
        let scroll = model.scroll;
        let content = model.content_mut(theme);
        let resolved = hume_ui::popup::resolve_popup(content, placement, scroll, border);

        self.state.views.popup.set(Some(resolved));
    }

    /// Write the current *docked* popup content into the shared
    /// `PopupBandState` Arc so `PopupBandWidget` can render it during this
    /// frame — the `PopupLayout::Docked` counterpart of
    /// [`Self::sync_popup_view`]. Unlike the cursor layout, geometry isn't
    /// resolved here: only content (wrapped lines + scroll clamp), mirroring
    /// the drawer's chrome contract — the engine resolves the band's actual
    /// position/height from `height(max)` at render time.
    ///
    /// Wraps against `last_terminal_area` (the raw, un-subtracted terminal
    /// area), not `last_pane_area` — the band spans the full width the
    /// engine will actually render into (`EngineView::render`'s bottom-band
    /// block), same convention `drawer_visible_rows` already relies on for
    /// its height ceiling.
    pub(super) fn sync_popup_band_view(&mut self) {
        let is_docked = matches!(
            self.state.config.popup.as_ref().map(|m| &m.layout),
            Some(hume_ui::popup::PopupLayout::Docked)
        );
        if !is_docked {
            if self.state.views.popup_band.read().is_some() {
                self.state.views.popup_band.set(None);
            }
            return;
        }

        let area = self.view.last_terminal_area;
        let max_rows = hume_engine::pipeline::EngineView::bottom_band_max(area.height);
        let border = self.state.settings.popup_border;
        let theme = &self.view.theme;
        let model = self
            .state
            .config
            .popup
            .as_mut()
            .expect("popup present: is_docked checked above");
        let scroll = model.scroll;
        let content = model.content_mut(theme);
        let resolved = hume_ui::popup::resolve_band(content, area.width, max_rows, scroll, border);

        self.state.views.popup_band.set(Some(resolved));
    }

    /// Write the current menu content into the shared `PopupState` Arc so
    /// `PopupOverlay` can render it during this frame — same geometry rules
    /// as [`Self::sync_popup_view`], but items are shown one-per-line as-is
    /// (no word-wrap: menu entries are short labels, not prose) and
    /// `selected` marks the highlighted row.
    pub(super) fn sync_menu_view(&mut self, ctx: &mut RenderContext) {
        if self.state.input.menu().is_none() {
            // Skip the write-lock when both sides are already None — common
            // case while no menu is open.
            if self.state.views.menu.read().is_none() {
                return;
            }
            self.state.views.menu.set(None);
            return;
        }

        // Hoisted out of the `and_then` below: resolving the anchor takes
        // `&mut self` (it may walk the pane's display-line map), which cannot overlap
        // the `&self.state.input` that closure's receiver holds.
        let anchor_char = self.focused_cursor_char();
        let placement = self.popup_placement(ctx, anchor_char);
        let border = self.state.settings.popup_border;

        let resolved = placement.and_then(|placement| {
            let model = self.state.input.menu()?;
            // `MenuRows::clone` is an `Arc` bump plus a `u16` copy, not a
            // re-measure: `MenuModel::rows` is pre-measured once at
            // `show-menu!` time (labels never change during a menu's
            // lifetime, only `selected` does), so there's nothing left for
            // this per-frame snapshot to recompute.
            Some(hume_ui::popup::resolve_menu(
                model.rows.clone(),
                model.selected,
                placement,
                border,
            ))
        });

        self.state.views.menu.set(resolved);
    }

    /// Write the LSP completion menu into the shared `PopupState` Arc —
    /// same widget as [`Self::sync_menu_view`] (unwrapped rows,
    /// selected-row styling), but anchored at the completion session's
    /// token-start char rather than the live cursor (which drifts as the
    /// user types further into the token). Called every frame from
    /// `prepare_frame`'s step 10, same as [`Self::sync_popup_view`]/
    /// [`Self::sync_menu_view`] and for the same reason: it needs
    /// `EngineView::pane_rect`, which reads `last_pane_area` — only current
    /// after step 9 runs.
    pub(super) fn sync_completion_menu_view(&mut self, ctx: &mut RenderContext) {
        if self.lsp.completion.is_none() && self.state.views.completion_menu.read().is_none() {
            return;
        }

        // `session.anchor()` is a char offset captured when the session
        // began; it isn't remapped through edits, so an out-of-band shrink
        // (LSP applyEdit, file reload) or a pane switch since can leave it
        // pointing past the focused buffer's current end, or at a buffer
        // that isn't even the one on screen. `DisplayLineMap::locate` (reached via
        // `popup_placement`) has no way to tell a stale offset from a live
        // one, so check both here.
        //
        // Sequential borrows rather than one closure over
        // `self.lsp.completion`: the session's shared borrow has to end
        // before `popup_placement` and `menu_rows` each take `&mut self`.
        let border = self.state.settings.popup_border;
        let resolved = (|| -> Option<hume_ui::popup::PopupState> {
            let session = self.lsp.completion.as_ref()?;
            if session.bid() != self.focused_buffer_id() {
                return None;
            }
            let anchor_char = session.anchor();
            let len = self.state.buffers.get(session.bid()).text().end();
            if anchor_char >= len {
                return None;
            }
            let placement = self.popup_placement(ctx, anchor_char)?;

            let selected_idx = self.lsp.completion_ui.as_ref().map_or(0, |ui| ui.selected);
            let session = self.lsp.completion.as_mut()?;
            let rows = session.menu_rows();
            Some(hume_ui::popup::resolve_menu(
                rows,
                selected_idx,
                placement,
                border,
            ))
        })();

        self.state.views.completion_menu.set(resolved);
    }

    /// Write the open picker session into the shared `PickerViewState` Arc
    /// so `PickerOverlay` can paint it this frame. Same step-10 timing as
    /// `sync_popup_view`/`sync_menu_view`/`sync_completion_menu_view` (needs
    /// `last_pane_area`, set in step 9) but, unlike them, centers in the
    /// panes region rather than anchoring at the cursor — no `RenderContext`
    /// needed.
    ///
    /// Takes `&mut self`: before snapshotting the visible window, it calls
    /// `PickerSession::move_selection(0, geo.list_rows)` — a delta-0 move is
    /// a pure scroll-clamp against the *current* geometry, so a terminal
    /// resize between the last keystroke and this frame self-heals here
    /// rather than leaving a stale scroll offset from a taller frame.
    pub(super) fn sync_picker_view(&mut self) {
        if self.state.input.picker().is_none() && self.state.views.picker.read().is_none() {
            return;
        }

        let geo = hume_ui::picker_panel::panel_geometry(self.view.last_pane_area);
        let resolved = match (self.state.input.picker_mut(), geo) {
            (Some(session), Some(geo)) => {
                session.move_selection(0, geo.list_rows);
                let rows: Vec<String> = session.window(geo.list_rows).map(str::to_string).collect();
                let selected_row =
                    (!rows.is_empty()).then(|| session.selected() - session.scroll());
                Some(hume_ui::picker_panel::PickerViewState {
                    prompt: session.prompt().to_string(),
                    query: session.query().to_string(),
                    rows,
                    selected_row,
                    matched: session.matched_len(),
                    total: session.total_len(),
                    pending: session.is_pending(),
                    rect: geo.rect,
                    list_rows: geo.list_rows,
                    border: self.state.settings.popup_border,
                    truncate: session.truncate(),
                })
            }
            _ => None,
        };

        self.state.views.picker.set(resolved);
    }
}
