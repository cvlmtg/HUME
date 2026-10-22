use std::path::PathBuf;

use hume_engine::pipeline::BufferId;

use hume_scripting::Effect;
use hume_scripting::host::KeymapTarget;

use super::error::CommandError;
use super::keymap::{LocalKeymaps, UnknownKeymap};
use super::reload::ReloadSnapshot;
use super::{Editor, Severity, host_impl::EditorHostImpl};
use crate::cli::ConfigSource;

/// The `init.scm` a session should evaluate, or why there is none:
/// `Editor::config_path`'s result. Carries the reason rather than collapsing
/// it to `Option<PathBuf>` so callers (`init_scripting`, `typed_reload_config`)
/// don't each have to re-derive it from `config_source` separately.
pub(in crate::editor) enum ConfigPath {
    /// The file to evaluate. `required` marks a `--config` override: the user
    /// asserted the file exists, so a missing one is an error (checked at
    /// `:reload-config` time in `init_scripting`), where a missing default
    /// `<config_dir>/init.scm` is normal and silently skipped.
    Resolved { path: PathBuf, required: bool },
    /// `--no-config` (`ConfigSource::Skip`): the user asked for no `init.scm`.
    Skipped,
    /// Default source with no resolvable config directory
    /// (`HOME`/`XDG_CONFIG_HOME`, `APPDATA` on Windows all unset).
    NoConfigDir,
}

impl Editor {
    /// Apply every effect a Steel eval queued, in the exact order the
    /// script emitted them (`hume_scripting::Effect`): one ordered log, not
    /// separate channels with a hardcoded apply order. Consecutive
    /// `Effect::LanguageReg` entries are grouped into one
    /// `apply_pending_language_regs` call so a large run (e.g.
    /// `languages.scm`'s ~700 `define-language!` calls) rebuilds the glob
    /// matcher once, not once per entry. Consecutive `Effect::LspServerOp`
    /// entries are likewise one `apply_lsp_server_ops` call, so buffers
    /// reconcile to a run of registry changes once. Every other effect kind
    /// applies one at a time.
    ///
    /// Finishes by draining `state.config.pending_language_detection`, which covers every
    /// buffer a disjoint-borrow Steel path opened via `buffer::lifecycle::
    /// open_buffer_and_notify` this eval (`(open-buffer! …)`, workspace edits,
    /// goto-definition). `drain_pending_work` would reach them on its next
    /// pass; draining here lets the next item of the same batch see their
    /// language. *After* the effect loop, not before: a script that opens a
    /// buffer and registers its language in the same eval must have the
    /// registration land first.
    pub(in crate::editor) fn apply_script_effects(&mut self, effects: Vec<Effect>) {
        let mut effects: std::collections::VecDeque<Effect> = effects.into();
        while let Some(effect) = effects.pop_front() {
            match effect {
                Effect::LanguageReg(reg) => {
                    let batch = take_run(&mut effects, reg, |effect| match effect {
                        Effect::LanguageReg(next) => Ok(next),
                        other => Err(Box::new(other)),
                    });
                    self.apply_pending_language_regs(batch);
                }
                Effect::LspServerOp(op) => {
                    let batch = take_run(&mut effects, op, |effect| match effect {
                        Effect::LspServerOp(next) => Ok(next),
                        other => Err(Box::new(other)),
                    });
                    self.apply_lsp_server_ops(batch);
                }
                Effect::SetBufferLanguage { buffer, language } => {
                    // `buffer` may have closed between the Steel call that
                    // queued this effect and this drain (e.g. `(set-buffer-
                    // option! bid "language" "rust") (close-buffer! bid)` in
                    // one eval), the same shape as `LspRequest`/`LspNotify`'s own
                    // `try_get` guard in `bridge.rs`. `set_buffer_language_
                    // explicit` uses the panicking `get_mut`, so this must
                    // check first rather than let it panic.
                    if !self.effect_buffer_gone("set-buffer-option!", buffer) {
                        self.set_buffer_language_by_name(buffer, language.as_deref())
                    }
                }
                Effect::GrammarSweep(name) => {
                    let id = self.state.config.languages.id_of(&name).expect(
                        "GrammarSweep is only emitted right after attach_grammar interns the name",
                    );
                    self.sweep_buffers_for_grammars(vec![id])
                }
                Effect::LspRequest(req) => self.state.lsp_send_request(&self.view, req),
                Effect::LspNotify(notify) => self.state.lsp_send_notify(notify),
                Effect::BindKey {
                    mode,
                    keys,
                    cmd,
                    force_extend,
                    keymap,
                } => {
                    let mode = to_editor_bind_mode(mode);
                    let cmd = std::borrow::Cow::Owned(cmd);
                    match keymap {
                        KeymapTarget::Global => self.state.config.keymap.bind_user_with_extend(
                            mode,
                            &keys,
                            cmd,
                            force_extend,
                        ),
                        // Bound after init too, so checked here rather than by
                        // the post-init lint, which covers the global keymap.
                        KeymapTarget::Named(name) => {
                            if self.state.config.registry.get_mappable(&cmd).is_none() {
                                self.report_unbound_key_command(&cmd);
                            }
                            self.state.config.local_keymaps.bind(
                                name,
                                mode,
                                &keys,
                                cmd,
                                force_extend,
                            );
                        }
                    }
                }
                Effect::UnbindKey { mode, keys, keymap } => {
                    let mode = to_editor_bind_mode(mode);
                    match keymap {
                        KeymapTarget::Global => self.state.config.keymap.unbind_user(mode, &keys),
                        KeymapTarget::Named(name) => {
                            self.state.config.local_keymaps.unbind(&name, mode, &keys)
                        }
                    }
                }
                Effect::AttachKeymap { buffer, name } => {
                    self.change_attachment("attach-keymap!", buffer, &name, LocalKeymaps::attach)
                }
                Effect::DetachKeymap { buffer, name } => {
                    self.change_attachment("detach-keymap!", buffer, &name, LocalKeymaps::detach)
                }
                Effect::RegisterCompletionSource(reg) => self.register_completion_source(reg),
                Effect::SetTriggers {
                    kind,
                    source,
                    scope,
                    chars,
                } => {
                    // Checked at apply time, not by the builtin that queued
                    // this effect: an earlier effect in the same batch may
                    // have just registered `source`, so whether it exists
                    // can't be known any sooner than this.
                    let applied = self.state.set_triggers(kind, source, scope, chars);
                    if let Err(e) = applied {
                        self.report(Severity::Error, e);
                    }
                }
            }
        }
        self.detect_pending_languages();
    }

