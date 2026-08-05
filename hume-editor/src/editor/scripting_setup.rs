use std::path::PathBuf;

use hume_engine::pipeline::BufferId;

use hume_scripting::Effect;
use steel::rvals::SteelVal;

use super::buffer::DiskCheckTrigger;
use super::event::{EditorEvent, PendingWork};
use super::reload::ReloadSnapshot;
use super::{Editor, Severity, host_impl::EditorHostImpl};

/// Upper bound on total work items processed per `settle()` boundary.
///
/// Bounding *passes* instead of total work would still let an amplifying
/// cascade (a handler that enqueues more hooks than it received) blow up the
/// batch size geometrically pass over pass — few passes, but exponential
/// total evals. Counting total hooks processed bounds both that shape and the
/// constant-width ping-pong loop; unreachable in any legitimate configuration.
const MAX_EVENT_DRAIN: usize = 1000;

impl Editor {
    /// Apply every effect a Steel eval queued, in the exact order the
    /// script emitted them (`hume_scripting::Effect`) — one ordered log, not
    /// separate channels with a hardcoded apply order. Consecutive
    /// `Effect::LanguageReg` entries are grouped into one
    /// `apply_pending_language_regs` call so a large run (e.g.
    /// `languages.scm`'s ~700 `define-language!` calls) rebuilds the glob
    /// matcher once, not once per entry; every other effect kind applies one
    /// at a time. Shared tail for `call_steel_cmd`'s call site, `settle`,
    /// and `init_scripting`.
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

