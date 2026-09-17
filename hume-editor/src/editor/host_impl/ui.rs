//! `EditorHostImpl`'s cursor-anchored popup, selection menu, bottom
//! drawer, minibuffer prompt, and the fuzzy-finder picker.

use super::EditorHostImpl;
use crate::editor::Severity;
use crate::editor::input_stack::picker;
use crate::editor::input_stack::{
    BaseLayer, DrawerLayer, MenuLayer, PickerSession, PopupLayer, PromptLayer,
};
use hume_scripting::host::{
    LivePickerOpts, PickerFeedMode, PickerOpts, PickerSourceOpts, PopupKind, UiHost,
};

impl<'a> EditorHostImpl<'a> {
    /// Synchronously parses `text` through the grammar named `lang`, if one
    /// is registered — `None` otherwise (no such grammar), which leaves the
    /// popup rendering plain. `show_popup`'s only caller, shared across its
    /// cursor and docked layouts.
    fn build_markup_syntax(
        &self,
        lang: &str,
        text: &str,
    ) -> Option<crate::editor::popup_syntax::MarkupSyntax> {
        let lang_id = self.state.config.languages.id_of(lang)?;
        let bundle = std::sync::Arc::clone(self.state.config.languages.grammar(lang_id)?);
        let text = hume_editing::text::BufferText::from(text);
        let syntax = hume_treesitter::syntax::Syntax::attach_sync(
            bundle,
            &text,
            &self.state.config.languages.grammar_snapshot(),
        );
        Some(crate::editor::popup_syntax::MarkupSyntax { syntax, text })
    }
}

impl<'a> UiHost for EditorHostImpl<'a> {
    // ── Minibuffer prompt ────────────────────────────────────────────────
    fn prompt(
        &mut self,
        label: String,
        prefill: String,
        callback: steel::rvals::SteelVal,
    ) -> Result<(), String> {
        // Not "a Command-mode minibuffer is open" — a `prompt!` called from
        // a `:command`'s body runs while that command line's own `Command`
        // layer is still on the stack (it's truncated only after the
        // command returns, per truncate-before-execute). Only a `Prompt`
        // layer already on top means a *prior* `prompt!` call has taken
        // over the session.
        if self
            .state
            .input
            .is::<PromptLayer>(self.state.input.mode_layer())
        {
            return Err("prompt!: a minibuffer session is already open".to_string());
        }
        let cursor = prefill.len();
        self.state.history.begin_session_all();
        self.state.push_mode_layer(
            self.view,
            PromptLayer {
                minibuf: crate::editor::MiniBuffer {
                    prompt: label,
                    input: prefill,
                    cursor,
                },
                callback,
            },
        );
        Ok(())
    }

