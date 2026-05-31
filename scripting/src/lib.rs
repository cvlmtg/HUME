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
pub(crate) use codegen::{HUME_CTX, cmd_arg_global_name};
pub(crate) use context::SteelCtx;

// ── Internal imports ──────────────────────────────────────────────────────────

use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use steel::rvals::SteelVal;
use steel::steel_vm::engine::Engine;

use hooks::HookRegistry;
use attribution::PluginStack;
use lazy::{LazyRegistry, PluginState};
use types::PendingSteelCmd;

use codegen::{build_hook_program, cmd_proc_name, hook_arg_name, hook_proc_name};

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

// ── run_steel ─────────────────────────────────────────────────────────────────

/// Arm the watchdog, run `program` inside `engine` with `ctx` visible as
/// `*hume.ctx*`, then cancel the watchdog and reset the interrupt flag.
///
/// Used by `eval_source_raw`, `call_steel_cmd`, and `fire_hook` to avoid
/// repeating the same arm / eval / cancel / reset ceremony in each entry point.
fn run_steel<'a>(
    engine: &mut Engine,
    ctx: &mut SteelCtx<'a>,
    program: String,
    budget_ms: u64,
) -> Result<(), String> {
    let watchdog = EvalWatchdog::arm(
        Arc::clone(&ctx.interrupt_flag),
        std::time::Duration::from_millis(budget_ms),
    );
    let result = engine
        .with_mut_reference::<SteelCtx<'a>, SteelCtx<'static>>(ctx)
        .consume_once(|engine, args| {
            let ctx_val = args
                .into_iter()
                .next()
                .expect("with_mut_reference yields one arg");
            engine.update_value(HUME_CTX, ctx_val);
            let res = engine.compile_and_run_raw_program(program);
            engine.update_value(HUME_CTX, SteelVal::Void);
            res
        })
        .map(|_| ())
        .map_err(|e| e.to_string());
    watchdog.cancel();
    ctx.interrupt_flag.store(false, Ordering::Relaxed);
    result
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
        self.eval_source_raw(source, builtin_names, host)
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
        host: &mut dyn EditorHost,
        builtin_names: std::collections::HashSet<String>,
    ) -> Result<Vec<SteelCmdDef>, String> {
        let source = match platform::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("reading {}: {e}", path.display())),
        };
        self.eval_source_raw(source, builtin_names, host)
    }

    /// Core eval machinery used by [`eval_init`].
    ///
    /// Evaluates `source` (init.scm) then, for each plugin queued by
    /// `(load-plugin …)` or `(declare-plugin …)` + explicit `(load-plugin …)`,
    /// submits `(require "<abs-path>")` on the same engine.  Each plugin is its
    /// own Steel module, so private helpers with the same name in different
    /// plugins are mangled to distinct globals and never collide.  Commands are
    /// drained between plugins so that a later plugin can bind keys to commands
    /// defined by an earlier one.
    fn eval_source_raw(
        &mut self,
        source: String,
        builtin_names: std::collections::HashSet<String>,
        host: &mut dyn EditorHost,
    ) -> Result<Vec<SteelCmdDef>, String> {
        let budget_ms = host.steel_init_budget_ms();

        // Step 1: eval init.scm.  Collect plugin IDs queued for activation from
        // `pending_plugin_loads` — populated by `%load-plugin!` (eager) and by
        // `%declare-plugin!` + `%load-plugin!` (force-activate after bare-declare).
        let (eval_result, init_cmds, pending_plugin_loads, startup_cmds) = {
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

            let mut steel_ctx = SteelCtx::new_init(
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
                builtin_names.clone(),
            );

            let result = run_steel(engine, &mut steel_ctx, source, budget_ms);
            (
                result,
                steel_ctx.pending_steel_cmds,
                steel_ctx.pending_plugin_loads,
                steel_ctx.cmd_queue,
            )
        };

        self.pending_startup_commands.extend(startup_cmds);
        eval_result?;

        let mut all_cmds = self.process_pending_cmds(init_cmds);

        // Step 2: activate each queued plugin via the shared activate_plugin path.
        // Steel's module system mangles the plugin's private bindings
        // (e.g. `##mm<id>~helper`), so same-named helpers in different plugins
        // live in disjoint globals.  Command lambdas close over their mangled
        // helpers and dispatch correctly via the name-based `CommandRegistry`.
        for id in pending_plugin_loads {
            all_cmds.extend(self.activate_plugin(&id, host, &builtin_names)?);
        }

        Ok(all_cmds)
    }

    /// Process `PendingSteelCmd`s collected during an eval:
    /// register each lambda in the engine's global namespace and record the
    /// owner in `cmd_owners`.  Returns the `SteelCmdDef`s for the caller to
    /// register in the `CommandRegistry`.
    fn process_pending_cmds(&mut self, pending: Vec<PendingSteelCmd>) -> Vec<SteelCmdDef> {
        let mut defs = Vec::new();
        for cmd in pending {
            let steel_proc = cmd_proc_name(&cmd.name);
            // Introspect arity before register_value takes ownership of cmd.proc.
            let (arity, is_variadic) = match &cmd.proc {
                SteelVal::Closure(gc) => (gc.arity() as u16, gc.is_multi_arity()),
                // FuncV/MutFunc are opaque native fns; treat as variadic so the
                // dispatcher never rejects them on arity grounds.
                _ => (0, true),
            };
            // Register (or overwrite) the lambda under its internal name.
            self.engine.register_value(&steel_proc, cmd.proc);
            // Record the owner string for `(command-plugin …)` introspection.
            self.cmd_owners
                .insert(cmd.name.clone(), cmd.current_owner.to_string());
            defs.push(SteelCmdDef {
                name: cmd.name,
                doc: cmd.doc,
                steel_proc,
                extendable: cmd.extendable,
                arity,
                is_variadic,
                inline_output: cmd.inline_output,
            });
        }
        defs
    }

    /// Evaluate a plugin body by requiring its file into the Steel engine.
    ///
    /// The plugin must be in the `Declared` state in `self.lazy_registry`;
    /// other states short-circuit:
    /// - `Loaded` / `Failed` — no-op (idempotent; `Failed` never retries).
    /// - `Loading` — no-op (re-entrancy guard: trigger cycle A→B→A skips).
    /// - Not present — no-op (plugin was absent on disk at declaration time).
    ///
    /// On success the state transitions to `Loaded` and the returned
    /// [`SteelCmdDef`]s are ready for insertion into the `CommandRegistry`.
    /// On error the state transitions to `Failed` and an `Err` is returned;
    /// eager callers (init path) propagate it to abort `eval_source_raw`, while
    /// lazy callers (dispatch path) catch it and push a soft error
    /// message instead.
    pub fn activate_plugin(
        &mut self,
        id: &attribution::PluginId,
        host: &mut dyn EditorHost,
        builtin_names: &std::collections::HashSet<String>,
    ) -> Result<Vec<SteelCmdDef>, String> {
        let budget_ms = host.steel_init_budget_ms();
        // Extract path from Declared state; short-circuit all other states.
        let path = match self.lazy_registry.plugins.get(id) {
            Some(PluginState::Declared { path }) => path.clone(),
            Some(PluginState::Loaded | PluginState::Failed | PluginState::Loading) | None => {
                return Ok(vec![]);
            }
        };

        let abs_str = path.to_string_lossy();
        if abs_str.contains('"') {
            self.lazy_registry
                .plugins
                .insert(id.clone(), PluginState::Failed);
            return Err(format!(
                "plugin path contains '\"' — cannot embed in require: {}",
                path.display()
            ));
        }
        let require_program = format!("(require \"{abs_str}\")");

        // Mark Loading before the eval so re-entrant activation of the same
        // plugin (via a trigger cycle) sees Loading and returns Ok(vec![]).
        self.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Loading);

        // Attribution: push before the require eval, pop after.
        self.plugin_stack.push(id.clone());

        let (plugin_result, plugin_cmds, requires, plugin_startup_cmds) = {
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

            let mut steel_ctx = SteelCtx::new_init(
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
                builtin_names.clone(),
            );

            let result = run_steel(engine, &mut steel_ctx, require_program, budget_ms);
            (result, steel_ctx.pending_steel_cmds, steel_ctx.pending_plugin_loads, steel_ctx.cmd_queue)
        };

        self.pending_startup_commands.extend(plugin_startup_cmds);

        self.plugin_stack.pop();

        match plugin_result {
            Ok(()) => {
                // Build parent defs while the parent is still `Loading` — the
                // cycle guard at the top of activate_plugin short-circuits any
                // re-entrant call for the same id.
                let mut defs = self.process_pending_cmds(plugin_cmds);
                // Drain transitive `(load-plugin …)` calls made by the body.
                // Activate them before finalising the parent so that a transitive
                // failure leaves the parent in `Failed` rather than an inconsistent
                // `Loaded`+no-commands state.
                for req in requires {
                    match self.activate_plugin(&req, host, builtin_names) {
                        Ok(d) => defs.extend(d),
                        Err(e) => {
                            self.lazy_registry
                                .plugins
                                .insert(id.clone(), PluginState::Failed);
                            self.lazy_registry.drop_triggers_for(id);
                            return Err(format!("loading plugin '{id}': transitive dep failed: {e}"));
                        }
                    }
                }
                self.lazy_registry
                    .plugins
                    .insert(id.clone(), PluginState::Loaded);
                // Drop all trigger-map entries — the real SteelBacked commands
                // are registered by the caller after this returns, and any Lazy
                // stub is overwritten by register_steel_cmds.  Trigger names the
                // body did NOT define are cleaned up by activate_lazy_plugin's
                // loop guard.
                self.lazy_registry.drop_triggers_for(id);
                Ok(defs)
            }
            Err(e) => {
                self.lazy_registry
                    .plugins
                    .insert(id.clone(), PluginState::Failed);
                // Drop trigger-map entries on failure so a spent trigger never
                // re-fires for a non-retrying plugin.
                self.lazy_registry.drop_triggers_for(id);
                Err(format!("loading plugin '{id}': {e}"))
            }
        }
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
        host: &'a mut dyn EditorHost,
    ) -> Result<SteelCmdResult, String> {
        let budget_ms = host.steel_command_budget_ms();
        let focused_pane_id = host.focused_pane_id();
        let focused_buffer_id = host.focused_buffer_id();

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
        host: &'a mut dyn EditorHost,
    ) -> Result<HookResult, String> {
        // Collect handler procs before borrowing self mutably for the SteelCtx.
        let handler_procs: Vec<SteelVal> = self.hooks.handlers_for(hook_id).to_vec();
        if handler_procs.is_empty() {
            return Ok(HookResult { cmd_queue: vec![], pending_language_sets: vec![], grammar_sweeps: vec![] });
        }

        let budget_ms = host.steel_command_budget_ms();
        let focused_pane_id = host.focused_pane_id();
        let focused_buffer_id = host.focused_buffer_id();

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
    /// empty `builtin_names`, which arms a watchdog using the default 10-second
    /// budget (harmless for normal tests that complete quickly).
    pub fn eval_source(
        &mut self,
        source: &str,
        host: &mut dyn EditorHost,
    ) -> Result<(), String> {
        self.eval_source_raw(source.to_owned(), Default::default(), host)
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
        // Temporarily override the init budget so the normal run_steel watchdog
        // path uses the requested budget.
        struct BudgetOverrideHost<'a> {
            inner: &'a mut dyn EditorHost,
            budget_ms: u64,
        }
        impl EditorHost for BudgetOverrideHost<'_> {
            fn focused_buffer_id(&self) -> engine::pipeline::BufferId { self.inner.focused_buffer_id() }
            fn focused_pane_id(&self) -> engine::pipeline::PaneId { self.inner.focused_pane_id() }
            fn buffer_ids(&self) -> Vec<engine::pipeline::BufferId> { self.inner.buffer_ids() }
            fn pane_ids(&self) -> Vec<engine::pipeline::PaneId> { self.inner.pane_ids() }
            fn buffer_exists(&self, id: engine::pipeline::BufferId) -> bool { self.inner.buffer_exists(id) }
            fn buffer_path(&self, id: engine::pipeline::BufferId) -> Option<std::path::PathBuf> { self.inner.buffer_path(id) }
            fn buffer_display_name(&self, id: engine::pipeline::BufferId) -> Option<String> { self.inner.buffer_display_name(id) }
            fn buffer_is_dirty(&self, id: engine::pipeline::BufferId) -> Option<bool> { self.inner.buffer_is_dirty(id) }
            fn buffer_stored_language(&self, id: engine::pipeline::BufferId) -> Option<String> { self.inner.buffer_stored_language(id) }
            fn open_buffer(&mut self, path: &std::path::Path) -> Result<engine::pipeline::BufferId, String> { self.inner.open_buffer(path) }
            fn close_buffer(&mut self, id: engine::pipeline::BufferId) -> engine::pipeline::BufferId { self.inner.close_buffer(id) }
            fn switch_to_buffer(&mut self, current: engine::pipeline::BufferId, target: engine::pipeline::BufferId) { self.inner.switch_to_buffer(current, target) }
            fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String> { self.inner.set_global_option(key, value) }
            fn configure_statusline(&mut self, left: Vec<String>, center: Vec<String>, right: Vec<String>) -> Result<(), String> { self.inner.configure_statusline(left, center, right) }
            fn bind_key(&mut self, mode: host::BindMode, keys: &[crossterm::event::KeyEvent], cmd: &str, force_extend: bool) -> Result<(), String> { self.inner.bind_key(mode, keys, cmd, force_extend) }
            fn bind_wait_char(&mut self, mode: host::BindMode, keys: &[crossterm::event::KeyEvent], cmd: &str) -> Result<(), String> { self.inner.bind_wait_char(mode, keys, cmd) }
            fn unbind_key(&mut self, mode: host::BindMode, keys: &[crossterm::event::KeyEvent]) -> Result<(), String> { self.inner.unbind_key(mode, keys) }
            fn attach_grammar(&mut self, name: &str, grammar_path: &std::path::Path, symbol: &str, highlights_path: &std::path::Path) -> Result<(), String> { self.inner.attach_grammar(name, grammar_path, symbol, highlights_path) }
            fn has_grammar(&self, language: &str) -> bool { self.inner.has_grammar(language) }
            fn is_valid_register_name(&self, ch: char) -> bool { self.inner.is_valid_register_name(ch) }
            fn steel_init_budget_ms(&self) -> u64 { self.budget_ms }
            fn steel_command_budget_ms(&self) -> u64 { self.inner.steel_command_budget_ms() }
        }
        let mut override_host = BudgetOverrideHost {
            inner: host,
            budget_ms: budget.as_millis() as u64,
        };
        self.eval_source_raw(source.to_owned(), Default::default(), &mut override_host)
            .map(|_| ())
    }
}

