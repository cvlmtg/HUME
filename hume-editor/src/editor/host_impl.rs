//! [`EditorHostImpl`] — the production implementation of the scripting crate's
//! [`EditorHost`] trait.
//!
//! Holds disjoint borrows of `EditorState` and `EngineView`, which enables the
//! Steel VM (`scripting.steel`) to take `&mut Engine` simultaneously without
//! aliasing editor data. Two construction sites create this:
//!
//! - **Command dispatch** (`editor/mod.rs`, `run_steel_command`): called with the
//!   live editor state and view from the focused pane.
//! - **Init dispatch** (`scripting_setup.rs`): called with the same fields
//!   during `init_scripting`; init-only builtins set settings.

use std::path::{Path, PathBuf};

use hume_engine::pipeline::{BufferId, EngineView, PaneId};

use crate::editor::lsp::LspState;
use crate::editor::registry::MappableCommand;
use crate::editor::timer_bridge::TimerHandle;
use crate::settings::SettingScope;
use crate::ui::statusline::StatusElement;
use hume_scripting::host::{
    BufferHost, CommandHost, CompletionHost, CursorHost, DecorationHost, EditHost, EditorHost,
    LanguageHost, LspHost, OptionValue, OutputHost, SettingsHost, TimerHost, UiHost,
};

use super::{EditorState, Severity};

pub(crate) struct EditorHostImpl<'a> {
    pub(crate) state: &'a mut EditorState,
    pub(crate) view: &'a mut EngineView,
    /// `Some` only at the three call sites that can reach an introspection
    /// builtin (command dispatch, hook fire, queued-call drain) — `None`
    /// everywhere else (init evals, which `require_cmd_ctx!` already blocks
    /// LSP builtins from anyway), so those sites don't need to thread it in.
    /// `&mut` (not `&`) because the LSP completion session lives on
    /// `LspState` — the completion builtins need to write it.
    pub(crate) lsp: Option<&'a mut LspState>,
    /// Same `Some`-at-three-sites shape as `lsp`, for the `(after …)` /
    /// `(cancel-timer! …)` — these mutate (schedule/cancel), so `&LspState`'s
    /// shared-borrow shape doesn't fit; `TimerHandle` bundles the two
    /// `&mut` pieces this needs.
    pub(crate) timers: Option<TimerHandle<'a>>,
    /// `Some` only at the one call site that can reach
    /// [`OutputHost::ensure_inline_output_screen`] (command dispatch) — `None`
    /// everywhere else. `state.inline_output` only ever reaches `Armed` from
    /// that same call site, so `ensure_inline_output_screen`'s early return
    /// guarantees this is never read as `None` when it matters.
    pub(crate) terminal: Option<&'a hume_platform::terminal::SharedTerm>,
}

impl<'a> EditorHostImpl<'a> {
    /// Convenience constructor for the (common) case with no LSP/timer
    /// access — init evals, and every non-LSP/non-timer test in the suite.
    pub(crate) fn new(state: &'a mut EditorState, view: &'a mut EngineView) -> Self {
        Self {
            state,
            view,
            lsp: None,
            timers: None,
            terminal: None,
        }
    }

    /// Look up a buffer by id.
    fn buffer(&self, id: BufferId) -> Option<&crate::editor::buffer::Buffer> {
        self.state.buffers.try_get(id)
    }

    /// Seeded pane-buffer state for the focused (pane, buffer), or `None` when
    /// unseeded (stale or never-focused ids) — the shared guard behind every
    /// live cursor/selection read.
    fn focused_pane_buffer_state(&self) -> Option<&crate::editor::pane_state::PaneBufferState> {
        let buf_id = crate::editor::commands::focused_buffer_id(self.state, self.view);
        self.state
            .panes
            .state
            .get(self.state.focused_pane_id)?
            .get(buf_id)
    }

    /// Delegates to the shared `clear_completion_menu(state, lsp)` free fn
    /// (`lsp/completion.rs`) — this struct holds disjoint `state`/`lsp`
    /// borrows, not a full `Editor`, so it can't call `Editor`'s method of
    /// the same name, but both now share one body.
    fn clear_completion_menu(&mut self) {
        crate::editor::lsp::completion::clear_completion_menu(self.state, self.lsp.as_deref_mut());
    }

