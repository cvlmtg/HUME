//! Steel scripting integration for HUME.
//!
//! [`ScriptingHost`] owns the Steel [`Engine`] and runs entirely on the main
//! event-loop thread — Steel's Engine is `!Send` (internal `Rc`/`RefCell`,
//! non-atomic `im-rs` lists), and edit commands are synchronous hot-key-path
//! operations where an IPC round-trip per keystroke would be strictly worse.
//!
//! ## Plugin loading pipeline
//! - `(load-plugin name)` — **inline**, init.scm/`:reload-config` only.
//!   Resolves, records, and activates the plugin immediately via
//!   `%activate-plugin-inline` (body run through `hm.eval-string` inside the
//!   live VM, no `&mut Engine` needed). Self-declares.
//! - `(declare-plugin name #:commands #:events #:languages #:config)` —
//!   **lazy manifest**: records a `Declared` state + activation maps in
//!   `LazyRegistry` without running the body. Requires at least one
//!   activation trigger.
//! - `#:config` (either form) is opaque data stored per-`PluginId`; the
//!   plugin reads it back via `(plugin-config)`, resolved from the top of
//!   `plugin_stack` for the duration of the eval.
//! - Activation entries are one-shot: the first exercised entry runs the body
//!   via `(require)`, flips state to `Loaded`, and drops that plugin's
//!   entries from all activation maps. The body then typically registers
//!   `register-hook!` callbacks for ongoing events — distinct from
//!   activation entries.
//! - States: `Declared → Loading → Loaded | Failed`. `Loading` guards
//!   re-entrant cycles; `Failed` doesn't retry until `:reload-config`.
//! - PLUM (`core:plum`) reads `(declared-plugins)` to install third-party
//!   deps; both forms record their name in `declared_plugins` up front, even
//!   before the body runs.
//!
//! ## Modules
//! - `attribution.rs`: plugin attribution types (`PluginId`, `Owner`, `PluginStack`).
//! - `hooks.rs`: `HookRegistry` + typed `HookId` enum.
//! - `host.rs`: `EditorHost` trait + `BindMode` — the editor-domain interface.
//! - `log.rs`: `LogLevel` — severity enum that doesn't depend on the editor crate.
//! - `builtins/`: `set-option!`, `bind-key!`, `define-command!`, multi-buffer ops,
//!   `(configure-statusline! …)`, `(hume/yield!)` step-budget interruption.

// ── Public submodules ─────────────────────────────────────────────────────────
pub mod attribution;
pub(crate) mod builtins;
pub mod hooks;
pub mod host;
pub mod json;
pub(crate) mod keys;
pub(crate) mod lazy;
pub mod log;
// ── Private implementation details ────────────────────────────────────────────
mod activation;
mod context;
#[cfg(test)]
mod null_host;
#[cfg(test)]
mod test_support;
mod types;
pub(crate) mod watchdog;

// ── Public API re-exports ─────────────────────────────────────────────────────
// Types the editor and editor tests use directly.
pub use attribution::PluginId;
pub use builtins::commands::parse_count_extend;
pub use builtins::ids::SteelBufferId;
pub use hooks::HookId;
pub use host::{
    BindMode, BufferHost, CommandHost, CompletionHost, CursorHost, DecorationHost, EditHost,
    EditorHost, LanguageHost, LspHost, OutputHost, SettingsHost, TimerHost, UiHost, unsupported,
};
pub use keys::parse_key_stream;
pub use log::LogLevel;
pub use types::{
    Effect, EvalError, LspServerStatusEntry, PendingLanguageReg, PendingLspNotify,
    PendingLspRequest, PendingLspServerOp, PendingLspServerReg, SteelCmdDef, SteelCmdResult,
};
pub use watchdog::EvalWatchdog;

// ── Internal re-exports (within-crate use) ────────────────────────────────────
pub(crate) use activation::run_steel_session;
pub(crate) use context::SteelCtx;

/// Steel global name under which the live [`SteelCtx`] reference is visible to
/// builtins during an eval (injected by `run_steel_session`, reset to `#void`
/// after).
pub(crate) const HUME_CTX: &str = "*hume.ctx*";

// ── Internal imports ──────────────────────────────────────────────────────────

use std::path::Path;
use std::sync::{Arc, atomic::AtomicBool};

use steel::rvals::SteelVal;
use steel::steel_vm::engine::Engine;

use attribution::PluginStack;
use hooks::HookRegistry;
use lazy::{LazyRegistry, PluginState};

// ── ScriptingRegistries ───────────────────────────────────────────────────────

