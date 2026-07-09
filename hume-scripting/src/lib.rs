//! Steel scripting integration for HUME.
//!
//! The [`ScriptingHost`] owns the Steel [`Engine`] and runs entirely on the
//! main event-loop thread — Steel's Engine is `!Send` by design (internal
//! `Rc`/`RefCell`, non-atomic `im-rs` lists). This is a deliberate choice:
//! edit commands are synchronous `(Buffer, SelectionSet) → (Buffer, SelectionSet)`
//! operations on the hot-key path; an IPC round-trip per keystroke would be
//! strictly worse than a direct function call.
//!
//! ## Plugin loading pipeline
//! - `(load-plugin name)` — **inline**. Valid only during init.scm (or
//!   `:reload-config`); calling it from a command/hook body is a hard error.
//!   Resolves the path, records the plugin, then activates it inline via
//!   `%activate-plugin-inline` (body evaluated via `hm.eval-string` inside the
//!   running VM — no `&mut Engine` borrow needed).  Self-declares: no prior
//!   `declare-plugin` needed.
//! - `(declare-plugin name #:commands #:events #:languages #:config)` — **lazy**.
//!   The plugin **manifest**: records a `Declared` state + activation maps in
//!   `LazyRegistry`; body is NOT run.  At least one of `#:commands`/`#:events`/
//!   `#:languages` is required — a manifest with no activation entries can never
//!   be activated and hard-errors.
//! - `#:config` (on both `load-plugin` and `declare-plugin`) is an opaque value
//!   (typically a hash) stored per-`PluginId`. The plugin body reads its own
//!   config back via `(plugin-config)`, resolved from the top of `plugin_stack`
//!   — identical for eager and lazy bodies, since both push there for the
//!   duration of the eval.
//! - Activation entries (command / event / language) are one-shot: the first one
//!   exercised calls `%activate-plugin-inline` (body via `(require)`), flips state
//!   to `Loaded`, and drops that plugin's entries from all activation maps.  The
//!   body then typically registers **hooks** (`register-hook!`) that fire on every
//!   subsequent event — hooks and activation entries are distinct.
//! - Activation states: `Declared → Loading → Loaded | Failed`. `Loading` guards
//!   re-entrant cycles (A→B→A); `Failed` does not retry until `:reload-config`.
//! - PLUM (`core:plum`) reads `(declared-plugins)` (non-`core:` only) to install
//!   third-party plugins. Both `load-plugin` and `declare-plugin` record the name
//!   in `declared_plugins` (persistent on `ScriptingHost`). Declaring a dep at
//!   init top-level records it up front so PLUM can install it, even before any
//!   plugin body runs.
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
#[cfg(any(test, feature = "test-util"))]
pub use builtins::sandbox::init_dirs;
pub use hooks::HookId;
pub use host::{BindMode, EditorHost};
pub use keys::parse_key_stream;
pub use log::LogLevel;
pub use types::{
    HookResult, LspServerStatusEntry, PendingLanguageReg, PendingLspNotify, PendingLspRequest,
    PendingLspServerReg, SteelCmdDef, SteelCmdResult,
};
pub use watchdog::EvalWatchdog;

// ── Internal re-exports (within-crate use) ────────────────────────────────────
pub(crate) use activation::{run_steel_call, run_steel_session};
pub(crate) use context::SteelCtx;

/// Steel global name under which the live [`SteelCtx`] reference is visible to
/// builtins during an eval (injected by `run_steel_session`, reset to `#void`
/// after).
pub(crate) const HUME_CTX: &str = "*hume.ctx*";

// ── Internal imports ──────────────────────────────────────────────────────────

use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::AtomicBool};

use steel::rvals::{IntoSteelVal as _, SteelVal};
use steel::steel_vm::engine::Engine;

use attribution::PluginStack;
use hooks::HookRegistry;
use lazy::{LazyRegistry, PluginState};

// ── ScriptingRegistries ───────────────────────────────────────────────────────

