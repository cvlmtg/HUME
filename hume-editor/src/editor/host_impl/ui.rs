//! `EditorHostImpl`'s cursor-anchored popup, selection menu, bottom
//! drawer, minibuffer prompt, and the fuzzy-finder picker.

use super::EditorHostImpl;
use crate::editor::Severity;
use crate::editor::input_stack::{
    BaseLayer, DrawerLayer, DrawerSelect, MenuLayer, PickerSession, PopupLayer,
};
use crate::editor::input_stack::{menu, picker, prompt};
use crate::editor::syntax::GrammarLoads;
use hume_scripting::PaneHandle;
use hume_scripting::host::{
    BufferHost, DrawerItems, HostToken, LivePickerOpts, NonEmpty, PickerFeedMode, PickerOpts,
    PickerSourceOpts, PopupKind, UiHost,
};

impl<'a> UiHost for EditorHostImpl<'a> {
    // ── Minibuffer prompt ────────────────────────────────────────────────
    fn on_next_char(&mut self, callback: hume_scripting::Rooted) -> Result<(), String> {
        crate::editor::input_stack::read_steel_char(self.state, self.view, callback)
    }

    fn prompt(
        &mut self,
        pane: PaneHandle,
        label: String,
        prefill: String,
        callback: hume_scripting::Rooted,
    ) -> Result<(), String> {
        self.require_focused_pane(pane)?;
        prompt::open_prompt(self.state, self.view, label, prefill, callback)
    }

    // ── Cursor-anchored / docked popup ───────────────────────────────────
    /// `Scrollable` pushes its own `Popup` layer, gated only against a
    /// full-modal `Picker`. Unlike `show_menu`/`show_drawer_list` below, a
    /// late hover response must still open even while a references drawer is
    /// up (browse-while-editing is the drawer's whole point), and a popup
    /// owns no input beyond Ctrl-u/Ctrl-d, so it never conflicts with
    /// whatever else is open. A picker is the one exception: it's
    /// full-modal and paints over everything else, so a popup landing above
    /// it would own Ctrl-u/d without ever being visible, so it is dropped the same
    /// way a stale `show-menu!`/`show-drawer-list!` response is. `Sticky`
    /// instead writes into the *current* mode layer's own slot:
    /// `Base`/`Insert` are the only kinds with one (`sticky_popup_slot_mut`
    /// is the SSOT for that), so a `Sticky` `show-popup!` with any other
    /// layer on top drops silently (`Trace`, `Ok`), same shape as
    /// `show_menu`'s own staleness drop. `Scrollable` gets its
    /// clear-every-home-first policy from `PopupLayer::setup`, run by
    /// `push_layer`; the `Sticky` arm never pushes (it writes straight into
    /// the slot), so it clears explicitly here instead. Both kinds end up
    /// clearing the same way, which is what makes `(show-popup! …)` replace
    /// rather than stack regardless of which of the two was showing before.
    fn show_popup(
        &mut self,
        pane: PaneHandle,
        text: String,
        kind: PopupKind,
        docked: bool,
        lang: Option<String>,
    ) -> Result<Option<HostToken>, String> {
        self.require_focused_pane(pane)?;
        let layout = if docked {
            hume_ui::popup::PopupLayout::Docked
        } else {
            hume_ui::popup::PopupLayout::Cursor
        };
        let mut loads =
            GrammarLoads::new(&mut self.state.config.languages, &mut self.view.registry);
        let syntax = lang.and_then(|lang| {
            crate::editor::popup_syntax::build_markup_syntax(&mut loads, &lang, &text)
        });
        let model = PopupLayer::new(
            text,
            syntax,
            layout,
            &mut self.view.theme,
            &self.view.registry,
        );
        let token = model.token();
        match kind {
            PopupKind::Sticky => {
                if self.state.input.sticky_popup_slot_mut().is_none() {
                    self.state.report(
                        Severity::Trace,
                        "show-popup!: no mode layer can hold a sticky popup right now, ignored"
                            .to_string(),
                    );
                    return Ok(None);
                }
                self.state.input.clear_popups();
                *self
                    .state
                    .input
                    .sticky_popup_slot_mut()
                    .expect("slot presence checked above") = Some(model);
            }
            PopupKind::Scrollable => {
                // A picker is full-modal (owns every key) and paints over
                // everything else (`OverlayViews::overlays`' fixed z-order), so
                // a popup landing above it would own Ctrl-u/d without ever
                // being visible. Dropped the same way a stale
                // show-menu!/show-drawer-list! response is: the user moved
                // on to something that occludes it before this had a
                // chance to land.
                if self.state.input.picker().is_some() {
                    self.state.report(
                        Severity::Trace,
                        "show-popup!: a picker is open, ignored".to_string(),
                    );
                    return Ok(None);
                }
                self.state.push_layer(self.view, model);
            }
        }
        Ok(Some(token))
    }

    /// Idempotent: a no-op if no popup is showing or the showing one
    /// carries a different token, same as `close_menu`/`close_drawer`
    /// below. Clearing both homes is sound because at most one popup is
    /// ever showing: every popup opener clears both homes first.
    fn close_popup(&mut self, token: HostToken) -> Result<(), String> {
        if self.state.input.popup().is_some_and(|p| p.token() == token) {
            self.state.input.clear_popups();
        }
        Ok(())
    }

    // ── Selection menu ────────────────────────────────────────────────────
    fn show_menu(
        &mut self,
        pane: PaneHandle,
        items: NonEmpty<String>,
        callback: hume_scripting::Rooted,
    ) -> Result<Option<HostToken>, String> {
        self.require_focused_pane(pane)?;
        menu::open_menu(self.state, self.view, items, callback)
    }