/// The persistent registry fields bundled as a unit so they can be
/// borrowed as a single `&mut ScriptingRegistries` — disjoint from the
/// Steel VM (`steel`) and the rest of `ScriptingHost`.
pub(crate) struct ScriptingRegistries {
    /// Command-to-owner index: maps each Steel-registered command name to a
    /// display string (`"hume"`, `"user"`, or a plugin id like `"core:plum"`).
    pub(crate) cmd_owners: std::collections::HashMap<String, String>,
    /// Persistent hook registry: handlers registered by `(register-hook! …)`.
    pub(crate) hooks: HookRegistry,
    /// Lazy plugin registry: populated by `%declare-plugin!` during init;
    /// activation maps consulted by command dispatch, event firing, and language-set.
    pub(crate) lazy_registry: LazyRegistry,
    /// Every plugin name passed to `(load-plugin …)` or `(declare-plugin …)`,
    /// including plugins absent on disk.
    pub(crate) declared_plugins: Vec<String>,
    /// In-Steel dispatch table: maps activated plugin command name to its Steel
    /// closure for synchronous inline application by `%dispatch-command`.
    ///
    /// Populated by `define_command_inner` inline during init or plugin activation.
    /// Consulted by `%lookup-plugin-proc` in both init and command mode.
    pub(crate) command_table: std::collections::HashMap<String, SteelVal>,
    /// Per-plugin config value passed via `#:config` on `(load-plugin …)` /
    /// `(declare-plugin …)`. Read back by the plugin body through `(plugin-config)`,
    /// resolved via the top of `plugin_stack` — works identically whether the
    /// plugin activates immediately (eager) or much later (lazy).
    pub(crate) plugin_configs: std::collections::HashMap<PluginId, SteelVal>,
    /// Handlers registered by `(on-lsp-notification method handler)`, keyed
    /// by protocol method name. Consulted by the editor's notification
    /// dispatch for any method Rust doesn't already special-case
    /// (window/logMessage, window/showMessage, $/progress, publishDiagnostics).
    pub(crate) lsp_notification_handlers: std::collections::HashMap<String, Vec<SteelVal>>,
}

// ── HostBundle ────────────────────────────────────────────────────────────────

/// Borrows of [`ScriptingHost`] fields needed to populate [`SteelCtx`].
///
/// Built by [`ScriptingHost::steel_and_bundle`] and passed to
/// [`SteelCtx::new_init`] or [`SteelCtx::new_command`]. Private to this module.
pub(crate) struct HostBundle<'a> {
    pub(crate) registries: &'a mut ScriptingRegistries,
    plugin_stack: &'a mut PluginStack,
    pending_messages: &'a mut Vec<(LogLevel, String)>,
    effects: &'a mut Vec<types::QueuedEffect>,
    dirs: &'a builtins::dirs::ScriptDirs,
    /// Owned `Arc` clone: `new_init`/`new_command` consume it via move into
    /// `SteelCtx::interrupt_flag`, avoiding a second clone at eval time.
    interrupt_flag: Arc<AtomicBool>,
}

// ── ScriptingHost ─────────────────────────────────────────────────────────────

/// The embedded Steel scripting host.
///
/// Owns the Steel `Engine` and all persistent scripting state.  Each eval or
/// command call constructs a `SteelCtx` that borrows the persistent fields
/// directly.
///
/// Constructed once during `Editor::init_scripting()` and held for the
/// lifetime of the process.
pub struct ScriptingHost {
    /// The Scheme VM — always called `steel` (never bare "engine", which refers to the `engine/` crate).
    steel: Engine,
    /// The persistent registries borrowed as a unit into `SteelCtx`, disjoint
    /// from `steel` so the VM and command/hook state can be borrowed
    /// simultaneously (NLL field-split).
    pub(crate) registries: ScriptingRegistries,
    /// Attribution stack: `stack.last()` is the plugin currently executing.
    /// Empty → top-level `init.scm` → `Owner::User`.
    plugin_stack: PluginStack,
    /// Log messages accumulated by `(log! …)` since the last drain.
    /// Drained by the editor via `take_pending_messages()`.
    pending_messages: Vec<(LogLevel, String)>,
    /// Side effects queued by Steel builtins, in push order. Each eval entry
    /// point (`call_steel_cmd`, `fire_hook`, `run_steel_calls`, `eval_init`,
    /// `activate_plugin_inline`) drains exactly what it pushed back out on
    /// success (`Vec<Effect>`). On error it drains only the entries committed
    /// by a nested successful plugin activation (`QueuedEffect::committed`) —
    /// see `take_eval_effects` and `types::EvalError`.
    effects: Vec<types::QueuedEffect>,
    /// Data/runtime directories (raw + display form) and the install-lock
    /// root, computed once at construction.
    dirs: builtins::dirs::ScriptDirs,
    /// Shared interrupt flag.  Set to `true` by the watchdog to signal that
    /// `(hume/yield!)` calls should abort the running script.  Reset to
    /// `false` after every eval — command dispatch, hook fires, and plugin
    /// activation, not just `eval_init`.
    interrupt_flag: Arc<AtomicBool>,
    /// Persistent budget-enforcement thread, re-armed around every eval.
    watchdog: EvalWatchdog,
}

impl ScriptingHost {
    /// Create a new scripting host with the Steel standard library and all HUME
    /// builtins loaded.
    ///
    /// Resolves base directories eagerly so builtins can use them without
    /// re-reading environment variables on every call.
    pub fn new() -> Self {
        let dirs = builtins::dirs::ScriptDirs::new(
            hume_platform::dirs::data_dir(),
            hume_platform::dirs::runtime_dir(),
        );
        let mut steel = Engine::new();
        builtins::register_all(&mut steel);
        Self {
            steel,
            registries: ScriptingRegistries {
                cmd_owners: std::collections::HashMap::new(),
                hooks: HookRegistry::default(),
                lazy_registry: LazyRegistry::default(),
                declared_plugins: Vec::new(),
                command_table: std::collections::HashMap::new(),
                plugin_configs: std::collections::HashMap::new(),
                lsp_notification_handlers: std::collections::HashMap::new(),
            },
            plugin_stack: PluginStack::default(),
            pending_messages: Vec::new(),
            effects: Vec::new(),
            dirs,
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            watchdog: EvalWatchdog::new(),
        }
    }
}

