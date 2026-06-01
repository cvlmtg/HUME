use std::path::PathBuf;

use engine::pipeline::{BufferId, PaneId};

use scripting::SteelBufferId;
use scripting::{HookResult, SteelCmdDef, hooks::HookId};

use super::{Editor, Severity, host_impl::EditorHostImpl, ops};

impl Editor {
    // ── Message reporting ─────────────────────────────────────────────────────

    /// Report a message, routing it based on severity:
    ///
    /// - `Info`    → set `status_msg` only (ephemeral, not logged)
    /// - `Warning` → push to `message_log` AND set `status_msg`
    /// - `Error`   → push to `message_log` AND set `status_msg`
    /// - `Trace`   → push to `message_log` only (not shown in statusline)
    pub(crate) fn report(&mut self, severity: Severity, text: String) {
        match severity {
            Severity::Info => {
                self.status_msg = Some(text);
            }
            Severity::Warning | Severity::Error => {
                self.message_log.push(severity, text.clone());
                self.status_msg = Some(text);
            }
            Severity::Trace => {
                self.message_log.push(severity, text);
            }
        }
    }

    /// Drain any pending `(log! …)` messages from the scripting host and
    /// report each one.  Collected into a temporary vec first to satisfy the
    /// borrow checker (both `self.scripting` and `self` are `&mut`).
    pub(crate) fn flush_script_messages(&mut self) {
        let msgs = self
            .scripting
            .as_mut()
            .map(|h| h.take_pending_messages())
            .unwrap_or_default();
        for (level, text) in msgs {
            self.report(log_level_to_severity(level), text);
        }
    }

    // ── Hook firing ──────────────────────────────────────────────────────────

    /// Fire `OnBufferSave` hooks for `bid`. Both `:w` write paths in
    /// `commands.rs` share this rather than duplicating the arg construction.
    pub(super) fn fire_hook_buffer_save(&mut self, bid: BufferId) {
        let val = SteelBufferId::new(bid).into_steel_val();
        self.fire_hook_silent(HookId::OnBufferSave, &[val]);
    }

    /// Fire all Steel handlers for `hook_id`, passing `args` to each.
    ///
    /// No-ops immediately if no scripting host is present or if no handlers
    /// are registered for the hook.  Commands queued by `(call! …)` inside
    /// hook bodies are dispatched after all handlers return.  Errors from
    /// handlers are reported as `Severity::Error`.
    pub(super) fn fire_hook_silent(&mut self, hook_id: HookId, args: &[steel::rvals::SteelVal]) {
        // Activate any lazy event-triggered plugins before checking for handlers,
        // so their register-hook! calls run before the early-exit guard below.
        self.activate_lazy_event_plugins(hook_id);
        if self
            .scripting
            .as_ref()
            .is_none_or(|h| !h.has_hook_handlers(hook_id))
        {
            return;
        }
        let pid = self.focused_pane_id;
        let bid = self.focused_buffer_id();
        // `scripting` is a distinct field from `settings`, `keymap`, `buffers`,
        // `engine_view`, `pane_state`, `pane_jumps`, and `languages`, so Rust
        // allows simultaneous `&mut` borrows of them through NLL splitting.
        let result = {
            let host_scr = self.scripting.as_mut().expect("checked above");
            let mut impl_host = EditorHostImpl {
                settings: &mut self.settings,
                keymap: &mut self.keymap,
                focused_pane_id: pid,
                buffers: Some(&mut self.buffers),
                engine_view: Some(&mut self.engine_view),
                pane_state: Some(&mut self.pane_state),
                pane_jumps: Some(&mut self.pane_jumps),
                languages: Some(&mut self.languages),
            };
            host_scr.fire_hook(hook_id, args, pid, bid, &mut impl_host)
        };
        self.flush_script_messages();
        match result {
            Ok(HookResult { cmd_queue, pending_language_sets, grammar_sweeps }) => {
                for (bid, lang) in pending_language_sets {
                    self.set_buffer_language(bid, lang);
                }
                if !grammar_sweeps.is_empty() {
                    self.sweep_buffers_for_grammars(grammar_sweeps);
                }
                self.drain_command_queue(cmd_queue, 1, false);
            }
            Err(e) => self.report(Severity::Error, format!("hook error: {e}")),
        }
    }

