use std::path::PathBuf;

use hume_engine::pipeline::BufferId;

use hume_scripting::SteelBufferId;
use hume_scripting::{Effect, hooks::HookId};
use steel::rvals::SteelVal;

use super::reload::ReloadSnapshot;
use super::{Editor, Severity, host_impl::EditorHostImpl};

/// Upper bound on total hooks processed per `drain_hooks` boundary.
///
/// Bounding *passes* instead of total work would still let an amplifying
/// cascade (a handler that enqueues more hooks than it received) blow up the
/// batch size geometrically pass over pass — few passes, but exponential
/// total evals. Counting total hooks processed bounds both that shape and the
/// constant-width ping-pong loop; unreachable in any legitimate configuration.
const MAX_HOOK_DRAIN_HOOKS: usize = 1000;

impl Editor {
    /// Apply every effect a Steel eval queued, in the exact order the
    /// script emitted them (`hume_scripting::Effect`) — one ordered log, not
    /// separate channels with a hardcoded apply order. Consecutive
    /// `Effect::LanguageReg` entries are grouped into one
    /// `apply_pending_language_regs` call so a large run (e.g.
    /// `languages.scm`'s ~700 `define-language!` calls) rebuilds the glob
    /// matcher once, not once per entry; every other effect kind applies one
    /// at a time. Shared tail for `call_steel_cmd`'s call site, `drain_hooks`,
    /// `drain_pending_steel_calls`, and `init_scripting`.
    ///
    /// Finishes by draining `state.config.pending_language_detection` — covers every
    /// buffer a disjoint-borrow Steel path opened via `buffer::lifecycle::
    /// open_buffer_and_notify` this eval (`(open-buffer! …)`, workspace edits,
    /// goto-definition). *After* the effect loop, not before: a script that
    /// opens a buffer and registers its language in the same eval must have
    /// the registration land first.
    pub(crate) fn apply_script_effects(&mut self, effects: Vec<Effect>) {
        let mut effects: std::collections::VecDeque<Effect> = effects.into();
        while let Some(effect) = effects.pop_front() {
            match effect {
                Effect::LanguageReg(reg) => {
                    let mut batch = vec![reg];
                    while matches!(effects.front(), Some(Effect::LanguageReg(_))) {
                        let Some(Effect::LanguageReg(next)) = effects.pop_front() else {
                            unreachable!("front() just confirmed a LanguageReg variant")
                        };
                        batch.push(next);
                    }
                    self.apply_pending_language_regs(batch);
                }
                Effect::LspServerOp(op) => self.apply_lsp_server_op(op),
                Effect::SetBufferLanguage { buffer, language } => {
                    let lang_id = language.map(|name| self.state.config.languages.intern(&name));
                    self.set_buffer_language_explicit(buffer, lang_id)
                }
                Effect::GrammarSweep(name) => {
                    let id = self.state.config.languages.id_of(&name).expect(
                        "GrammarSweep is only emitted right after attach_grammar interns the name",
                    );
                    self.sweep_buffers_for_grammars(vec![id])
                }
                Effect::LspRequest(req) => {
                    self.flush_lsp_pending_changes();
                    self.send_one_lsp_request(req);
                }
                Effect::LspNotify(notify) => {
                    self.flush_lsp_pending_changes();
                    self.send_one_lsp_notify(notify);
                }
                Effect::BindKey {
                    mode,
                    keys,
                    cmd,
                    force_extend,
                } => self.state.config.keymap.bind_user_with_extend(
                    to_editor_bind_mode(mode),
                    &keys,
                    std::borrow::Cow::Owned(cmd),
                    force_extend,
                ),
                Effect::BindWaitChar { mode, keys, cmd } => {
                    self.state.config.keymap.bind_wait_char_user(
                        to_editor_bind_mode(mode),
                        &keys,
                        std::borrow::Cow::Owned(cmd),
                    )
                }
                Effect::UnbindKey { mode, keys } => self
                    .state
                    .config
                    .keymap
                    .unbind_user(to_editor_bind_mode(mode), &keys),
            }
        }
        self.detect_pending_languages();
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
        let (first_line, last_line) = super::lsp::introspect::pane_visible_range(pane, total_lines);
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

    /// Fire `OnTriggerChar (bid char-string source)` — Insert mode, after
    /// `ch` has already been inserted into `bid` (mappings/insert.rs), once
    /// per source registered for `ch` under `bid`'s language via
    /// `(register-trigger-chars! source language chars)`.
    pub(super) fn fire_hook_trigger_char(&mut self, bid: BufferId, ch: char, source: &str) {
        let bid_val = SteelBufferId::new(bid).into_steel_val();
        let ch_val = SteelVal::StringV(ch.to_string().into());
        let source_val = SteelVal::StringV(source.into());
        self.fire_hook_silent(HookId::OnTriggerChar, &[bid_val, ch_val, source_val]);
    }

    /// Fire all Steel handlers for `hook_id`, passing `args` to each.
    ///
    /// Enqueue `hook_id` to fire after the current command returns.
    ///
    /// The unified hook-firing path: all hook scheduling goes through
    /// `state.config.pending_hooks`; `Editor::drain_hooks` does the actual Steel eval.
    /// This prevents re-entrant Steel calls during command execution and gives
    /// a single drain point for both the keypress and sync-Steel paths.
    pub(super) fn fire_hook_silent(&mut self, hook_id: HookId, args: &[steel::rvals::SteelVal]) {
        self.state
            .config
            .pending_hooks
            .push((hook_id, args.to_vec()));
    }

    /// Fire every hook in `state.config.pending_hooks`, draining the queue.
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
        while !self.state.config.pending_hooks.is_empty() {
            let hooks = std::mem::take(&mut self.state.config.pending_hooks);
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
                        terminal: self.terminal.as_ref(),
                    };
                    host_scr.fire_hook(hook_id, &args, pid, bid, &mut impl_host)
                };
                self.flush_script_messages();
                match result {
                    Ok(effects) => self.apply_script_effects(effects),
                    Err(e) => {
                        self.apply_script_effects(e.effects);
                        self.report(Severity::Error, format!("hook error: {}", e.message));
                    }
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
        self.state.config.pending_steel_calls.push((proc, args));
    }