impl Default for ScriptingHost {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptingHost {
    /// Split `self` into the Steel engine, the watchdog, and the [`HostBundle`]
    /// of persistent borrows needed to build a [`SteelCtx`].
    ///
    /// The returned borrows are disjoint fields of `self` (NLL field-split), so
    /// the VM can be run while the bundle is lent to the eval context. Shared
    /// by `eval_source_raw`, `activate_plugin_inline`, `call_steel_cmd`, and
    /// `fire_hook`.
    pub(crate) fn steel_and_bundle(&mut self) -> (&mut Engine, &EvalWatchdog, HostBundle<'_>) {
        let Self {
            steel,
            registries,
            plugin_stack,
            pending_messages,
            effects,
            dirs,
            interrupt_flag,
            watchdog,
            ..
        } = self;
        (
            steel,
            watchdog,
            HostBundle {
                registries,
                plugin_stack,
                pending_messages,
                effects,
                dirs: &*dirs,
                interrupt_flag: Arc::clone(interrupt_flag),
            },
        )
    }
}

impl ScriptingHost {
    // ── Outward API (clean; no direct field access outside this module) ────────

    /// Pre-register native command names as callable Steel bindings.
    ///
    /// For each name, evaluates `(define name (lambda args (%dispatch-command
    /// "name" args)))` — makes bare `(move-left)` callable without
    /// `(call! "move-left")`, and variadic, so `(move-down 3)` / `(move-down 0)`
    /// work too (count `0` = "no count typed", see `parse_count_extend`).
    ///
    /// Calls `%dispatch-command` directly rather than the public `call!` macro
    /// (which desugars to exactly this) — the variadic lambda's args are
    /// already the list `%dispatch-command` expects, no intermediate
    /// `(list ...)` needed.
    ///
    /// Called from `Editor::init_scripting`, after `ScriptingHost::new` and
    /// before any `eval_init`, so init.scm/plugins can use bare command names
    /// without a `FreeIdentifier` compile error.
    pub fn register_command_names(&mut self, names: &[&str]) {
        if names.is_empty() {
            return;
        }
        // Build one compound source string: one define per command.
        let mut source = String::with_capacity(names.len() * 64);
        for &name in names {
            source.push_str("(define ");
            source.push_str(name);
            source.push_str(" (lambda args (%dispatch-command \"");
            source.push_str(name);
            source.push_str("\" args)))\n");
        }
        self.steel
            .compile_and_run_raw_program(source)
            .expect("command name pre-registration failed — this is a bug");
    }

    /// Runtime directory for core plugins, themes, and docs.
    pub fn runtime_dir(&self) -> Option<&Path> {
        self.dirs.runtime_dir.as_deref()
    }

    /// Data directory for user/third-party plugins.
    pub fn data_dir(&self) -> Option<&Path> {
        self.dirs.data_dir.as_deref()
    }

    /// Drain all accumulated log messages since the last drain.
    pub fn take_pending_messages(&mut self) -> Vec<(LogLevel, String)> {
        std::mem::take(&mut self.pending_messages)
    }

    /// Returns `true` if no handlers are registered for `hook_id`.
    pub fn has_hook_handlers(&self, hook_id: hooks::HookId) -> bool {
        !self.registries.hooks.is_empty_for(hook_id)
    }

    /// Handlers registered for `method`, or empty if none. Cloned (Steel
    /// closures are cheap `Gc` clones) so the editor can queue calls without
    /// holding a borrow into `self.registries` across the queueing.
    pub fn lsp_notification_handlers_for(&self, method: &str) -> Vec<SteelVal> {
        self.registries
            .lsp_notification_handlers
            .get(method)
            .cloned()
            .unwrap_or_default()
    }

    /// A snapshot of the language activation entries declared during init (language → plugins).
    ///
    /// Used by the post-init lint in `init_scripting` to detect `#:languages` entries
    /// that don't match any registered language identity.
    pub fn activation_languages(
        &self,
    ) -> std::collections::HashMap<String, Vec<attribution::PluginId>> {
        self.registries.lazy_registry.activation_languages.clone()
    }