    // ── Scripting ─────────────────────────────────────────────────────────────

    /// Initialise the Steel scripting host and evaluate `init.scm`.
    ///
    /// Must be called once, after `Editor::open` returns and before
    /// `Editor::run` starts. Any error from `init.scm` is reported as
    /// `Severity::Error` and shown in the statusline.
    pub(crate) fn init_scripting(&mut self) {
        // Resolve the config path up front. `None` means neither XDG_CONFIG_HOME
        // nor HOME (APPDATA on Windows) is set — there is no meaningful place
        // to look for init.scm, so we skip scripting entirely and log a warning.
        let Some(config_dir) = platform::dirs::config_dir() else {
            self.report(
                Severity::Warning,
                "scripting: no config directory — HOME/APPDATA unset; init.scm skipped".into(),
            );
            return;
        };
        let init_path = config_dir.join("init.scm");
        let mut host = scripting::ScriptingHost::new();
        // Trace the resolved directories so they're visible in `:messages`.
        // A missing runtime dir is a warning because `core:*` plugins need it.
        match host.runtime_dir() {
            Some(rt) => self.report(Severity::Trace, format!("scripting: runtime dir = {}", rt.display())),
            None => self.report(Severity::Warning, "scripting: no runtime directory found — core:* plugins unavailable; set HUME_RUNTIME to fix".into()),
        }
        match host.data_dir() {
            Some(d) => self.report(
                Severity::Trace,
                format!("scripting: data dir = {}", d.display()),
            ),
            None => self.report(
                Severity::Warning,
                "scripting: no data directory — HOME/APPDATA unset; user plugins unavailable"
                    .into(),
            ),
        }
        // Capture built-in names before any plugin code runs; stable for the
        // editor's lifetime.  Stored on Editor so dispatch-time activation can
        // borrow it disjointly from &mut self.scripting / settings / keymap.
        let builtin_names: std::collections::HashSet<String> =
            self.registry.names().map(String::from).collect();
        self.builtin_cmd_names = builtin_names.clone();
        // Reset the language registry so `:reload-config` gets a fresh set
        // of registrations from languages.scm rather than accumulating duplicates.
        self.languages = crate::editor::syntax::LanguageRegistry::new();
        // Load runtime/scheme/prelude.scm before init.scm so its macros
        // (bind-keys! etc.) are available to init.scm and plugin modules.
        // Missing prelude is a silent no-op (optional sugar); a prelude that
        // exists but fails to parse/eval is an error reported separately.
        //
        // Each eval gets a fresh init-mode EditorHostImpl (buffer/pane refs
        // are None; init builtins are guard-protected and never reach them).
        if let Some(prelude_path) = host.runtime_dir().map(|rt| rt.join("scheme/prelude.scm")) {
            let init_budget = self.settings.steel_init_budget_ms as u64;
            let mut ih = make_init_host(&mut self.settings, &mut self.keymap);
            match host.eval_init(&prelude_path, init_budget, &mut ih, builtin_names.clone()) {
                Ok(cmds) => debug_assert!(
                    cmds.is_empty(),
                    "runtime/scheme/prelude.scm must not define commands"
                ),
                Err(msg) => self.report(
                    Severity::Error,
                    format!("runtime/scheme/prelude.scm: {msg}"),
                ),
            }
        }
        // Load languages.scm between prelude and init.scm so (define-language! …)
        // calls are available when init.scm and plugins run.
        let langs_path = host.runtime_dir().map(|rt| rt.join("scheme/languages.scm"));
        if let Some(langs_path) = langs_path {
            let init_budget = self.settings.steel_init_budget_ms as u64;
            let mut ih = make_init_host(&mut self.settings, &mut self.keymap);
            match host.eval_init(&langs_path, init_budget, &mut ih, builtin_names.clone()) {
                Ok(cmds) => debug_assert!(
                    cmds.is_empty(),
                    "runtime/scheme/languages.scm must not define commands"
                ),
                Err(msg) => self.report(
                    Severity::Error,
                    format!("runtime/scheme/languages.scm: {msg}"),
                ),
            }
            self.flush_pending_language_regs(&mut host);
        }
        {
            let init_budget = self.settings.steel_init_budget_ms as u64;
            let mut ih = make_init_host(&mut self.settings, &mut self.keymap);
            match host.eval_init(&init_path, init_budget, &mut ih, builtin_names) {
                Ok(cmds) => self.register_steel_cmds(cmds),
                Err(msg) => self.report(Severity::Error, format!("init.scm: {msg}")),
            }
        }
        // Register lazy-command stubs for every #:on-command trigger declared
        // during init.scm.  Must run after register_steel_cmds (eager plugins
        // may have defined commands that would collide) and before scripting=Some
        // so the borrow of &host is independent.
        let triggers = host.command_triggers();
        self.register_lazy_command_stubs(&triggers);
        // Second flush: picks up any (define-language! …) calls from init.scm /
        // plugins that ran during init.scm.
        self.flush_pending_language_regs(&mut host);
        // Pick up any (set-option! "history-capacity" N) calls from init.scm.
        self.history.set_capacity(self.settings.history_capacity);
        // Flush any `(log! …)` messages produced during init.scm evaluation.
        for (level, text) in host.take_pending_messages() {
            self.report(log_level_to_severity(level), text);
        }
        self.scripting = Some(host);
        // Post-init lint: warn on keymap leaves that target an unknown command.
        // Runs after register_lazy_command_stubs so Lazy stubs count as valid.
        // Built-in keymaps only reference registered built-ins, so any warnings
        // here come from user bind-key! calls to typos / undeclared commands.
        {
            let mut names = self.keymap.all_command_names();
            names.sort_unstable();
            names.dedup();
            for name in &names {
                if !self.registry.contains(name) {
                    self.report(
                        Severity::Warning,
                        format!("key bound to unknown command '{name}' — typo, or missing from #:on-command?"),
                    );
                }
            }
        }
        // Load theme set by (set-option! 'theme "…") in init.scm.
        if !self.settings.theme.is_empty() {
            ops::load_theme_by_name(
                &mut self.engine_view,
                &mut self.message_log,
                &mut self.status_msg,
                &self.settings.theme,
            );
        }
        // Re-detect language for buffers opened before init_scripting ran
        // (the initial buffer is opened in lib.rs before init_scripting is called).
        let open_bids: Vec<_> = self.buffers.iter().map(|(id, _)| id).collect();
        for bid in open_bids {
            self.detect_and_set_language(bid);
        }
    }