    /// Attach or detach the keymap `name` in `buffer`. A name nothing is bound
    /// into warns, since the call then does nothing.
    fn change_attachment(
        &mut self,
        builtin: &str,
        buffer: BufferId,
        name: &str,
        change: fn(&mut LocalKeymaps, BufferId, &str) -> Result<(), UnknownKeymap>,
    ) {
        if !self.effect_buffer_gone(builtin, buffer)
            && change(&mut self.state.config.local_keymaps, buffer, name).is_err()
        {
            self.report(
                Severity::Warning,
                format!("{builtin}: no keymap named '{name}'"),
            );
        }
    }

    /// Whether `buffer`, which an effect from `builtin` targets, closed after
    /// the effect was queued. Traced when it did.
    fn effect_buffer_gone(&mut self, builtin: &str, buffer: BufferId) -> bool {
        let gone = self.state.buffers.try_get(buffer).is_none();
        if gone {
            self.report(
                Severity::Trace,
                format!("{builtin}: buffer {buffer:?} no longer exists"),
            );
        }
        gone
    }

    /// End a Steel session that returned `Result<Vec<Effect>, EvalError>`:
    /// close the `#:inline-output` bracket if a builtin opened it, report the
    /// session's queued `log!` messages, and apply its effects either way (an
    /// `EvalError` still carries whatever committed before the error).
    /// Returns the error's message unreported.
    ///
    /// Every such Steel entry point ends here: a command body, a hook, a
    /// queued-call batch, `init.scm`, a `runtime/` file, and a plugin
    /// activation. The close comes first so a Steel error raised after screen
    /// entry still gets the TUI restored. The flush comes before any error
    /// report, so a body's log lines land ahead of its own error. `init.scm`
    /// and `runtime/` files run before `self.scripting` is installed, so they
    /// come through [`Self::apply_init_result`], which reports their messages
    /// first.
    pub(in crate::editor) fn finish_script_session(
        &mut self,
        result: Result<Vec<Effect>, hume_scripting::EvalError>,
    ) -> Result<(), String> {
        self.close_inline_output_bracket();
        self.flush_script_messages();
        match result {
            Ok(effects) => {
                self.apply_script_effects(effects);
                Ok(())
            }
            Err(e) => {
                self.apply_script_effects(e.effects);
                Err(e.message)
            }
        }
    }