    /// Plugin ids that should be activated when `hook_id` fires.
    pub fn activation_event_plugins(&self, hook_id: hooks::HookId) -> Vec<attribution::PluginId> {
        self.registries
            .lazy_registry
            .activation_events
            .get(&hook_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Plugin ids that should be activated when `language` is set on a buffer.
    ///
    /// Unions entries keyed by `language` with entries keyed by the wildcard
    /// `"*"` (a manifest that can't enumerate every language it might ever
    /// support) — deduped so a plugin listed under both activates once.
    pub fn activation_language_plugins(&self, language: &str) -> Vec<attribution::PluginId> {
        let activation_languages = &self.registries.lazy_registry.activation_languages;
        let specific = activation_languages.get(language);
        let wildcard = activation_languages.get("*");
        let mut plugins: Vec<attribution::PluginId> = specific.cloned().unwrap_or_default();
        if let Some(wildcard) = wildcard {
            for id in wildcard {
                if !plugins.contains(id) {
                    plugins.push(id.clone());
                }
            }
        }
        plugins
    }

    /// Status of a plugin in the lazy registry.
    pub fn plugin_status(&self, id: &attribution::PluginId) -> Option<PluginStatus> {
        self.registries
            .lazy_registry
            .plugins
            .get(id)
            .map(|state| match state {
                PluginState::Declared { .. } => PluginStatus::Declared,
                PluginState::Loading => PluginStatus::Loading,
                PluginState::Loaded => PluginStatus::Loaded,
                PluginState::Failed => PluginStatus::Failed,
            })
    }

    /// Returns `true` if any plugin in the registry has transitioned to `Loaded`.
    #[cfg(any(test, feature = "test-util"))]
    pub fn has_any_loaded_plugin(&self) -> bool {
        self.registries
            .lazy_registry
            .plugins
            .values()
            .any(|s| matches!(s, PluginState::Loaded))
    }

    /// All plugin names ever passed to `(load-plugin …)` or `(declare-plugin …)`.
    #[cfg(any(test, feature = "test-util"))]
    pub fn declared_plugins(&self) -> &[String] {
        &self.registries.declared_plugins
    }

    /// Format a human-readable plugin status table for `:plugin-status`.
    ///
    /// `lazy_cmds` is the editor's current `Lazy`-stub list (`name`, owning
    /// plugin) — this crate doesn't track pending command activations itself,
    /// so the caller supplies its live registry snapshot.
    pub fn lazy_status_string(&self, lazy_cmds: &[(String, attribution::PluginId)]) -> String {
        self.registries.lazy_registry.format_status(lazy_cmds)
    }

    /// Peek at pending messages without draining.  Only for test assertions.
    #[cfg(any(test, feature = "test-util"))]
    pub fn peek_pending_messages(&self) -> &[(LogLevel, String)] {
        &self.pending_messages
    }

    /// Peek at the effect log without draining.  Only for test assertions —
    /// production code always goes through an eval entry point's own
    /// atomic drain (`take_eval_effects`).
    #[cfg(any(test, feature = "test-util"))]
    pub fn effects_for_test(&self) -> Vec<&Effect> {
        self.effects.iter().map(|e| &e.effect).collect()
    }

    /// Override the data directory.  Used only in tests that need a predictable
    /// plugin install location. Rebuilds the whole `ScriptDirs` so the
    /// display form and install-lock root stay in sync with the override.
    #[cfg(any(test, feature = "test-util"))]
    pub fn set_data_dir(&mut self, dir: std::path::PathBuf) {
        self.dirs = builtins::dirs::ScriptDirs::new(Some(dir), self.dirs.runtime_dir.clone());
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn interrupt_flag_for_test(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        std::sync::Arc::clone(&self.interrupt_flag)
    }

    /// Current plugin activation nesting depth (number of bodies on the call
    /// stack).  Replaces the retired `activation_depth` field in tests that
    /// verify `%begin-lazy-activation` / `%finish-lazy-activation` side effects.
    #[cfg(any(test, feature = "test-util"))]
    pub fn plugin_stack_depth_for_test(&self) -> usize {
        self.plugin_stack.len()
    }

    /// Push a fake plugin id onto the attribution stack.  Used by tests that
    /// need to pre-seed the stack depth before calling `%begin-lazy-activation`.
    #[cfg(any(test, feature = "test-util"))]
    pub fn push_plugin_for_test(&mut self, id: attribution::PluginId) {
        self.plugin_stack.push(id);
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn cmd_owners_for_test(&self) -> &std::collections::HashMap<String, String> {
        &self.registries.cmd_owners
    }

    /// Read-only view of the in-Steel plugin dispatch table.
    ///
    /// Maps activated plugin command name → its Steel closure. Used in tests to
    /// assert inline `define-command!` populated the table correctly — which is the
    /// precondition for `%lookup-plugin-proc` returning the closure rather than `#f`.
    #[cfg(any(test, feature = "test-util"))]
    pub fn command_table_for_test(
        &self,
    ) -> &std::collections::HashMap<String, steel::rvals::SteelVal> {
        &self.registries.command_table
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn eval_source_returning_defs(
        &mut self,
        source: String,
        builtin_names: std::collections::HashSet<String>,
        host: &mut dyn EditorHost,
    ) -> Result<(), String> {
        self.eval_source_raw(source, builtin_names, 10_000, host)
            .map(|_| ())
            .map_err(|e| e.message)
    }

    // ── Eval machinery ────────────────────────────────────────────────────────

    /// Evaluate `init.scm` at `path`, giving builtins access to editor state
    /// (settings, keymap) via `host` for the duration of the call.
    ///
    /// - Returns `Ok(effects)` if the file does not exist (missing config is
    ///   normal — `effects` is empty) or if eval succeeds, with every effect
    ///   this file's eval queued, in emission order.  Commands defined during
    ///   eval are registered into the `CommandRegistry` inline via
    ///   `host.register_command`.
    /// - Returns `Err(EvalError)` if the file exists but fails to parse or
    ///   evaluate; the failed eval's own queued effects are discarded, except
    ///   any committed by a nested successful plugin activation, which are
    ///   salvaged onto the error (see `take_eval_effects`).  The caller is
    ///   responsible for applying `EvalError::effects` and surfacing the error.
    ///
    /// Atomicity is **not** all-or-nothing across the whole file: only
    /// deferred effects (keybinds, LSP registration, …) roll back on error.
    /// `define-command!` and `register-hook!` mutate `command_table` /
    /// `HookRegistry` inline the instant the builtin runs (see
    /// `builtins::commands`/`builtins::hooks`), so a `define-command!` or
    /// `register-hook!` that ran before the failing form stays live in the
    /// degraded session — only *plugin activation* (`declare-plugin` bodies)
    /// is a true atomic unit with full command/hook rollback (see
    /// `builtins::plugins::finish_lazy_activation`). This is deliberate: a
    /// config error already surfaces to the user, and `:reload-config`
    /// rebuilds a fresh `ScriptingHost`, so the half-applied state never
    /// accumulates across reloads.
    pub fn eval_init(
        &mut self,
        path: &Path,
        budget_ms: u64,
        host: &mut dyn EditorHost,
        builtin_names: std::collections::HashSet<String>,
    ) -> Result<Vec<Effect>, EvalError> {
        let source = match hume_platform::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("reading {}: {e}", path.display()).into()),
        };
        self.eval_source_raw(source, builtin_names, budget_ms, host)
    }

    /// Shared tail for every eval entry point's atomicity contract: on
    /// success, drain exactly the effects this eval pushed (`effects_start`
    /// onward) and return them. On error, drain the same range but keep only
    /// the entries committed by a nested successful plugin activation
    /// (`QueuedEffect::committed`, set by `SteelCtx::pop_effect_marks`) —
    /// those effects are salvaged into the returned `EvalError` so the caller
    /// can still apply them, since the activation that queued them already
    /// committed irreversible state (`PluginState::Loaded`). Everything else
    /// the failed eval queued is discarded.
    fn take_eval_effects(
        &mut self,
        effects_start: usize,
        result: Result<(), String>,
    ) -> Result<Vec<Effect>, EvalError> {
        let tail = self.effects.split_off(effects_start);
        match result {
            Ok(()) => Ok(tail.into_iter().map(|e| e.effect).collect()),
            Err(message) => Err(EvalError {
                message,
                effects: tail
                    .into_iter()
                    .filter(|e| e.committed)
                    .map(|e| e.effect)
                    .collect(),
            }),
        }
    }

    /// Invoke a Steel proc by its internal name and return the list of
    /// commands it queued via `(call! …)`, plus an optional WaitChar
    /// command name requested via `(request-wait-char! …)`.
    ///
    /// The caller executes the returned commands and, if a wait-char was
    /// requested, enters WaitChar mode for that command.
    ///
    /// A watchdog thread enforces `host.steel_command_budget_ms()`.  If the
    /// script runs past the budget, `(hume/yield!)` calls abort it (cooperative
    /// interruption).
    ///
    /// # Design note — Steel in the signature
    /// `args` is `Vec<SteelVal>` rather than an opaque owned type because the
    /// caller (editor dispatch) constructs the args by wrapping already-resolved
    /// Rust values via `IntoSteelVal` and passing them straight in. Introducing
    /// an intermediate arg type would add conversion with no practical benefit:
    /// the editor crate already depends on `steel-core` for `SteelBufferId`.
    /// Encapsulating Steel on this side of the API is not
    /// cost-free; the trade-off is accepted intentionally.
    pub fn call_steel_cmd<'a>(
        &'a mut self,
        name: &str,
        pending_char: Option<char>,
        args: Vec<SteelVal>,
        focused_pane_id: hume_engine::pipeline::PaneId,
        focused_buffer_id: hume_engine::pipeline::BufferId,
        host: &'a mut dyn EditorHost,
    ) -> Result<SteelCmdResult, EvalError> {
        let budget_ms = host.settings().steel_command_budget_ms();

        // The editor already resolved any Lazy stub and activated its owning
        // plugin before calling here (see Editor::run_steel_command), so
        // `name` must already have a live closure in command_table. A miss
        // means the editor's registry and this table have desynced — a bug,
        // not a retry case — so this fails loudly rather than falling back to
        // %dispatch-command's own miss-handling (that dispatcher is reserved
        // for call!/bare-name calls originating inside the VM).
        let proc = self
            .registries
            .command_table
            .get(name)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "call_steel_cmd: '{name}' has no registered command body \
                     — registry/command_table desync"
                )
            })?;