    /// Shared `Ok`/`Err` handling for every scripting eval that returns
    /// `Result<Vec<Effect>, EvalError>`: apply the effects either way (an
    /// `EvalError` still carries whatever committed before the error), and
    /// on `Err`, report `{err_prefix}{message}`. `err_prefix` distinguishes
    /// which eval failed (a hook, a queued call, `init.scm`, a `runtime/`
    /// file) in the reported message; pass `""` for a caller with nothing to
    /// prefix.
    pub(crate) fn apply_script_result(
        &mut self,
        result: Result<Vec<Effect>, hume_scripting::EvalError>,
        err_prefix: &str,
    ) {
        match result {
            Ok(effects) => self.apply_script_effects(effects),
            Err(e) => {
                self.apply_script_effects(e.effects);
                self.report(Severity::Error, format!("{err_prefix}{}", e.message));
            }
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

    /// Drain any pending `(log! …)` messages from the scripting host and
    /// report each one. No-op if scripting isn't initialized.
    pub(crate) fn flush_script_messages(&mut self) {
        let msgs = self
            .scripting
            .as_mut()
            .map(hume_scripting::ScriptingHost::take_pending_messages)
            .unwrap_or_default();
        for (level, text) in msgs {
            self.report(log_level_to_severity(level), text);
        }
    }

    // ── Event queueing ───────────────────────────────────────────────────────

    /// Queue `OnBufferSave` for `bid`. One production caller
    /// (`commands/typed_file.rs`'s `mark_written_and_synced`, shared by
    /// every `:w`-family command); kept as a named seam anyway since tests
    /// across `unix/plugins.rs`, `unix/lsp_format.rs`, and this file's own
    /// `events.rs` call it directly to raise the event without dispatching
    /// a real write.
    pub(super) fn queue_buffer_save(&mut self, bid: BufferId) {
        self.state
            .queue_event(EditorEvent::OnBufferSave { buffer: bid });
    }

    /// Fire `OnLspAttach (bid server-name)` — called both when a buffer
    /// attaches to an already-Running server (`lsp_attach_buffer`) and, for
    /// every buffer already attached, when a Starting client reaches
    /// Running (`dispatch_lsp_action`'s `BecameRunning` arm).
    pub(super) fn queue_lsp_attach(&mut self, bid: BufferId, server_name: &str) {
        self.state.queue_event(EditorEvent::OnLspAttach {
            buffer: bid,
            server: server_name.to_owned(),
        });
    }

    /// Fire `OnDiagnosticsChanged (bid)` — payload-free signal, once per
    /// buffer a `publishDiagnostics` drain batch actually touched
    /// (`drain_lsp`). Handlers pull via `(diagnostics-for-buffer bid …)`.
    pub(super) fn queue_diagnostics_changed(&mut self, bid: BufferId) {
        self.state
            .queue_event(EditorEvent::OnDiagnosticsChanged { buffer: bid });
    }

    /// Fire `OnViewportChange (bid first-line last-line)` for `pane_id` —
    /// called only when its debounce timer actually fires (`timer_bridge`),
    /// reading the pane's *current* bounds rather than whatever they were
    /// when the timer was armed. A no-op if the pane closed in the meantime.
    pub(super) fn queue_viewport_change(&mut self, pane_id: hume_engine::pipeline::PaneId) {
        let Some(pane) = self.view.panes.get(pane_id) else {
            return;
        };
        let bid = pane.buffer_id;
        let total_lines = self.state.buffers.get(bid).text().len_lines();
        let (first_line, last_line) = super::lsp::introspect::pane_visible_range(pane, total_lines);
        self.state.queue_event(EditorEvent::OnViewportChange {
            buffer: bid,
            first_line,
            last_line,
        });
    }

    /// Advance editor state to quiescence: drain completed async work (parse
    /// results, LSP responses, timer fires — `drain_async_sources`), then
    /// drain `state.config.pending_work` to a fixpoint.
    ///
    /// This is the single consumer of the merged work queue (SPEC.md §3): a
    /// `Call` (an `lsp-request` callback, a timer thunk, a prompt/menu/
    /// drawer/picker callback) and an `Event` (fired to every handler
    /// registered for its name) drain in the exact order they were queued —
    /// grouped only where that's free, i.e. a contiguous run of `Call`s
    /// shares one Steel session, matching `run_steel_calls`' existing
    /// batching. A handler that queues more work is picked up within the
    /// same `settle()` call, not the next frame.
    ///
    /// Takes no arguments, so it's callable without a terminal — the
    /// headless path (`hume_editor::run_keys`) and `render_to_buf` both call
    /// it directly, alongside `Editor::run`'s loop.
    ///
    /// `drain_async_sources` runs once, *outside* [`Self::drain_pending_work`]'s
    /// fixpoint, deliberately: a timer thunk that re-arms itself
    /// (`(after 0 (lambda () (after 0 …)))`) would otherwise never leave the
    /// loop — each firing converts straight back into a due timer the same
    /// pass would immediately redrain. Outside the fixpoint, a re-arm is
    /// picked up on the *next* `settle()` instead, one frame later, which
    /// bounds it. `:reload-config`'s `resync_config_state` call site drains
    /// only `drain_pending_work` for exactly this reason — see its doc.
    pub(crate) fn settle(&mut self) {
        self.drain_async_sources();
        // Any mode change queued between the last consumption point and now
        // (a handler earlier this same batch, or something that ran before
        // `settle` was even called) must not survive into the handler calls
        // below. Unconditional, at the top, so no early-return branch below
        // can skip it.
        self.take_pending_lsp_completion_dismiss();
        if self.drain_pending_work() {
            // A call/handler just run above (an LSP-request callback, a
            // timer thunk, a hook) can itself dispatch a command that exits
            // Insert, setting the flag the top-of-function consumption
            // already passed. Consume it again so `prepare_frame`'s later
            // `sync_completion_menu_view` never repaints a session
            // `set_mode` asked to close mid-drain.
            self.take_pending_lsp_completion_dismiss();
            // The span `Editor::handle_input` opened ("this input's own
            // dispatch just logged a message") closes here, now that this
            // settle() has run the buffer-enter disk check that span exists
            // to protect against — see `EditorState::message_logged_this_input`'s
            // doc.
            self.state.message_logged_this_input = false;
        }
        // On abort, both consumes above are skipped: a mode change or
        // message mid-drain defers one frame to the next `settle()`'s
        // top-of-fn consume rather than being lost — see
        // `drain_pending_work`'s abort branch.
    }

    /// Fixpoint over `state.config.pending_work` only — no async sources.
    /// The loop body of `settle`, split out so `:reload-config`'s accounting
    /// window (`typed_reload_config`) can drain exactly the config's own
    /// queued hooks without an unrelated LSP/parse/timer message landing
    /// inside it and being mistaken for a reload failure.
    ///
    /// Also the single observation point for `OnBufferEnter` (SPEC.md §4):
    /// `detect_buffer_enter` runs at the top of every pass, not just once
    /// before the loop, so a handler-driven `switch-to-buffer!` is caught by
    /// the very next pass instead of waiting a frame, and the loop's exit
    /// condition is "queue empty **and** focus stable" rather than just
    /// "queue empty" — a pass that only detects a focus change still has
    /// work to do (queuing and then draining `OnBufferEnter`) even though
    /// `pending_work` was empty when the pass began.
    ///
    /// Capped at [`MAX_EVENT_DRAIN`] total items processed (not passes), so
    /// neither a constant-width ping-pong loop (two `on-language-set`
    /// handlers flipping a value between two settings) nor an amplifying
    /// cascade (a handler that queues more work than it received, doubling
    /// the batch pass over pass) can livelock the editor. The watchdog only
    /// bounds each individual eval, not this loop.
    ///
    /// Returns `false` if the cap aborted the drain with work still
    /// unprocessed, so callers can skip whatever follow-up assumes a clean
    /// quiescent state.
    pub(super) fn drain_pending_work(&mut self) -> bool {
        let mut total_processed = 0usize;
        loop {
            self.detect_buffer_enter();
            if self.state.config.pending_work.is_empty() {
                return true;
            }
            let batch = std::mem::take(&mut self.state.config.pending_work);
            total_processed += batch.len();
            if total_processed > MAX_EVENT_DRAIN {
                // `batch` was just drained from `pending_work` above, so
                // nothing has been re-enqueued yet — it's the entire drop.
                let dropped = batch.len();
                self.report(
                    Severity::Error,
                    format!(
                        "event/callback cascade exceeded {MAX_EVENT_DRAIN} total drained work \
                         item(s) — dropping {dropped} pending item(s); handler feedback loop?"
                    ),
                );
                self.state.message_logged_this_input = false;
                // `detect_buffer_enter` already advanced `last_entered_buffer`
                // to the buffer this dropped batch's `OnBufferEnter` (if any)
                // was raised for — undo that so the next `settle()` observes
                // the diff again and re-raises it, instead of the buffer-enter
                // disk check being lost for good because the baseline already
                // matches. Only when the batch actually held one: resetting
                // unconditionally would re-raise (and re-check) a buffer whose
                // event already fired earlier in this same drop.
                if batch
                    .iter()
                    .any(|w| matches!(w, PendingWork::Event(EditorEvent::OnBufferEnter { .. })))
                {
                    self.state.last_entered_buffer = None;
                }
                return false;
            }
            self.run_pending_batch(batch);
        }
    }

    /// Observation point for `focused_buffer_id()` — a derived join of
    /// `focused_pane_id` (5 write sites) and `pane.buffer_id` (1 write
    /// site), so it has no write-site chokepoint to hang a raise on
    /// (SPEC.md §4, `docs/LESSONS.md` L9). Diffed against
    /// `EditorState::last_entered_buffer` every pass of `settle`'s loop
    /// rather than once before it, so a pane-focus move and a buffer switch
    /// in the same pass coalesce into one event, and a handler that itself
    /// switches buffers is caught by the very next pass.
    fn detect_buffer_enter(&mut self) {
        let now = self.focused_buffer_id();
        if self.state.last_entered_buffer != Some(now) {
            self.state.last_entered_buffer = Some(now);
            self.state
                .queue_event(EditorEvent::OnBufferEnter { buffer: now });
        }
    }

    /// Run one snapshot of `pending_work` in queued order: event handlers
    /// fire one event at a time, and a contiguous run of `Call`s batches
    /// into one Steel session before the next `Event` (or end of batch).
    fn run_pending_batch(&mut self, mut items: std::collections::VecDeque<PendingWork>) {
        while let Some(item) = items.pop_front() {
            match item {
                PendingWork::Event(event) => {
                    self.react_to_event(&event);
                    self.fire_one_event(event);
                }
                PendingWork::Call(proc, args) => {
                    let mut calls = vec![(proc, args)];
                    while matches!(items.front(), Some(PendingWork::Call(..))) {
                        let Some(PendingWork::Call(proc, args)) = items.pop_front() else {
                            unreachable!("front() just confirmed a Call variant")
                        };
                        calls.push((proc, args));
                    }
                    self.run_call_batch(calls);
                }
            }
        }
    }

    /// Editor-internal reactions to an event — the Rust counterpart to Steel
    /// handlers, and the only `match` over `EditorEvent` that drives editor
    /// behaviour (SPEC.md §4). Runs unconditionally, before `fire_one_event`
    /// and its `has_hook_handlers` early-exit: unlike a Steel handler, a
    /// Rust reaction has no registration to short-circuit on, and editor
    /// behaviour must not depend on whether a plugin happens to be
    /// installed.
    fn react_to_event(&mut self, event: &EditorEvent) {
        match event {
            EditorEvent::OnBufferEnter { buffer } => self.enter_buffer_disk_check(*buffer),
            EditorEvent::OnFocusGained => self.check_all_disk_state(DiskCheckTrigger::Ambient),
            _ => {}
        }
    }

    /// Fire every handler registered for one `EditorEvent`, if any are —
    /// the per-item body of `settle`'s `Event` arm.
    fn fire_one_event(&mut self, event: EditorEvent) {
        let name = event.name();
        // Activate lazy event plugins first so their register-hook! calls
        // land before the has_hook_handlers check below.
        self.activate_lazy_event_plugins(name);
        if self
            .scripting
            .as_ref()
            .is_none_or(|h| !h.has_hook_handlers(name))
        {
            return;
        }
        // Built only once a handler is confirmed registered — an event
        // nobody subscribes to never allocates a `SteelVal`.
        let args = event.steel_args();
        let pid = self.state.focused_pane_id;
        let bid = self.focused_buffer_id();
        let result = {
            let host_scr = self.scripting.as_mut().expect("checked above");
            let mut impl_host = EditorHostImpl::full(
                &mut self.state,
                &mut self.view,
                &mut self.lsp,
                &mut self.timer_wheel,
                &mut self.timer_payloads,
                self.terminal.as_ref(),
            );
            host_scr.fire_hook(name, &args, pid, bid, &mut impl_host)
        };
        self.flush_script_messages();
        self.apply_script_result(result, "hook error: ");
    }

    /// Run one contiguous run of queued `Call` items in a single Steel
    /// session — the per-batch body of `settle`'s `Call` arm. Preserves
    /// `run_steel_calls`' existing "one session, first error aborts the
    /// rest" semantics for calls that were queued back-to-back.
    fn run_call_batch(&mut self, calls: Vec<(SteelVal, Vec<SteelVal>)>) {
        let pid = self.state.focused_pane_id;
        let bid = self.focused_buffer_id();
        let Some(host_scr) = self.scripting.as_mut() else {
            return;
        };
        let result = {
            let mut impl_host = EditorHostImpl::full(
                &mut self.state,
                &mut self.view,
                &mut self.lsp,
                &mut self.timer_wheel,
                &mut self.timer_payloads,
                self.terminal.as_ref(),
            );
            host_scr.run_steel_calls(calls, pid, bid, &mut impl_host)
        };
        self.flush_script_messages();
        self.apply_script_result(result, "steel call error: ");
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
            self.apply_script_result(result, "init.scm: ");
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
        self.apply_script_result(result, &format!("runtime/{rel_path}: "));
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
/// and [`crate::editor::completion::ThemeCompleter`] use this list as the
/// single source of truth.
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
