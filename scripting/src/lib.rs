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
//! - `(load-plugin name)` — **eager**. Resolves path, queues, body evaluated via
//!   `(require "<abs>")` during init. Self-declares: works on a never-declared
//!   plugin (no prior `declare-plugin` needed).
//! - `(declare-plugin name #:on-command #:on-event #:on-language)` — **lazy**.
//!   Records a `Declared` state + trigger maps in `LazyRegistry`; body is NOT run.
//! - Triggers (command / event / language) are one-shot: the first one to fire
//!   calls `activate_plugin` (body via `(require)`), flips state to `Loaded`, and
//!   drops that plugin's entries from all trigger maps so it never refires.
//! - A **bare** `(declare-plugin name)` (no triggers) stays `Declared` forever
//!   until something explicitly `(load-plugin name)`s it (e.g. a dependent plugin).
//! - Activation states: `Declared → Loading → Loaded | Failed`. `Loading` guards
//!   re-entrant trigger cycles (A→B→A); `Failed` does not retry until `:reload-config`.
//! - PLUM (`core:plum`) reads `(declared-plugins)` (non-`core:` only) to install
//!   third-party plugins. Both `load-plugin` and `declare-plugin` record the name
//!   in `declared_plugins` (persistent on `ScriptingHost`). The point of a top-level
//!   bare declare is the fresh-machine chicken-and-egg: a dep `(load-plugin)`'d only
//!   inside another plugin's body hard-errors when absent, so it is never recorded
//!   for PLUM to install. Declaring it at init top-level records it up front; PLUM
//!   installs it, then the in-body load succeeds.
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
pub(crate) mod keys;
pub(crate) mod lazy;
pub mod log;
// ── Private implementation details ────────────────────────────────────────────
mod activation;
mod codegen;
mod context;
mod types;
pub(crate) mod watchdog;
#[cfg(test)]
mod null_host;
#[cfg(test)]
mod test_support;

// ── Public API re-exports ─────────────────────────────────────────────────────
// Types the editor and editor tests use directly.
pub use hooks::HookId;
pub use host::{BindMode, EditorHost};
pub use log::LogLevel;
pub use types::{
    HookResult, PendingLanguageReg, QueuedCommand, SteelCmdDef, SteelCmdResult,
};
pub use attribution::PluginId;
pub use builtins::ids::SteelBufferId;
pub use watchdog::EvalWatchdog;
#[cfg(any(test, feature = "test-util"))]
pub use builtins::sandbox::init_dirs;

// ── Internal re-exports (within-crate use) ────────────────────────────────────
pub(crate) use activation::run_steel;
pub(crate) use codegen::{HUME_CTX, cmd_arg_global_name};
pub(crate) use context::SteelCtx;

// ── Internal imports ──────────────────────────────────────────────────────────

use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::AtomicBool,
};

use steel::rvals::SteelVal;
use steel::steel_vm::engine::Engine;

use hooks::HookRegistry;
use attribution::PluginStack;
use lazy::{LazyRegistry, PluginState};

use codegen::{build_hook_program, hook_arg_name, hook_proc_name};

// ── HostBundle ────────────────────────────────────────────────────────────────