        let effects_start = self.effects.len();
        let (result, wait_char_request) = {
            let (steel, watchdog, bundle) = self.steel_and_bundle();
            let mut steel_ctx = SteelCtx::new_command(
                host,
                bundle,
                focused_pane_id,
                focused_buffer_id,
                pending_char,
            );

            let result = run_steel_session(steel, watchdog, &mut steel_ctx, budget_ms, |steel| {
                steel.call_function_with_args(proc, args)?;
                Ok(())
            });
            (result, steel_ctx.wait_char_request)
        };

        let effects = self.take_eval_effects(effects_start, result)?;
        Ok(SteelCmdResult {
            wait_char_request,
            effects,
        })
    }

    /// Fire all registered handlers for `hook_id`, passing `args` to each.
    ///
    /// Handlers are called in registration order inside a single
    /// `with_mut_reference` session so they have full access to HUME builtins
    /// (`current-buffer`, `call!`, etc.).
    ///
    /// Returns immediately (no Steel engine call, no watchdog) if no handlers are
    /// registered for `hook_id`.
    pub fn fire_hook<'a>(
        &'a mut self,
        hook_id: hooks::HookId,
        args: &[SteelVal],
        focused_pane_id: hume_engine::pipeline::PaneId,
        focused_buffer_id: hume_engine::pipeline::BufferId,
        host: &'a mut dyn EditorHost,
    ) -> Result<Vec<Effect>, EvalError> {
        // Collect handler procs before borrowing self mutably for the SteelCtx.
        let handler_procs: Vec<SteelVal> = self
            .registries
            .hooks
            .handlers_for(hook_id)
            .iter()
            .map(|e| e.proc.clone())
            .collect();
        if handler_procs.is_empty() {
            return Ok(Vec::new());
        }

        let budget_ms = host.settings().steel_command_budget_ms();

        let effects_start = self.effects.len();
        let result = {
            let (steel, watchdog, bundle) = self.steel_and_bundle();
            let mut steel_ctx =
                SteelCtx::new_command(host, bundle, focused_pane_id, focused_buffer_id, None);

            // Call each handler directly with the arg values — no source
            // program, no per-fire globals. The first handler error aborts
            // the remaining handlers.
            run_steel_session(steel, watchdog, &mut steel_ctx, budget_ms, |steel| {
                for proc in handler_procs {
                    steel.call_function_with_args(proc, args.to_vec())?;
                }
                Ok(())
            })
        };

        self.take_eval_effects(effects_start, result)
    }

    /// Calls each `(proc, args)` pair directly, in order, inside one
    /// `with_mut_reference` session.
    ///
    /// Unlike [`fire_hook`](Self::fire_hook), which looks up every handler
    /// registered for a hook id, this delivers to a *specific* Steel closure
    /// captured earlier by Rust — the shared mechanism behind the
    /// `lsp-request` callback, timer thunks, and the prompt callback.
    /// Same discipline: never called from inside a completion-detection
    /// borrow (LSP drain, timer drain, minibuffer key handling) — the caller
    /// queues `(proc, args)` and this runs at the drain boundary. The first
    /// error aborts the remaining calls in the batch, same as `fire_hook`.
    pub fn run_steel_calls<'a>(
        &'a mut self,
        calls: Vec<(SteelVal, Vec<SteelVal>)>,
        focused_pane_id: hume_engine::pipeline::PaneId,
        focused_buffer_id: hume_engine::pipeline::BufferId,
        host: &'a mut dyn EditorHost,
    ) -> Result<Vec<Effect>, EvalError> {
        if calls.is_empty() {
            return Ok(Vec::new());
        }

        let budget_ms = host.settings().steel_command_budget_ms();

        let effects_start = self.effects.len();
        let result = {
            let (steel, watchdog, bundle) = self.steel_and_bundle();
            let mut steel_ctx =
                SteelCtx::new_command(host, bundle, focused_pane_id, focused_buffer_id, None);

            run_steel_session(steel, watchdog, &mut steel_ctx, budget_ms, |steel| {
                for (proc, args) in calls {
                    steel.call_function_with_args(proc, args)?;
                }
                Ok(())
            })
        };

        self.take_eval_effects(effects_start, result)
    }
}

