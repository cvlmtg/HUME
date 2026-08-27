//! Per-frame sync of the popup / docked-popup / menu / LSP-completion-menu /
//! picker overlay views into their shared `Arc` buffers. The docked popup
//! (`sync_popup_band_view`) syncs from `prepare_frame`'s step 0 — it only
//! needs `last_terminal_area`, which the pre-settle `sync_viewport_dims`
//! already wrote this frame. The cursor-anchored overlays (`sync_popup_view`
//! and the rest of this module) need the focused pane's post-scroll rect
//! instead, so they sync later, from step 7.

use hume_engine::pipeline::RenderContext;
use hume_grid::Rect;

use super::Editor;
use crate::lock_ext::LockExt;

impl Editor {
    /// Write the current completion state into the shared `MinibufCompletionView` Arc
    /// so `MinibufCompletionOverlay` can render it during this frame.
    ///
    /// Called from `prepare_frame` after highlight data is synced.
    pub(super) fn sync_minibuf_completion_view(&self) {
        // Skip the write-lock when both sides are already None — common case
        // while no popup is open.
        if self.state.minibuf_completion.is_none()
            && self.state.minibuf_completion_view.read_or_panic().is_none()
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
            crate::ui::completion_overlay::MinibufCompletionView {
                rows: state.candidates.iter().map(|c| c.display.clone()).collect(),
                selected: state.selected,
                anchor_x,
                border: self.state.settings.popup_border,
            }
        });
        *self.state.minibuf_completion_view.write_or_panic() = view;
    }

    /// The focused pane's primary cursor position — the anchor char for
    /// [`Self::sync_popup_view`] and [`Self::sync_menu_view`] (unlike the
    /// LSP completion menu, which anchors at the session's token-start
    /// char instead, via a separately-computed `anchor_char`).
    fn focused_cursor_char(&self) -> usize {
        let pid = self.state.focused_pane_id;
        self.state.panes.state[pid][self.focused_buffer_id()]
            .selections
            .primary()
            .head()
    }

    /// Screen anchor (absolute cell) + geometry bounds for the focused
    /// pane, given an arbitrary buffer char position — shared by
    /// [`Self::sync_popup_view`], [`Self::sync_menu_view`], and the LSP
    /// completion menu (each passes a different `anchor_char`). Returns
    /// `(anchor, pane_rect, max_width, max_height)`; `None` when the pane
    /// has no rect yet or `anchor_char` isn't currently visible.
    fn popup_anchor_and_bounds(
        &self,
        ctx: &mut RenderContext,
        anchor_char: usize,
    ) -> Option<((u16, u16), Rect, u16, u16)> {
        let focused = self.state.focused_pane_id;
        let pane_rect = self.view.pane_rect(focused)?;
        let (_, gutter_w) = self.resolve_pane_settings(focused);
        let content_width = pane_rect.width.saturating_sub(gutter_w);
        // Step 6 (`scroll_into_view`) already resolved the focused cursor's
        // screen cell this frame, via the same locate/distance walk
        // `content_pos` runs below — nothing between steps 6 and 10 moves the
        // cursor or the viewport, so the two callers anchored at the live
        // cursor (`sync_popup_view`, `sync_menu_view`) can reuse it instead of
        // re-walking the row list (a full per-line format in wrap mode).
        let (content_x, row) = match ctx.cursor_content_pos {
            Some(cell) if anchor_char == self.focused_cursor_char() => cell,
            _ => {
                let vp = &self.view.panes[focused].viewport;
                let buf = self.state.buffers.get(self.focused_buffer_id());
                let mut rm = super::commands::pane_row_map(
                    buf,
                    &self.state.settings,
                    &self.view.panes[focused],
                    &mut ctx.cursor_format,
                );
                super::cursor::content_pos(vp, &mut rm, anchor_char)?
            }
        };
        let anchor = super::mouse::content_pos_to_screen(content_x, row, gutter_w, pane_rect);
        // Reserve 2 cells on each axis for the popup's 1-cell frame, so
        // content + border together fit the same envelope this budget used
        // to give to content alone.
        let max_width = crate::ui::popup::MAX_POPUP_WIDTH
            .min(content_width.saturating_sub(4))
            .saturating_sub(2);
        let max_height = (pane_rect.height / 3).max(1).saturating_sub(2).max(1);
        Some((anchor, pane_rect, max_width, max_height))
    }

    /// Write the current *cursor-anchored* popup content into the shared
    /// `PopupState` Arc so `PopupOverlay` can render it during this frame.
    /// Geometry (wrap width, flip/clamp position) is resolved fresh every
    /// frame against the focused pane's *current* rect — never pre-computed
    /// at `show-popup!` call time — so a resize or scroll never leaves it
    /// stale. A docked popup (`PopupLayout::Docked`) is handled by
    /// [`Self::sync_popup_band_view`] instead — this clears `popup_view` for
    /// that case, same as when no popup is open at all.
    ///
    /// Called from `prepare_frame` after `last_pane_area` is set (step 10):
    /// `EngineView::pane_rect` reads that field, so calling this any earlier
    /// would position against the previous frame's geometry.
    pub(super) fn sync_popup_view(&mut self, ctx: &mut RenderContext) {
        let is_cursor = matches!(
            self.state.config.popup.as_ref().map(|m| &m.layout),
            Some(crate::ui::popup::PopupLayout::Cursor)
        );
        if !is_cursor && self.state.popup_view.read_or_panic().is_none() {
            return;
        }
        if !is_cursor {
            *self.state.popup_view.write_or_panic() = None;
            return;
        }

        let bounds = self.popup_anchor_and_bounds(ctx, self.focused_cursor_char());
        let resolved = bounds.and_then(|(anchor, pane_rect, max_width, max_height)| {
            // Wrap the *full* text, unbounded — a scrollable popup must keep
            // every row reachable, not just the first `max_height` of them.
            // The box itself still caps at `max_height` via `outer_dims`
            // below; `scroll` (clamped against that cap) picks which window
            // of `lines` is visible. `resolve_popup_text` caches this by
            // `max_width`, so an unchanged width across frames is O(1), not
            // a re-wrap.
            let text = self.resolve_popup_text(max_width)?;
            let model_scroll = self.state.config.popup.as_ref()?.scroll;
            let (outer_w, outer_h) = crate::ui::menu_box::outer_dims(&text.lines, max_height);
            let (x, y, outer_w, outer_h) =
                crate::ui::popup::resolve_popup_geometry(outer_w, outer_h, anchor, pane_rect);
            let inner_h = outer_h.saturating_sub(2) as usize;
            let scroll = model_scroll.min(text.lines.len().saturating_sub(inner_h));
            Some(crate::ui::popup::PopupState {
                lines: text.lines,
                rect: Rect::new(x, y, outer_w, outer_h),
                selected: None,
                scroll,
                styled_rows: text.styled_rows,
                border: self.state.settings.popup_border,
            })
        });

        *self.state.popup_view.write_or_panic() = resolved;
    }

    /// Resolve (or reuse the cached) wrap+highlight of the open popup's text
    /// at `max_width` — shared by [`Self::sync_popup_view`] (cursor layout)
    /// and [`Self::sync_popup_band_view`] (docked layout), which never run in
    /// the same frame (mutually exclusive on `PopupModel::layout`), so there
    /// is one cache and one width per frame. See
    /// [`crate::ui::popup::ResolvedPopupText`] for the invalidation contract.
    ///
    /// Returns `None` only if no popup is open — should not happen at either
    /// call site (both gated on `self.state.config.popup` being `Some`), but mirrors
    /// the `Option`-chaining style of the surrounding sync functions rather
    /// than `.expect`-ing a caller invariant.
    fn resolve_popup_text(
        &mut self,
        max_width: u16,
    ) -> Option<crate::ui::popup::ResolvedPopupText> {
        let theme = &self.view.theme;
        let model = self.state.config.popup.as_mut()?;
        let stale = model.resolved.as_ref().is_none_or(|r| r.width != max_width);
        if stale {
            let (lines, styled_rows) = if let Some(popup_syntax) = model.syntax.as_ref() {
                let base_style = theme
                    .resolve_by_name(hume_engine::types::Scope("ui.popup"))
                    .into();
                let runs = popup_syntax.styled_runs(&model.text, theme, base_style);
                let rows = crate::ui::popup::wrap_styled(&runs, max_width);
                let lines: Vec<String> = rows
                    .iter()
                    .map(|row| row.iter().map(|(s, _)| s.as_str()).collect())
                    .collect();
                (std::sync::Arc::new(lines), Some(std::sync::Arc::new(rows)))
            } else {
                (
                    std::sync::Arc::new(crate::ui::popup::wrap_text(&model.text, max_width)),
                    None,
                )
            };
            model.resolved = Some(crate::ui::popup::ResolvedPopupText {
                width: max_width,
                lines,
                styled_rows,
            });
        }
        model.resolved.clone()
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
            Some(crate::ui::popup::PopupLayout::Docked)
        );
        if !is_docked && self.state.popup_band_view.read_or_panic().is_none() {
            return;
        }
        if !is_docked {
            *self.state.popup_band_view.write_or_panic() = None;
            return;
        }

        let area = self.view.last_terminal_area;
        let max_width = area.width.saturating_sub(2);
        let resolved = self.resolve_popup_text(max_width).map(|text| {
            let model_scroll = self.state.config.popup.as_ref().map_or(0, |m| m.scroll);
            // Shares `crate::ui::popup::band_capacity` with
            // `PopupBandWidget::height`, so the scroll clamp always agrees
            // with what the engine will next paint (same pattern as
            // `drawer_visible_rows`).
            let max_rows = area.height / 2;
            let capacity = crate::ui::popup::band_capacity(text.lines.len(), max_rows);
            let inner_h = capacity.saturating_sub(2) as usize;
            let scroll = model_scroll.min(text.lines.len().saturating_sub(inner_h));
            crate::ui::popup::PopupBandState {
                lines: text.lines,
                scroll,
                styled_rows: text.styled_rows,
                border: self.state.settings.popup_border,
            }
        });

        *self.state.popup_band_view.write_or_panic() = resolved;
    }

    /// Write the current menu content into the shared `PopupState` Arc so
    /// `PopupOverlay` can render it during this frame — same geometry rules
    /// as [`Self::sync_popup_view`], but items are shown one-per-line as-is
    /// (no word-wrap: menu entries are short labels, not prose) and
    /// `selected` marks the highlighted row.
    pub(super) fn sync_menu_view(&self, ctx: &mut RenderContext) {
        if self.state.config.menu.is_none() && self.state.menu_view.read_or_panic().is_none() {
            return;
        }

        let resolved = self.state.config.menu.as_ref().and_then(|model| {
            let (anchor, pane_rect, _max_width, _max_height) =
                self.popup_anchor_and_bounds(ctx, self.focused_cursor_char())?;
            let lines: Vec<String> = model.items.clone();
            let (outer_w, outer_h) =
                crate::ui::menu_box::outer_dims(&lines, crate::ui::menu_box::MAX_MENU_ROWS);
            let (x, y, outer_w, outer_h) =
                crate::ui::popup::resolve_popup_geometry(outer_w, outer_h, anchor, pane_rect);
            let selected = if lines.is_empty() {
                None
            } else {
                Some(model.selected.min(lines.len() - 1))
            };
            Some(crate::ui::popup::PopupState {
                lines: std::sync::Arc::new(lines),
                rect: Rect::new(x, y, outer_w, outer_h),
                selected,
                scroll: 0, // ignored: a menu windows around `selected`, not `scroll`
                styled_rows: None, // menus never highlight per-span, only per-row
                border: self.state.settings.popup_border,
            })
        });

        *self.state.menu_view.write_or_panic() = resolved;
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
        if self.lsp.completion.is_none()
            && self.state.completion_menu_view.read_or_panic().is_none()
        {
            return;
        }

        // `session.anchor()` is a char offset captured when the session
        // began; it isn't remapped through edits, so an out-of-band shrink
        // (LSP applyEdit, file reload) or a pane switch since can leave it
        // pointing past the focused buffer's current end, or at a buffer
        // that isn't even the one on screen. `RowMap::locate` (reached via
        // `popup_anchor_and_bounds`) has no way to tell a stale offset from
        // a live one, so check both here.
        //
        // Two sequential immutable-then-mutable borrows of `self`, not one
        // closure over `self.lsp.completion`: `popup_anchor_and_bounds`
        // needs `&self` as a whole (pane/viewport/cursor state), which
        // can't overlap the `&mut` `menu_labels_and_width` needs below.
        let resolved = (|| -> Option<crate::ui::popup::PopupState> {
            let session = self.lsp.completion.as_ref()?;
            if session.bid() != self.focused_buffer_id() {
                return None;
            }
            let anchor_char = session.anchor();
            let len = self.state.buffers.get(session.bid()).text().len_chars();
            if anchor_char >= len {
                return None;
            }
            let (anchor, pane_rect, _max_width, _max_height) =
                self.popup_anchor_and_bounds(ctx, anchor_char)?;

            let selected_idx = self.lsp.completion_ui.as_ref().map_or(0, |ui| ui.selected);
            let session = self.lsp.completion.as_mut()?;
            let (lines, inner_w) = session.menu_labels_and_width();
            let (outer_w, outer_h) = crate::ui::menu_box::outer_dims_from_width(
                inner_w,
                lines.len(),
                crate::ui::menu_box::MAX_MENU_ROWS,
            );
            let (x, y, outer_w, outer_h) =
                crate::ui::popup::resolve_popup_geometry(outer_w, outer_h, anchor, pane_rect);
            let selected = if lines.is_empty() {
                None
            } else {
                Some(selected_idx.min(lines.len() - 1))
            };
            Some(crate::ui::popup::PopupState {
                lines,
                rect: Rect::new(x, y, outer_w, outer_h),
                selected,
                scroll: 0, // ignored: a menu windows around `selected`, not `scroll`
                styled_rows: None, // menus never highlight per-span, only per-row
                border: self.state.settings.popup_border,
            })
        })();

        *self.state.completion_menu_view.write_or_panic() = resolved;
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
        if self.state.config.picker.is_none() && self.state.picker_view.read_or_panic().is_none() {
            return;
        }

        let geo = crate::ui::picker_panel::panel_geometry(self.view.last_pane_area);
        let resolved = match (self.state.config.picker.as_mut(), geo) {
            (Some(session), Some(geo)) => {
                session.move_selection(0, geo.list_rows);
                let rows: Vec<String> = session.window(geo.list_rows).map(str::to_string).collect();
                let selected_row =
                    (!rows.is_empty()).then(|| session.selected() - session.scroll());
                Some(crate::ui::picker_panel::PickerViewState {
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

        *self.state.picker_view.write_or_panic() = resolved;
    }
}
