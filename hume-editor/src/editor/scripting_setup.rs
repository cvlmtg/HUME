use std::path::PathBuf;

use hume_engine::pipeline::BufferId;

use hume_scripting::SteelBufferId;
use hume_scripting::{HookResult, hooks::HookId};

use super::{Editor, Severity, host_impl::EditorHostImpl, ops};

/// Upper bound on `drain_hooks` re-drain passes per drain boundary.
///
/// Same philosophy as the plugin-activation depth cap: unreachable in any
/// legitimate configuration, but stops a handler feedback loop from hanging
/// the editor forever.
const MAX_HOOK_DRAIN_PASSES: usize = 100;

impl Editor {
    // ── Message reporting ─────────────────────────────────────────────────────

    /// Report a message, routing it based on severity:
    ///
    /// - `Info`    → set `status_msg` only (ephemeral, not logged)
    /// - `Warning` → push to `message_log` AND set `status_msg`
    /// - `Error`   → push to `message_log` AND set `status_msg`
    /// - `Trace`   → push to `message_log` only (not shown in statusline)
    pub(crate) fn report(&mut self, severity: Severity, text: String) {
        self.state.report(severity, text);
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
    /// Enqueue `hook_id` to fire after the current command returns.
    ///
    /// The unified hook-firing path: all hook scheduling goes through
    /// `state.pending_hooks`; `Editor::drain_hooks` does the actual Steel eval.
    /// This prevents re-entrant Steel calls during command execution and gives
    /// a single drain point for both the keypress and sync-Steel paths.
    pub(super) fn fire_hook_silent(&mut self, hook_id: HookId, args: &[steel::rvals::SteelVal]) {
        self.state.pending_hooks.push((hook_id, args.to_vec()));
    }

    /// Fire every hook in `state.pending_hooks`, draining the queue.
    ///
    /// Called once per interactive input event by `handle_event` (the single
    /// interactive drain boundary), and once at startup in `lib.rs` before the
    /// event loop begins. Inner hook handlers may enqueue more hooks; the outer
    /// loop re-drains until the queue is empty — capped at
    /// [`MAX_HOOK_DRAIN_PASSES`] so a handler feedback loop (e.g. two
    /// `on-language-set` handlers ping-ponging `set-buffer-language!` between
    /// two values) cannot livelock the editor.  The watchdog only bounds each
    /// individual eval, not this loop.
    pub(crate) fn drain_hooks(&mut self) {
        let mut passes = 0;
        while !self.state.pending_hooks.is_empty() {
            passes += 1;
            if passes > MAX_HOOK_DRAIN_PASSES {
                let dropped = self.state.pending_hooks.len();
                self.state.pending_hooks.clear();
                self.report(
                    Severity::Error,
                    format!(
                        "hook cascade exceeded {MAX_HOOK_DRAIN_PASSES} drain passes — \
                         dropping {dropped} pending hook(s); handler feedback loop?"
                    ),
                );
                return;
            }
            let hooks = std::mem::take(&mut self.state.pending_hooks);
            for (hook_id, args) in hooks {
                // Activate lazy event plugins first so their register-hook! calls
                // land before the has_hook_handlers check below.
                self.activate_lazy_event_plugins(hook_id);
                if self
                    .scripting
                    .as_ref()
                    .is_none_or(|h| !h.has_hook_handlers(hook_id))
                {
                    continue;
                }
                let pid = self.state.focused_pane_id;
                let bid = self.focused_buffer_id();
                let result = {
                    let host_scr = self.scripting.as_mut().expect("checked above");
                    let mut impl_host = EditorHostImpl {
                        state: &mut self.state,
                        view: &mut self.view,
                    };
                    host_scr.fire_hook(hook_id, &args, pid, bid, &mut impl_host)
                };
                self.flush_script_messages();
                match result {
                    Ok(HookResult {
                        pending_language_sets,
                        grammar_sweeps,
                    }) => {
                        for (bid, lang) in pending_language_sets {
                            self.set_buffer_language(bid, lang);
                        }
                        if !grammar_sweeps.is_empty() {
                            self.sweep_buffers_for_grammars(grammar_sweeps);
                        }
                    }
                    Err(e) => self.report(Severity::Error, format!("hook error: {e}")),
                }
            }
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
        let Some(config_dir) = hume_platform::dirs::config_dir() else {
            self.report(
                Severity::Warning,
                "scripting: no config directory — HOME/APPDATA unset; init.scm skipped".into(),
            );
            return;
        };
        let init_path = config_dir.join("init.scm");
        let mut host = hume_scripting::ScriptingHost::new();
        // Pre-register every native command name as a callable Steel binding before
        // any user code sees the engine.  This lets `init.scm` call `(move-left)`
        // directly without a FreeIdentifier compile error.  Only native commands
        // (Motion/Selection/Edit/EditorCmd) are registered — plugin commands
        // (`SteelBacked`/`Lazy`) don't exist yet and use `(call! …)` instead.
        {
            let names: Vec<&str> = self.state.registry.native_mappable_names().collect();
            host.register_command_names(&names);
        }
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
            self.state.registry.names().map(String::from).collect();
        self.builtin_cmd_names = builtin_names.clone();
        // Reset the language registry so `:reload-config` gets a fresh set
        // of registrations from languages.scm rather than accumulating duplicates.
        self.state.languages = crate::editor::syntax::LanguageRegistry::new();
        // Load runtime/scheme/prelude.scm before init.scm so its macros
        // (bind-keys! etc.) are available to init.scm and plugin modules.
        // Missing prelude is a silent no-op (optional sugar); a prelude that
        // exists but fails to parse/eval is an error reported separately.
        //
        // Each eval gets a fresh EditorHostImpl over `state` + `view`; the
        // `require_cmd_ctx!` guard keeps command-mode builtins unreachable
        // during init evals.
        if let Some(prelude_path) = host.runtime_dir().map(|rt| rt.join("scheme/prelude.scm")) {
            let init_budget = self.state.settings.steel_init_budget_ms as u64;
            let mut ih = make_init_host(&mut self.state, &mut self.view);
            if let Err(msg) =
                host.eval_init(&prelude_path, init_budget, &mut ih, builtin_names.clone())
            {
                self.report(
                    Severity::Error,
                    format!("runtime/scheme/prelude.scm: {msg}"),
                );
            }
        }
        // Load languages.scm between prelude and init.scm so (define-language! …)
        // calls are available when init.scm and plugins run.
        let langs_path = host.runtime_dir().map(|rt| rt.join("scheme/languages.scm"));
        if let Some(langs_path) = langs_path {
            let init_budget = self.state.settings.steel_init_budget_ms as u64;
            let mut ih = make_init_host(&mut self.state, &mut self.view);
            if let Err(msg) =
                host.eval_init(&langs_path, init_budget, &mut ih, builtin_names.clone())
            {
                self.report(
                    Severity::Error,
                    format!("runtime/scheme/languages.scm: {msg}"),
                );
            }
            self.flush_pending_language_regs(&mut host);
        }
        {
            let init_budget = self.state.settings.steel_init_budget_ms as u64;
            let mut ih = make_init_host(&mut self.state, &mut self.view);
            if let Err(msg) = host.eval_init(&init_path, init_budget, &mut ih, builtin_names) {
                self.report(Severity::Error, format!("init.scm: {msg}"));
            }
        }
        // Register lazy-command stubs for every #:commands activation entry
        // declared during init.scm.  Must run after register_steel_cmds (eager
        // plugins may have defined commands that would collide) and before
        // scripting=Some so the borrow of &host is independent.
        let activation_commands = host.activation_commands();
        let collided = self.register_lazy_command_stubs(&activation_commands);
        for name in collided {
            host.drop_activation_command(&name);
        }
        // Snapshot language activation entries before the second flush so the
        // post-init lint (below) can compare them against the final language registry.
        let lang_activations = host.activation_languages();
        // Second flush: picks up any (define-language! …) calls from init.scm /
        // plugins that ran during init.scm.
        self.flush_pending_language_regs(&mut host);
        // Pick up any (set-option! "history-capacity" N) calls from init.scm.
        self.state
            .history
            .set_capacity(self.state.settings.history_capacity);
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
            let mut names = self.state.keymap.all_command_names();
            names.sort_unstable();
            names.dedup();
            for name in &names {
                if !self.state.registry.contains(name) {
                    self.report(
                        Severity::Warning,
                        format!("key bound to unknown command '{name}' — typo, or missing from #:commands?"),
                    );
                }
            }
        }
        // Post-init lint: warn on #:languages activation entries whose language name
        // is not in the final LanguageRegistry.  Runs after the second flush (line 199)
        // so both languages.scm and init.scm's own define-language! calls are visible.
        // Uses by_name() (identity registered), not has_grammar() (grammar attached):
        // an activation entry is valid for any known language identity even without a
        // grammar.  Not a hard error: inert but harmless, and language sets are
        // open/dynamic (a future define-language! + reload may make the name valid).
        for (lang, plugins) in &lang_activations {
            if self.state.languages.by_name(lang).is_none() {
                for plugin in plugins {
                    self.report(
                        Severity::Warning,
                        format!(
                            "plugin '{plugin}' declares #:languages activation for unknown \
                             language '{lang}' — typo, or missing (define-language!)?"
                        ),
                    );
                }
            }
        }
        // Load theme set by (set-option! 'theme "…") in init.scm.
        if !self.state.settings.theme.is_empty() {
            ops::load_theme_by_name(
                &mut self.view,
                &mut self.state.message_log,
                &mut self.state.status_msg,
                &self.state.settings.theme,
            );
        }
        // Re-detect language for buffers opened before init_scripting ran
        // (the initial buffer is opened in lib.rs before init_scripting is called).
        let open_bids: Vec<_> = self.state.buffers.iter().map(|(id, _)| id).collect();
        for bid in open_bids {
            self.detect_and_set_language(bid);
        }
    }

    /// Register a `Lazy` stub for each command activation entry from the plugin manifests.
    ///
    /// Called after `register_steel_cmds` (eager plugins run first) so a
    /// command defined eagerly is detected as a conflict before a lazy stub
    /// for the same name would shadow it.
    ///
    /// Returns the names that were skipped due to collision so the caller can
    /// remove their declare-time entries from the scripting host's activation
    /// maps — preventing stale attribution that would mis-route a future dispatch.
    pub(super) fn register_lazy_command_stubs(
        &mut self,
        activations: &std::collections::HashMap<String, hume_scripting::attribution::PluginId>,
    ) -> Vec<String> {
        let mut collided = Vec::new();
        for (name, plugin) in activations {
            if self.state.registry.contains(name) {
                self.report(
                    Severity::Error,
                    format!("lazy command '{name}' conflicts with an existing command"),
                );
                collided.push(name.clone());
            } else {
                self.state
                    .registry
                    .register(super::registry::MappableCommand::Lazy {
                        name: name.clone().into(),
                        plugin: plugin.clone(),
                    });
            }
        }
        collided
    }
}

// ---------------------------------------------------------------------------
// Module-level helpers
// ---------------------------------------------------------------------------

/// Build an `EditorHostImpl` from disjoint borrows of the editor's state and view.
pub(crate) fn make_init_host<'a>(
    state: &'a mut super::EditorState,
    view: &'a mut hume_engine::pipeline::EngineView,
) -> EditorHostImpl<'a> {
    EditorHostImpl { state, view }
}

/// Map scripting `LogLevel` → editor `Severity`.
pub(crate) fn log_level_to_severity(level: hume_scripting::LogLevel) -> Severity {
    use hume_scripting::LogLevel;
    match level {
        LogLevel::Info => Severity::Info,
        LogLevel::Warning => Severity::Warning,
        LogLevel::Error => Severity::Error,
        LogLevel::Trace => Severity::Trace,
    }
}

/// Ordered list of directories to search for theme TOML files.
///
/// Config themes (user-defined) are listed before runtime themes (bundled) so
/// that user overrides shadow built-in ones. Both `ops::load_theme_by_name`
/// and [`ThemeCompleter`] use this list as the single source of truth.
pub(super) fn theme_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(cfg) = hume_platform::dirs::config_dir() {
        paths.push(cfg.join("themes"));
    }
    if let Some(rt) = hume_platform::dirs::runtime_dir() {
        paths.push(rt.join("themes"));
    }
    paths
}
