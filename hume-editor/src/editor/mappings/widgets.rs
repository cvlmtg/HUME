//! Key handling for transient chrome — the selection menu, bottom drawer,
//! popup (hover/signature-help), and picker — plus the scroll/geometry
//! helpers they share. `handle_key` (in `mod.rs`) intercepts into these
//! before per-mode dispatch.

use termina::event::{KeyCode, KeyEvent, Modifiers};

use super::super::Editor;

impl Editor {
    /// Handles one key while a selection menu is open. Returns `true`
    /// if the key was fully consumed (movement, `Enter`, `Esc`) — `false` if
    /// a stray key dismissed the menu but should still fall through to
    /// normal dispatch this same call: a stray key both closes the menu
    /// (with a `#f` callback) *and* executes its usual effect.
    ///
    /// The callback fires exactly once (one-shot `.take()` discipline) —
    /// `queue_steel_call` never invokes it inline, matching every other
    /// Rust→Steel callback in this codebase.
    pub(super) fn handle_menu_key(&mut self, key: KeyEvent) -> bool {
        let Some(menu) = self.state.menu.as_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if menu.selected + 1 < menu.items.len() {
                    menu.selected += 1;
                }
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                menu.selected = menu.selected.saturating_sub(1);
                true
            }
            KeyCode::Enter => {
                let menu = self.state.menu.take().expect("checked by the caller above");
                let idx = steel::rvals::SteelVal::IntV(menu.selected as isize);
                self.queue_steel_call(menu.callback, vec![idx]);
                true
            }
            KeyCode::Escape => {
                let menu = self.state.menu.take().expect("checked by the caller above");
                self.queue_steel_call(menu.callback, vec![steel::rvals::SteelVal::BoolV(false)]);
                true
            }
            _ => {
                let menu = self.state.menu.take().expect("checked by the caller above");
                self.queue_steel_call(menu.callback, vec![steel::rvals::SteelVal::BoolV(false)]);
                false
            }
        }
    }

    /// Handles one key while the bottom drawer is open. Returns `true`
    /// if the key was fully consumed (movement, `Enter`, `Esc`) — `false`
    /// for any other key, which the drawer leaves completely untouched (no
    /// close, no callback) so normal dispatch runs as if the drawer weren't
    /// open at all.
    ///
    /// Unlike the menu, `Enter` does not close the drawer or take the
    /// callback — it clones it and queues a call, so the drawer can fire
    /// `on-select` repeatedly across a browse session (Helix-style: pick a
    /// diagnostic, jump, come back, pick another).
    pub(super) fn handle_drawer_key(&mut self, key: KeyEvent) -> bool {
        if self.state.drawer.is_none() {
            return false;
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let drawer = self.state.drawer.as_mut().expect("checked above");
                if drawer.selected + 1 < drawer.items.len() {
                    drawer.selected += 1;
                    self.clamp_drawer_scroll();
                }
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let drawer = self.state.drawer.as_mut().expect("checked above");
                if drawer.selected > 0 {
                    drawer.selected -= 1;
                    self.clamp_drawer_scroll();
                }
                true
            }
            KeyCode::Char('d') if key.modifiers.contains(Modifiers::CONTROL) => {
                let half = (self.drawer_visible_rows() / 2).max(1);
                if let Some(drawer) = self.state.drawer.as_mut() {
                    drawer.selected =
                        (drawer.selected + half).min(drawer.items.len().saturating_sub(1));
                }
                self.clamp_drawer_scroll();
                true
            }
            KeyCode::Char('u') if key.modifiers.contains(Modifiers::CONTROL) => {
                let half = (self.drawer_visible_rows() / 2).max(1);
                if let Some(drawer) = self.state.drawer.as_mut() {
                    drawer.selected = drawer.selected.saturating_sub(half);
                }
                self.clamp_drawer_scroll();
                true
            }
            KeyCode::Enter => {
                let drawer = self.state.drawer.as_ref().expect("checked above");
                let idx = steel::rvals::SteelVal::IntV(drawer.selected as isize);
                let callback = drawer.callback.clone();
                self.queue_steel_call(callback, vec![idx]);
                true
            }
            KeyCode::Escape => {
                let drawer = self.state.drawer.take().expect("checked above");
                self.queue_steel_call(drawer.callback, vec![steel::rvals::SteelVal::BoolV(false)]);
                self.state.sync_drawer_view();
                true
            }
            _ => false,
        }
    }

    /// Scrolls a `Scrollable` popup half its visible content height, in the
    /// direction given by `down`. Reads the visible height and total row
    /// count from the *already-resolved* view for the popup's current
    /// layout — this same frame's paint — rather than recomputing geometry,
    /// so the scroll step always matches what's on screen. Both views are
    /// re-synced every frame regardless (`Editor::sync_popup_view`/
    /// `sync_popup_band_view`), so no explicit sync is needed here. Before
    /// the first frame both are empty — a documented no-op, same as
    /// `handle_picker_key`'s geometry.
    ///
    /// Returns `true` if the popup actually has content past one screenful
    /// (`max_scroll > 0`) — the caller (`handle_key`) uses this to tell a
    /// real scroll from a popup too short to scroll, so Ctrl+d/Ctrl+u fall
    /// through to their usual buffer effect instead of being silently eaten.
    pub(super) fn scroll_popup(&mut self, down: bool) -> bool {
        let Some(layout) = self.state.popup.as_ref().map(|p| &p.layout) else {
            return false;
        };
        let (inner_h, total) = match layout {
            crate::ui::popup::PopupLayout::Cursor => {
                let Some(pair) = self
                    .state
                    .popup_view
                    .read()
                    .expect("RwLock not poisoned")
                    .as_ref()
                    .map(|s| (s.outer_h.saturating_sub(2) as usize, s.lines.len()))
                else {
                    return false;
                };
                pair
            }
            crate::ui::popup::PopupLayout::Docked => {
                let Some(total) = self
                    .state
                    .popup_band_view
                    .read()
                    .expect("RwLock not poisoned")
                    .as_ref()
                    .map(|s| s.lines.len())
                else {
                    return false;
                };
                (self.popup_band_visible_rows(total), total)
            }
        };
        let max_scroll = total.saturating_sub(inner_h);
        if max_scroll == 0 {
            return false;
        }
        let Some(popup) = self.state.popup.as_mut() else {
            return false;
        };
        let half = (inner_h / 2).max(1);
        // `popup.scroll` is the model value, re-clamped for rendering only in
        // the per-frame view sync (see `PopupModel::scroll`) — it can be
        // stale-large after the popup's content shrinks (e.g. terminal grows
        // between frames without a key event dismissing it), so clamp before
        // applying the delta rather than after.
        let clamped = popup.scroll.min(max_scroll);
        popup.scroll = if down {
            (clamped + half).min(max_scroll)
        } else {
            clamped.saturating_sub(half)
        };
        true
    }

    /// Number of docked-popup rows visible at once, given `total` wrapped
    /// lines — computed via `crate::ui::popup::band_capacity`, the same
    /// helper `PopupBandWidget::height` calls to size what it paints next
    /// frame (`max` = half the last-rendered *terminal* height). Mirrors
    /// `drawer_visible_rows`'s contract for the drawer's own band.
    fn popup_band_visible_rows(&self, total: usize) -> usize {
        let max = self.view.last_terminal_area.height / 2;
        let capacity = crate::ui::popup::band_capacity(total, max);
        capacity.saturating_sub(2) as usize
    }

    /// Number of drawer rows visible at once — the same capacity
    /// `DrawerProvider::height` will paint against next frame. `max` mirrors
    /// that provider's own ceiling (half the last-rendered *terminal*
    /// height, not the already-chrome-reduced pane height). Shared by
    /// `clamp_drawer_scroll` and the Ctrl+u/Ctrl+d half-page handlers so
    /// "half a page" always agrees with what's on screen.
    fn drawer_visible_rows(&self) -> usize {
        let max = self.view.last_terminal_area.height / 2;
        let Some(drawer) = self.state.drawer.as_ref() else {
            return 0;
        };
        let capacity = (drawer.items.len() as u16 + 1).min(max);
        capacity.saturating_sub(1) as usize
    }

    /// Clamps `drawer.scroll` so `drawer.selected` stays within the visible
    /// window, then syncs the view.
    fn clamp_drawer_scroll(&mut self) {
        let visible_rows = self.drawer_visible_rows();
        let Some(drawer) = self.state.drawer.as_mut() else {
            return;
        };
        if visible_rows > 0 {
            if drawer.selected >= drawer.scroll + visible_rows {
                drawer.scroll = drawer.selected + 1 - visible_rows;
            } else if drawer.selected < drawer.scroll {
                drawer.scroll = drawer.selected;
            }
        }
        self.state.sync_drawer_view();
    }

    /// Handles one key while the picker is open. Always returns `true` —
    /// unlike the menu (a stray key closes it and falls through) or the
    /// drawer (a stray key falls through untouched), the picker is
    /// full-modal: it owns the entire interaction, so an unrecognized key
    /// (Left/Right/Home/Tab/…) is simply consumed and ignored rather than
    /// leaking through to whatever mode sits underneath.
    ///
    /// `on_select` fires exactly once via `.take()` + `queue_steel_call`
    /// (never invoked inline) — same one-shot discipline as the menu.
    /// `visible_rows` comes from `panel_geometry` against the same
    /// `last_pane_area` the next frame's `sync_picker_view` will use, so a
    /// keystroke and the following paint always agree on how many rows are
    /// visible (before the first frame, geometry is `None` and paging is a
    /// documented no-op on the store).
    pub(super) fn handle_picker_key(&mut self, key: KeyEvent) -> bool {
        let visible_rows = crate::ui::picker_panel::panel_geometry(self.view.last_pane_area)
            .map_or(0, |geo| geo.list_rows);

        // Every movement key differs only in the delta passed to
        // `move_selection` — collapsed to one borrow instead of one per key.
        let step: Option<isize> = match key.code {
            KeyCode::Down => Some(1),
            KeyCode::Up => Some(-1),
            KeyCode::Char('n') if key.modifiers.contains(Modifiers::CONTROL) => Some(1),
            KeyCode::Char('p') if key.modifiers.contains(Modifiers::CONTROL) => Some(-1),
            KeyCode::PageDown => Some(visible_rows as isize),
            KeyCode::PageUp => Some(-(visible_rows as isize)),
            _ => None,
        };
        if let Some(delta) = step {
            self.picker_mut().move_selection(delta, visible_rows);
            return true;
        }

        match key.code {
            KeyCode::Backspace => {
                self.picker_mut().pop_grapheme(); // no-op on an already-empty query
            }
            KeyCode::Enter => {
                // No match (or nothing pushed yet) behaves like Esc — Enter
                // is always a terminal action, never a silent no-op. Read
                // the payload before closing: `close_picker` takes the
                // session.
                let payload = self
                    .picker_mut()
                    .selected_payload()
                    .cloned()
                    .unwrap_or(steel::rvals::SteelVal::BoolV(false));
                super::super::picker::close_picker(&mut self.state, payload);
            }
            KeyCode::Escape => {
                super::super::picker::close_picker(
                    &mut self.state,
                    steel::rvals::SteelVal::BoolV(false),
                );
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(Modifiers::CONTROL | Modifiers::ALT) =>
            {
                self.picker_mut().insert_char(ch);
            }
            _ => {}
        }
        true
    }

    /// The open picker session — only ever called from `handle_picker_key`,
    /// whose caller (`handle_key`) already checked `state.picker.is_some()`
    /// before dispatching here.
    fn picker_mut(&mut self) -> &mut super::super::picker::PickerSession {
        self.state
            .picker
            .as_mut()
            .expect("handle_picker_key is only called while state.picker.is_some()")
    }
}