    /// [`Self::finish_script_session`], reporting an error as
    /// `{err_prefix}{message}`. `err_prefix` distinguishes which eval failed
    /// (a hook, a queued call, `init.scm`, a `runtime/` file) in the reported
    /// message; pass `""` for a caller with nothing to prefix.
    pub(in crate::editor) fn apply_script_result(
        &mut self,
        result: Result<Vec<Effect>, hume_scripting::EvalError>,
        err_prefix: &str,
    ) {
        if let Err(message) = self.finish_script_session(result) {
            self.report(Severity::Error, format!("{err_prefix}{message}"));
        }
    }

    // ── Message reporting ─────────────────────────────────────────────────────

    /// Report a message, routing it based on severity:
    ///
    /// - `Info`    → set `status_msg` only (ephemeral, not logged)
    /// - `Warning` → push to `message_log` AND set `status_msg`
    /// - `Error`   → push to `message_log` AND set `status_msg`
    /// - `Trace`   → push to `message_log` only (not shown in statusline)
    pub(in crate::editor) fn report(&mut self, severity: Severity, text: String) {
        self.state.report(severity, text);
    }

    /// The Steel host with an [`EditorHostImpl`] over the rest of the editor,
    /// built together because the two borrow disjoint fields of `self`; `None`
    /// when scripting is not initialized.
    pub(in crate::editor) fn scripting_with_host(
        &mut self,
    ) -> Option<(&mut hume_scripting::ScriptingHost, EditorHostImpl<'_>)> {
        let scripting = self.scripting.as_mut()?;
        let host = EditorHostImpl::full(
            &mut self.state,
            &mut self.view,
            &mut self.timers,
            self.tui.clone(),
            self.kitty_enabled,
        );
        Some((scripting, host))
    }

    /// Report `e` at its own severity.
    pub(in crate::editor) fn report_error(&mut self, e: &CommandError) {
        self.state.report_error(e);
    }

    /// Drain any pending `(log! …)` messages from the scripting host and
    /// report each one. No-op if scripting isn't initialized.
    pub(in crate::editor) fn flush_script_messages(&mut self) {
        let msgs = self
            .scripting
            .as_mut()
            .map(hume_scripting::ScriptingHost::take_pending_messages)
            .unwrap_or_default();
        self.report_script_messages(msgs);
    }

    /// Report each drained `(log! …)` message at its own severity.
    fn report_script_messages(&mut self, msgs: Vec<(hume_scripting::LogLevel, String)>) {
        for (level, text) in msgs {
            self.report(log_level_to_severity(level), text);
        }
    }

    /// [`Self::apply_script_result`] for an init-mode eval of `host`, which
    /// is not yet installed in `self.scripting`, so the funnel's own flush
    /// cannot reach it. Its queued messages are reported first, so an eval's
    /// log lines land ahead of its own error.
    fn apply_init_result(
        &mut self,
        host: &mut hume_scripting::ScriptingHost,
        result: Result<Vec<Effect>, hume_scripting::EvalError>,
        err_prefix: &str,
    ) {
        self.report_script_messages(host.take_pending_messages());
        self.apply_script_result(result, err_prefix);
    }

    // ── Scripting ─────────────────────────────────────────────────────────────