/// The five persistent registry fields bundled as a unit so they can be
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
    pending_language_regs: &'a mut Vec<PendingLanguageReg>,
    pending_lsp_server_regs: &'a mut Vec<PendingLspServerReg>,
    data_dir: Option<&'a std::path::Path>,
    runtime_dir: Option<&'a std::path::Path>,
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
    /// The four persistent registries borrowed as a unit into `SteelCtx`,
    /// disjoint from `steel` so the VM and command/hook state can be borrowed
    /// simultaneously (NLL field-split).
    pub(crate) registries: ScriptingRegistries,
    /// Attribution stack: `stack.last()` is the plugin currently executing.
    /// Empty → top-level `init.scm` → `Owner::User`.
    plugin_stack: PluginStack,
    /// Log messages accumulated by `(log! …)` since the last drain.
    /// Drained by the editor via `take_pending_messages()`.
    pending_messages: Vec<(LogLevel, String)>,
    /// Language identity registrations queued by `(define-language! …)`.
    /// Drained by `Editor::flush_pending_language_regs` after each `eval_init` boundary.
    pending_language_regs: Vec<PendingLanguageReg>,
    /// LSP server registrations queued by `(register-lsp-server! …)`.
    /// Drained by `Editor::flush_pending_lsp_server_regs` after init.scm finishes.
    pending_lsp_server_regs: Vec<PendingLspServerReg>,
    /// `$XDG_DATA_HOME/hume/` — where PLUM installs user/third-party plugins.
    data_dir: Option<PathBuf>,
    /// The runtime directory (core plugins, themes, docs), or `None` if absent.
    runtime_dir: Option<PathBuf>,
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
        let data_dir = hume_platform::dirs::data_dir();
        let runtime_dir = hume_platform::dirs::runtime_dir();
        // Initialize the fs builtin directory TLS before the Steel engine registers
        // builtins — the `data-dir` / `runtime-dir` / sandbox functions read
        // from this TLS whenever they are called.
        builtins::sandbox::init_dirs(data_dir.clone(), runtime_dir.clone());
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
            pending_language_regs: Vec::new(),
            pending_lsp_server_regs: Vec::new(),
            data_dir,
            runtime_dir,
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
            pending_language_regs,
            pending_lsp_server_regs,
            data_dir,
            runtime_dir,
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
                pending_language_regs,
                pending_lsp_server_regs,
                data_dir: data_dir.as_deref(),
                runtime_dir: runtime_dir.as_deref(),
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
    /// "name" args)))`. This makes bare `(move-left)`, `(collapse-selection)`
    /// etc. callable from Steel without requiring `(call! "move-left")` — and,
    /// since the wrapper is variadic, also `(move-down 3)` / `(move-down 0)`
    /// (count `0` is the Scheme spelling of "no count typed", see
    /// `parse_count_extend`).
    ///
    /// Calls `%dispatch-command` directly rather than expanding the public
    /// `call!` macro (`(call! name args...)` desugars to exactly this — see
    /// `%dispatch-command`/`call!` in the bootstrap source) — a variadic lambda
    /// already binds its args as the list `%dispatch-command` expects, so no
    /// intermediate `(list ...)` call is needed.
    ///
    /// Called from `Editor::init_scripting` after `ScriptingHost::new` and
    /// **before** any `eval_init` call, so that `init.scm` and plugins can use
    /// bare command names without a `FreeIdentifier` compile error.
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
        self.runtime_dir.as_deref()
    }

    /// Data directory for user/third-party plugins.
    pub fn data_dir(&self) -> Option<&Path> {
        self.data_dir.as_deref()
    }

    /// Drain all accumulated log messages since the last drain.
    pub fn take_pending_messages(&mut self) -> Vec<(LogLevel, String)> {
        std::mem::take(&mut self.pending_messages)
    }

    /// Drain all pending language identity/grammar registrations.
    pub fn take_pending_language_regs(&mut self) -> Vec<PendingLanguageReg> {
        std::mem::take(&mut self.pending_language_regs)
    }

    /// Drain all pending LSP server registrations.
    pub fn take_pending_lsp_server_regs(&mut self) -> Vec<PendingLspServerReg> {
        std::mem::take(&mut self.pending_lsp_server_regs)
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

    /// A snapshot of the command activation entries declared during init.
    pub fn activation_commands(&self) -> std::collections::HashMap<String, attribution::PluginId> {
        self.registries.lazy_registry.activation_commands.clone()
    }

    /// Drop a single command activation entry and its pre-seeded ownership.
    ///
    /// Called when the editor's command registry already owns `name`, so no
    /// `Lazy` stub was registered — the declare-time claim (seeded in
    /// `declare_plugin`) must not linger in the activation maps.
    pub fn drop_activation_command(&mut self, name: &str) {
        self.registries
            .lazy_registry
            .activation_commands
            .remove(name);
        self.registries.cmd_owners.remove(name);
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
    pub fn activation_language_plugins(&self, language: &str) -> Vec<attribution::PluginId> {
        self.registries
            .lazy_registry
            .activation_languages
            .get(language)
            .cloned()
            .unwrap_or_default()
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
    pub fn lazy_status_string(&self) -> String {
        self.registries.lazy_registry.format_status()
    }

    /// Peek at pending messages without draining.  Only for test assertions.
    #[cfg(any(test, feature = "test-util"))]
    pub fn peek_pending_messages(&self) -> &[(LogLevel, String)] {
        &self.pending_messages
    }

    /// Override the data directory.  Used only in tests that need a predictable
    /// plugin install location.
    #[cfg(any(test, feature = "test-util"))]
    pub fn set_data_dir(&mut self, dir: std::path::PathBuf) {
        self.data_dir = Some(dir);
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
    }

    // ── Eval machinery ────────────────────────────────────────────────────────

    /// Evaluate `init.scm` at `path`, giving builtins access to editor state
    /// (settings, keymap) via `host` for the duration of the call.
    ///
    /// - Returns `Ok(())` if the file does not exist (missing config is normal)
    ///   or if eval succeeds.  Commands defined during eval are registered into
    ///   the `CommandRegistry` inline via `host.register_command`.
    /// - Returns `Err(message)` if the file exists but fails to parse or
    ///   evaluate.  The caller is responsible for surfacing the error.
    pub fn eval_init(
        &mut self,
        path: &Path,
        budget_ms: u64,
        host: &mut dyn EditorHost,
        builtin_names: std::collections::HashSet<String>,
    ) -> Result<(), String> {
        let source = match hume_platform::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("reading {}: {e}", path.display())),
        };
        self.eval_source_raw(source, builtin_names, budget_ms, host)
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
    ) -> Result<SteelCmdResult, String> {
        let budget_ms = host.steel_command_budget_ms();

        // Keypress dispatch routes through %dispatch-command so Lazy-miss
        // auto-activation and command_table lookup use the same path as call!.
        // The name and args are passed as values via a direct function call —
        // nothing is spliced into source and nothing is compiled per dispatch.
        let args_list = args
            .into_steelval()
            .map_err(|e| format!("call_steel_cmd: cannot convert args: {e}"))?;
        let call_args = vec![SteelVal::StringV(name.into()), args_list];

        let (
            result,
            wait_char_request,
            pending_language_sets,
            grammar_sweeps,
            pending_lsp_requests,
            pending_lsp_notifies,
        ) = {
            let (steel, watchdog, bundle) = self.steel_and_bundle();
            let mut steel_ctx = SteelCtx::new_command(
                host,
                bundle,
                focused_pane_id,
                focused_buffer_id,
                pending_char,
            );

            let result = run_steel_call(
                steel,
                watchdog,
                &mut steel_ctx,
                "%dispatch-command",
                call_args,
                budget_ms,
            );
            (
                result,
                steel_ctx.wait_char_request,
                steel_ctx.pending_language_sets,
                steel_ctx.pending_grammar_sweeps,
                steel_ctx.pending_lsp_requests,
                steel_ctx.pending_lsp_notifies,
            )
        };

        result?;
        Ok(SteelCmdResult {
            wait_char_request,
            pending_language_sets,
            grammar_sweeps,
            pending_lsp_requests,
            pending_lsp_notifies,
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
    ) -> Result<HookResult, String> {
        // Collect handler procs before borrowing self mutably for the SteelCtx.
        let handler_procs: Vec<SteelVal> = self.registries.hooks.handlers_for(hook_id).to_vec();
        if handler_procs.is_empty() {
            return Ok(HookResult {
                pending_language_sets: vec![],
                grammar_sweeps: vec![],
                pending_lsp_requests: vec![],
                pending_lsp_notifies: vec![],
            });
        }

        let budget_ms = host.steel_command_budget_ms();

        let (
            result,
            pending_language_sets,
            grammar_sweeps,
            pending_lsp_requests,
            pending_lsp_notifies,
        ) = {
            let (steel, watchdog, bundle) = self.steel_and_bundle();
            let mut steel_ctx =
                SteelCtx::new_command(host, bundle, focused_pane_id, focused_buffer_id, None);

            // Call each handler directly with the arg values — no source
            // program, no per-fire globals.  The first handler error aborts
            // the remaining handlers, matching the composite-program semantics
            // this replaced.
            let result = run_steel_session(steel, watchdog, &mut steel_ctx, budget_ms, |steel| {
                for proc in handler_procs {
                    steel.call_function_with_args(proc, args.to_vec())?;
                }
                Ok(())
            });
            (
                result,
                steel_ctx.pending_language_sets,
                steel_ctx.pending_grammar_sweeps,
                steel_ctx.pending_lsp_requests,
                steel_ctx.pending_lsp_notifies,
            )
        };

        result?;
        Ok(HookResult {
            pending_language_sets,
            grammar_sweeps,
            pending_lsp_requests,
            pending_lsp_notifies,
        })
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
    ) -> Result<HookResult, String> {
        if calls.is_empty() {
            return Ok(HookResult {
                pending_language_sets: vec![],
                grammar_sweeps: vec![],
                pending_lsp_requests: vec![],
                pending_lsp_notifies: vec![],
            });
        }

        let budget_ms = host.steel_command_budget_ms();

        let (
            result,
            pending_language_sets,
            grammar_sweeps,
            pending_lsp_requests,
            pending_lsp_notifies,
        ) = {
            let (steel, watchdog, bundle) = self.steel_and_bundle();
            let mut steel_ctx =
                SteelCtx::new_command(host, bundle, focused_pane_id, focused_buffer_id, None);

            let result = run_steel_session(steel, watchdog, &mut steel_ctx, budget_ms, |steel| {
                for (proc, args) in calls {
                    steel.call_function_with_args(proc, args)?;
                }
                Ok(())
            });
            (
                result,
                steel_ctx.pending_language_sets,
                steel_ctx.pending_grammar_sweeps,
                steel_ctx.pending_lsp_requests,
                steel_ctx.pending_lsp_notifies,
            )
        };

        result?;
        Ok(HookResult {
            pending_language_sets,
            grammar_sweeps,
            pending_lsp_requests,
            pending_lsp_notifies,
        })
    }
}

// ── Test helpers ──────────────────────────────────────────────────────────────

#[cfg(any(test, feature = "test-util"))]
impl ScriptingHost {
    /// Evaluate a Steel source string directly, without a file.
    ///
    /// Convenience wrapper for testing.  Delegates to `eval_source_raw` with
    /// empty `builtin_names` and the default 10-second init budget (harmless
    /// for normal tests that complete quickly).
    pub fn eval_source(&mut self, source: &str, host: &mut dyn EditorHost) -> Result<(), String> {
        self.eval_source_raw(source.to_owned(), Default::default(), 10_000, host)
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
