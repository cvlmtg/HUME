use std::path::PathBuf;

use hume_engine::pipeline::BufferId;

use hume_scripting::SteelBufferId;
use hume_scripting::{HookResult, hooks::HookId};
use steel::rvals::SteelVal;

use super::{Editor, Severity, host_impl::EditorHostImpl, theme};

/// Upper bound on total hooks processed per `drain_hooks` boundary.
///
/// Bounding *passes* instead of total work would still let an amplifying
/// cascade (a handler that enqueues more hooks than it received) blow up the
/// batch size geometrically pass over pass — few passes, but exponential
/// total evals. Counting total hooks processed bounds both that shape and the
/// constant-width ping-pong loop; unreachable in any legitimate configuration.
const MAX_HOOK_DRAIN_HOOKS: usize = 1000;

impl Editor {
    /// Apply a `HookResult`'s queued side effects: drain any queued language
    /// registrations and LSP server registrations/unregistrations,
    /// send/notify any queued LSP calls, apply deferred `set-buffer-language!`
    /// calls, and sweep buffers for newly attached grammars. Shared tail for
    /// `call_steel_cmd`'s call site, `drain_hooks`, and `drain_pending_steel_calls`.
    pub(crate) fn apply_script_effects(&mut self, effects: HookResult) {
        // Language regs drain first: a `define-language!` queued earlier in
        // this same eval must register the identity before the LSP-server-ops
        // and `pending_language_sets` drains below, so a same-eval
        // `set-buffer-language!` for that language detects it immediately.
        // LSP server ops drain next: `register-lsp-server!` queued earlier
        // in this same eval must apply before the `pending_language_sets`
        // loop below, so a `set-buffer-language!` in the same eval attaches
        // through the new config directly rather than waiting a drain cycle.
        // Both queues are persistent on the host (unlike `HookResult`'s
        // per-eval fields), so they're pulled with the same two-phase take
        // `flush_script_messages` uses.
        let lang_regs =
            self.take_from_host(hume_scripting::ScriptingHost::take_pending_language_regs);
        self.apply_pending_language_regs(lang_regs);

        let lsp_server_ops =
            self.take_from_host(hume_scripting::ScriptingHost::take_pending_lsp_server_ops);
        self.apply_lsp_server_ops(lsp_server_ops);

        let HookResult {
            pending_language_sets,
            grammar_sweeps,
            pending_lsp_requests,
            pending_lsp_notifies,
        } = effects;
        self.flush_pending_lsp_calls(pending_lsp_requests, pending_lsp_notifies);
        for (bid, lang) in pending_language_sets {
            self.set_buffer_language(bid, lang);
        }
        if !grammar_sweeps.is_empty() {
            self.sweep_buffers_for_grammars(grammar_sweeps);
        }
    }

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

    /// Take a persistent queue off the scripting host, or an empty `Vec` if
    /// scripting isn't initialized. Collecting into an owned `Vec` first
    /// (rather than draining in place) satisfies the borrow checker at every
    /// call site, which also needs `&mut self` to apply what's taken.
    fn take_from_host<T>(
        &mut self,
        taker: impl FnOnce(&mut hume_scripting::ScriptingHost) -> Vec<T>,
    ) -> Vec<T> {
        self.scripting.as_mut().map(taker).unwrap_or_default()
    }