    /// Initialise the Steel scripting host, then evaluate `init.scm` unless
    /// this session has no `init.scm` to run.
    ///
    /// Called once at startup, after `Editor::open` returns and before
    /// `Editor::run` starts (with `snapshot` at its `Default`: nothing to
    /// restore, no pre-reload buffers), and by `typed_reload_config` on
    /// every `:reload-config` (with the `ReloadSnapshot` `reset_config_state`
    /// just produced). Any error from `init.scm` is reported as
    /// `Severity::Error` and shown in the statusline.
    pub(in crate::editor) fn init_scripting(&mut self, snapshot: &mut ReloadSnapshot) {
        let mut host = hume_scripting::ScriptingHost::new(&self.state.dirs);
        // Pre-register every native command name as a callable Steel binding before
        // any user code sees the engine.  This lets `init.scm` call `(move-left)`
        // directly without a FreeIdentifier compile error.  Only native commands
        // (Motion/Selection/Edit/EditorCmd) are registered; plugin commands
        // (`SteelBacked`/`Lazy`) don't exist yet and use `(call! …)` instead.
        {
            let names: Vec<&str> = self.state.config.registry.native_mappable_names().collect();
            host.register_command_names(&names);
        }
        // Trace the resolved directories so they're visible in `:messages`.
        // A missing runtime dir is a warning because `core:*` plugins need it.
        match host.runtime_dir() {
            Some(rt) => self.report(
                Severity::Trace,
                format!("scripting: runtime dir = {}", rt.display()),
            ),
            None => self.report(
                Severity::Warning,
                "scripting: no runtime directory found; set HUME_RUNTIME to fix".into(),
            ),
        }
        match host.data_dir() {
            Some(d) => self.report(
                Severity::Trace,
                format!("scripting: data dir = {}", d.display()),
            ),
            None => self.report(
                Severity::Warning,
                "scripting: no data directory: HOME/APPDATA unset; user plugins unavailable".into(),
            ),
        }
        // Capture built-in names before any plugin code runs; stable for the
        // editor's lifetime.  Stored on Editor so dispatch-time activation can
        // borrow it disjointly from &mut self.scripting / settings / keymap.
        let builtin_names: hume_scripting::command_name::CommandNameSet =
            self.state.config.registry.names().collect();
        self.builtin_cmd_names = builtin_names.clone();
        // Load runtime/scheme/prelude.scm before init.scm so its macros
        // (bind-keys! etc.) are available to init.scm and plugin modules,
        // and shells.scm so init.scm can add or replace a shell dialect.
        // Then languages.scm ((define-language! …) identities) and
        // grammars.scm (registers already-compiled grammars; see its own
        // header). languages.scm runs first so identity registration owns
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
        self.eval_runtime_scheme(&mut host, "scheme/shells.scm", builtin_names.clone());
        self.eval_runtime_scheme(&mut host, "scheme/languages.scm", builtin_names.clone());
        self.eval_runtime_scheme(&mut host, "scheme/grammars.scm", builtin_names.clone());
        // `Skipped` (`--no-config`) and `NoConfigDir` both mean there's no
        // `init.scm` to evaluate, so every plugin it would otherwise
        // `load-plugin!` is skipped for free. The runtime scheme above
        // always loads regardless, it's HUME's own, not the user's. `Skipped`
        // is what the user asked for (no warning); `NoConfigDir` means
        // there's no meaningful place to look (warned).
        match self.config_path() {
            ConfigPath::Skipped => {}
            ConfigPath::NoConfigDir => {
                self.report(
                    Severity::Warning,
                    "scripting: no config directory: HOME/APPDATA unset; init.scm skipped".into(),
                );
            }
            ConfigPath::Resolved { path, required } => {
                self.report(
                    Severity::Trace,
                    format!("scripting: config file = {}", path.display()),
                );
                let init_budget = self.state.settings.steel_init_budget_ms as u64;
                // A missing default `init.scm` is normal (`eval_init` treats
                // `NotFound` as a silent no-op), but a `--config` override
                // (`required`) is an assertion (see `resolve` in `main.rs`),
                // and that assertion is only checked once, at process start.
                // A path valid at startup can go missing by the time
                // `:reload-config` re-evaluates it (moved, deleted, or,
                // before the startup-time absolutize in `main.rs`, a
                // relative path outrun by an intervening `:cd`), so re-check
                // it here rather than let it fall through `eval_init`'s
                // silent-skip path and read as a successful, empty reload.
                let result = if required && !path.is_file() {
                    Err(format!("--config: not found: {}", path.display()).into())
                } else {
                    let mut ih = EditorHostImpl::init(
                        &mut self.state,
                        &mut self.view,
                        self.tui.clone(),
                        self.kitty_enabled,
                    );
                    host.eval_init(&path, init_budget, &mut ih, builtin_names)
                };
                // Named by the path actually evaluated ("init.scm: " for the
                // default location, the override's own file name under
                // `--config`), so an eval error names the file the user
                // actually pointed HUME at, not always the default. Falls
                // back to the full path for the pathological case of a path
                // with no file name component (e.g. one ending in `..`).
                let err_prefix = match path.file_name() {
                    Some(name) => format!("{}: ", name.to_string_lossy()),
                    None => format!("{}: ", path.display()),
                };
                self.apply_init_result(&mut host, result, &err_prefix);
            }
        }
        crate::editor::settings::ops::apply_pending_theme(&mut self.state, &mut self.view);
        // Snapshot language activation entries for the post-init lint below:
        // every eval's effects (identities, grammars, LSP server ops) are
        // already applied above, each right after its own eval, so
        // `self.state.config.languages` is fully populated by this point.
        let lang_activations = host.activation_languages();
        self.scripting = Some(host);
        // Every language is registered by now, so the startup buffer queued
        // by `Editor::open` announces `on-language-set` before
        // `on-buffer-open`, like any other open.
        self.detect_pending_languages();
        // Post-init lint: warn on keymap leaves that target a name a key
        // binding can't actually reach. Lazy stubs are registered live as
        // each declare-plugin! call runs during init.scm eval, so they
        // already count as valid mappable commands by this point.
        // Built-in keymaps only reference registered built-ins, so any
        // warnings here come from user bind-key! calls to typos, undeclared
        // commands, or a typed-only command's name (`:`-only, never
        // key-bindable; see `registry/mod.rs`'s module doc).
        {
            let config = &self.state.config;
            for name in config.keymap.unknown_command_names(&config.registry) {
                self.report_unbound_key_command(&name);
            }
        }
        // Post-init lint: warn on #:languages activation entries whose language name
        // is not in the final LanguageRegistry.  Runs after the message flush above
        // so both languages.scm and init.scm's own define-language! calls are visible.
        // Uses defined_id() (identity registered), not has_grammar() (grammar attached):
        // an activation entry is valid for any known language identity even without a
        // grammar.  Not a hard error: inert but harmless, and language sets are
        // open/dynamic (a future define-language! + reload may make the name valid).
        for (lang, plugins) in &lang_activations {
            // "*" is the any-language wildcard (manifest.scm can't enumerate every
            // language it might ever support), not a language identity to look up.
            if lang != "*" && self.state.config.languages.defined_id(lang).is_none() {
                for plugin in plugins {
                    self.report(
                        Severity::Warning,
                        format!(
                            "plugin '{plugin}' declares #:languages activation for unknown \
                             language '{lang}': typo, or missing (define-language!)?"
                        ),
                    );
                }
            }
        }
        // Re-detect language for every open buffer (a `:reload-config`
        // cleared every buffer's language in `reset_config_state`; the
        // startup buffer, detected by the drain above, detects the same
        // language again, which changes nothing).
        // A buffer with an explicit assertion, restored from `snapshot`
        // (this is a `:reload-config`) or made by a lazy plugin that the
        // drain above or an earlier iteration activated, skips detection
        // entirely: `set_buffer_language_impl` stamps `language_explicit`
        // unconditionally, so a later detection pass would otherwise
        // overwrite that assertion with whatever plain detection finds. Restoring the
        // snapshot *inside* this loop, rather than as a second
        // detect-then-correct pass afterward, also avoids firing language
        // hooks for the buffer's *detected* language before its real,
        // explicit one goes back.
        let explicit_restore: rustc_hash::FxHashMap<BufferId, Option<String>> =
            snapshot.take_explicit_languages().into_iter().collect();
        let open_bids: Vec<_> = self.state.buffers.iter().map(|(id, _)| id).collect();
        for bid in open_bids {
            // A lazy language plugin activated earlier in this same loop
            // (`detect_and_set_language` → `set_buffer_language_impl` →
            // `activate_lazy_language_plugins`) runs at `EvalMode::
            // PluginActivation`, where `close-buffer!` is callable, so it can
            // close a *later* bid in this same `open_bids` list before this
            // loop ever reaches it. Same hazard `detect_pending_languages`
            // guards against with `try_get`; skip rather than hit
            // `BufferStore::get`'s "unseeded BufferId" panic below.
            if self.state.buffers.try_get(bid).is_none() {
                continue;
            }
            if let Some(name) = explicit_restore.get(&bid) {
                // Redundant with the `try_get` skip above in production (no
                // Steel eval runs between the two), but `survives` also
                // checks `bid` predates this reload; see its own doc.
                if snapshot.survives(bid, &self.state.buffers) {
                    match name {
                        Some(name) => match self.state.config.languages.id_of(name) {
                            Some(lang_id) => self.set_buffer_language_explicit(bid, Some(lang_id)),
                            None => {
                                self.report(
                                    Severity::Warning,
                                    format!(
                                        "language '{name}' was explicitly set on a buffer \
                                         before reload, but is no longer registered; \
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
                // Asserted by a lazy plugin activated since the host was
                // attached (`init.scm` itself cannot set a buffer option), so
                // detection must not clobber it either; see
                // `set_buffer_language_impl`'s doc.
                continue;
            }
            self.detect_and_set_language(bid);
        }
    }

    /// The `init.scm` this session evaluates, at startup and on every
    /// `:reload-config`, which must re-run the file the session booted from
    /// rather than falling back to the default one.
    pub(in crate::editor) fn config_path(&self) -> ConfigPath {
        match &self.config_source {
            ConfigSource::File(path) => ConfigPath::Resolved {
                path: path.clone(),
                required: true,
            },
            ConfigSource::Skip => ConfigPath::Skipped,
            ConfigSource::Default => match &self.state.dirs.config {
                Some(dir) => ConfigPath::Resolved {
                    path: dir.join("init.scm"),
                    required: false,
                },
                None => ConfigPath::NoConfigDir,
            },
        }
    }

    /// Evaluate a bundled runtime Scheme file (`rel_path`, relative to the
    /// runtime dir) as an init-mode eval. A missing runtime dir or a missing
    /// file at that path is a silent no-op (every bundled layer is optional);
    /// a file that exists but fails to parse/eval is reported as an error.
    fn eval_runtime_scheme(
        &mut self,
        host: &mut hume_scripting::ScriptingHost,
        rel_path: &str,
        builtin_names: hume_scripting::command_name::CommandNameSet,
    ) {
        let Some(path) = host.runtime_dir().map(|rt| rt.join(rel_path)) else {
            return;
        };
        let init_budget = self.state.settings.steel_init_budget_ms as u64;
        let result = {
            let mut ih = EditorHostImpl::init(
                &mut self.state,
                &mut self.view,
                self.tui.clone(),
                self.kitty_enabled,
            );
            host.eval_runtime(&path, init_budget, &mut ih, builtin_names)
        };
        self.apply_init_result(host, result, &format!("runtime/{rel_path}: "));
    }
}

// ---------------------------------------------------------------------------
// Module-level helpers
// ---------------------------------------------------------------------------

/// Map the scripting layer's `BindMode` (carried in `Effect::BindKey` and
/// friends) to the editor's own. Fully qualified on both sides: this module
/// works with `crate::editor::keymap::BindMode` too, so importing either name
/// bare would shadow the other.
fn to_editor_bind_mode(mode: hume_scripting::host::BindMode) -> crate::editor::keymap::BindMode {
    match mode {
        hume_scripting::host::BindMode::Normal => crate::editor::keymap::BindMode::Normal,
        hume_scripting::host::BindMode::Extend => crate::editor::keymap::BindMode::Extend,
        hume_scripting::host::BindMode::Insert => crate::editor::keymap::BindMode::Insert,
        hume_scripting::host::BindMode::Command => crate::editor::keymap::BindMode::Command,
    }
}

/// Map scripting `LogLevel` → editor `Severity`.
pub(in crate::editor::scripting_setup) fn log_level_to_severity(
    level: hume_scripting::LogLevel,
) -> Severity {
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
/// Config themes (hand-authored, edited in place like `init.scm`) come first,
/// then data-dir themes (installed by a tool, same provenance as PLUM's
/// `data/plugins/`), then runtime themes (bundled) last. Each tier shadows
/// the next by name. Both `theme::load_theme_by_name` and
/// `completion::complete_theme` use this list as the single source of
/// truth.
pub(super) fn theme_search_paths(dirs: &hume_platform::dirs::Dirs) -> Vec<PathBuf> {
    [&dirs.config, &dirs.data, &dirs.runtime]
        .into_iter()
        .flatten()
        .map(|dir| dir.join("themes"))
        .collect()
}

/// `first` and the effects right behind it that `take` unwraps, in order;
/// the first effect `take` refuses stays at the front of `effects`.
fn take_run<T>(
    effects: &mut std::collections::VecDeque<Effect>,
    first: T,
    take: fn(Effect) -> Result<T, Box<Effect>>,
) -> Vec<T> {
    let mut run = vec![first];
    while let Some(effect) = effects.pop_front() {
        match take(effect) {
            Ok(next) => run.push(next),
            Err(other) => {
                effects.push_front(*other);
                break;
            }
        }
    }
    run
}