    /// Synchronously parses `text` as markdown, if a `markdown` grammar is
    /// registered — `None` otherwise, which leaves the popup rendering
    /// plain, exactly as it did before `#:markdown` existed.
    fn build_popup_syntax(&self, text: &str) -> Option<crate::ui::popup::PopupSyntax> {
        let lang_id = self.state.languages.id_of("markdown")?;
        let bundle = std::sync::Arc::clone(self.state.languages.grammar(lang_id)?);
        let text = hume_editing::text::Text::from(text);
        let syntax = hume_treesitter::syntax::Syntax::attach_sync(
            bundle,
            &text,
            &self.state.languages.grammar_snapshot(),
        );
        Some(crate::ui::popup::PopupSyntax { syntax, text })
    }
}

impl<'a> EditorHost for EditorHostImpl<'a> {
    // ── Optional capability accessors ────────────────────────────────────────
    fn ui(&mut self) -> Option<&mut dyn UiHost> {
        Some(self)
    }
    fn edits(&mut self) -> Option<&mut dyn EditHost> {
        Some(self)
    }
    fn completions(&mut self) -> Option<&mut dyn CompletionHost> {
        Some(self)
    }
    fn decorations(&mut self) -> Option<&mut dyn DecorationHost> {
        Some(self)
    }
    // `Some(self)` unconditionally, even though `self.lsp` is itself an
    // `Option` — every method below already self-guards on `self.lsp.as_deref()`,
    // and a conditional accessor here would change what "no attached server"
    // vs. "no LSP state at all" reports at the Steel boundary.
    fn lsp(&mut self) -> Option<&mut dyn LspHost> {
        Some(self)
    }
    // Same unconditional-Some rationale as `lsp()` above.
    fn timers(&mut self) -> Option<&mut dyn TimerHost> {
        Some(self)
    }
    fn output(&mut self) -> Option<&mut dyn OutputHost> {
        Some(self)
    }
    fn cursor(&mut self) -> &mut dyn CursorHost {
        self
    }
    fn commands(&mut self) -> &mut dyn CommandHost {
        self
    }
    fn language(&mut self) -> &mut dyn LanguageHost {
        self
    }
    fn settings(&mut self) -> &mut dyn SettingsHost {
        self
    }
    fn buffers(&mut self) -> &mut dyn BufferHost {
        self
    }
}

impl<'a> BufferHost for EditorHostImpl<'a> {
    // ── Enumeration ──────────────────────────────────────────────────────────
    fn buffer_ids(&self) -> Vec<BufferId> {
        self.state.buffers.iter().map(|(id, _)| id).collect()
    }
    fn pane_ids(&self) -> Vec<PaneId> {
        self.view.panes.iter().map(|(id, _)| id).collect()
    }

    // ── Buffer reads ─────────────────────────────────────────────────────────
    fn buffer_exists(&self, id: BufferId) -> bool {
        self.buffer(id).is_some()
    }
    fn buffer_path(&self, id: BufferId) -> Option<PathBuf> {
        self.buffer(id)?.path().map(Path::to_path_buf)
    }
    fn buffer_display_name(&self, id: BufferId) -> Option<String> {
        self.buffer(id).map(|buf| buf.display_name().to_owned())
    }
    fn buffer_is_dirty(&self, id: BufferId) -> Option<bool> {
        self.buffer(id).map(|buf| buf.is_dirty())
    }
    fn buffer_stored_language(&self, id: BufferId) -> Option<String> {
        let lang_id = self.buffer(id)?.language?;
        Some(self.state.languages.name_of(lang_id).to_owned())
    }

    // ── Buffer lifecycle ─────────────────────────────────────────────────────
    fn open_buffer(&mut self, path: &Path) -> Result<BufferId, String> {
        let canonical = hume_platform::fs::canonicalize(path)
            .map_err(|e| format!("open-buffer!: {}: {e}", path.display()))?;
        // Language detection is deliberately not done here — see
        // `Effect::DetectBufferLanguage`'s doc; the `open-buffer!` builtin
        // queues it once this returns.
        let (bid, _is_new) = crate::editor::buffer::lifecycle::open_or_dedup_and_notify(
            self.view, self.state, &canonical,
        )
        .map_err(|e| format!("open-buffer!: {}: {e}", canonical.display()))?;
        Ok(bid)
    }
    fn close_buffer(&mut self, id: BufferId) -> Result<BufferId, String> {
        if self.state.buffers.try_get(id).is_none() {
            return Err(format!("close-buffer!: buffer {id:?} does not exist"));
        }
        Ok(crate::editor::buffer::lifecycle::close_buffer_and_notify(
            self.view,
            self.state,
            self.lsp.as_deref_mut(),
            id,
        ))
    }
    fn switch_to_buffer(&mut self, current: BufferId, target: BufferId) -> Result<(), String> {
        crate::editor::buffer::lifecycle::switch_to_buffer_with_jump(
            self.view,
            &self.state.buffers,
            &mut self.state.panes.state,
            &mut self.state.panes.jumps,
            self.state.focused_pane_id,
            current,
            target,
        );
        Ok(())
    }

