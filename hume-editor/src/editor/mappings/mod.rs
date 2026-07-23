use termina::event::{KeyCode, KeyEvent, Modifiers};

use super::{Editor, Mode};

pub(super) mod command_mode;
mod execute;
mod insert;
mod lazy;
mod normal;
mod paste;
mod search_mode;
mod select_mode;

impl Editor {
    // ── Key dispatch ──────────────────────────────────────────────────────────

    pub(crate) fn handle_key(&mut self, key: KeyEvent) {
        // How many keystrokes the message-log summary stays visible after
        // `status_msg` clears. Chosen for UX, not technical constraint.
        const SUMMARY_TTL: u8 = 3;

        // Any keypress dismisses the previous transient status message.
        // When it clears with unseen log entries the summary becomes visible —
        // arm its countdown. On later keypresses tick it down and auto-dismiss
        // at zero. The countdown only runs when the minibuffer is closed so
        // typing a long `:` command doesn't burn the budget invisibly.
        let had_status = self.state.status_msg.take().is_some();
        if self.state.minibuf.is_none() {
            if had_status {
                if self.state.message_log.has_unseen() {
                    self.state.summary_ttl = SUMMARY_TTL;
                }
            } else if self.state.summary_ttl > 0 {
                self.state.summary_ttl -= 1;
                if self.state.summary_ttl == 0 {
                    self.state.message_log.mark_all_seen();
                }
            }
        }

        // Popup dismissal/scroll, before mode dispatch — see `PopupDismiss`.
        // `OnAnyKey` (`gn`/`gp`) clears unconditionally and the key still
        // dispatches below. `OnKeyExceptScroll` (scrollable hover) consumes
        // Ctrl+u/Ctrl+d to scroll; any other key closes the popup and falls
        // through to normal dispatch this same call.
        match self.state.popup.as_ref().map(|p| &p.dismiss) {
            Some(crate::ui::popup::PopupDismiss::AnyKey) => {
                self.state.popup = None;
            }
            Some(crate::ui::popup::PopupDismiss::KeyExceptScroll) => {
                let ctrl = key.modifiers.contains(Modifiers::CONTROL);
                match key.code {
                    KeyCode::Char('d') if ctrl => {
                        self.scroll_popup(true);
                        return;
                    }
                    KeyCode::Char('u') if ctrl => {
                        self.scroll_popup(false);
                        return;
                    }
                    _ => self.state.popup = None,
                }
            }
            Some(crate::ui::popup::PopupDismiss::ModeChange) | None => {}
        }

        // ── Picker intercept ──────────────────────────────────────────────
        // Sits above the menu/drawer intercepts and is mode-agnostic (Q-B7,
        // `docs/FUZZY-FINDERS.md`: the picker opens from any mode, unlike
        // the menu/drawer's Normal/Extend-only gate) — key ownership mirrors
        // the picker's top z-order registration (`ui/mod.rs`'s `build_pane`),
        // the most action-relevant surface when more than one could be
        // visible. Full-modal: `handle_picker_key` always consumes, so while
        // a picker is open `handle_insert`'s own completion intercept never
        // runs — no conflict between the two.
        let picker_consumed = self.state.picker.is_some() && self.handle_picker_key(key);

        // ── Selection menu intercept ─────────────────────────────────────
        // Guarded early-return before mode dispatch, not a new `Mode` — a
        // menu is transient chrome, not an editing mode (no `on-mode-change`,
        // no statusline/cursor-shape changes). Normal/Extend only: menus
        // don't open from Insert in v1.
        let menu_consumed = !picker_consumed
            && self.state.menu.is_some()
            && matches!(self.state.mode(), Mode::Normal | Mode::Extend)
            && self.handle_menu_key(key);

        // ── Bottom drawer intercept ──────────────────────────────────────
        // Same guarded-early-return shape as the menu's, but unlike the menu
        // a stray key neither closes the drawer nor invokes its callback —
        // it falls through untouched, leaving the drawer open while focus
        // stays on the pane (Helix-style browse-while-editing).
        let drawer_consumed = !picker_consumed
            && !menu_consumed
            && self.state.drawer.is_some()
            && matches!(self.state.mode(), Mode::Normal | Mode::Extend)
            && self.handle_drawer_key(key);

        if !picker_consumed && !menu_consumed && !drawer_consumed {
            match self.state.mode() {
                Mode::Normal | Mode::Extend => self.handle_normal(key),
                Mode::Insert => self.handle_insert(key),
                Mode::Command => self.handle_command(key),
                Mode::Search => self.handle_search(key),
                Mode::Select => self.handle_select(key),
            }
        }

        // ── Macro recording ───────────────────────────────────────────────────
        // Runs after all mode handlers so Insert, Command, and Search keys
        // are captured. `skip_macro_record` excludes the stop `Q` itself.
        if let Some((_, ref mut keys)) = self.state.macro_recording
            && !self.state.skip_macro_record
        {
            keys.push(key);
        }
        self.state.skip_macro_record = false;

        // Replay a pending dot-repeat action, if cmd_repeat set one.
        // Runs after macro recording so the `.` key itself is captured,
        // but the replayed command executes as a fresh dispatch with &mut Editor.
        if let Some(pending) = self.state.pending_repeat.take() {
            self.replay_dot(pending.count);
        }

        // Any mode change this dispatch made (`set_mode`'s Insert-exit arm)
        // dismisses a completion session synchronously, before this
        // function returns — same timing tests already assert on.
        self.take_pending_lsp_completion_dismiss();
    }

