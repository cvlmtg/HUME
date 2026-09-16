//! Input handling for transient chrome — the confirm overlay, selection
//! menu, bottom drawer, a `Scrollable` popup (hover, `gn`/`gp`'s diagnostic
//! overlay), and the picker — plus the scroll/geometry helpers they share.
//! `dispatch_at` (in `mod.rs`) routes into these by layer kind. A `Sticky`
//! popup (signature help) has no handler here: it lives in a mode layer's
//! slot, never its own layer, so it's never a dispatch target — see
//! `PopupModel`'s `Layer` doc, `input_stack/stack.rs`.

use termina::event::{KeyCode, Modifiers};

use super::super::Editor;
use super::super::input_stack::{InputEvent, LayerRef};
use super::super::mouse::is_fresh_gesture;
use super::super::overlay_models::{ConfirmAction, ConfirmModel, DrawerModel, MenuModel};

impl Editor {
    /// Handles one key while a native confirm overlay
    /// ([`crate::editor::overlay_models::ConfirmModel`]) is open. Always
    /// truncates itself first (there is exactly one way for this layer to
    /// leave the stack — every key retires it, matched or not — so
    /// "answer, then act" and "dismiss, then fall through" both start from
    /// the same truncate), then answers the matched choice or falls through
    /// for any other key.
    ///
    /// `choices[0]`'s key runs `action`; `choices[1]`'s key (currently always
    /// "keep", set by `open_disk_change_confirm`) records an explicit decline
    /// (`Editor::decline_disk_change`) so `check_buffer_disk_state` doesn't
    /// reopen the same question for the same on-disk signature. Both require
    /// no modifiers — a modified key (`Ctrl-k`, the kitty one-shot extend for
    /// `move-up`) was aimed at its own binding, not at this prompt, so it must
    /// not be mistaken for a bare `k`. Every other key — `Esc`, or a stray
    /// keystroke that happens to land here — is a plain dismissal: it answers
    /// neither choice, leaving the question open for the next `BufferEnter`,
    /// exactly as if the confirm had never opened. `Esc` is fully consumed;
    /// any other unmatched key falls through instead, so a prompt the user
    /// didn't notice never eats a keystroke meant for the editor (e.g. `/`
    /// opening search).
    ///
    /// A mouse event gets the same "stray input dismisses, then falls
    /// through" treatment — but only a fresh press or wheel notch counts as
    /// stray; a release, drag, or move is the tail of a gesture already in
    /// flight (see [`is_fresh_gesture`]'s doc) and falls through untouched,
    /// leaving the confirm open. Without that split the confirm would be
    /// unreachable by its own most common trigger: clicking into another
    /// pane opens it at the next `settle()`, and the click's matching `Up`
    /// arrives one loop iteration later.
    pub(super) fn confirm_input(&mut self, r: LayerRef, ev: InputEvent) {
        let key = match ev {
            InputEvent::Key(key) => key,
            // A paste is not one of the choice keys and is never a stray
            // keystroke meant for whatever lies underneath — swallowed
            // outright, the confirm left open, matching this layer's
            // full-modal choice-key policy for anything else unmatched.
            InputEvent::Paste(_) => return,
            InputEvent::Mouse(mouse) => {
                if is_fresh_gesture(mouse.kind) {
                    self.state.input.truncate(r);
                }
                self.fall_through(r, InputEvent::Mouse(mouse));
                return;
            }
        };
        let mut removed = self.state.input.truncate(r);
        let Some(confirm) = removed.pop().and_then(|l| l.downcast::<ConfirmModel>()) else {
            unreachable!("dispatch_at already checked kind(r) == ConfirmModel");
        };
        let confirm = *confirm;

        let matched = (key.modifiers == Modifiers::NONE)
            .then(|| {
                confirm
                    .choices
                    .iter()
                    .position(|c| key.code == KeyCode::Char(c.key))
            })
            .flatten();

        match confirm.action {
            ConfirmAction::ReloadBuffer(bid) => match matched {
                Some(0) => self.reload_buffer_from_disk(bid),
                Some(1) => self.decline_disk_change(bid),
                _ => {}
            },
        }

        if matched.is_none() && key.code != KeyCode::Escape {
            self.fall_through(r, InputEvent::Key(key));
        }
    }