    fn buffer_generation(&self, id: BufferId) -> Option<u64> {
        Some(self.buffer(id)?.text_gen)
    }

    fn viewport_range(&self, id: BufferId) -> Option<(usize, usize)> {
        crate::editor::lsp::introspect::viewport_range(self.state, self.view, id)
    }
}

impl<'a> SettingsHost for EditorHostImpl<'a> {
    fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String> {
        crate::editor::settings_ops::apply(
            self.state,
            self.view,
            SettingScope::Global,
            key,
            value,
            None,
        )
    }

    fn get_option(&self, key: &str, bid: BufferId) -> Result<OptionValue, String> {
        let overrides = self.state.buffers.try_get(bid).map(|b| &b.overrides);
        crate::settings::setting_value(key, &self.state.settings, overrides)
            .ok_or_else(|| format!("get-option: unknown setting '{key}'"))
    }

    fn configure_statusline(
        &mut self,
        left: Vec<String>,
        center: Vec<String>,
        right: Vec<String>,
    ) -> Result<(), String> {
        // Validate here (for a section-labeled error message), then hand the
        // re-serialized wire string to the chokepoint so the write itself goes
        // through `write_setting` like every other setting — see
        // `settings_ops::apply`'s doc for why a raw field write must not
        // bypass it.
        let parse = |list: Vec<String>, section: &str| -> Result<Vec<StatusElement>, String> {
            list.iter()
                .map(|s| {
                    s.parse::<StatusElement>()
                        .map_err(|e| format!("configure-statusline! {section}: {e}"))
                })
                .collect()
        };
        let left = parse(left, "left")?;
        let center = parse(center, "center")?;
        let right = parse(right, "right")?;

        let join = |elems: &[StatusElement]| {
            elems
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        };
        let wire = format!("{}|{}|{}", join(&left), join(&center), join(&right));

        crate::editor::settings_ops::apply(
            self.state,
            self.view,
            SettingScope::Global,
            "statusline",
            &wire,
            None,
        )
    }

    fn steel_command_budget_ms(&self) -> u64 {
        self.state.settings.steel_command_budget_ms as u64
    }
}

impl<'a> LanguageHost for EditorHostImpl<'a> {
    fn attach_grammar(
        &mut self,
        name: &str,
        grammar_path: &Path,
        symbol: &str,
        highlights_path: &Path,
        injections_path: Option<&Path>,
    ) -> Result<(), String> {
        self.state
            .languages
            .attach_grammar(
                name,
                grammar_path,
                symbol,
                highlights_path,
                injections_path,
                &mut self.view.registry,
            )
            .map_err(|e| format!("register-grammar! '{name}': {e}"))?;
        Ok(())
    }

    fn has_grammar(&self, language: &str) -> bool {
        self.state.languages.has_grammar(language)
    }

    fn register_trigger_chars(&mut self, source: String, language: String, chars: Vec<char>) {
        if chars.is_empty() {
            self.state.trigger_chars.remove(&(source, language));
        } else {
            self.state.trigger_chars.insert((source, language), chars);
        }
    }
}

impl<'a> CommandHost for EditorHostImpl<'a> {
    fn register_command(&mut self, def: hume_scripting::SteelCmdDef) -> Result<(), String> {
        match self.state.registry.get_mappable(&def.name) {
            Some(MappableCommand::Lazy { .. }) | None => {
                self.state.registry.register(MappableCommand::SteelBacked {
                    name: def.name.into(),
                    doc: def.doc.into(),
                    arity: def.arity,
                    is_variadic: def.is_variadic,
                    inline_output: def.inline_output,
                    repeatable: def.repeatable,
                });
                Ok(())
            }
            Some(_) => Err(format!(
                "define-command!: '{}' conflicts with existing command",
                def.name
            )),
        }
    }

    fn unregister_command(&mut self, name: &str) {
        self.state.registry.unregister(name);
    }