    /// Handles one key while a selection menu is open. Returns `true`
    /// if the key was fully consumed (movement, `Enter`, `Esc`) — `false` if
    /// a stray key dismissed the menu but should still fall through to
    /// normal dispatch this same call: a stray key both closes the menu
    /// (with a `#f` callback) *and* executes its usual effect.
    ///
    /// The callback fires exactly once (one-shot `.take()` discipline) —
    /// `queue_steel_call` never invokes it inline, matching every other
    /// Rust→Steel callback in this codebase.
    fn handle_menu_key(&mut self, key: KeyEvent) -> bool {
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
    fn handle_drawer_key(&mut self, key: KeyEvent) -> bool {
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

    /// Scrolls an `OnKeyExceptScroll` popup half its visible content height,
    /// in the direction given by `down`. Reads the visible height and total
    /// row count from the *already-resolved* `popup_view` — this same
    /// frame's paint — rather than recomputing geometry, so the scroll step
    /// always matches what's on screen. `popup_view` is re-synced every
    /// frame regardless (`Editor::sync_popup_view`), so no explicit sync is
    /// needed here. Before the first frame `popup_view` is empty — a
    /// documented no-op, same as `handle_picker_key`'s geometry.
    fn scroll_popup(&mut self, down: bool) {
        let Some((inner_h, total)) = self
            .state
            .popup_view
            .read()
            .expect("RwLock not poisoned")
            .as_ref()
            .map(|s| (s.outer_h.saturating_sub(2) as usize, s.lines.len()))
        else {
            return;
        };
        let Some(popup) = self.state.popup.as_mut() else {
            return;
        };
        let half = (inner_h / 2).max(1);
        let max_scroll = total.saturating_sub(inner_h);
        popup.scroll = if down {
            (popup.scroll + half).min(max_scroll)
        } else {
            popup.scroll.saturating_sub(half)
        };
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
    fn handle_picker_key(&mut self, key: KeyEvent) -> bool {
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
                super::picker::close_picker(&mut self.state, payload);
            }
            KeyCode::Escape => {
                super::picker::close_picker(&mut self.state, steel::rvals::SteelVal::BoolV(false));
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
    fn picker_mut(&mut self) -> &mut super::picker::PickerSession {
        self.state
            .picker
            .as_mut()
            .expect("handle_picker_key is only called while state.picker.is_some()")
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