    // ── Startup command drain ─────────────────────────────────────────────────

    /// Drain commands queued by `(call! …)` in init.scm or plugin load bodies.
    ///
    /// Called from `lib.rs` after `init_scripting` and `open_extra_files`.
    /// No-op when nothing is pending (common case).  Commands with
    /// `inline_output` get the alt-screen bracket automatically via the normal
    /// `execute_keymap_command` / `SteelBacked` dispatch path.
    pub(crate) fn run_startup_commands(&mut self) {
        let cmds = self
            .scripting
            .as_mut()
            .map(|s| s.take_startup_commands())
            .unwrap_or_default();
        if cmds.is_empty() {
            return;
        }
        self.drain_command_queue(cmds, 1, false);
    }

    // ── Theme loading ─────────────────────────────────────────────────────────

    /// Test-only wrapper: splits the three disjoint `Editor` fields so tests can
    /// load a theme via `&mut self` without manual field extraction.
    #[cfg(test)]
    pub(crate) fn load_theme_by_name(&mut self, name: &str) -> bool {
        ops::load_theme_by_name(
            &mut self.engine_view,
            &mut self.message_log,
            &mut self.status_msg,
            name,
        )
    }

    // ── Scripting helpers ─────────────────────────────────────────────────────

    /// Register each `SteelCmdDef` in the command registry, reporting
    /// conflicts as errors.  Used after both init and plugin-reload evals.
    ///
    /// A `Lazy` stub for the same name is silently overwritten — this is the
    /// expected path when a lazy plugin's body is evaluated and its
    /// `define-command!` replaces the stub it was triggered by.
    pub(super) fn register_steel_cmds(&mut self, defs: impl IntoIterator<Item = SteelCmdDef>) {
        use super::registry::MappableCommand;
        for def in defs {
            match self.registry.get_mappable(&def.name) {
                Some(MappableCommand::Lazy { .. }) | None => {
                    self.registry.register(MappableCommand::SteelBacked {
                        name: def.name.into(),
                        doc: def.doc.into(),
                        steel_proc: def.steel_proc,
                        extendable: def.extendable,
                        arity: def.arity,
                        is_variadic: def.is_variadic,
                        inline_output: def.inline_output,
                    });
                }
                Some(_) => {
                    self.report(
                        Severity::Error,
                        format!(
                            "define-command!: '{}' conflicts with existing command",
                            def.name
                        ),
                    );
                }
            }
        }
    }