// ── Test helpers ──────────────────────────────────────────────────────────────

#[cfg(any(test, feature = "test-util"))]
impl ScriptingHost {
    /// Evaluate a Steel source string directly, without a file.
    ///
    /// Convenience wrapper for testing.  Delegates to `eval_source_raw` with
    /// empty `builtin_names` and the default 10-second init budget (harmless
    /// for normal tests that complete quickly). Returns the effects the eval
    /// queued, in emission order — same contract as `eval_init`, so a test can
    /// assert on what a builtin marshalled across the boundary.
    pub fn eval_source(
        &mut self,
        source: &str,
        host: &mut dyn EditorHost,
    ) -> Result<Vec<Effect>, String> {
        self.eval_source_raw(source.to_owned(), Default::default(), 10_000, host)
            .map_err(|e| e.message)
    }

    /// Like [`eval_source`] but arms a real [`EvalWatchdog`] with the
    /// given budget.  Used by watchdog-specific tests that need to verify the
    /// watchdog actually fires rather than pre-setting the interrupt flag.
    pub fn eval_source_watchdog(
        &mut self,
        source: &str,
        budget: std::time::Duration,
        host: &mut dyn EditorHost,
    ) -> Result<(), String> {
        self.eval_source_raw(
            source.to_owned(),
            Default::default(),
            budget.as_millis() as u64,
            host,
        )
        .map(|_| ())
        .map_err(|e| e.message)
    }
}

// ── Public status enum ────────────────────────────────────────────────────────

/// Payload-free public view of a plugin's lifecycle state.
///
/// Maps from the private `PluginState` enum (which carries a `PathBuf` for
/// `Declared`) without leaking internal details.  Used by `plugin_status()` so
/// the editor-crate test suite can assert on states without reaching into
/// `ScriptingHost` fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginStatus {
    /// Declared but not yet loaded.
    Declared,
    /// Currently being loaded (re-entrancy guard).
    Loading,
    /// Loaded successfully.
    Loaded,
    /// Load failed (will not retry until `:reload-config`).
    Failed,
}

/// Pins the full-trust plugin model's load-bearing assumption: Steel's
/// `steel/process`/`steel/filesystem`/`steel/ports` globals are reachable
/// from plugin code with no `require-builtin` (they ride in via `steel/base`,
/// required by the auto-loaded prelude). PLUM's migration off HUME's removed
/// hardened builtins depends on this; a steel-core upgrade that changes it
/// must fail these tests before it silently breaks every core plugin.
#[cfg(test)]
mod steel_stdlib_availability {
    use crate::ScriptingHost;
    use crate::null_host::NullHost;