    fn register_lazy_command(
        &mut self,
        name: &str,
        plugin: &hume_scripting::PluginId,
    ) -> Result<(), String> {
        if let Some(MappableCommand::Lazy { plugin: owner, .. }) =
            self.state.registry.get_mappable(name)
        {
            return if owner == plugin {
                // Duplicate declare-plugin call for the same plugin — no-op;
                // first declaration wins.
                Ok(())
            } else {
                Err(format!("'{name}' already claimed by lazy plugin '{owner}'"))
            };
        }
        if self.state.registry.contains(name) {
            return Err(format!("'{name}' conflicts with an existing command"));
        }
        self.state.registry.register(MappableCommand::Lazy {
            name: name.to_owned().into(),
            plugin: plugin.clone(),
        });
        Ok(())
    }

    fn lazy_command_owner(&self, name: &str) -> Option<hume_scripting::PluginId> {
        match self.state.registry.get_mappable(name) {
            Some(MappableCommand::Lazy { plugin, .. }) => Some(plugin.clone()),
            _ => None,
        }
    }

    fn unregister_lazy_stubs_of(&mut self, plugin: &hume_scripting::PluginId) {
        self.state.registry.unregister_lazy_stubs_of(plugin);
    }

    fn is_valid_register_name(&self, ch: char) -> bool {
        crate::ops::register::is_valid_register_name(ch)
    }

    fn command_is_native(&self, name: &str) -> Result<bool, String> {
        self.state
            .registry
            .get_mappable(name)
            .map(MappableCommand::is_native)
            .ok_or_else(|| format!("unknown command: {name}"))
    }

    fn run_command_sync(
        &mut self,
        name: &str,
        count: Option<usize>,
        extend: bool,
        register: Option<char>,
    ) -> Result<(), String> {
        let Some(cmd) = self.state.registry.get_mappable(name).cloned() else {
            return Err(format!("unknown command: {name}"));
        };
        if !cmd.is_native() {
            return Err(format!(
                "{name} is not a native command — use call! instead of call-native!"
            ));
        }
        // Arm the register prefix so register-aware commands (yank, delete,
        // paste-after, …) route to the right destination.
        if let Some(r) = register {
            self.state.register_prefix =
                Some(crate::editor::register_ops::RegisterPrefix::Selected(r));
        }
        // Delegate to the shared pipeline — all bookkeeping (paste session, jump
        // list, dot-repeat, last_command) lives there so the sync path is
        // identical to the keypress path.
        crate::editor::commands::run_dispatch_pipeline(
            self.state,
            self.view,
            cmd,
            crate::editor::dispatch::CmdCtx {
                // `count` came from `parse_count_extend`, which decodes a
                // Steel-side count of 0 to `None` — the script's way of asking
                // for "as if no count was typed" (move-down/move-up read this
                // as visual-row movement instead of buffer-line movement).
                count,
                extend,
                arg_source: crate::editor::dispatch::ArgSource::Keymap,
            },
        );
        // Clear the prefix when we armed it, so it does not bleed into the
        // next interactive command.
        if register.is_some() {
            self.state.register_prefix = None;
        }
        Ok(())
    }
}

impl<'a> CursorHost for EditorHostImpl<'a> {
    fn current_line_number(&self) -> Option<usize> {
        let pbs = self.focused_pane_buffer_state()?;
        self.char_index_to_line(pbs.selections.primary().head())
    }

    fn current_selections(&self) -> Option<Vec<(usize, usize, bool)>> {
        let pbs = self.focused_pane_buffer_state()?;
        let primary_index = pbs.selections.primary_index();
        Some(
            pbs.selections
                .iter_sorted()
                .enumerate()
                .map(|(i, sel)| (sel.anchor(), sel.head(), i == primary_index))
                .collect(),
        )
    }

    fn char_index_to_line(&self, idx: usize) -> Option<usize> {
        let buf_id = crate::editor::commands::focused_buffer_id(self.state, self.view);
        let text = self.buffer(buf_id)?.text();
        if idx > text.len_chars() {
            return None;
        }
        Some(text.char_to_line(idx) + 1)
    }

    fn symbol_under_cursor(&self, bid: BufferId) -> String {
        let Some(buf) = self.state.buffers.try_get(bid) else {
            return String::new();
        };
        let pid = self.state.focused_pane_id;
        let Some(pbs) = self
            .state
            .panes
            .state
            .get(pid)
            .and_then(|by_buf| by_buf.get(bid))
        else {
            return String::new();
        };
        let text = buf.text();
        let head = pbs.selections.primary().head();
        let Some(ch) = text.char_at(head) else {
            return String::new();
        };
        if hume_editing::word::classify_char(ch) != hume_editing::word::CharClass::Word {
            return String::new();
        }
        let Some((start, end)) = crate::ops::text_object::inner_word_impl(
            text,
            head,
            hume_editing::word::is_word_boundary,
        ) else {
            return String::new();
        };
        text.slice(start..end + 1).to_string()
    }