    /// Drain any pending `(log! …)` messages from the scripting host and
    /// report each one.
    pub(crate) fn flush_script_messages(&mut self) {
        let msgs = self.take_from_host(hume_scripting::ScriptingHost::take_pending_messages);
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

    /// Fire `OnLspAttach (bid server-name)` — called both when a buffer
    /// attaches to an already-Running server (`lsp_attach_buffer`) and, for
    /// every buffer already attached, when a Starting client reaches
    /// Running (`dispatch_lsp_action`'s `BecameRunning` arm).
    pub(super) fn fire_hook_lsp_attach(&mut self, bid: BufferId, server_name: &str) {
        let bid_val = SteelBufferId::new(bid).into_steel_val();
        let name_val = SteelVal::StringV(server_name.into());
        self.fire_hook_silent(HookId::OnLspAttach, &[bid_val, name_val]);
    }

    /// Fire `OnLspDetach (bid server-name)` — called from `lsp_stop_one` for
    /// every buffer that was attached to the server being stopped, right
    /// after `buf.lsp_server` is cleared.
    pub(super) fn fire_hook_lsp_detach(&mut self, bid: BufferId, server_name: &str) {
        let bid_val = SteelBufferId::new(bid).into_steel_val();
        let name_val = SteelVal::StringV(server_name.into());
        self.fire_hook_silent(HookId::OnLspDetach, &[bid_val, name_val]);
    }

    /// Fire `OnDiagnosticsChanged (bid)` — payload-free signal, once per
    /// buffer a `publishDiagnostics` drain batch actually touched
    /// (`drain_lsp`). Handlers pull via `(diagnostics-for-buffer bid …)`.
    pub(super) fn fire_hook_diagnostics_changed(&mut self, bid: BufferId) {
        let val = SteelBufferId::new(bid).into_steel_val();
        self.fire_hook_silent(HookId::OnDiagnosticsChanged, &[val]);
    }

    /// Fire `OnViewportChange (bid first-line last-line)` for `pane_id` —
    /// called only when its debounce timer actually fires (`timer_bridge`),
    /// reading the pane's *current* bounds rather than whatever they were
    /// when the timer was armed. A no-op if the pane closed in the meantime.
    pub(super) fn fire_hook_viewport_change(&mut self, pane_id: hume_engine::pipeline::PaneId) {
        let Some(pane) = self.view.panes.get(pane_id) else {
            return;
        };
        let bid = pane.buffer_id;
        let total_lines = self.state.buffers.get(bid).text().len_lines();
        let (first_line, last_line) =
            super::lsp::introspect::pane_visible_range(pane, total_lines);
        let bid_val = SteelBufferId::new(bid).into_steel_val();
        self.fire_hook_silent(
            HookId::OnViewportChange,
            &[
                bid_val,
                SteelVal::IntV(first_line as isize),
                SteelVal::IntV(last_line as isize),
            ],
        );
    }

    /// Fire `OnTriggerChar (bid char-string)` — Insert mode, after `ch` has
    /// already been inserted into `bid` (mappings/insert.rs), and only for
    /// a char registered via `(register-trigger-chars! source chars)`.
    pub(super) fn fire_hook_trigger_char(&mut self, bid: BufferId, ch: char) {
        let bid_val = SteelBufferId::new(bid).into_steel_val();
        let ch_val = SteelVal::StringV(ch.to_string().into());
        self.fire_hook_silent(HookId::OnTriggerChar, &[bid_val, ch_val]);
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
    /// [`MAX_HOOK_DRAIN_HOOKS`] total hooks processed (not passes) so neither a
    /// constant-width ping-pong loop (e.g. two `on-language-set` handlers
    /// flipping `set-buffer-language!` between two values) nor an amplifying
    /// cascade (a handler that enqueues more hooks than it received, doubling
    /// the batch pass over pass) can livelock the editor.  The watchdog only
    /// bounds each individual eval, not this loop.
    pub(crate) fn drain_hooks(&mut self) {
        // Any mode change queued between the last consumption point and now
        // (a hook handler earlier this same batch, or something that ran
        // before `drain_hooks` was even called) must not survive into the
        // handler calls below. Unconditional, at the top, so no early-return
        // branch below can skip it.
        self.take_pending_lsp_completion_dismiss();
        let mut total_processed = 0usize;
        while !self.state.pending_hooks.is_empty() {
            let hooks = std::mem::take(&mut self.state.pending_hooks);
            total_processed += hooks.len();
            if total_processed > MAX_HOOK_DRAIN_HOOKS {
                // `hooks` was just drained from `pending_hooks` above, so
                // nothing has been re-enqueued yet — it's the entire drop.
                let dropped = hooks.len();
                self.report(
                    Severity::Error,
                    format!(
                        "hook cascade exceeded {MAX_HOOK_DRAIN_HOOKS} total drained hook(s) — \
                         dropping {dropped} pending hook(s); handler feedback loop?"
                    ),
                );
                return;
            }
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
                        lsp: Some(&mut self.lsp),
                        timers: Some(super::timer_bridge::TimerHandle {
                            wheel: &mut self.timer_wheel,
                            payloads: &mut self.timer_payloads,
                        }),
                    };
                    host_scr.fire_hook(hook_id, &args, pid, bid, &mut impl_host)
                };
                self.flush_script_messages();
                match result {
                    Ok(effects) => self.apply_script_effects(effects),
                    Err(e) => self.report(Severity::Error, format!("hook error: {e}")),
                }
            }
        }
    }

    /// Queue `(proc, args)` for evaluation at the next drain boundary —
    /// never called inline (LSP dispatch, timer fire, and minibuffer key
    /// handling all detect their completion from inside a borrow that can't
    /// re-enter Steel). Shared delivery mechanism for the `lsp-request`
    /// callback, timer thunks, and the prompt callback.
    pub(crate) fn queue_steel_call(&mut self, proc: SteelVal, args: Vec<SteelVal>) {
        self.state.pending_steel_calls.push((proc, args));
    }

    /// Drain `state.pending_steel_calls`, evaluating each queued call in one
    /// Steel session. Called once per frame from `prepare_frame` — the
    /// per-frame cadence LSP responses and timer fires already drain on, so
    /// a completion queued this frame runs before the next render rather
    /// than waiting for the next keystroke (unlike hooks, nothing here is
    /// naturally re-triggering, so a single pass is enough — anything a
    /// callback itself queues lands in next frame's drain, not this one).
    pub(crate) fn drain_pending_steel_calls(&mut self) {
        // Same reasoning as the top of `drain_hooks` — unconditional so no
        // early-return branch below can skip it. `prepare_frame` calls this
        // every frame, so no separate render-time consumption is needed.
        self.take_pending_lsp_completion_dismiss();
        let calls = std::mem::take(&mut self.state.pending_steel_calls);
        if calls.is_empty() {
            return;
        }
        let pid = self.state.focused_pane_id;
        let bid = self.focused_buffer_id();
        let Some(host_scr) = self.scripting.as_mut() else {
            return;
        };
        let result = {
            let mut impl_host = EditorHostImpl {
                state: &mut self.state,
                view: &mut self.view,
                lsp: Some(&mut self.lsp),
                timers: Some(super::timer_bridge::TimerHandle {
                    wheel: &mut self.timer_wheel,
                    payloads: &mut self.timer_payloads,
                }),
            };
            host_scr.run_steel_calls(calls, pid, bid, &mut impl_host)
        };
        self.flush_script_messages();
        match result {
            Ok(effects) => self.apply_script_effects(effects),
            Err(e) => self.report(Severity::Error, format!("steel call error: {e}")),
        }
        // A call just run above (an LSP-request callback, a timer thunk) can
        // itself dispatch a command that exits Insert, setting the flag the
        // top-of-function consumption already passed. Consume it again so
        // `prepare_frame`'s later `sync_lsp_completion_view` never repaints a
        // session `set_mode` asked to close mid-drain.
        self.take_pending_lsp_completion_dismiss();
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
        self.state.languages = hume_treesitter::registry::LanguageRegistry::new();
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
        // Picks up any (register-lsp-server! …) / (unregister-lsp-server! …)
        // calls from init.scm / plugins.
        self.flush_pending_lsp_server_ops(&mut host);
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
            // "*" is the any-language wildcard (manifest.scm can't enumerate every
            // language it might ever support) — not a language identity to look up.
            if lang != "*" && self.state.languages.by_name(lang).is_none() {
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
            theme::load_theme_by_name(
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
    EditorHostImpl::new(state, view)
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
/// that user overrides shadow built-in ones. Both `theme::load_theme_by_name`
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