    #[test]
    fn process_and_fs_globals_are_available_unrequired() {
        let mut host = ScriptingHost::new();
        let mut null_host = NullHost;
        let src = r#"
            (if (and (function? command)
                     (function? spawn-process)
                     (function? wait)
                     (function? wait->stdout)
                     (function? which)
                     (function? with-current-dir)
                     (function? with-stdout-piped)
                     (function? read-dir)
                     (function? create-directory!)
                     (function? delete-directory!)
                     (function? is-dir?)
                     (function? is-file?)
                     (function? open-output-file)
                     ; `Ok?`/`Err?`/`Ok->value`/`Err->value` are the raw
                     ; `steel/core/result` struct ops and ARE globally bound;
                     ; the higher-level `unwrap-ok`/`unwrap-err` wrapper
                     ; (`steel/result`) is NOT reachable here — `(require-builtin
                     ; steel/result)` fails with "module not found" in HUME's
                     ; embedding, unlike steel-core's own bundled module-name
                     ; resolution. Use `Ok->value`/`Err->value` in plugin code.
                     (function? Ok?)
                     (function? Err?)
                     (function? Ok->value)
                     (function? Err->value)
                     ; needed by plum/list-dir (Phase 1 helper): sort takes an
                     ; explicit comparator ((sort lst less?)), not 1-arg.
                     (function? sort)
                     (function? string<?)
                     (function? file-name))
                (begin)
                (error "one or more steel/process, steel/filesystem, steel/ports, or result globals are missing"))
        "#;
        host.eval_source(src, &mut null_host)
            .expect("steel stdlib availability pin failed");

        // string-downcase, needed by lsp/verify-sha256! (Phase 4 helper).
        let mut host3 = ScriptingHost::new();
        let mut null_host3 = NullHost;
        let downcase_src = r#"
            (if (equal? (string-downcase "ABC123def") "abc123def")
                (begin)
                (error "string-downcase did not lowercase"))
        "#;
        host3
            .eval_source(downcase_src, &mut null_host3)
            .expect("string-downcase probe failed");

        // Round-trip proof, not just presence: `plum/list-dir` depends on
        // `sort` taking `(lst less?)` and `file-name` extracting a basename.
        let mut host2 = ScriptingHost::new();
        let mut null_host2 = NullHost;
        let sort_src = r#"
            (define sorted (sort (list "b" "a" "c") string<?))
            (if (equal? sorted (list "a" "b" "c"))
                (begin)
                (error (string-append "sort did not sort: " (to-string sorted))))
            (if (equal? (file-name "/tmp/foo/bar.txt") "bar.txt")
                (begin)
                (error "file-name did not extract basename"))
        "#;
        host2
            .eval_source(sort_src, &mut null_host2)
            .expect("sort/file-name round trip failed");
    }

    /// End-to-end proof, not just presence checks: a real spawn writes to a
    /// real temp directory and its piped stdout is captured back correctly.
    #[test]
    fn spawn_process_round_trip_with_fs_ops_and_piped_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_string_lossy().replace('\\', "\\\\");