    fn selection_spans_full_line(&self, bid: BufferId) -> bool {
        crate::editor::lsp::edits::selection_spans_full_line(self.state, bid)
    }
}

impl<'a> CompletionHost for EditorHostImpl<'a> {
    fn completion_begin(
        &mut self,
        bid: BufferId,
        items: Vec<serde_json::Value>,
        incomplete: bool,
    ) -> Result<(), String> {
        if self.state.buffers.try_get(bid).is_none() {
            return Err("completion-begin!: no such buffer".to_string());
        }
        // A malformed item (e.g. missing the spec-required `label`) is
        // skipped, not fatal to the whole batch — one bad item from a
        // misbehaving server must not silently drop every good one.
        let mut parsed = Vec::with_capacity(items.len());
        for v in &items {
            match crate::editor::lsp::completion::StoredCompletionItem::from_json(v) {
                Ok(item) => parsed.push(item),
                Err(e) => self.state.report(
                    Severity::Trace,
                    format!("completion-begin!: skipped malformed item: {e}"),
                ),
            }
        }
        if parsed.is_empty() {
            // Replaces any open session too — an isIncomplete re-request
            // that comes back empty (or entirely malformed) must close the
            // menu, not leave the old one live.
            self.clear_completion_menu();
            self.state
                .report(Severity::Info, "no completions".to_string());
            return Ok(());
        }
        let Some(session) = crate::editor::lsp::completion::CompletionSession::begin(
            self.state, bid, parsed, incomplete,
        ) else {
            // Benign race: the async completion response landed after the
            // user switched away from `bid`'s pane. Not an error — raising
            // here would abort the whole drain_pending_steel_calls batch and
            // drop every other queued LSP callback/timer this frame.
            self.state.report(
                Severity::Trace,
                "completion-begin!: buffer not shown in focused pane — ignored".to_string(),
            );
            return Ok(());
        };
        let Some(lsp) = self.lsp.as_deref_mut() else {
            return Err("completion-begin!: no LSP state available".to_string());
        };
        lsp.completion = Some(session);
        Ok(())
    }

    fn completion_update_filter(&mut self, text: String) -> Result<(), String> {
        let Some(lsp) = self.lsp.as_deref_mut() else {
            return Err("completion-update-filter!: no LSP state available".to_string());
        };
        let Some(session) = lsp.completion.as_mut() else {
            return Err("completion-update-filter!: no active completion session".to_string());
        };
        session.update_filter(self.state, text);
        Ok(())
    }

    fn completion_top(&self, n: usize) -> Vec<serde_json::Value> {
        self.lsp
            .as_deref()
            .and_then(|lsp| lsp.completion.as_ref())
            .map(|s| s.top(n))
            .unwrap_or_default()
    }

    fn completion_accept(&mut self, idx: usize) -> Result<(), String> {
        let Some(lsp) = self.lsp.as_deref_mut() else {
            return Err("completion-accept!: no LSP state available".to_string());
        };
        let Some(session) = lsp.completion.take() else {
            return Err("completion-accept!: no active completion session".to_string());
        };
        // Ends the session either way — success or failure — so a rejected
        // accept never leaves a stale session lingering; the ui/view clear
        // matches `clear_completion_menu`'s scope even though `completion`
        // itself is already `None` here (via `take` above).
        crate::editor::lsp::completion::clear_completion_state(lsp);
        *self
            .state
            .completion_menu_view
            .write()
            .expect("RwLock not poisoned") = None;
        session.accept(self.state, lsp, idx)
    }

    fn completion_dismiss(&mut self) {
        self.clear_completion_menu();
    }
}

impl<'a> OutputHost for EditorHostImpl<'a> {
    fn is_inline_output_command(&self) -> bool {
        !matches!(
            self.state.inline_output,
            super::InlineOutputDispatch::Inactive
        )
    }

    fn ensure_inline_output_screen(&mut self) -> Result<(), String> {
        // Only `Armed` needs action: `Entered` already left the alt-screen,
        // `Headless`/`Inactive` have no bracket to enter at all.
        let super::InlineOutputDispatch::Armed { kitty, mouse, name } = &self.state.inline_output
        else {
            return Ok(());
        };
        let (kitty, mouse, name) = (*kitty, *mouse, name.clone());
        let term = self
            .terminal
            .expect("Armed implies tui_active implies terminal is Some");
        hume_platform::terminal::enter_inline_output(term, kitty, mouse)
            .map_err(|e| format!("inline-output enter failed: {e}"))?;
        hume_platform::terminal::print_running_banner(&name);
        self.state.inline_output = super::InlineOutputDispatch::Entered;
        #[cfg(test)]
        {
            self.state.inline_output_entered = true;
        }
        Ok(())
    }
}