    /// Drain `state.config.pending_steel_calls`, evaluating each queued call in one
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
        let calls = std::mem::take(&mut self.state.config.pending_steel_calls);
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
                terminal: self.terminal.as_ref(),
            };
            host_scr.run_steel_calls(calls, pid, bid, &mut impl_host)
        };
        self.flush_script_messages();
        match result {
            Ok(effects) => self.apply_script_effects(effects),
            Err(e) => {
                self.apply_script_effects(e.effects);
                self.report(Severity::Error, format!("steel call error: {}", e.message));
            }
        }
        // A call just run above (an LSP-request callback, a timer thunk) can
        // itself dispatch a command that exits Insert, setting the flag the
        // top-of-function consumption already passed. Consume it again so
        // `prepare_frame`'s later `sync_completion_menu_view` never repaints a
        // session `set_mode` asked to close mid-drain.
        self.take_pending_lsp_completion_dismiss();
    }

    // ── Scripting ─────────────────────────────────────────────────────────────

    /// Initialise the Steel scripting host and evaluate `init.scm`.
    ///
    /// Called once at startup, after `Editor::open` returns and before
    /// `Editor::run` starts (with `snapshot` at its `Default` — nothing to
    /// restore, no pre-reload buffers), and by `typed_reload_config` on
    /// every `:reload-config` (with the `ReloadSnapshot` `reset_config_state`
    /// just produced). Any error from `init.scm` is reported as
    /// `Severity::Error` and shown in the statusline.
    pub(crate) fn init_scripting(&mut self, snapshot: &mut ReloadSnapshot) {
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
            let names: Vec<&str> = self.state.config.registry.native_mappable_names().collect();
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
        let builtin_names: rustc_hash::FxHashSet<String> = self
            .state
            .config
            .registry
            .names()
            .map(String::from)
            .collect();
        self.builtin_cmd_names = builtin_names.clone();
        // Load runtime/scheme/prelude.scm before init.scm so its macros
        // (bind-keys! etc.) are available to init.scm and plugin modules.
        // Then languages.scm ((define-language! …) identities) and
        // grammars.scm (registers already-compiled grammars — see its own
        // header) — languages.scm runs first so identity registration owns
        // the one glob-set rebuild (attach_grammar would otherwise create a
        // bare default identity per grammar, ahead of languages.scm's own).
        // Each is a silent no-op when the runtime dir or the file itself is
        // missing (optional layers); a file that exists but fails to
        // parse/eval is an error reported separately.
        //
        // Each eval gets a fresh EditorHostImpl over `state` + `view`; the
        // `require_cmd_ctx!` guard keeps command-mode builtins unreachable
        // during init evals.
        self.eval_runtime_scheme(&mut host, "scheme/prelude.scm", builtin_names.clone());
        self.eval_runtime_scheme(&mut host, "scheme/languages.scm", builtin_names.clone());
        self.eval_runtime_scheme(&mut host, "scheme/grammars.scm", builtin_names.clone());
        {
            let init_budget = self.state.settings.steel_init_budget_ms as u64;
            let result = {
                let mut ih = make_init_host(&mut self.state, &mut self.view);
                host.eval_init(&init_path, init_budget, &mut ih, builtin_names)
            };
            match result {
                Ok(effects) => self.apply_script_effects(effects),
                Err(e) => {
                    self.apply_script_effects(e.effects);
                    self.report(Severity::Error, format!("init.scm: {}", e.message));
                }
            }
        }
        // Snapshot language activation entries for the post-init lint below —
        // every eval's effects (identities, grammars, LSP server ops) are
        // already applied above, each right after its own eval, so
        // `self.state.config.languages` is fully populated by this point.
        let lang_activations = host.activation_languages();
        // Flush any `(log! …)` messages produced during init.scm evaluation.
        for (level, text) in host.take_pending_messages() {
            self.report(log_level_to_severity(level), text);
        }
        self.scripting = Some(host);
        // Post-init lint: warn on keymap leaves that target an unknown command.
        // Lazy stubs are registered live as each declare-plugin call runs during
        // init.scm eval, so they already count as valid commands by this point.
        // Built-in keymaps only reference registered built-ins, so any warnings
        // here come from user bind-key! calls to typos / undeclared commands.
        {
            let mut names = self.state.config.keymap.all_command_names();
            names.sort_unstable();
            names.dedup();
            for name in &names {
                if !self.state.config.registry.contains(name) {
                    self.report(
                        Severity::Warning,
                        format!("key bound to unknown command '{name}' — typo, or missing from #:commands?"),
                    );
                }
            }
        }
        // Post-init lint: warn on #:languages activation entries whose language name
        // is not in the final LanguageRegistry.  Runs after the message flush above
        // so both languages.scm and init.scm's own define-language! calls are visible.
        // Uses by_name() (identity registered), not has_grammar() (grammar attached):
        // an activation entry is valid for any known language identity even without a
        // grammar.  Not a hard error: inert but harmless, and language sets are
        // open/dynamic (a future define-language! + reload may make the name valid).
        for (lang, plugins) in &lang_activations {
            // "*" is the any-language wildcard (manifest.scm can't enumerate every
            // language it might ever support) — not a language identity to look up.
            if lang != "*" && self.state.config.languages.by_name(lang).is_none() {
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
        // Re-detect language for every open buffer (the startup buffer is
        // opened in lib.rs before init_scripting is called; a
        // `:reload-config` cleared every buffer's language in
        // `reset_config_state`). A buffer with an explicit assertion —
        // restored from `snapshot` (this is a `:reload-config`) or made by
        // `init.scm` itself against a buffer that predates this call (e.g.
        // the startup buffer) — skips detection entirely: `set_buffer_
        // language_impl` stamps `language_explicit` unconditionally, so a
        // later detection pass would otherwise silently overwrite that
        // assertion with whatever plain detection finds. Restoring the
        // snapshot *inside* this loop, rather than as a second
        // detect-then-correct pass afterward, also avoids attaching an LSP
        // server for the buffer's *detected* language before its real,
        // explicit one goes back — `lsp_attach_buffer` is a no-op once
        // attached, so that wrong attach would otherwise stick.
        let explicit_restore: rustc_hash::FxHashMap<BufferId, Option<String>> =
            snapshot.take_explicit_languages().into_iter().collect();
        let open_bids: Vec<_> = self.state.buffers.iter().map(|(id, _)| id).collect();
        for bid in open_bids {
            // A lazy language plugin activated earlier in this same loop
            // (`detect_and_set_language` → `set_buffer_language_impl` →
            // `activate_lazy_language_plugins`) runs at `EvalMode::
            // PluginActivation`, where `close-buffer!` is callable — it can
            // close a *later* bid in this same `open_bids` list before this
            // loop ever reaches it. Same hazard `detect_pending_languages`
            // guards against with `try_get`; skip rather than hit
            // `BufferStore::get`'s "unseeded BufferId" panic below.
            if self.state.buffers.try_get(bid).is_none() {
                continue;
            }
            if let Some(name) = explicit_restore.get(&bid) {
                // Only valid if `bid` is still the same buffer instance the
                // snapshot meant — `close_buffer`'s last-buffer scratch
                // replacement can otherwise alias a closed buffer's explicit
                // language onto unrelated fresh content (see
                // `ReloadSnapshot::survives`).
                if snapshot.survives(bid, &self.state.buffers) {
                    match name {
                        Some(name) => match self.state.config.languages.id_of(name) {
                            Some(lang_id) => self.set_buffer_language_explicit(bid, Some(lang_id)),
                            None => {
                                self.report(
                                    Severity::Warning,
                                    format!(
                                        "language '{name}' was explicitly set on a buffer \
                                         before reload, but is no longer registered — \
                                         falling back to detection"
                                    ),
                                );
                                self.detect_and_set_language(bid);
                            }
                        },
                        None => self.set_buffer_language_explicit(bid, None),
                    }
                    continue;
                }
            } else if self.state.buffers.get(bid).language_explicit {
                // Asserted during *this very* init.scm eval (e.g. on a
                // buffer that predates this call) — detection must not
                // clobber it either; see `set_buffer_language_impl`'s doc.
                continue;
            }
            self.detect_and_set_language(bid);
        }
    }

    /// Evaluate a bundled runtime Scheme file (`rel_path`, relative to the
    /// runtime dir) as an init-mode eval — shared by `init_scripting`'s
    /// prelude/languages/grammars loads. A missing runtime dir or a missing
    /// file at that path is a silent no-op (all three are optional layers);
    /// a file that exists but fails to parse/eval is reported as an error.
    fn eval_runtime_scheme(
        &mut self,
        host: &mut hume_scripting::ScriptingHost,
        rel_path: &str,
        builtin_names: rustc_hash::FxHashSet<String>,
    ) {
        let Some(path) = host.runtime_dir().map(|rt| rt.join(rel_path)) else {
            return;
        };
        let init_budget = self.state.settings.steel_init_budget_ms as u64;
        let result = {
            let mut ih = make_init_host(&mut self.state, &mut self.view);
            host.eval_init(&path, init_budget, &mut ih, builtin_names)
        };
        match result {
            Ok(effects) => self.apply_script_effects(effects),
            Err(e) => {
                self.apply_script_effects(e.effects);
                self.report(
                    Severity::Error,
                    format!("runtime/{rel_path}: {}", e.message),
                );
            }
        }
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

/// Map the scripting layer's `BindMode` (carried in `Effect::BindKey` and
/// friends) to the editor's own. Fully qualified on both sides: this module
/// works with `crate::editor::keymap::BindMode` too, so importing either name
/// bare would shadow the other.
fn to_editor_bind_mode(mode: hume_scripting::host::BindMode) -> crate::editor::keymap::BindMode {
    match mode {
        hume_scripting::host::BindMode::Normal => crate::editor::keymap::BindMode::Normal,
        hume_scripting::host::BindMode::Extend => crate::editor::keymap::BindMode::Extend,
        hume_scripting::host::BindMode::Insert => crate::editor::keymap::BindMode::Insert,
    }
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