        let mut host = ScriptingHost::new();
        let mut null_host = NullHost;
        let src = format!(
            r#"
            (define target (string-append "{base}" "/probe-dir"))
            (create-directory! target)
            (if (is-dir? target) (begin) (error "create-directory!/is-dir? failed"))
            (delete-directory! target)
            (if (is-dir? target) (error "delete-directory! did not remove dir") (begin))

            (define builder (with-current-dir (with-stdout-piped (command "echo" (list "hello-from-probe"))) "{base}"))
            (define spawned (spawn-process builder))
            (if (Ok? spawned)
                (let ([child (Ok->value spawned)])
                  (let ([out (wait->stdout child)])
                    (if (and (Ok? out) (string-contains? (Ok->value out) "hello-from-probe"))
                        (begin)
                        (error (string-append "unexpected wait->stdout result: " (to-string out))))))
                (error (to-string (Err->value spawned))))
            "#
        );
        host.eval_source(&src, &mut null_host)
            .expect("spawn-process round trip with fs ops and piped stdout failed");
    }

    /// Pins a real gotcha `plum/run!` (Phase 1 helper) depends on: `child-stderr`
    /// (and by extension `child-stdin`/`child-stdout`) must be captured
    /// *before* calling `wait` — calling it after returns `#f` even though the
    /// stream was piped. Also pins the stdin-close-for-EOF pattern needed
    /// since stdin is not inherited by default.
    #[test]
    #[cfg(unix)]
    fn child_stderr_must_be_captured_before_wait() {
        let mut host = ScriptingHost::new();
        let mut null_host = NullHost;
        // Failure path: stdout+stderr+stdin all piped, stdin closed
        // immediately, wait for exit, read stderr on nonzero.
        let src = r#"
            (define builder
              (with-stdin-piped (with-stderr-piped (with-stdout-piped (command "sh" (list "-c" "echo err-marker 1>&2; exit 3"))))))
            (define spawned (spawn-process builder))
            (if (Ok? spawned)
                (let* ([child (Ok->value spawned)]
                       [stderr-port (child-stderr child)])
                  (close-output-port (child-stdin child))
                  (let ([wait-result (wait child)])
                    (if (Ok? wait-result)
                        (let ([code (Ok->value wait-result)])
                          (if (= code 3)
                              (let ([stderr (read-port-to-string stderr-port)])
                                (if (string-contains? stderr "err-marker")
                                    (begin)
                                    (error (string-append "stderr missing marker: " stderr))))
                              (error (string-append "unexpected exit code: " (to-string code)))))
                        (error (to-string (Err->value wait-result))))))
                (error (to-string (Err->value spawned))))
        "#;
        host.eval_source(src, &mut null_host)
            .expect("plum/run! shape probe failed");
    }

    /// Pins the file read/write port round trip `plum/read-file`/`plum/write-file`
    /// (Phase 1 helpers) depend on.
    #[test]
    fn file_write_read_port_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("probe.txt")
            .to_string_lossy()
            .replace('\\', "\\\\");
        let mut host = ScriptingHost::new();
        let mut null_host = NullHost;
        let src = format!(
            r#"
            (define out (open-output-file "{path}"))
            (write-string "hello-file-probe" out)
            (close-output-port out)
            (define in (open-input-file "{path}"))
            (define content (read-port-to-string in))
            (close-input-port in)
            (if (equal? content "hello-file-probe")
                (begin)
                (error (string-append "unexpected file content: " content)))
            "#
        );
        host.eval_source(&src, &mut null_host)
            .expect("file write/read port probe failed");
    }

    /// **Known steel-core 0.8.2 limitation, not a HUME bug**: re-raising a
    /// native-builtin error (via `raise-error`) from an inner `with-handler`,
    /// caught by an *outer* `with-handler`, corrupts the VM's continuation
    /// stack and panics "Failed to find an open continuation on the stack".
    /// Also reachable via `grammars.scm`'s `plum/resolve-query` (see
    /// `plum/fetch-raw-query`'s doc comment for the fix: never wrap the
    /// raising call in an inner catch-and-reraise). `#[should_panic]`
    /// regression pin — if a steel-core upgrade fixes this, revisit the
    /// `grammars.scm` workaround.
    #[test]
    #[cfg(unix)]
    #[should_panic(expected = "Failed to find an open continuation on the stack")]
    fn known_limitation_reraise_via_raise_error_inside_outer_tolerant_handler_corrupts_vm_stack() {
        let mut host = ScriptingHost::new();
        let mut null_host = NullHost;
        // Exact shape of plum/fetch-raw-query (inner: catch, cleanup, re-raise
        // via raise-error) nested inside plum/resolve-query's tolerant outer
        // with-handler.
        let src = r#"
            (define (inner-fetch)
              (with-handler
                (lambda (err) (raise-error err))
                (run-inline-output! "false" '())))

            (define (tolerant-outer)
              (with-handler (lambda (err) #f) (inner-fetch)))

            (tolerant-outer)
        "#;
        host.eval_source(src, &mut null_host)
            .expect("raise-error re-raise inside outer tolerant handler failed");
    }

    #[test]
    #[cfg(unix)]
    fn uncaught_native_error_propagates_one_hop_to_outer_tolerant_handler() {
        // Fix shape: the native-builtin-raising call (run-inline-output!) is
        // NOT wrapped by an inner with-handler at all — it propagates in one
        // hop straight to the outer tolerant handler, exactly like the
        // original (pre-migration) curl-fetch call site.
        let mut host = ScriptingHost::new();
        let mut null_host = NullHost;
        let src = r#"
            (define (inner-fetch)
              (run-inline-output! "false" '()))

            (define (tolerant-outer)
              (with-handler (lambda (err) #f) (inner-fetch)))

            (tolerant-outer)
        "#;
        host.eval_source(src, &mut null_host)
            .expect("uncaught native error one-hop propagation to outer handler failed");
    }

    /// **Second known steel-core 0.8.2 limitation**: `dynamic-wind`'s
    /// `after` thunk is not guaranteed to run when its body raises through an
    /// outer `with-handler` — reproduces the panic-pinning test's failure,
    /// wrapped in `dynamic-wind` instead of catch-and-reraise. This would
    /// otherwise be a safe way to guarantee `declare-plugin`'s manifest
    /// cleanup (`%finish-manifest-declare!`) runs without an inner handler,
    /// but `cleanup-ran` never fires — confirms the decision (see
    /// `project_steel_raii_vs_dynamicwind.md`) to keep cleanup-on-unwind in
    /// Rust (explicit push/pop), never Steel `dynamic-wind`. Pinned like the
    /// test above: a steel-core fix flips `cleanup-ran` to `#t` and this
    /// starts failing — revisit then.
    #[test]
    #[cfg(unix)]
    fn known_limitation_dynamic_wind_cleanup_does_not_run_across_an_outer_handlers_unwind() {
        let mut host = ScriptingHost::new();
        let mut null_host = NullHost;
        let src = r#"
            (define cleanup-ran #f)
            (define (inner-fetch)
              (dynamic-wind
                (lambda () (void))
                (lambda () (run-inline-output! "false" '()))
                (lambda () (set! cleanup-ran #t))))

            (define (tolerant-outer)
              (with-handler (lambda (err) #f) (inner-fetch)))

            (tolerant-outer)
            (if cleanup-ran (begin) (error "cleanup did not run"))
        "#;
        let result = host.eval_source(src, &mut null_host);
        let err = result.expect_err(
            "dynamic-wind's cleanup thunk unexpectedly ran across the outer handler's unwind — \
             if steel-core fixed this, declare-plugin's manifest branch could use dynamic-wind \
             instead of catch-and-reraise to avoid the panic pinned above",
        );
        assert!(
            err.contains("cleanup did not run"),
            "expected the cleanup-did-not-run assertion to fire, got a different error: {err}"
        );
    }
}