// ── Activation state-machine tests ───────────────────────────────────────────

#[cfg(test)]
mod activation_tests {
    use std::collections::HashSet;
    use std::io::Write as _;

    use tempfile::TempDir;

    use super::*;
    use crate::attribution::PluginId;
    use crate::lazy::PluginState;
    use crate::null_host::NullHost;

    /// Write a Steel source file into `dir` and return its path.
    fn write_plugin(dir: &TempDir, name: &str, src: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(src.as_bytes()).unwrap();
        path
    }

    fn plugin_id(name: &str) -> PluginId {
        PluginId::parse(name).unwrap()
    }

    fn no_builtins() -> HashSet<String> {
        HashSet::new()
    }

    // ── Case 1: Declared → Loaded with a valid command body ──────────────────

    #[test]
    fn declared_to_loaded_registers_command() {
        let dir = TempDir::new().unwrap();
        let path = write_plugin(
            &dir,
            "plugin.scm",
            r#"(define-command! "test-cmd" "A test command." (lambda () 0))"#,
        );
        let id = plugin_id("core:test");
        let mut host = ScriptingHost::new();
        host.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Declared { path });

        let defs = host.activate_plugin(&id, &mut NullHost, &no_builtins()).unwrap();

        assert_eq!(defs.len(), 1, "expected exactly one SteelCmdDef");
        assert_eq!(defs[0].name, "test-cmd");
        assert!(
            matches!(host.lazy_registry.plugins.get(&id), Some(PluginState::Loaded)),
            "plugin must be in Loaded state after successful activation"
        );
    }

    // ── Case 2: Syntax error → Failed, Err returned ──────────────────────────

    #[test]
    fn syntax_error_transitions_to_failed() {
        let dir = TempDir::new().unwrap();
        let path = write_plugin(&dir, "bad.scm", "(((invalid syntax");
        let id = plugin_id("core:bad");
        let mut host = ScriptingHost::new();
        host.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Declared { path });

        let result = host.activate_plugin(&id, &mut NullHost, &no_builtins());

        assert!(result.is_err(), "must return Err on syntax error");
        assert!(
            matches!(host.lazy_registry.plugins.get(&id), Some(PluginState::Failed)),
            "plugin must be in Failed state after syntax error"
        );
    }

    // ── Case 3: Idempotent no-ops for non-Declared states ────────────────────

    #[test]
    fn already_loaded_is_noop() {
        let id = plugin_id("core:loaded");
        let mut host = ScriptingHost::new();
        host.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Loaded);

        let defs = host.activate_plugin(&id, &mut NullHost, &no_builtins()).unwrap();

        assert!(defs.is_empty(), "Loaded plugin must be a no-op");
        assert!(
            matches!(host.lazy_registry.plugins.get(&id), Some(PluginState::Loaded)),
            "state must remain Loaded"
        );
    }

    #[test]
    fn already_failed_is_noop() {
        let id = plugin_id("core:failed");
        let mut host = ScriptingHost::new();
        host.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Failed);

        let defs = host.activate_plugin(&id, &mut NullHost, &no_builtins()).unwrap();

        assert!(defs.is_empty(), "Failed plugin must be a no-op");
        assert!(
            matches!(host.lazy_registry.plugins.get(&id), Some(PluginState::Failed)),
            "state must remain Failed"
        );
    }

    #[test]
    fn absent_plugin_is_noop() {
        let id = plugin_id("core:absent");
        let mut host = ScriptingHost::new();
        // Do not seed anything in lazy_registry.

        let defs = host.activate_plugin(&id, &mut NullHost, &no_builtins()).unwrap();

        assert!(defs.is_empty(), "absent plugin must be a no-op");
        assert!(
            host.lazy_registry.plugins.get(&id).is_none(),
            "absent plugin must not appear in registry after no-op"
        );
    }

    // ── Case 4: Loading re-entrancy guard → no-op ────────────────────────────

    #[test]
    fn loading_reentrancy_guard_is_noop() {
        let id = plugin_id("core:cycling");
        let mut host = ScriptingHost::new();
        host.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Loading);

        let defs = host.activate_plugin(&id, &mut NullHost, &no_builtins()).unwrap();

        assert!(defs.is_empty(), "Loading plugin must be a no-op (re-entrancy guard)");
        assert!(
            matches!(host.lazy_registry.plugins.get(&id), Some(PluginState::Loading)),
            "state must remain Loading (re-entrancy guard must not overwrite)"
        );
    }

    // ── Case 5: Transitive-dep failure rolls parent to Failed ─────────────────

    #[test]
    fn transitive_dep_failure_rolls_parent_to_failed() {
        let dir = TempDir::new().unwrap();
        // Plugin B has a syntax error.
        let path_b = write_plugin(&dir, "b.scm", "(((bad");
        // Plugin A's body loads B via the %load-plugin! primitive.
        // B is already in lazy_registry (seeded below), so %load-plugin! just
        // pushes "core:b" to pending_plugin_loads — no disk access needed.
        let path_a = write_plugin(&dir, "a.scm", r#"(%load-plugin! "core:b")"#);

        let id_a = plugin_id("core:a");
        let id_b = plugin_id("core:b");
        let mut host = ScriptingHost::new();
        host.lazy_registry
            .plugins
            .insert(id_a.clone(), PluginState::Declared { path: path_a });
        host.lazy_registry
            .plugins
            .insert(id_b.clone(), PluginState::Declared { path: path_b });

        let result = host.activate_plugin(&id_a, &mut NullHost, &no_builtins());

        assert!(result.is_err(), "transitive failure must propagate as Err");
        assert!(
            matches!(host.lazy_registry.plugins.get(&id_a), Some(PluginState::Failed)),
            "parent plugin A must be Failed when its dep B fails"
        );
        assert!(
            matches!(host.lazy_registry.plugins.get(&id_b), Some(PluginState::Failed)),
            "dep plugin B must itself be Failed"
        );
    }

    // ── Case 6: Path containing '"' rejected before any eval ─────────────────

    #[test]
    fn path_with_quote_char_transitions_to_failed() {
        let id = plugin_id("core:quoted");
        let mut host = ScriptingHost::new();
        host.lazy_registry.plugins.insert(
            id.clone(),
            PluginState::Declared {
                path: std::path::PathBuf::from("/some/path\"with/quote/plugin.scm"),
            },
        );

        let result = host.activate_plugin(&id, &mut NullHost, &no_builtins());

        assert!(result.is_err(), "path with '\"' must be rejected");
        assert!(
            matches!(host.lazy_registry.plugins.get(&id), Some(PluginState::Failed)),
            "plugin must be Failed after path-with-quote rejection"
        );
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