impl<'a> TimerHost for EditorHostImpl<'a> {
    fn schedule_timer(&mut self, ms: u64, thunk: steel::rvals::SteelVal) -> Option<u64> {
        Some(
            self.timers
                .as_mut()?
                .schedule(std::time::Duration::from_millis(ms), thunk),
        )
    }

    fn cancel_timer(&mut self, id: u64) {
        if let Some(timers) = self.timers.as_mut() {
            timers.cancel(id);
        }
    }
}

impl<'a> LspHost for EditorHostImpl<'a> {
    fn lsp_capabilities(&self, server: Option<&str>) -> Option<serde_json::Value> {
        let lsp = self.lsp.as_deref()?;
        let bid = crate::editor::commands::focused_buffer_id(self.state, self.view);
        crate::editor::lsp::introspect::capabilities(self.state, lsp, bid, server)
    }

    fn lsp_server_status(&self) -> Vec<hume_scripting::LspServerStatusEntry> {
        self.lsp
            .as_deref()
            .map(crate::editor::lsp::introspect::server_status)
            .unwrap_or_default()
    }

    fn lsp_server_for_buffer(&self, id: BufferId) -> Option<String> {
        crate::editor::lsp::introspect::server_for_buffer(self.state, self.lsp.as_deref()?, id)
    }

    fn lsp_registered_for_language(&self, language: &str) -> bool {
        self.lsp.as_deref().is_some_and(|lsp| {
            crate::editor::lsp::introspect::registered_for_language(lsp, language)
        })
    }

    fn lsp_position_params(&self, id: BufferId) -> Option<serde_json::Value> {
        crate::editor::lsp::introspect::position_params(self.state, self.lsp.as_deref()?, id)
    }

    fn lsp_range_params(&self, id: BufferId) -> Option<serde_json::Value> {
        crate::editor::lsp::introspect::range_params(self.state, self.lsp.as_deref()?, id)
    }
}

impl<'a> DecorationHost for EditorHostImpl<'a> {
    fn set_inlay_hints(&mut self, bid: BufferId, hints: Vec<(serde_json::Value, String, bool)>) {
        let Some(lsp) = self.lsp.as_deref() else {
            return;
        };
        let encoding = crate::editor::lsp::introspect::encoding_for_buffer(self.state, lsp, bid);
        let Some(rope) = self
            .state
            .buffers
            .try_get(bid)
            .map(|b| b.text().rope().clone())
        else {
            return;
        };
        // The `set-inlay-hints!` builtin already validates each position has
        // numeric `line`/`character` before this ever runs — a malformed
        // shape errors loudly at that boundary instead of being silently
        // dropped here.
        let entries: Vec<crate::editor::decorations::InlayHintEntry> = hints
            .into_iter()
            .map(|(wire_pos, text, before)| {
                let line = wire_pos["line"].as_u64().expect("validated by builtin") as usize;
                let character = wire_pos["character"]
                    .as_u64()
                    .expect("validated by builtin") as usize;
                let pos =
                    hume_editing::position_encoding::wire_to_char(&rope, line, character, encoding);
                crate::editor::decorations::InlayHintEntry { pos, text, before }
            })
            .collect();
        self.state.decorations.set_inlay_hints(bid, entries);
    }

    fn set_signs(
        &mut self,
        source: String,
        bid: BufferId,
        signs: Vec<(usize, String, String, i64)>,
    ) {
        let entries = signs
            .into_iter()
            .map(
                |(line, text, scope, priority)| crate::editor::decorations::SignEntry {
                    line,
                    text,
                    scope,
                    priority,
                },
            )
            .collect();
        self.state.decorations.set_signs(source, bid, entries);
    }

    fn set_virtual_lines(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<(usize, String, Option<String>)>,
    ) {
        let entries = lines
            .into_iter()
            .map(
                |(line, text, scope)| crate::editor::decorations::VirtualLineEntry {
                    line,
                    text,
                    scope,
                },
            )
            .collect();
        self.state
            .decorations
            .set_virtual_lines(source, bid, entries);
    }

    fn set_extra_highlights(
        &mut self,
        source: String,
        bid: BufferId,
        spans: Vec<(usize, usize, String)>,
    ) {
        let entries = spans
            .into_iter()
            .map(
                |(start, end, scope)| crate::editor::decorations::ExtraHighlightEntry {
                    start,
                    end,
                    scope,
                },
            )
            .collect();
        self.state
            .decorations
            .set_extra_highlights(source, bid, entries);
    }