/// Borrows of [`ScriptingHost`] fields needed to populate [`SteelCtx`].
///
/// Built from a `let Self { engine, plugin_stack, … } = &mut *self` destructure
/// and passed to [`SteelCtx::new_init`] or [`SteelCtx::new_command`].
/// Private to this module.
pub(crate) struct HostBundle<'a> {
    plugin_stack: &'a mut PluginStack,
    cmd_owners: &'a mut std::collections::HashMap<String, String>,
    hooks: &'a mut HookRegistry,
    lazy_registry: &'a mut LazyRegistry,
    declared_plugins: &'a mut Vec<String>,
    pending_messages: &'a mut Vec<(LogLevel, String)>,
    pending_language_regs: &'a mut Vec<PendingLanguageReg>,
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
/// directly — no `mem::take`/put-back needed.
///
/// Constructed once during `Editor::init_scripting()` and held for the
/// lifetime of the process.
pub struct ScriptingHost {
    engine: Engine,
    /// Attribution stack: `stack.last()` is the plugin currently executing.
    /// Empty → top-level `init.scm` → `Owner::User`.
    plugin_stack: PluginStack,
    /// Command-to-owner index: maps each Steel-registered command name to a
    /// display string (`"hume"`, `"user"`, or a plugin id like `"core:plum"`).
    /// Populated by `process_pending_cmds`; queried by `(command-plugin name)`.
    cmd_owners: std::collections::HashMap<String, String>,
    /// Persistent hook registry: handlers registered by `(register-hook! …)`.
    hooks: HookRegistry,
    /// Lazy plugin registry: populated by `%declare-plugin!` during init;
    /// trigger maps consulted by command dispatch, event firing, and language-set.
    lazy_registry: LazyRegistry,
    /// Every plugin name passed to `(load-plugin …)` or `(declare-plugin …)`,
    /// including plugins absent on disk.  Persists across evals so that
    /// `(declared-plugins)` returns the full init-time list at command time (PLUM).
    declared_plugins: Vec<String>,
    /// Log messages accumulated by `(log! …)` since the last drain.
    /// Drained by the editor via `take_pending_messages()`.
    pending_messages: Vec<(LogLevel, String)>,
    /// Language identity registrations queued by `(define-language! …)`.
    /// Drained by `Editor::flush_pending_language_regs` after each `eval_init` boundary.
    pending_language_regs: Vec<PendingLanguageReg>,
    /// Commands queued by `(call! …)` during init (init.scm or plugin load).
    /// Drained by `Editor::run_startup_commands` after all plugins activate.
    pending_startup_commands: Vec<QueuedCommand>,
    /// `$XDG_DATA_HOME/hume/` — where PLUM installs user/third-party plugins.
    data_dir: Option<PathBuf>,
    /// The runtime directory (core plugins, themes, docs), or `None` if absent.
    runtime_dir: Option<PathBuf>,
    /// Shared interrupt flag.  Set to `true` by the watchdog to signal that
    /// `(hume/yield!)` calls should abort the running script.  Reset to
    /// `false` after every `eval_init` call.
    interrupt_flag: Arc<AtomicBool>,
    /// Cache of pre-built hook invocation programs keyed by
    /// `(arg_count, handler_count)`.  The program text is deterministic given
    /// those two values, so it is built once and reused across fires.
    hook_program_cache: std::collections::HashMap<(usize, usize), String>,
}

impl ScriptingHost {
    /// Create a new scripting host with the Steel standard library and all HUME
    /// builtins loaded.
    ///
    /// Resolves base directories eagerly so builtins can use them without
    /// re-reading environment variables on every call.
    pub fn new() -> Self {
        let data_dir = platform::dirs::data_dir();
        let runtime_dir = platform::dirs::runtime_dir();
        // Initialize the fs builtin directory TLS before the engine registers
        // builtins — the `data-dir` / `runtime-dir` / sandbox functions read
        // from this TLS whenever they are called.
        builtins::sandbox::init_dirs(data_dir.clone(), runtime_dir.clone());
        let mut engine = Engine::new();
        builtins::register_all(&mut engine);
        Self {
            engine,
            plugin_stack: PluginStack::default(),
            cmd_owners: std::collections::HashMap::new(),
            hooks: HookRegistry::default(),
            lazy_registry: LazyRegistry::default(),
            declared_plugins: Vec::new(),
            pending_messages: Vec::new(),
            pending_language_regs: Vec::new(),
            pending_startup_commands: Vec::new(),
            data_dir,
            runtime_dir,
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            hook_program_cache: std::collections::HashMap::new(),
        }
    }

    // ── Outward API (clean; no direct field access outside this module) ────────

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

    /// Drain all startup commands queued during init / plugin activation.
    pub fn take_startup_commands(&mut self) -> Vec<QueuedCommand> {
        std::mem::take(&mut self.pending_startup_commands)
    }

    /// Number of startup commands currently queued.
    pub fn pending_startup_commands_len(&self) -> usize {
        self.pending_startup_commands.len()
    }

    /// Drain and return only the startup commands added since index `base`.
    ///
    /// Used by `activate_and_register` (lazy dispatch) which needs to diff the
    /// command list before and after a single plugin activation.
    pub fn split_off_startup_commands(&mut self, base: usize) -> Vec<QueuedCommand> {
        self.pending_startup_commands.split_off(base)
    }

    /// Returns `true` if no handlers are registered for `hook_id`.
    pub fn has_hook_handlers(&self, hook_id: hooks::HookId) -> bool {
        !self.hooks.is_empty_for(hook_id)
    }

    /// A snapshot of the command triggers declared during init.
    pub fn command_triggers(
        &self,
    ) -> std::collections::HashMap<String, attribution::PluginId> {
        self.lazy_registry.command_triggers.clone()
    }