    // ── Cursor-anchored / docked popup ───────────────────────────────────
    /// `Scrollable` pushes its own `Popup` layer, gated only against a
    /// full-modal `Picker` — unlike `show_menu`/`show_drawer_list` below, a
    /// late hover response must still open even while a references drawer is
    /// up (browse-while-editing is the drawer's whole point), and a popup
    /// owns no input beyond Ctrl-u/Ctrl-d, so it never conflicts with
    /// whatever else is open. A picker is the one exception: it's
    /// full-modal and paints over everything else, so a popup landing above
    /// it would own Ctrl-u/d without ever being visible — dropped the same
    /// way a stale `show-menu!`/`show-drawer-list!` response is. `Sticky`
    /// instead writes into the *current* mode layer's own slot —
    /// `Base`/`Insert` are the only kinds with one (`sticky_popup_slot_mut`
    /// is the SSOT for that), so a `Sticky` `show-popup!` with any other
    /// layer on top drops silently (`Trace`, `Ok`), same shape as
    /// `show_menu`'s own staleness drop. `Scrollable` gets its
    /// clear-every-home-first policy from `PopupLayer::setup`, run by
    /// `push_layer`; the `Sticky` arm never pushes (it writes straight into
    /// the slot), so it clears explicitly here instead — both kinds end up
    /// clearing the same way, which is what makes `(show-popup! …)` replace
    /// rather than stack regardless of which of the two was showing before.
    fn show_popup(
        &mut self,
        text: String,
        kind: PopupKind,
        docked: bool,
        lang: Option<String>,
    ) -> Result<(), String> {
        let layout = if docked {
            hume_ui::popup::PopupLayout::Docked
        } else {
            hume_ui::popup::PopupLayout::Cursor
        };
        let syntax = lang.and_then(|lang| self.build_markup_syntax(&lang, &text));
        let model = PopupLayer {
            text,
            scroll: 0,
            syntax,
            layout,
            content: None,
        };
        match kind {
            PopupKind::Sticky => {
                if self.state.input.sticky_popup_slot_mut().is_none() {
                    self.state.report(
                        Severity::Trace,
                        "show-popup!: no mode layer can hold a sticky popup right now — ignored"
                            .to_string(),
                    );
                    return Ok(());
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
                // everything else (`register_overlays`'s fixed z-order) —
                // a popup landing above it would own Ctrl-u/d without ever
                // being visible. Dropped the same way a stale
                // show-menu!/show-drawer-list! response is: the user moved
                // on to something that occludes it before this had a
                // chance to land.
                if self.state.input.picker().is_some() {
                    self.state.report(
                        Severity::Trace,
                        "show-popup!: a picker is open — ignored".to_string(),
                    );
                    return Ok(());
                }
                self.state.push_layer(self.view, model);
            }
        }
        Ok(())
    }

    /// Idempotent — clears whichever home currently holds a popup, or does
    /// nothing if neither does, same as `close_menu`/`close_drawer` below: a
    /// `Popup` layer is never buried (see `PopupLayer`'s `Layer` doc,
    /// `input_stack/stack.rs`) and a `Sticky` popup's slot never occupies
    /// `top()` at all, so this can never observe one it isn't allowed to
    /// close.
    fn close_popup(&mut self) -> Result<(), String> {
        self.state.input.clear_popups();
        Ok(())
    }

    // ── Selection menu ────────────────────────────────────────────────────
    fn show_menu(
        &mut self,
        items: Vec<String>,
        callback: steel::rvals::SteelVal,
    ) -> Result<(), String> {
        // Async staleness: the request that led here (a `codeAction`
        // response callback) fired against an earlier stack state, and
        // either the mode layer or a *modal* overlay above it may have
        // moved since — the user left Normal, or opened a picker while the
        // response was in flight (a non-modal drawer/popup staying open
        // doesn't count — `InputStack::is_modal`). Both are timing, not a
        // plugin bug, so this drops silently (`Trace`, `Ok`) rather than
        // erroring, which would abort the whole `run_call_batch` this
        // `Call` was batched into. `top` `Menu` is the self-replace
        // exception, same shape as `show_drawer_list`'s own `top_is_drawer`:
        // a second `lsp-code-action` response while the first menu is still
        // open replaces it rather than being read as stale.
        if !self
            .state
            .input
            .is::<BaseLayer>(self.state.input.mode_layer())
            || !self.state.input.is_settled_for::<MenuLayer>()
        {
            self.state.report(
                Severity::Trace,
                "show-menu!: the stack moved before the menu could open — ignored".to_string(),
            );
            return Ok(());
        }
        // Retires a prior `Menu` on the self-replace path, firing its
        // callback with `#f` explicitly — `take_layer`, not `retire`
        // (`truncate_layers`)/`MenuLayer::tear_down` (empty by design, so an
        // explicit `close-menu!` reaching a buried Menu some other way stays
        // silent): only *this* path, a genuine refresh, should fire one.
        // Any open popup is `MenuLayer::setup`'s concern, run by
        // `push_layer` below.
        if let Some(r) = self.state.input.ref_of::<MenuLayer>() {
            let old = self.state.take_layer::<MenuLayer>(self.view, r);
            self.state
                .queue_steel_call(old.callback, vec![steel::rvals::SteelVal::BoolV(false)]);
        }
        self.state.push_layer(
            self.view,
            MenuLayer {
                rows: hume_ui::popup::MenuRows::measure(std::sync::Arc::new(items)),
                selected: 0,
                callback,
            },
        );
        Ok(())
    }

    /// Idempotent — a no-op if no menu is open. Truncates at the menu's own
    /// ref rather than only when it's `top()`: a `Popup` (non-modal) can now
    /// land above it, so being buried is an ordinary state, not a mistake —
    /// same as `close_drawer` below.
    fn close_menu(&mut self) -> Result<(), String> {
        self.state.retire::<MenuLayer>(self.view);
        Ok(())
    }

    // ── Bottom drawer ──────────────────────────────────────────────────────
    fn show_drawer_list(
        &mut self,
        items: Vec<String>,
        callback: steel::rvals::SteelVal,
    ) -> Result<(), String> {
        // Same async staleness as `show_menu` above (a references response
        // landing after the user left Normal, or after a *modal* overlay
        // opened while it was in flight) — see its comment. `top` `Drawer`
        // is the self-replace exception, same shape as `completion-begin!`'s
        // own `top` `Completion` case below: a second `show-drawer-list!`
        // call while the first is still open (a `:refresh`-style re-run,
        // or a references response the user re-triggered before the first
        // one closed) replaces it rather than being read as stale — closed
        // the same way `close-drawer!` already closes one, without firing
        // its callback, since the new call is what Steel considers "done"
        // with the old drawer.
        if !self
            .state
            .input
            .is::<BaseLayer>(self.state.input.mode_layer())
            || !self.state.input.is_settled_for::<DrawerLayer>()
        {
            self.state.report(
                Severity::Trace,
                "show-drawer-list!: the stack moved before the drawer could open — ignored"
                    .to_string(),
            );
            return Ok(());
        }
        self.state.retire::<DrawerLayer>(self.view);
        self.state.push_layer(
            self.view,
            DrawerLayer {
                items: std::sync::Arc::new(items),
                selected: 0,
                scroll: 0,
                callback,
            },
        );
        self.state.sync_drawer_view();
        Ok(())
    }

    /// Idempotent — a no-op if no drawer is open. Truncates at the drawer's
    /// own ref rather than only when it's `top()`: since the drawer stays
    /// open across `Insert`/a `Popup`/etc. by design, being buried is its
    /// *normal* state, not a mistake — see `show_drawer_list`'s own doc.
    fn close_drawer(&mut self) -> Result<(), String> {
        self.state.retire::<DrawerLayer>(self.view);
        self.state.sync_drawer_view();
        Ok(())
    }

    // ── Fuzzy picker ──────────────────────────────────────────────────────
    fn open_picker(
        &mut self,
        items: Vec<(String, steel::rvals::SteelVal)>,
        on_select: steel::rvals::SteelVal,
        opts: PickerOpts,
    ) -> Result<u64, String> {
        let mut session = PickerSession::new(on_select, opts);
        let token = session.token();
        session.seed(picker::picker_items(items));
        picker::open_picker(self.state, self.view, session);
        Ok(token)
    }

    fn open_live_picker(
        &mut self,
        on_select: steel::rvals::SteelVal,
        opts: LivePickerOpts,
    ) -> Result<u64, String> {
        let session = PickerSession::new_live(on_select, opts);
        let token = session.token();
        picker::open_picker(self.state, self.view, session);
        Ok(token)
    }

    fn picker_feed(
        &mut self,
        token: u64,
        items: Vec<(String, steel::rvals::SteelVal)>,
        mode: PickerFeedMode,
    ) -> bool {
        let Some(session) = picker::session_for_token(self.state, token) else {
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
        token: u64,
        cmd: &str,
        args: Vec<String>,
        opts: PickerSourceOpts,
    ) -> Result<bool, String> {
        crate::editor::picker_source::spawn_source(self.state, token, cmd, args, opts)
    }

    fn picker_source_stop(&mut self, token: u64) -> bool {
        crate::editor::picker_source::stop_source(self.state, token)
    }

    fn picker_close(&mut self, token: Option<u64>) {
        if let Some(token) = token
            && picker::session_for_token(self.state, token).is_none()
        {
            return;
        }
        picker::close_picker(self.state, self.view, steel::rvals::SteelVal::BoolV(false));
    }
}