    fn set_inline_diagnostics(&mut self, bid: BufferId, lines: Vec<(usize, String, String)>) {
        let entries = lines
            .into_iter()
            .map(
                |(line, text, scope)| crate::editor::decorations::InlineDiagnosticEntry {
                    line,
                    text,
                    scope,
                },
            )
            .collect();
        self.state.decorations.set_inline_diagnostics(bid, entries);
    }

    fn diagnostics_for_buffer(
        &self,
        bid: BufferId,
        severity_floor: Option<&str>,
        range: Option<(usize, usize)>,
    ) -> Result<Vec<serde_json::Value>, String> {
        let Some(lsp) = self.lsp.as_deref() else {
            return Ok(Vec::new());
        };
        crate::editor::lsp::introspect::diagnostics_for_buffer(
            self.state,
            lsp,
            bid,
            severity_floor,
            range,
        )
    }

    fn diagnostic_counts(&self, bid: BufferId) -> (usize, usize) {
        let Some(lsp) = self.lsp.as_deref() else {
            return (0, 0);
        };
        crate::editor::lsp::introspect::diagnostic_counts(lsp, bid)
    }
}

impl<'a> EditHost for EditorHostImpl<'a> {
    // ── Edit + navigation primitives ────────────────────────────────────
    fn apply_text_edits(
        &mut self,
        bid: BufferId,
        edits: Vec<(usize, usize, usize, usize, String)>,
        expect_gen: Option<u64>,
    ) -> Result<(), String> {
        let Some(lsp) = self.lsp.as_deref() else {
            return Err("apply-text-edits!: no LSP state available".to_string());
        };
        // Untrusted plugin input, not an internal invariant — a position
        // that doesn't fit `u32` is a malformed edit, reported as an error,
        // never a panic.
        let to_u32 = |v: usize| {
            u32::try_from(v)
                .map_err(|_| "apply-text-edits!: position exceeds u32 (malformed edit)".to_string())
        };
        let mut typed_edits = Vec::with_capacity(edits.len());
        for (start_line, start_char, end_line, end_char, new_text) in edits {
            typed_edits.push(lsp_types::TextEdit {
                range: lsp_types::Range {
                    start: lsp_types::Position {
                        line: to_u32(start_line)?,
                        character: to_u32(start_char)?,
                    },
                    end: lsp_types::Position {
                        line: to_u32(end_line)?,
                        character: to_u32(end_char)?,
                    },
                },
                new_text,
            });
        }
        crate::editor::lsp::edits::apply_text_edits(self.state, lsp, bid, typed_edits, expect_gen)
    }

    fn apply_workspace_edit(&mut self, edit: serde_json::Value) -> Result<usize, String> {
        let Some(lsp) = self.lsp.as_deref() else {
            return Err("apply-workspace-edit!: no LSP state available".to_string());
        };
        let we: lsp_types::WorkspaceEdit =
            serde_json::from_value(edit).map_err(|e| format!("malformed WorkspaceEdit: {e}"))?;
        let summary =
            crate::editor::lsp::edits::apply_workspace_edit(self.state, self.view, lsp, we)?;
        Ok(summary.buffers_modified)
    }

    fn goto_location_wire(
        &mut self,
        uri: String,
        line: usize,
        character: usize,
    ) -> Result<(), String> {
        let Some(lsp) = self.lsp.as_deref() else {
            return Err("goto-location!: no LSP state available".to_string());
        };
        let uri: lsp_types::Uri = uri
            .parse()
            .map_err(|_| format!("goto-location!: bad uri {uri:?}"))?;
        let target = crate::editor::lsp::edits::GotoTarget::Wire {
            uri,
            line,
            character,
        };
        crate::editor::lsp::edits::goto_location(self.state, self.view, lsp, target)
    }

    fn goto_location_path(
        &mut self,
        path_or_uri: String,
        line: usize,
        col: usize,
    ) -> Result<(), String> {
        let Some(lsp) = self.lsp.as_deref() else {
            return Err("goto-location!: no LSP state available".to_string());
        };
        let target = crate::editor::lsp::edits::GotoTarget::Path {
            path_or_uri,
            line,
            col,
        };
        crate::editor::lsp::edits::goto_location(self.state, self.view, lsp, target)
    }