    /// Plugin ids that should be activated when `hook_id` fires.
    pub fn event_trigger_plugins(&self, hook_id: hooks::HookId) -> Vec<attribution::PluginId> {
        self.lazy_registry
            .event_triggers
            .get(&hook_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Plugin ids that should be activated when `language` is set on a buffer.
    pub fn language_trigger_plugins(&self, language: &str) -> Vec<attribution::PluginId> {
        self.lazy_registry
            .language_triggers
            .get(language)
            .cloned()
            .unwrap_or_default()
    }

    /// Status of a plugin in the lazy registry.
    pub fn plugin_status(&self, id: &attribution::PluginId) -> Option<PluginStatus> {
        self.lazy_registry.plugins.get(id).map(|state| match state {
            PluginState::Declared { .. } => PluginStatus::Declared,
            PluginState::Loading => PluginStatus::Loading,
            PluginState::Loaded => PluginStatus::Loaded,
            PluginState::Failed => PluginStatus::Failed,
        })
    }

    /// Returns `true` if any plugin in the registry has transitioned to `Loaded`.
    #[cfg(any(test, feature = "test-util"))]
    pub fn has_any_loaded_plugin(&self) -> bool {
        self.lazy_registry
            .plugins
            .values()
            .any(|s| matches!(s, PluginState::Loaded))
    }

    /// All plugin names ever passed to `(load-plugin …)` or `(declare-plugin …)`.
    #[cfg(any(test, feature = "test-util"))]
    pub fn declared_plugins(&self) -> &[String] {
        &self.declared_plugins
    }

    /// Format a human-readable plugin status table for `:plugin-status`.
    pub fn lazy_status_string(&self) -> String {
        self.lazy_registry.format_status()
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


    #[cfg(any(test, feature = "test-util"))]
    pub fn cmd_owners_for_test(&self) -> &std::collections::HashMap<String, String> {
        &self.cmd_owners
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn eval_source_returning_defs(
        &mut self,
        source: String,
        builtin_names: std::collections::HashSet<String>,
        host: &mut dyn EditorHost,
    ) -> Result<Vec<SteelCmdDef>, String> {
        self.eval_source_raw(source, builtin_names, 10_000, host)
    }

    // ── Eval machinery ────────────────────────────────────────────────────────

    /// Evaluate `init.scm` at `path`, giving builtins access to editor state
    /// (settings, keymap) via `host` for the duration of the call.
    ///
    /// - Returns `Ok(defs)` if the file does not exist (empty defs, missing
    ///   config is normal) or if eval succeeds.  `defs` is the list of Steel
    ///   commands defined during eval; the caller registers them in the
    ///   `CommandRegistry`.
    /// - Returns `Err(message)` if the file exists but fails to parse or
    ///   evaluate.  The caller is responsible for surfacing the error.
    pub fn eval_init(
        &mut self,
        path: &Path,
        budget_ms: u64,
        host: &mut dyn EditorHost,
        builtin_names: std::collections::HashSet<String>,
    ) -> Result<Vec<SteelCmdDef>, String> {
        let source = match platform::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("reading {}: {e}", path.display())),
        };
        self.eval_source_raw(source, builtin_names, budget_ms, host)
    }

    /// Invoke a Steel proc by its internal engine name and return the list of
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
    /// the editor crate already depends on `steel-core` for `QueuedCommand`
    /// and `SteelBufferId`. Encapsulating Steel on this side of the API is not
    /// cost-free; the trade-off is accepted intentionally.
    pub fn call_steel_cmd<'a>(
        &'a mut self,
        steel_proc: &str,
        pending_char: Option<char>,
        args: Vec<SteelVal>,
        focused_pane_id: engine::pipeline::PaneId,
        focused_buffer_id: engine::pipeline::BufferId,
        host: &'a mut dyn EditorHost,
    ) -> Result<SteelCmdResult, String> {
        let budget_ms = host.steel_command_budget_ms();

        // Pre-bind positional args as *hume.ca{i}* globals, then build the
        // invocation string referencing them — mirrors the hook arg pattern.
        let invocation = if args.is_empty() {
            format!("({steel_proc})")
        } else {
            for (i, arg) in args.iter().enumerate() {
                self.engine.register_value(&cmd_arg_global_name(i), arg.clone());
            }
            let arg_refs: Vec<String> = (0..args.len()).map(cmd_arg_global_name).collect();
            format!("({steel_proc} {})", arg_refs.join(" "))
        };

        let (result, cmd_queue, wait_char_request, pending_language_sets, grammar_sweeps) = {
            let Self {
                engine,
                plugin_stack,
                cmd_owners,
                hooks,
                lazy_registry,
                declared_plugins,
                pending_messages,
                pending_language_regs,
                data_dir,
                runtime_dir,
                interrupt_flag,
                ..
            } = &mut *self;

            let mut steel_ctx = SteelCtx::new_command(
                host,
                HostBundle {
                    plugin_stack,
                    cmd_owners,
                    hooks,
                    lazy_registry,
                    declared_plugins,
                    pending_messages,
                    pending_language_regs,
                    data_dir: data_dir.as_deref(),
                    runtime_dir: runtime_dir.as_deref(),
                    interrupt_flag: Arc::clone(interrupt_flag),
                },
                focused_pane_id,
                focused_buffer_id,
                pending_char,
            );

            let result = run_steel(engine, &mut steel_ctx, invocation, budget_ms);
            (result, steel_ctx.cmd_queue, steel_ctx.wait_char_request, steel_ctx.pending_language_sets, steel_ctx.pending_grammar_sweeps)
        };

        // Null out arg globals — releases any Arc references and prevents stale
        // values leaking into later calls.
        for i in 0..args.len() {
            self.engine.update_value(&cmd_arg_global_name(i), SteelVal::Void);
        }

        result?;
        Ok(SteelCmdResult { cmd_queue, wait_char_request, pending_language_sets, grammar_sweeps })
    }

    /// Fire all registered handlers for `hook_id`, passing `args` to each.
    ///
    /// Handlers are called in registration order inside a single
    /// `with_mut_reference` session so they have full access to HUME builtins
    /// (`current-buffer`, `call!`, etc.).  Returns the combined `cmd_queue`
    /// from all handlers, or an empty vec if no handlers are registered.
    ///
    /// Returns immediately (no engine call, no watchdog) if no handlers are
    /// registered for `hook_id`.
    pub fn fire_hook<'a>(
        &'a mut self,
        hook_id: hooks::HookId,
        args: &[SteelVal],
        focused_pane_id: engine::pipeline::PaneId,
        focused_buffer_id: engine::pipeline::BufferId,
        host: &'a mut dyn EditorHost,
    ) -> Result<HookResult, String> {
        // Collect handler procs before borrowing self mutably for the SteelCtx.
        let handler_procs: Vec<SteelVal> = self.hooks.handlers_for(hook_id).to_vec();
        if handler_procs.is_empty() {
            return Ok(HookResult { cmd_queue: vec![], pending_language_sets: vec![], grammar_sweeps: vec![] });
        }

        let budget_ms = host.steel_command_budget_ms();

        // Pre-bind each arg global.
        for (i, arg) in args.iter().enumerate() {
            self.engine.register_value(&hook_arg_name(i), arg.clone());
        }

        // Pre-bind each handler proc global.
        for (i, proc) in handler_procs.iter().enumerate() {
            self.engine.register_value(&hook_proc_name(i), proc.clone());
        }

        // Look up (or build once) the composite invocation program.
        let program = self
            .hook_program_cache
            .entry((args.len(), handler_procs.len()))
            .or_insert_with(|| build_hook_program(args.len(), handler_procs.len()))
            .clone();

        let (result, cmd_queue, pending_language_sets, grammar_sweeps) = {
            let Self {
                engine,
                plugin_stack,
                cmd_owners,
                hooks,
                lazy_registry,
                declared_plugins,
                pending_messages,
                pending_language_regs,
                data_dir,
                runtime_dir,
                interrupt_flag,
                ..
            } = &mut *self;

            let mut steel_ctx = SteelCtx::new_command(
                host,
                HostBundle {
                    plugin_stack,
                    cmd_owners,
                    hooks,
                    lazy_registry,
                    declared_plugins,
                    pending_messages,
                    pending_language_regs,
                    data_dir: data_dir.as_deref(),
                    runtime_dir: runtime_dir.as_deref(),
                    interrupt_flag: Arc::clone(interrupt_flag),
                },
                focused_pane_id,
                focused_buffer_id,
                None,
            );

            let result = run_steel(engine, &mut steel_ctx, program, budget_ms);
            (result, steel_ctx.cmd_queue, steel_ctx.pending_language_sets, steel_ctx.pending_grammar_sweeps)
        };

        // Null out arg and proc globals before returning — releases Arc references
        // to closed buffers and prevents stale values leaking into later fires.
        for i in 0..args.len() {
            self.engine.update_value(&hook_arg_name(i), SteelVal::Void);
        }
        for i in 0..handler_procs.len() {
            self.engine.update_value(&hook_proc_name(i), SteelVal::Void);
        }

        result?;
        Ok(HookResult { cmd_queue, pending_language_sets, grammar_sweeps })
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
    pub fn eval_source(
        &mut self,
        source: &str,
        host: &mut dyn EditorHost,
    ) -> Result<(), String> {
        self.eval_source_raw(source.to_owned(), Default::default(), 10_000, host)
            .map(|_| ())
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

