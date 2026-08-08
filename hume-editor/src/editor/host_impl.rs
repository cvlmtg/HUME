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

use std::ops::Range;
use std::path::{Path, PathBuf};

use hume_editing::text::strip_line_break;
use hume_engine::pipeline::{BufferId, EngineView, PaneId};

use crate::editor::diff_bridge;
use crate::editor::lsp::LspState;
use crate::editor::registry::MappableCommand;
use crate::editor::timer_bridge::TimerHandle;
use crate::lock_ext::LockExt;
use crate::ui::statusline::StatusLineConfig;
use hume_scripting::host::{
    AsyncProcessHost, BufferHost, CommandHost, CompletionHost, CursorHost, DecorationHost,
    DiffHost, DiffHunk, EditHost, EditorHost, EventHost, LanguageHost, LspHost, OptionValue,
    OutputHost, PopupKind, SettingsHost, TimerHost, UiHost, WordDiffHunk,
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
    /// `Some` at the three call sites that thread every capability
    /// (command dispatch, hook fire, queued-call drain) — `None` everywhere
    /// else. Only [`OutputHost::ensure_inline_output_screen`] (reachable
    /// from command dispatch) actually reads it; `state.inline_output` only
    /// ever reaches `Armed` from that same call site, so its early return
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

    /// Constructor for the three call sites that thread every capability:
    /// command dispatch, hook fire, and queued-call drain. Takes the fields
    /// already split out (rather than `&mut Editor`) because each call site
    /// holds a simultaneous disjoint borrow of `self.scripting` — passing
    /// `self` as a whole would conflict with that borrow.
    pub(in crate::editor) fn full(
        state: &'a mut EditorState,
        view: &'a mut EngineView,
        lsp: &'a mut LspState,
        timer_wheel: &'a mut super::timers::TimerWheel,
        timer_payloads: &'a mut rustc_hash::FxHashMap<
            super::timers::TimerId,
            super::timer_bridge::TimerPayload,
        >,
        terminal: Option<&'a hume_platform::terminal::SharedTerm>,
    ) -> Self {
        Self {
            state,
            view,
            lsp: Some(lsp),
            timers: Some(TimerHandle {
                wheel: timer_wheel,
                payloads: timer_payloads,
            }),
            terminal,
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

    /// Synchronously parses `text` through the grammar named `lang`, if one
    /// is registered — `None` otherwise (no such grammar), which leaves the
    /// popup rendering plain. `show_popup`'s only caller, shared across its
    /// cursor and docked layouts.
    fn build_markup_syntax(
        &self,
        lang: &str,
        text: &str,
    ) -> Option<crate::ui::popup::MarkupSyntax> {
        let lang_id = self.state.config.languages.id_of(lang)?;
        let bundle = std::sync::Arc::clone(self.state.config.languages.grammar(lang_id)?);
        let text = hume_editing::text::Text::from(text);
        let syntax = hume_treesitter::syntax::Syntax::attach_sync(
            bundle,
            &text,
            &self.state.config.languages.grammar_snapshot(),
        );
        Some(crate::ui::popup::MarkupSyntax { syntax, text })
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
    // The job registry lives on `self.state.config` — always reachable, no
    // `Option`-wrapped upstream field to gate on (unlike `timers`/`lsp`).
    fn async_process(&mut self) -> Option<&mut dyn AsyncProcessHost> {
        Some(self)
    }
    fn diff(&mut self) -> Option<&mut dyn DiffHost> {
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
    fn events(&mut self) -> &mut dyn EventHost {
        self
    }
}

impl<'a> EventHost for EditorHostImpl<'a> {
    fn known_event_names(&self) -> &'static [&'static str] {
        super::event::known_event_names()
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
    fn buffer_display_path(&self, id: BufferId) -> Option<String> {
        self.buffer(id)?.display_path().map(str::to_owned)
    }
    fn buffer_display_name(&self, id: BufferId) -> Option<String> {
        self.buffer(id).map(|buf| buf.display_name())
    }
    fn buffer_is_dirty(&self, id: BufferId) -> Option<bool> {
        self.buffer(id).map(|buf| buf.is_dirty())
    }
    fn buffer_stored_language(&self, id: BufferId) -> Option<String> {
        let lang_id = self.buffer(id)?.language?;
        Some(self.state.config.languages.name_of(lang_id).to_owned())
    }

    // ── Buffer lifecycle ─────────────────────────────────────────────────────
    fn open_buffer(&mut self, path: &Path) -> Result<BufferId, String> {
        // `resolve_buffer_path`, not a hard `canonicalize`: a missing path is
        // openable here exactly like `:e` on one — see `Buffer::from_file_or_new`.
        let resolved = crate::editor::Editor::resolve_buffer_path(path, &self.state.cwd);
        // Language detection is deliberately not done here — see
        // `Effect::DetectBufferLanguage`'s doc; the `open-buffer!` builtin
        // queues it once this returns.
        let (bid, _is_new) = crate::editor::buffer::lifecycle::open_or_dedup_and_notify(
            self.view, self.state, &resolved,
        )
        .map_err(|e| format!("open-buffer!: {}: {e}", resolved.display()))?;
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

    fn buffer_text(&self, id: BufferId) -> Option<String> {
        Some(self.buffer(id)?.text().to_string())
    }

    fn buffer_line_count(&self, id: BufferId) -> Option<usize> {
        Some(self.buffer(id)?.text().content_line_count())
    }

    fn buffer_lines(&self, id: BufferId, range: Range<usize>) -> Option<Vec<String>> {
        let text = self.buffer(id)?.text();
        Some(
            text.line_tokens_at(range.start)
                .take(range.len())
                .map(|line| {
                    // Both branches allocate exactly once — `into_owned`
                    // copies a `Borrowed` line, `Owned` is already a copy —
                    // so truncating in place covers both without a match.
                    let mut s = line.into_owned();
                    s.truncate(strip_line_break(&s).len());
                    s
                })
                .collect(),
        )
    }

    fn viewport_range(&self, id: BufferId) -> Option<Range<usize>> {
        crate::editor::lsp::introspect::viewport_range(self.state, self.view, id)
    }
}

impl<'a> SettingsHost for EditorHostImpl<'a> {
    fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String> {
        crate::editor::settings_ops::apply_global(self.state, self.view, key, value)
    }

    fn set_buffer_option(&mut self, key: &str, value: &str, bid: BufferId) -> Result<(), String> {
        // `settings_ops::apply_buffer`'s `get_mut` panics on a stale id —
        // validate first so a bad `bid` from Steel becomes an `Err`, not a
        // panic.
        if self.state.buffers.try_get(bid).is_none() {
            return Err(format!("set-buffer-option!: invalid buffer id {bid:?}"));
        }
        crate::editor::settings_ops::apply_buffer(self.state, bid, key, value)
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
        // through `write_global` like every other setting — see
        // `settings_ops::apply_global`'s doc for why a raw field write must
        // not bypass it.
        let cfg = StatusLineConfig {
            left: crate::ui::statusline::parse_statusline_section(left, "left")?,
            center: crate::ui::statusline::parse_statusline_section(center, "center")?,
            right: crate::ui::statusline::parse_statusline_section(right, "right")?,
        };
        let wire = crate::settings::format_statusline(&cfg);

        crate::editor::settings_ops::apply_global(self.state, self.view, "statusline", &wire)
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
            .config
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
        self.state.config.languages.has_grammar(language)
    }

    fn register_trigger_chars(&mut self, source: String, language: String, chars: Vec<char>) {
        if chars.is_empty() {
            self.state.config.trigger_chars.remove(&(source, language));
        } else {
            self.state
                .config
                .trigger_chars
                .insert((source, language), chars);
        }
    }
}

impl<'a> CommandHost for EditorHostImpl<'a> {
    fn register_command(&mut self, def: hume_scripting::SteelCmdDef) -> Result<(), String> {
        match self.state.config.registry.get_mappable(&def.name) {
            Some(MappableCommand::Lazy { .. }) | None => {
                self.state
                    .config
                    .registry
                    .register(MappableCommand::SteelBacked {
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
        self.state.config.registry.unregister(name);
    }

    fn register_lazy_command(
        &mut self,
        name: &str,
        plugin: &hume_scripting::attribution::PluginId,
    ) -> Result<(), String> {
        if let Some(MappableCommand::Lazy { plugin: owner, .. }) =
            self.state.config.registry.get_mappable(name)
        {
            return if owner == plugin {
                // Duplicate declare-plugin call for the same plugin — no-op;
                // first declaration wins.
                Ok(())
            } else {
                Err(format!("'{name}' already claimed by lazy plugin '{owner}'"))
            };
        }
        if self.state.config.registry.contains(name) {
            return Err(format!("'{name}' conflicts with an existing command"));
        }
        self.state.config.registry.register(MappableCommand::Lazy {
            name: name.to_owned().into(),
            plugin: plugin.clone(),
        });
        Ok(())
    }

    fn lazy_command_owner(&self, name: &str) -> Option<hume_scripting::attribution::PluginId> {
        match self.state.config.registry.get_mappable(name) {
            Some(MappableCommand::Lazy { plugin, .. }) => Some(plugin.clone()),
            _ => None,
        }
    }

    fn unregister_lazy_stubs_of(&mut self, plugin: &hume_scripting::attribution::PluginId) {
        self.state.config.registry.unregister_lazy_stubs_of(plugin);
    }

    fn is_valid_register_name(&self, ch: char) -> bool {
        hume_ops::register::is_valid_register_name(ch)
    }

    fn command_is_native(&self, name: &str) -> Result<bool, String> {
        self.state
            .config
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
        let Some(cmd) = self.state.config.registry.get_mappable(name).cloned() else {
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
        // list, dot-repeat) lives there so the sync path is identical to the
        // keypress path.
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
        let Some((start, end)) = hume_ops::text_object::inner_word_impl(
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
            // here would abort the whole `run_call_batch` this `Call` was
            // batched into and drop every other queued LSP callback/timer
            // batched alongside it.
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
        *self.state.completion_menu_view.write_or_panic() = None;
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

impl<'a> AsyncProcessHost for EditorHostImpl<'a> {
    fn spawn_async(
        &mut self,
        cmd: &str,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        callback: steel::rvals::SteelVal,
    ) -> u64 {
        let id = self.state.config.next_async_job_id;
        self.state.config.next_async_job_id += 1;

        match hume_platform::process::job::spawn_job(
            cmd,
            &args,
            cwd.as_deref(),
            std::sync::Arc::clone(&self.state.wake),
        ) {
            Ok(job) => {
                self.state
                    .config
                    .async_jobs
                    .insert(id, crate::editor::async_job::PendingJob { job, callback });
            }
            // Spawn failed before a job/callback contract could exist — fire
            // the callback right here rather than leaving it unfired, with
            // the same "no output, -1 exit code" shape a signal-killed
            // child produces (the sentinel `%run-inline-output!` already
            // uses — a real exit code can never be -1, it's u8-wide).
            Err(e) => {
                self.state.queue_steel_call(
                    callback,
                    vec![
                        steel::rvals::SteelVal::StringV("".into()),
                        steel::rvals::SteelVal::StringV(format!("cannot run '{cmd}': {e}").into()),
                        steel::rvals::SteelVal::IntV(-1),
                    ],
                );
                // The `Ok` arm needs no wake: the job thread wakes the loop
                // itself on completion. This callback has no background
                // thread behind it — `settle()`'s fixpoint (unlike the old
                // single-pass `drain_pending_steel_calls`) does pick it up
                // within the same `settle()` call even when `spawn-async!`
                // was itself invoked from a queued Steel callback, but this
                // wake is kept anyway: cheap, harmless if the loop is already
                // awake, and the one thing standing between "unfired" and
                // "fired eventually" if that invariant ever changes.
                (self.state.wake)();
            }
        }
        id
    }

    fn cancel_async(&mut self, id: u64) {
        // Dropping the entry drops its `SpawnedJob` (kills + reaps the
        // child) and its callback `SteelVal` without ever calling it — a
        // no-op if `id` already completed, was already cancelled, or never
        // existed (a spawn failure that already fired its callback above).
        self.state.config.async_jobs.remove(&id);
    }
}

impl<'a> DiffHost for EditorHostImpl<'a> {
    fn diff_lines(&self, old: &str, new: &str) -> Vec<DiffHunk> {
        diff_bridge::line_hunks(old, new)
    }

    fn diff_buffer_lines(&self, bid: BufferId, ref_text: &str) -> Option<Vec<DiffHunk>> {
        let buffer_text = self.buffer(bid)?.text();
        Some(diff_bridge::line_hunks_against_buffer(
            ref_text,
            buffer_text,
        ))
    }

    fn diff_words(&self, old: &str, new: &str) -> (Vec<WordDiffHunk>, bool) {
        diff_bridge::word_hunks(old, new)
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

    fn lsp_wire_to_char(&self, id: BufferId, line: usize, character: usize) -> Option<usize> {
        crate::editor::lsp::introspect::wire_to_char_for_buffer(
            self.state,
            self.lsp.as_deref()?,
            id,
            line,
            character,
        )
    }

    fn lsp_wire_point_to_char(&self, id: BufferId, line: usize, character: usize) -> Option<usize> {
        crate::editor::lsp::introspect::wire_point_to_char_for_buffer(
            self.state,
            self.lsp.as_deref()?,
            id,
            line,
            character,
        )
    }
}

impl<'a> DecorationHost for EditorHostImpl<'a> {
    fn set_inlay_hints(
        &mut self,
        source: String,
        bid: BufferId,
        hints: Vec<(usize, String, bool)>,
    ) -> Result<(), String> {
        let text = buffer_text(self.state, bid, "set-inlay-hints!")?;
        let entries = hints
            .into_iter()
            .map(|(pos, hint_text, before)| {
                validate_offset(text, pos, before, "set-inlay-hints!")?;
                Ok(crate::editor::decorations::InlayHintEntry {
                    pos,
                    text: hint_text,
                    before,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.state
            .config
            .decorations
            .set_inlay_hints(source, bid, entries);
        Ok(())
    }

    fn set_signs(
        &mut self,
        source: String,
        bid: BufferId,
        signs: Vec<(usize, String, String, i64)>,
    ) -> Result<(), String> {
        let text = buffer_text(self.state, bid, "set-signs!")?;
        let entries = signs
            .into_iter()
            .map(|(line, sign_text, scope, priority)| {
                Ok(crate::editor::decorations::SignEntry {
                    pos: line_start_offset(text, line, "set-signs!")?,
                    text: sign_text,
                    scope,
                    priority,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.state
            .config
            .decorations
            .set_signs(source, bid, entries);
        Ok(())
    }

    fn set_virtual_lines(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<hume_scripting::VirtualLineSpec>,
    ) -> Result<(), String> {
        let text = buffer_text(self.state, bid, "set-virtual-lines!")?;
        let entries = lines
            .into_iter()
            .map(|spec| {
                let pos = line_start_offset(text, spec.line, "set-virtual-lines!")?;
                let segments = virtual_line_segments_to_bytes(&spec.text, spec.segments)?;
                Ok(crate::editor::decorations::VirtualLineEntry {
                    pos,
                    text: spec.text,
                    before: spec.before,
                    scope: spec.scope,
                    segments,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.state
            .config
            .decorations
            .set_virtual_lines(source, bid, entries);
        Ok(())
    }

    fn set_extra_highlights(
        &mut self,
        source: String,
        bid: BufferId,
        spans: Vec<(usize, usize, String)>,
    ) -> Result<(), String> {
        let text = buffer_text(self.state, bid, "set-extra-highlights!")?;
        let entries = spans
            .into_iter()
            .map(|(start, end, scope)| {
                validate_range(text, start, end, "set-extra-highlights!")?;
                Ok(crate::editor::decorations::ExtraHighlightEntry { start, end, scope })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.state
            .config
            .decorations
            .set_extra_highlights(source, bid, entries);
        Ok(())
    }

    fn set_eol_text(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<(usize, String, String)>,
    ) -> Result<(), String> {
        let text = buffer_text(self.state, bid, "set-eol-text!")?;
        let entries = lines
            .into_iter()
            .map(|(line, eol_text, scope)| {
                Ok(crate::editor::decorations::EolTextEntry {
                    pos: line_start_offset(text, line, "set-eol-text!")?,
                    text: eol_text,
                    scope,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.state
            .config
            .decorations
            .set_eol_text(source, bid, entries);
        Ok(())
    }

    fn set_line_backgrounds(
        &mut self,
        source: String,
        bid: BufferId,
        entries: Vec<(usize, String)>,
    ) -> Result<(), String> {
        let text = buffer_text(self.state, bid, "set-line-backgrounds!")?;
        let entries = entries
            .into_iter()
            .map(|(line, scope)| {
                Ok(crate::editor::decorations::LineBgEntry {
                    pos: line_start_offset(text, line, "set-line-backgrounds!")?,
                    scope,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.state
            .config
            .decorations
            .set_line_backgrounds(source, bid, entries);
        Ok(())
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

/// Converts `segments`' char offsets into `text` to byte offsets, sorting by
/// `start` and validating in the process — the sole enforcement point for
/// `set-virtual-lines!`'s segment contract (bounds, ordering, non-overlap,
/// grapheme-cluster alignment), now that the Steel boundary
/// (`virtual_line_specs` in `hume-scripting`'s `builtins/decorations.rs`)
/// only decodes shape. See `VirtualLineSpec::segments`'s doc.
///
/// Grapheme boundaries, not merely char boundaries: the engine
/// (`hume-engine/src/rows.rs`'s `segment_virtual_row`) resolves each virtual
/// grapheme's scope once per cluster, at the cluster's start byte. A segment
/// edge that splits a multi-codepoint cluster (e.g. `e` + combining acute)
/// would still pass a char-boundary check, but the engine's per-cluster
/// lookup would either paint the whole cluster with a segment that only
/// claimed part of it, or miss a segment that only claimed part of it — both
/// silent.
fn virtual_line_segments_to_bytes(
    text: &str,
    mut segments: Vec<(usize, usize, String)>,
) -> Result<Vec<(usize, usize, String)>, String> {
    use unicode_segmentation::UnicodeSegmentation;

    segments.sort_by_key(|(start, _, _)| *start);

    // Char index -> byte offset, plus a sentinel one past the last char so
    // `end == char_count` resolves to `text.len()`.
    let char_to_byte: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect();
    let char_count = char_to_byte.len() - 1;

    // Every grapheme-cluster start byte offset, plus end-of-text — sorted,
    // since `grapheme_indices` yields ascending byte offsets. Built once per
    // entry rather than re-walking `text` on every boundary check below.
    let grapheme_boundaries: Vec<usize> = text
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect();
    let is_grapheme_boundary =
        |byte_offset: usize| grapheme_boundaries.binary_search(&byte_offset).is_ok();

    let mut prev_end = 0usize;
    let mut out = Vec::with_capacity(segments.len());
    for (start, end, scope) in segments {
        if start >= end {
            return Err(format!(
                "set-virtual-lines! segments: segment ({start}, {end}) must have start < end"
            ));
        }
        if end > char_count {
            return Err(format!(
                "set-virtual-lines! segments: segment end {end} is past text's char length {char_count}"
            ));
        }
        let start_byte = char_to_byte[start];
        let end_byte = char_to_byte[end];
        if !is_grapheme_boundary(start_byte) || !is_grapheme_boundary(end_byte) {
            return Err(format!(
                "set-virtual-lines! segments: segment ({start}, {end}) is not aligned to a \
                 grapheme-cluster boundary in text"
            ));
        }
        if start < prev_end {
            return Err(format!(
                "set-virtual-lines! segments: segments must not overlap (segment starting at \
                 {start} overlaps the previous one ending at {prev_end})"
            ));
        }
        prev_end = end;
        out.push((start_byte, end_byte, scope));
    }

    Ok(out)
}

/// The live text for `bid`, or `Err` naming `builtin` if `bid` doesn't name
/// an open buffer. Every decoration setter needs this to validate/convert
/// its Steel-facing positions, so a bogus `bid` fails loudly here rather
/// than silently storing data no pane will ever render.
fn buffer_text<'s>(
    state: &'s EditorState,
    bid: BufferId,
    builtin: &str,
) -> Result<&'s hume_editing::text::Text, String> {
    state
        .buffers
        .try_get(bid)
        .map(|b| b.text())
        .ok_or_else(|| format!("{builtin}: unknown buffer"))
}

/// `line`'s line-start char offset, or `Err` naming `builtin` if `line` is
/// out of range. Signs/virtual-lines/EOL-text keep their Steel-facing
/// `line` unit (SPEC.md §6's semantic-units-at-the-surface decision);
/// this is the one place — already holding the rope — where that converts
/// to the internal char-offset position model.
///
/// Rejects the buffer's last *ropey* line, not just any out-of-range line:
/// the buffer invariant (every buffer ends with a structural `\n`) means
/// that last line is always the empty phantom line the trailing `\n`
/// produces — zero-width, at `pos == len_chars()`, nothing to decorate.
/// `RowMap::last_line()` never lays it out, so admitting it would hand a
/// caller a position no render pass can resolve to a real line.
fn line_start_offset(
    text: &hume_editing::text::Text,
    line: usize,
    builtin: &str,
) -> Result<usize, String> {
    if !text.content_lines_range().contains(&line) {
        return Err(format!(
            "{builtin}: line {line} is out of range (buffer has {} content lines)",
            text.content_line_count()
        ));
    }
    Ok(text.line_to_char(line))
}

/// `pos` must address a real char in `text` (`<` its length) — `Err` naming
/// `builtin` otherwise. One past the last char looks tempting for an
/// `'after` hint at end-of-buffer, but there's no char there to anchor to:
/// `visible_char_range` is half-open, so `pos == len_chars()` can never pass
/// its `contains` check and the hint would silently never render — reject it
/// here instead, same as every other position-taking decoration kind.
///
/// `before == false` ('after') gets a second check: the render bridge
/// (`decoration_providers.rs`'s `update_inlay_hint_providers`) anchors an
/// 'after' hint at `pos + 1`, so a hint on the buffer's last content char
/// (its trailing structural `\n`) would resolve to the trailing phantom
/// line — same unresolvable position `line_start_offset` already refuses
/// for the line-anchored kinds, but reachable here through a char offset
/// instead of a line number, so that check alone doesn't catch it.
fn validate_offset(
    text: &hume_editing::text::Text,
    pos: usize,
    before: bool,
    builtin: &str,
) -> Result<(), String> {
    if pos >= text.len_chars() {
        return Err(format!(
            "{builtin}: offset {pos} is out of range (buffer has {} chars)",
            text.len_chars()
        ));
    }
    if !before {
        let landing_line = text.char_to_line(pos + 1);
        if landing_line >= text.content_line_count() {
            return Err(format!(
                "{builtin}: offset {pos} anchored 'after would land on the buffer's trailing \
                 empty line"
            ));
        }
    }
    Ok(())
}

/// `(start, end)` must be a valid, non-empty char range into `text` — `Err`
/// naming `builtin` otherwise.
fn validate_range(
    text: &hume_editing::text::Text,
    start: usize,
    end: usize,
    builtin: &str,
) -> Result<(), String> {
    if start >= end {
        return Err(format!(
            "{builtin}: range ({start}, {end}) must have start < end"
        ));
    }
    if end > text.len_chars() {
        return Err(format!(
            "{builtin}: range end {end} is past the buffer's char length {}",
            text.len_chars()
        ));
    }
    Ok(())
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
            crate::ui::popup::PopupLayout::Docked
        } else {
            crate::ui::popup::PopupLayout::Cursor
        };
        let syntax = lang.and_then(|lang| self.build_markup_syntax(&lang, &text));
        self.state.config.popup = Some(crate::ui::popup::PopupModel {
            text,
            kind,
            scroll: 0,
            syntax,
            layout,
            resolved: None,
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
        self.state.config.menu = Some(crate::ui::popup::MenuModel {
            items,
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
        self.state.config.drawer = Some(crate::ui::drawer::DrawerModel {
            items,
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
        prompt: String,
        on_select: steel::rvals::SteelVal,
        pending: bool,
    ) -> Result<u64, String> {
        let mut session = crate::editor::picker::PickerSession::new(on_select, prompt, pending);
        let token = session.token();
        let picker_items = items
            .into_iter()
            .map(|(display, payload)| crate::editor::picker::PickerItem { display, payload })
            .collect();
        session.seed(picker_items);
        crate::editor::picker::open_picker(self.state, self.lsp.as_deref_mut(), session);
        Ok(token)
    }

    fn picker_push(&mut self, token: u64, items: Vec<(String, steel::rvals::SteelVal)>) -> bool {
        let Some(session) = self.state.config.picker.as_mut() else {
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
        let Some(session) = self.state.config.picker.as_mut() else {
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
            .config
            .picker
            .as_mut()
            .expect("checked Some above")
            .attach_source(source);
        Ok(true)
    }

    fn picker_close(&mut self, token: Option<u64>) {
        if let Some(token) = token {
            let Some(session) = self.state.config.picker.as_ref() else {
                return;
            };
            if session.token() != token {
                return;
            }
        }
        crate::editor::picker::close_picker(self.state, steel::rvals::SteelVal::BoolV(false));
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