    /// Idempotent: a no-op if no menu is open, or if `token` doesn't match
    /// the open menu's own (the caller's menu was replaced since). Excises at the menu's own
    /// ref rather than only when it's `top()`: a `Popup` (non-modal) can now
    /// land above it, so being buried is an ordinary state, not a mistake.
    /// `MenuLayer::removal_scope` is `SelfOnly` for exactly this reason,
    /// same as `close_drawer` below.
    fn close_menu(&mut self, token: HostToken) -> Result<(), String> {
        if self.state.input.menu().is_some_and(|m| m.token() == token) {
            self.state.retire::<MenuLayer>(self.view);
        }
        Ok(())
    }

    // ── Bottom drawer ──────────────────────────────────────────────────────
    fn show_drawer_list(
        &mut self,
        pane: PaneHandle,
        items: DrawerItems,
        callback: hume_scripting::Rooted,
        selected: usize,
    ) -> Result<Option<HostToken>, String> {
        self.require_focused_pane(pane)?;
        // Async staleness; see `EditorState::async_opener_stale`'s own doc.
        // `top` `Drawer` is the self-replace exception below: a second
        // `show-drawer-list!` call while the first is still open (a
        // `:refresh`-style re-run, or a references response the user
        // re-triggered before the first one closed) replaces it rather than
        // being read as stale.
        if self
            .state
            .async_opener_stale::<BaseLayer, DrawerLayer>("show-drawer-list!")
        {
            return Ok(None);
        }
        let drawer = DrawerLayer::new(items, DrawerSelect::Steel(callback), selected);
        Ok(Some(self.state.open_drawer(self.view, drawer)))
    }

    /// Idempotent: a no-op if no drawer is open, or if `token` doesn't
    /// match the open drawer's own (an expected-normal race, the same shape
    /// `update_drawer_list` already has: the caller's drawer was closed or
    /// replaced since). Excises the drawer at its own ref rather than only
    /// when it's `top()`: since the drawer stays open across `Insert`/a
    /// `Popup`/etc. by design, being buried is its *normal* state, not a
    /// mistake; see `show_drawer_list`'s own doc. Excise, not retire's
    /// default: `DrawerLayer::removal_scope` is `SelfOnly` for exactly this
    /// reason (whatever sits above is browsing over it by coincidence, not
    /// because it depends on the drawer being open). This call excises
    /// directly rather than through `retire::<DrawerLayer>` because it needs
    /// the token-matched ref, not just "topmost of type".
    fn close_drawer(&mut self, token: HostToken) -> Result<(), String> {
        if let Some(r) = self.state.input.drawer_ref_with_token(token) {
            self.state.excise_layer(self.view, r);
        }
        self.state.sync_drawer_view();
        Ok(())
    }

    /// In-place refresh. Clamping `selected` into the new list lives in
    /// `EditorState::set_drawer_items`; `scroll` is caught by the frame-time
    /// `clamp_drawer_scroll_to_terminal` pass instead (see its own doc), so
    /// this needs no terminal geometry of its own. A buried drawer refreshes
    /// the same way: browse-while-editing is its normal state, and the model
    /// is what `sync_drawer_view` mirrors, not its stack position. `token`
    /// must match the open drawer's own; see `set_drawer_items`'s own doc
    /// for the no-op contract on a mismatch.
    fn update_drawer_list(
        &mut self,
        token: HostToken,
        items: DrawerItems,
        callback: hume_scripting::Rooted,
        selected: usize,
    ) -> bool {
        self.state
            .set_drawer_items(token, items, callback, selected)
    }

    fn drawer_selected_index(&self, token: HostToken) -> Option<usize> {
        self.state
            .input
            .drawer_with_token(token)
            .map(|d| d.selected)
    }

    // ── Fuzzy picker ──────────────────────────────────────────────────────
    fn open_picker(
        &mut self,
        pane: PaneHandle,
        items: Vec<(String, hume_scripting::Rooted)>,
        on_select: hume_scripting::Rooted,
        opts: PickerOpts,
    ) -> Result<HostToken, String> {
        self.require_focused_pane(pane)?;
        let session = PickerSession::new(on_select, opts, picker::picker_items(items));
        picker::open_picker(self.state, self.view, session, "picker!")
    }

    fn open_live_picker(
        &mut self,
        pane: PaneHandle,
        on_select: hume_scripting::Rooted,
        opts: LivePickerOpts,
    ) -> Result<HostToken, String> {
        self.require_focused_pane(pane)?;
        let session = PickerSession::new_live(on_select, opts);
        picker::open_picker(self.state, self.view, session, "live-picker!")
    }

    fn picker_feed(
        &mut self,
        token: HostToken,
        items: Vec<(String, hume_scripting::Rooted)>,
        mode: PickerFeedMode,
    ) -> bool {
        let Some(session) = picker::session_for_feed(self.state, token) else {
            return false;
        };
        let items = picker::picker_items(items);
        match mode {
            PickerFeedMode::Append => session.push(items),
            PickerFeedMode::Replace => session.replace(items),
        }
        true
    }

    fn picker_source_spawn(
        &mut self,
        token: HostToken,
        cmd: &str,
        args: Vec<String>,
        opts: PickerSourceOpts,
    ) -> Result<bool, String> {
        crate::editor::picker_source::spawn_source(self.state, token, cmd, args, opts)
    }

    fn picker_source_stop(&mut self, token: HostToken) -> bool {
        crate::editor::picker_source::stop_source(self.state, token)
    }

    fn picker_close(&mut self, token: HostToken) {
        if picker::session_for_token(self.state, token).is_none() {
            return;
        }
        picker::close_picker(self.state, self.view);
    }
}
