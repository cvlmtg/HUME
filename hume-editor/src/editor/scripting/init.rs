//! Starting the scripting host and evaluating the runtime files and
//! `init.scm`.

use std::path::PathBuf;

use hume_core::{BufferId, Severity};
use hume_scripting::Effect;

use crate::cli::ConfigSource;
use crate::editor::reload::{ExplicitLanguage, ReloadSnapshot};
use crate::editor::{Editor, host_impl::EditorHostImpl};

use super::ConfigPath;

impl Editor {
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
        // header).
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
                // (`required`) is an assertion (see `resolve` in
                // `bin/hume/args.rs`), and that assertion is only checked
                // once, at process start. A path valid at startup can go
                // missing by the time `:reload-config` re-evaluates it
                // (moved, deleted, or, before `resolve` absolutized it, a
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
        // Uses defined_id() (identity or grammar registered), not has_grammar() (grammar
        // attached): an activation entry is valid for any defined language even without a
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
        // drain above or an earlier iteration activated, keeps it:
        // detection leaves an asserted language as it is. Restoring the
        // snapshot *inside* this loop, rather than as a second
        // detect-then-correct pass afterward, avoids firing language
        // hooks for the buffer's *detected* language before its real,
        // explicit one goes back.
        let explicit_restore: rustc_hash::FxHashMap<BufferId, ExplicitLanguage> =
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
            if let Some(ExplicitLanguage { name, by }) = explicit_restore.get(&bid) {
                // Redundant with the `try_get` skip above in production (no
                // Steel eval runs between the two), but `survives` also
                // checks `bid` predates this reload; see its own doc.
                if snapshot.survives(bid, &self.state.buffers) {
                    match name {
                        Some(name) => match self.state.config.languages.id_of(name) {
                            Some(lang_id) => {
                                self.set_buffer_language_explicit(bid, Some(lang_id), *by)
                            }
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
                        None => self.set_buffer_language_explicit(bid, None, *by),
                    }
                    continue;
                }
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

/// Ordered list of directories to search for theme TOML files.
///
/// Config themes (hand-authored, edited in place like `init.scm`) come first,
/// then data-dir themes (installed by a tool, same provenance as PLUM's
/// `data/plugins/`), then runtime themes (bundled) last. Each tier shadows
/// the next by name. Both `theme::load_theme_by_name` and
/// `completion::complete_theme` use this list as the single source of
/// truth.
pub(in crate::editor) fn theme_search_paths(dirs: &hume_platform::dirs::Dirs) -> Vec<PathBuf> {
    [&dirs.config, &dirs.data, &dirs.runtime]
        .into_iter()
        .flatten()
        .map(|dir| dir.join("themes"))
        .collect()
}
