//! `UiHost` — moved out of `host_impl.rs`'s per-capability split.

use super::EditorHostImpl;
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
        // Not `self.state.minibuf.is_some()` — a `prompt!` called from a
        // `:command`'s body runs while that command line's own minibuffer
        // session is still open (it closes only after the command
        // returns). `steel_prompt_callback` is only `Some` once a *prior*
        // `prompt!` call has actually taken over the session.
        if self.state.config.steel_prompt_callback.is_some() {
            return Err("prompt!: a minibuffer session is already open".to_string());
        }
        let cursor = prefill.len();
        self.state.minibuf = Some(crate::editor::MiniBuffer {
            prompt: label,
            input: prefill,
            cursor,
        });
        self.state.config.steel_prompt_callback = Some(callback);
        self.state.history.begin_session_all();
        self.state.set_mode(crate::editor::Mode::Command);
        Ok(())
    }

    // ── Cursor-anchored / docked popup ───────────────────────────────────
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
        self.state.config.popup = Some(crate::editor::overlay_models::PopupModel {
            text,
            kind,
            scroll: 0,
            syntax,
            layout,
            content: None,
        });
        Ok(())
    }

    fn close_popup(&mut self) -> Result<(), String> {
        self.state.config.popup = None;
        Ok(())
    }

    // ── Selection menu ────────────────────────────────────────────────────
    fn show_menu(
        &mut self,
        items: Vec<String>,
        callback: steel::rvals::SteelVal,
    ) -> Result<(), String> {
        // Excludes Insert specifically, not an allowlist of Normal/Extend —
        // a command triggered via `:name` runs while `mode()` still reports
        // `Command` (mode reverts to Normal only after the command body
        // returns), so an allowlist would reject the common `:`-triggered
        // case too.
        if self.state.mode() == hume_engine::types::EditorMode::Insert {
            return Err("show-menu!: not available in Insert mode".to_string());
        }
        self.state.config.menu = Some(crate::editor::overlay_models::MenuModel {
            rows: hume_ui::popup::MenuRows::measure(std::sync::Arc::new(items)),
            selected: 0,
            callback,
        });
        Ok(())
    }

    fn close_menu(&mut self) -> Result<(), String> {
        self.state.config.menu = None;
        Ok(())
    }

    // ── Bottom drawer ──────────────────────────────────────────────────────
    fn show_drawer_list(
        &mut self,
        items: Vec<String>,
        callback: steel::rvals::SteelVal,
    ) -> Result<(), String> {
        self.state.config.drawer = Some(crate::editor::overlay_models::DrawerModel {
            items: std::sync::Arc::new(items),
            selected: 0,
            scroll: 0,
            callback,
        });
        self.state.sync_drawer_view();
        Ok(())
    }

    fn close_drawer(&mut self) -> Result<(), String> {
        self.state.config.drawer = None;
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
        let mut session = crate::editor::picker::PickerSession::new(on_select, opts);
        let token = session.token();
        session.seed(crate::editor::picker::picker_items(items));
        crate::editor::picker::open_picker(self.state, self.lsp.as_deref_mut(), session);
        Ok(token)
    }

    fn open_live_picker(
        &mut self,
        on_select: steel::rvals::SteelVal,
        opts: LivePickerOpts,
    ) -> Result<u64, String> {
        let session = crate::editor::picker::PickerSession::new_live(on_select, opts);
        let token = session.token();
        crate::editor::picker::open_picker(self.state, self.lsp.as_deref_mut(), session);
        Ok(token)
    }

    fn picker_feed(
        &mut self,
        token: u64,
        items: Vec<(String, steel::rvals::SteelVal)>,
        mode: PickerFeedMode,
    ) -> bool {
        let Some(session) = crate::editor::picker::session_for_token(self.state, token) else {
            return false;
        };
        let items = crate::editor::picker::picker_items(items);
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
            && crate::editor::picker::session_for_token(self.state, token).is_none()
        {
            return;
        }
        crate::editor::picker::close_picker(self.state, steel::rvals::SteelVal::BoolV(false));
    }
}