    /// Handles one key while a selection menu is open. Movement is handled
    /// in place; `Enter`/`Esc`/a stray key all retire the layer and fire the
    /// callback exactly once (one-shot `.take()`-equivalent discipline via
    /// `truncate`) — `queue_steel_call` never invokes it inline, matching
    /// every other Rust→Steel callback in this codebase. A stray key both
    /// closes the menu (with a `#f` callback) *and* falls through to normal
    /// dispatch this same call.
    ///
    /// No mode gate of its own: `show-menu!` only pushes with the mode
    /// layer at `Base` (its own async-staleness check, `host_impl/ui.rs`),
    /// and `push_mode_layer` always pushes a new mode layer *above* whatever
    /// overlay sits on `Base` — so a `Menu` layer is dispatch's top only
    /// while the mode layer beneath it is still `Base`.
    ///
    /// A fresh mouse press or wheel notch gets the same treatment as a
    /// stray key (close with `#f`, then fall through); a release, drag, or
    /// move is the tail of a gesture already in flight (see
    /// [`is_fresh_gesture`]'s doc) and falls through untouched, leaving the
    /// menu open.
    pub(super) fn menu_input(&mut self, r: LayerRef, ev: InputEvent) {
        let key = match ev {
            InputEvent::Key(key) => key,
            // A paste is swallowed without closing the menu — same
            // "consumes stray input" treatment the menu gives any other key
            // it doesn't recognize as a choice, minus the `#f` callback
            // that arm fires: a paste was never a choice attempt.
            InputEvent::Paste(_) => return,
            InputEvent::Mouse(mouse) => {
                if !is_fresh_gesture(mouse.kind) {
                    self.fall_through(r, InputEvent::Mouse(mouse));
                    return;
                }
                let menu = self.take_menu(r);
                self.state
                    .queue_steel_call(menu.callback, vec![steel::rvals::SteelVal::BoolV(false)]);
                self.fall_through(r, InputEvent::Mouse(mouse));
                return;
            }
        };
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let menu = self
                    .state
                    .input
                    .menu_mut()
                    .expect("dispatch_at already checked kind(r) == MenuModel");
                if menu.selected + 1 < menu.rows.len() {
                    menu.selected += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let menu = self
                    .state
                    .input
                    .menu_mut()
                    .expect("dispatch_at already checked kind(r) == MenuModel");
                menu.selected = menu.selected.saturating_sub(1);
            }
            KeyCode::Enter => {
                let menu = self.take_menu(r);
                let idx = steel::rvals::SteelVal::IntV(menu.selected as isize);
                self.state.queue_steel_call(menu.callback, vec![idx]);
            }
            KeyCode::Escape => {
                let menu = self.take_menu(r);
                self.state
                    .queue_steel_call(menu.callback, vec![steel::rvals::SteelVal::BoolV(false)]);
            }
            _ => {
                let menu = self.take_menu(r);
                self.state
                    .queue_steel_call(menu.callback, vec![steel::rvals::SteelVal::BoolV(false)]);
                self.fall_through(r, InputEvent::Key(key));
            }
        }
    }

    /// Take the menu at `r` off the stack, handing back its model so the
    /// caller can fire the one callback this layer owes
    /// (`.take()`-equivalent one-shot discipline via `truncate`) — shared by
    /// every `menu_input` retirement path (Enter, Escape, a stray key, a
    /// fresh mouse gesture).
    fn take_menu(&mut self, r: LayerRef) -> MenuModel {
        let mut removed = self.state.input.truncate(r);
        let Some(menu) = removed.pop().and_then(|l| l.downcast::<MenuModel>()) else {
            unreachable!("dispatch_at already checked kind(r) == MenuModel");
        };
        *menu
    }

    /// Handles one key while the bottom drawer is open. Movement, half-page
    /// scroll, and `Enter` (which fires `on-select` repeatedly across a
    /// browse session, unlike the menu, without closing the drawer) are
    /// handled in place; `Esc` retires the layer and fires `#f`; any other
    /// key falls through completely untouched (no close, no callback),
    /// leaving the drawer open while focus moves to whatever the fallen-
    /// through key does (Helix-style browse-while-editing).
    ///
    /// No mode gate of its own — same reasoning as [`Self::menu_input`]. A
    /// mouse event falls through untouched, same as any other key the
    /// drawer doesn't bind — browsing never blocks the cursor from moving.
    pub(super) fn drawer_input(&mut self, r: LayerRef, ev: InputEvent) {
        let key = match ev {
            InputEvent::Key(key) => key,
            // Swallowed like every other key the drawer doesn't bind to
            // movement/scroll/Enter/Esc — stays open, same as today's
            // `Paste` handling under a drawer (`bracketed_paste.rs`'s old
            // menu/drawer guard).
            InputEvent::Paste(_) => return,
            InputEvent::Mouse(mouse) => {
                self.fall_through(r, InputEvent::Mouse(mouse));
                return;
            }
        };
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let drawer = self
                    .state
                    .input
                    .drawer_mut()
                    .expect("dispatch_at already checked kind(r) == DrawerModel");
                if drawer.selected + 1 < drawer.items.len() {
                    drawer.selected += 1;
                    self.clamp_drawer_scroll();
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let drawer = self
                    .state
                    .input
                    .drawer_mut()
                    .expect("dispatch_at already checked kind(r) == DrawerModel");
                if drawer.selected > 0 {
                    drawer.selected -= 1;
                    self.clamp_drawer_scroll();
                }
            }
            KeyCode::Char('d') if key.modifiers.contains(Modifiers::CONTROL) => {
                let half = (self.drawer_visible_rows() / 2).max(1);
                if let Some(drawer) = self.state.input.drawer_mut() {
                    drawer.selected =
                        (drawer.selected + half).min(drawer.items.len().saturating_sub(1));
                }
                self.clamp_drawer_scroll();
            }
            KeyCode::Char('u') if key.modifiers.contains(Modifiers::CONTROL) => {
                let half = (self.drawer_visible_rows() / 2).max(1);
                if let Some(drawer) = self.state.input.drawer_mut() {
                    drawer.selected = drawer.selected.saturating_sub(half);
                }
                self.clamp_drawer_scroll();
            }
            KeyCode::Enter => {
                let drawer = self
                    .state
                    .input
                    .drawer()
                    .expect("dispatch_at already checked kind(r) == DrawerModel");
                let idx = steel::rvals::SteelVal::IntV(drawer.selected as isize);
                let callback = drawer.callback.clone();
                self.state.queue_steel_call(callback, vec![idx]);
            }
            KeyCode::Escape => {
                let mut removed = self.state.input.truncate(r);
                let Some(drawer) = removed.pop().and_then(|l| l.downcast::<DrawerModel>()) else {
                    unreachable!("dispatch_at already checked kind(r) == DrawerModel");
                };
                self.state
                    .queue_steel_call(drawer.callback, vec![steel::rvals::SteelVal::BoolV(false)]);
                self.state.sync_drawer_view();
            }
            _ => {
                self.fall_through(r, InputEvent::Key(key));
            }
        }
    }

    /// Handles one event while a `Scrollable` popup is open (a `PopupModel`
    /// layer — a `Sticky` popup lives in a mode layer's slot instead and is
    /// never dispatch's target). Ctrl-u/Ctrl-d consume the key only when
    /// [`Self::scroll_popup`] finds content past one screenful; every other
    /// key, a paste, and any mouse gesture retire the layer and fall
    /// through this same call, so a short popup never blocks the buffer's
    /// own half-page scroll and a click still moves the cursor underneath
    /// it. Unlike `Confirm`/`Menu`'s mouse arm, this doesn't gate on
    /// [`is_fresh_gesture`]: nothing opens a popup mid-gesture, so there is
    /// no release to protect the way a click-opened confirm needs its own
    /// `Up` protected.
    pub(super) fn popup_input(&mut self, r: LayerRef, ev: InputEvent) {
        let key = match ev {
            InputEvent::Key(key) => key,
            InputEvent::Paste(_) | InputEvent::Mouse(_) => {
                self.state.input.truncate(r);
                self.fall_through(r, ev);
                return;
            }
        };
        if key.modifiers.contains(Modifiers::CONTROL)
            && let KeyCode::Char(c @ ('u' | 'd')) = key.code
            && self.scroll_popup(c == 'd')
        {
            return;
        }
        self.state.input.truncate(r);
        self.fall_through(r, InputEvent::Key(key));
    }

    /// Scrolls the open popup half its visible content height, in the
    /// direction given by `down`. Reads the visible height and total row
    /// count from the *already-resolved* view for the popup's current
    /// layout — this same frame's paint — rather than recomputing geometry,
    /// so the scroll step always matches what's on screen. Both views are
    /// re-synced every frame regardless (`Editor::sync_popup_view`/
    /// `sync_popup_band_view`), so no explicit sync is needed here. Before
    /// the first frame both are empty — a documented no-op, same as
    /// `picker_input`'s own geometry.
    ///
    /// Returns `true` if the popup actually has content past one screenful
    /// (`max_scroll > 0`) — [`Self::popup_input`] uses this to tell a real
    /// scroll from a popup too short to scroll, so Ctrl-d/Ctrl-u fall
    /// through to their usual buffer effect instead of being silently eaten.
    fn scroll_popup(&mut self, down: bool) -> bool {
        let Some(layout) = self.state.input.popup().map(|p| &p.layout) else {
            return false;
        };
        let (inner_h, total) = match layout {
            hume_ui::popup::PopupLayout::Cursor => {
                let Some(pair) = self
                    .state
                    .views
                    .popup
                    .read()
                    .as_ref()
                    .map(|s| (s.rect.height.saturating_sub(2) as usize, s.lines.len()))
                else {
                    return false;
                };
                pair
            }
            hume_ui::popup::PopupLayout::Docked => {
                let Some(total) = self
                    .state
                    .views
                    .popup_band
                    .read()
                    .as_ref()
                    .map(|s| s.lines.len())
                else {
                    return false;
                };
                let max = hume_engine::pipeline::EngineView::bottom_band_max(
                    self.view.last_terminal_area.height,
                );
                (hume_ui::popup::band_visible_rows(total, max), total)
            }
        };
        let max_scroll = total.saturating_sub(inner_h);
        if max_scroll == 0 {
            return false;
        }
        let Some(popup) = self.state.input.popup_mut() else {
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

    /// Number of drawer rows visible at once — computed via
    /// `hume_ui::drawer::visible_rows`, the same arithmetic
    /// `DrawerWidget::height` uses to size what it paints next frame. `max`
    /// is `EngineView::bottom_band_max` of the last-rendered *terminal*
    /// height (not the already-chrome-reduced pane height) — the same call
    /// the engine itself makes, so this can never drift from what it will
    /// next paint. Shared by `clamp_drawer_scroll` and the Ctrl-u/Ctrl-d
    /// half-page handlers so "half a page" always agrees with what's on
    /// screen.
    fn drawer_visible_rows(&self) -> usize {
        let max =
            hume_engine::pipeline::EngineView::bottom_band_max(self.view.last_terminal_area.height);
        let Some(drawer) = self.state.input.drawer() else {
            return 0;
        };
        hume_ui::drawer::visible_rows(drawer.items.len(), max)
    }

    /// Clamps `drawer.scroll` so `drawer.selected` stays within the visible
    /// window (`clamp_scroll_to_window`, shared with `PickerSession::
    /// move_selection`), then syncs the view.
    fn clamp_drawer_scroll(&mut self) {
        let visible_rows = self.drawer_visible_rows();
        let Some(drawer) = self.state.input.drawer_mut() else {
            return;
        };
        drawer.scroll =
            hume_ui::menu_box::clamp_scroll_to_window(drawer.selected, drawer.scroll, visible_rows);
        self.state.sync_drawer_view();
    }

    /// Handles one key while the picker is open. Always fully consumes —
    /// unlike the menu (a stray key closes it and falls through) or the
    /// drawer (a stray key falls through untouched), the picker is
    /// full-modal: it owns the entire interaction, so an unrecognized key
    /// (Left/Right/Home/Tab/…) is simply consumed and ignored rather than
    /// leaking through to whatever mode sits underneath. `r` is unused —
    /// every retirement path goes through `picker::close_picker[_with]`,
    /// which finds the picker's own ref itself rather than needing this
    /// call's.
    ///
    /// `on_select` fires exactly once via `.take()`-equivalent truncate +
    /// `queue_steel_call` (never invoked inline) — same one-shot discipline
    /// as the menu. `visible_rows` comes from `panel_geometry` against the
    /// same `last_pane_area` the next frame's `sync_picker_view` will use,
    /// so a keystroke and the following paint always agree on how many rows
    /// are visible (before the first frame, geometry is `None` and paging is
    /// a documented no-op on the store). A mouse event — click, drag, or
    /// wheel — is swallowed the same way: full-modal means the picker owns
    /// the pointer too, not just the keyboard, so a click can't move the
    /// cursor in the buffer underneath it.
    pub(super) fn picker_input(&mut self, _r: LayerRef, ev: InputEvent) {
        let key = match ev {
            InputEvent::Key(key) => key,
            // Flattened to one line, same as a minibuffer paste, then
            // appended to the query in one bulk mutation — one rerank, at
            // most one `on_query_change` callback, instead of one per
            // pasted char.
            InputEvent::Paste(text) => {
                let cb = self
                    .picker_mut()
                    .insert_str(&super::bracketed_paste::flatten_single_line(&text));
                self.queue_query_change(cb);
                return;
            }
            InputEvent::Mouse(_) => return,
        };
        let visible_rows = hume_ui::picker_panel::panel_geometry(self.view.last_pane_area)
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
            // `div_ceil`, not the `(visible_rows / 2).max(1)` the drawer and
            // `cmd_half_page_*` use, so this is `0` when `visible_rows` is `0` —
            // keeping paging a documented no-op before the first frame.
            KeyCode::Char('d') if key.modifiers.contains(Modifiers::CONTROL) => {
                Some(visible_rows.div_ceil(2) as isize)
            }
            KeyCode::Char('u') if key.modifiers.contains(Modifiers::CONTROL) => {
                Some(-(visible_rows.div_ceil(2) as isize))
            }
            _ => None,
        };
        if let Some(delta) = step {
            self.picker_mut().move_selection(delta, visible_rows);
            return;
        }

        match key.code {
            KeyCode::Backspace => {
                // `None` (and so no fire) both on an already-empty query and
                // on a non-live session — see `pop_grapheme`'s doc.
                let cb = self.picker_mut().pop_grapheme();
                self.queue_query_change(cb);
            }
            KeyCode::Enter => {
                // No match (or nothing pushed yet) behaves like Esc — Enter
                // is always a terminal action, never a silent no-op.
                self.close_picker_with_selection(None);
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
                let cb = self.picker_mut().insert_char(ch);
                self.queue_query_change(cb);
            }
            _ => {
                // `#:actions` — tried only here, after every built-in key
                // above has already had first refusal, so a declared action
                // can never override movement/Backspace/Enter/Escape/query
                // input (see `PickerSession::action_for`'s doc).
                if let Some(proc) = self.picker_mut().action_for(key).cloned() {
                    self.close_picker_with_selection(Some(proc));
                }
            }
        }
    }

    /// Queues `cb` — the `on_query_change` callback a `PickerSession` query
    /// mutator (`insert_char`/`pop_grapheme`) just returned, if any — with
    /// `(token query)` as it stands right now, via `queue_steel_call`,
    /// never invoked inline, same discipline as every other picker callback
    /// (`close_picker`). The token lets `live-picker!`'s Scheme wrapper
    /// (`bootstrap.scm`) spawn/stop/replace against the right session
    /// without closing over a not-yet-bound `define` — see
    /// `builtins/mod.rs`'s `picker!`/`live-picker!` rationale block. A bare
    /// `None` is a silent no-op: the mutator itself already decided there
    /// was nothing to fire (a `picker!` session, or a backspace on an
    /// already-empty query). Debouncing (a query changes on every
    /// keystroke, but a live source shouldn't re-spawn on every one) is
    /// composed once inside `live-picker!`'s own wrapper, not hand-wired by
    /// every plugin author — keystrokes are human-rate, unlike
    /// `debounce_viewport_change`'s fire site (`timer_bridge.rs`), which is
    /// every scroll step of every frame and so earns its own Rust-side
    /// coalescer.
    fn queue_query_change(&mut self, cb: Option<steel::rvals::SteelVal>) {
        let Some(cb) = cb else {
            return;
        };
        let session = self.picker_mut();
        let token = session.token();
        let query = session.query().to_string();
        self.state.queue_steel_call(
            cb,
            vec![
                steel::rvals::SteelVal::IntV(token as isize),
                steel::rvals::SteelVal::StringV(query.into()),
            ],
        );
    }

    /// The open picker session — only ever called from `picker_input`, which
    /// `dispatch_at` only reaches while a `Picker` layer is on top.
    fn picker_mut(&mut self) -> &mut super::super::picker::PickerSession {
        self.state
            .input
            .picker_mut()
            .expect("picker_input is only called while a Picker layer is on top of the stack")
    }

    /// Close the picker, firing `callback` (or `on_select` when `None`) with
    /// the selected payload — `#f` when nothing matches, so accepting is
    /// always a terminal action rather than a silent no-op.
    fn close_picker_with_selection(&mut self, callback: Option<steel::rvals::SteelVal>) {
        let payload = self
            .picker_mut()
            .selected_payload()
            .cloned()
            .unwrap_or(steel::rvals::SteelVal::BoolV(false));
        super::super::picker::close_picker_with(&mut self.state, callback, payload);
    }
}