    /// Register a `Lazy` stub for each command trigger from the init manifest.
    ///
    /// Called after `register_steel_cmds` (eager plugins run first) so a
    /// command defined eagerly is detected as a conflict before a lazy stub
    /// for the same name would shadow it.
    pub(super) fn register_lazy_command_stubs(
        &mut self,
        triggers: &std::collections::HashMap<String, scripting::attribution::PluginId>,
    ) {
        for (name, plugin) in triggers {
            if self.registry.contains(name) {
                self.report(
                    Severity::Error,
                    format!("lazy command '{name}' conflicts with an existing command"),
                );
            } else {
                self.registry.register(super::registry::MappableCommand::Lazy {
                    name: name.clone().into(),
                    plugin: plugin.clone(),
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Module-level helpers
// ---------------------------------------------------------------------------

/// Ordered list of directories to search for theme TOML files.
///
/// Config themes (user-defined) are listed before runtime themes (bundled) so
/// that user overrides shadow built-in ones. Both `ops::load_theme_by_name`
/// and [`ThemeCompleter`] use this list as the single source of truth.
/// Build an init-mode `EditorHostImpl` with no buffer/pane refs.
///
/// Init builtins (`set-option!`, `bind-key!`, etc.) are guard-protected —
/// they never access buffer/pane data.
pub(crate) fn make_init_host<'a>(
    settings: &'a mut crate::settings::EditorSettings,
    keymap: &'a mut crate::editor::keymap::Keymap,
) -> EditorHostImpl<'a> {
    EditorHostImpl {
        settings,
        keymap,
        focused_pane_id: PaneId::default(),
        buffers: None,
        engine_view: None,
        pane_state: None,
        pane_jumps: None,
        languages: None,
    }
}

/// Map scripting `LogLevel` → editor `Severity`.
pub(crate) fn log_level_to_severity(level: scripting::LogLevel) -> Severity {
    use scripting::LogLevel;
    match level {
        LogLevel::Info => Severity::Info,
        LogLevel::Warning => Severity::Warning,
        LogLevel::Error => Severity::Error,
        LogLevel::Trace => Severity::Trace,
    }
}

pub(super) fn theme_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(cfg) = platform::dirs::config_dir() {
        paths.push(cfg.join("themes"));
    }
    if let Some(rt) = platform::dirs::runtime_dir() {
        paths.push(rt.join("themes"));
    }
    paths
}