    fn goto_location_buffer(
        &mut self,
        bid: BufferId,
        line: usize,
        col: usize,
    ) -> Result<(), String> {
        let Some(lsp) = self.lsp.as_deref() else {
            return Err("goto-location!: no LSP state available".to_string());
        };
        let target = crate::editor::lsp::edits::GotoTarget::Buffer { bid, line, col };
        crate::editor::lsp::edits::goto_location(self.state, self.view, lsp, target)
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
        if self.state.steel_prompt_callback.is_some() {
            return Err("prompt!: a minibuffer session is already open".to_string());
        }
        let cursor = prefill.len();
        self.state.minibuf = Some(crate::editor::MiniBuffer {
            prompt: label,
            input: prefill,
            cursor,
        });
        self.state.steel_prompt_callback = Some(callback);
        self.state.history.begin_session_all();
        self.state.set_mode(crate::editor::Mode::Command);
        Ok(())
    }

    // ── Cursor-anchored popup ────────────────────────────────────────────
    fn show_popup(
        &mut self,
        text: String,
        dismiss_on_key: bool,
        scrollable: bool,
        markdown: bool,
    ) -> Result<(), String> {
        let dismiss = if scrollable {
            crate::ui::popup::PopupDismiss::KeyExceptScroll
        } else if dismiss_on_key {
            crate::ui::popup::PopupDismiss::AnyKey
        } else {
            crate::ui::popup::PopupDismiss::ModeChange
        };
        let syntax = markdown.then(|| self.build_popup_syntax(&text)).flatten();
        self.state.popup = Some(crate::ui::popup::PopupModel {
            text,
            dismiss,
            scroll: 0,
            syntax,
        });
        Ok(())
    }

    fn close_popup(&mut self) -> Result<(), String> {
        self.state.popup = None;
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
        self.state.menu = Some(crate::ui::popup::MenuModel {
            items,
            selected: 0,
            callback,
        });
        Ok(())
    }

    fn close_menu(&mut self) -> Result<(), String> {
        self.state.menu = None;
        Ok(())
    }

    // ── Bottom drawer ──────────────────────────────────────────────────────
    fn show_drawer_list(
        &mut self,
        items: Vec<String>,
        callback: steel::rvals::SteelVal,
    ) -> Result<(), String> {
        self.state.drawer = Some(crate::ui::drawer::DrawerModel {
            items,
            selected: 0,
            scroll: 0,
            callback,
        });
        self.state.sync_drawer_view();
        Ok(())
    }

    fn close_drawer(&mut self) -> Result<(), String> {
        self.state.drawer = None;
        self.state.sync_drawer_view();
        Ok(())
    }

    // ── Fuzzy picker ──────────────────────────────────────────────────────
    fn open_picker(
        &mut self,
        items: Vec<(String, steel::rvals::SteelVal)>,
        prompt: String,
        on_select: steel::rvals::SteelVal,
    ) -> Result<u64, String> {
        let mut session = crate::editor::picker::PickerSession::new(on_select, prompt);
        let token = session.token();
        let picker_items = items
            .into_iter()
            .map(|(display, payload)| crate::editor::picker::PickerItem { display, payload })
            .collect();
        session.push(token, picker_items); // fresh token — always applies
        crate::editor::picker::open_picker(self.state, self.lsp.as_deref_mut(), session);
        Ok(token)
    }

    fn picker_push(&mut self, token: u64, items: Vec<(String, steel::rvals::SteelVal)>) -> bool {
        let Some(session) = self.state.picker.as_mut() else {
            return false;
        };
        let picker_items = items
            .into_iter()
            .map(|(display, payload)| crate::editor::picker::PickerItem { display, payload })
            .collect();
        session.push(token, picker_items)
    }

    fn picker_source_spawn(
        &mut self,
        token: u64,
        cmd: &str,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        nul: bool,
    ) -> Result<bool, String> {
        let Some(session) = self.state.picker.as_mut() else {
            return Ok(false);
        };
        if session.token() != token {
            return Ok(false);
        }
        let delimiter = if nul { b'\0' } else { b'\n' };
        let source = hume_platform::process::line_source::spawn_line_source(
            cmd,
            &args,
            cwd.as_deref(),
            delimiter,
            std::sync::Arc::clone(&self.state.wake),
        )
        .map_err(|e| format!("cannot run '{cmd}': {e}"))?;
        self.state
            .picker
            .as_mut()
            .expect("checked Some above")
            .attach_source(source);
        Ok(true)
    }

    fn picker_close(&mut self) {
        crate::editor::picker::close_picker(self.state, steel::rvals::SteelVal::BoolV(false));
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
