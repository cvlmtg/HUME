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
//! - `builtins/`: `set-option!`, `bind-key!`, `define-command!`, multi-buffer ops,
//!   `(configure-statusline! …)`, `(hume/yield!)` step-budget interruption.

pub(crate) mod builtins;
pub(crate) mod hooks;
pub(crate) mod keys;
pub(crate) mod attribution;
pub(crate) mod lazy;
mod codegen;
mod context;
mod types;
mod watchdog;
#[cfg(test)]
mod test_harness;

// ── Re-exports ────────────────────────────────────────────────────────────────

pub(crate) use codegen::{HUME_CTX, cmd_arg_global_name};
pub(crate) use context::SteelCtx;
pub(crate) use types::{
    EditorSteelRefs, HookResult, PendingLanguageReg, PendingSteelCmd, SteelCmdDef, SteelCmdResult,
};
#[cfg(test)]
pub(crate) use test_harness::SteelCtxTestHarness;

// ── Internal imports ──────────────────────────────────────────────────────────

use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use steel::rvals::SteelVal;
use steel::steel_vm::engine::Engine;

use crate::editor::keymap::Keymap;
use crate::settings::EditorSettings;

use hooks::HookRegistry;
use attribution::PluginStack;
use lazy::{LazyRegistry, PluginState};

use codegen::{build_hook_program, cmd_proc_name, hook_arg_name, hook_proc_name};
use watchdog::EvalWatchdog;

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
    pending_messages: &'a mut Vec<(crate::editor::Severity, String)>,
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
/// Owns the [`Engine`] and all persistent scripting state.  Each eval or
/// command call constructs a [`SteelCtx`] that borrows the persistent fields
/// directly — no `mem::take`/put-back needed.
///
/// Constructed once during `Editor::init_scripting()` and held for the
/// lifetime of the process.
pub(crate) struct ScriptingHost {
    engine: Engine,
    /// Attribution stack: `stack.last()` is the plugin currently executing.
    /// Empty → top-level `init.scm` → `Owner::User`.
    pub(crate) plugin_stack: PluginStack,
    /// Command-to-owner index: maps each Steel-registered command name to a
    /// display string (`"hume"`, `"user"`, or a plugin id like `"core:plum"`).
    /// Populated by `process_pending_cmds`; queried by `(command-plugin name)`.
    pub(crate) cmd_owners: std::collections::HashMap<String, String>,
    /// Persistent hook registry: handlers registered by `(register-hook! …)`.
    pub(crate) hooks: HookRegistry,
    /// Lazy plugin registry: populated by `%declare-plugin!` during init;
    /// trigger maps consulted by command dispatch, event firing, and language-set.
    pub(crate) lazy_registry: LazyRegistry,
    /// Every plugin name passed to `(load-plugin …)` or `(declare-plugin …)`,
    /// including plugins absent on disk.  Persists across evals so that
    /// `(declared-plugins)` returns the full init-time list at command time (PLUM).
    pub(crate) declared_plugins: Vec<String>,
    /// Log messages accumulated by `(log! …)` since the last drain.
    /// Drained by the editor after each `eval_init` / `call_steel_cmd` call.
    pub(crate) pending_messages: Vec<(crate::editor::Severity, String)>,
    /// Language identity registrations queued by `(define-language! …)`.
    /// Drained by `Editor::flush_pending_language_regs` after each `eval_init` boundary.
    pub(crate) pending_language_regs: Vec<PendingLanguageReg>,
    /// `$XDG_DATA_HOME/hume/` — where PLUM installs user/third-party plugins.
    pub(crate) data_dir: Option<PathBuf>,
    /// The runtime directory (core plugins, themes, docs), or `None` if absent.
    pub(crate) runtime_dir: Option<PathBuf>,
    /// Shared interrupt flag.  Set to `true` by the watchdog to signal that
    /// `(hume/yield!)` calls should abort the running script.  Reset to
    /// `false` after every `eval_init` call.
    pub(crate) interrupt_flag: Arc<AtomicBool>,
    /// Cache of pre-built hook invocation programs keyed by
    /// `(arg_count, handler_count)`.  The program text is deterministic given
    /// those two values, so it is built once and reused across fires.
    hook_program_cache: std::collections::HashMap<(usize, usize), String>,
}

impl ScriptingHost {
    /// Evaluate a Steel source string directly, without a file.
    ///
    /// Convenience wrapper for testing.  Delegates to `eval_source_raw` with
    /// empty `builtin_names`, which arms a watchdog using the default 10-second
    /// budget (harmless for normal tests that complete quickly).
    #[cfg(test)]
    pub(crate) fn eval_source(
        &mut self,
        source: &str,
        settings: &mut EditorSettings,
        keymap: &mut Keymap,
    ) -> Result<(), String> {
        self.eval_source_raw(source.to_owned(), Default::default(), settings, keymap)
            .map(|_| ())
    }

    /// Create a new scripting host with the Steel standard library and all HUME
    /// builtins loaded.
    ///
    /// Resolves base directories eagerly so builtins can use them without
    /// re-reading environment variables on every call.
    pub(crate) fn new() -> Self {
        let data_dir = crate::os::dirs::data_dir();
        let runtime_dir = crate::os::dirs::runtime_dir();
        // Initialize the fs builtin directory TLS before the engine registers
        // builtins — the `data-dir` / `runtime-dir` / sandbox functions read
        // from this TLS whenever they are called.
        builtins::fs::init_dirs(data_dir.clone(), runtime_dir.clone());
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
            data_dir,
            runtime_dir,
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            hook_program_cache: std::collections::HashMap::new(),
        }
    }

    /// Evaluate `init.scm` at `path`, giving builtins access to `settings` and
    /// `keymap` for the duration of the call.
    ///
    /// - Returns `Ok(defs)` if the file does not exist (empty defs, missing
    ///   config is normal) or if eval succeeds.  `defs` is the list of Steel
    ///   commands defined during eval; the caller registers them in the
    ///   `CommandRegistry`.
    /// - Returns `Err(message)` if the file exists but fails to parse or
    ///   evaluate.  The caller is responsible for surfacing the error.
    ///
    /// `settings` and `keymap` are moved into a [`SteelCtx`] before evaluation
    /// and restored afterwards — even on error.  Builtins such as `set-option!`
    /// and `bind-key!` mutate them through the borrowed reference.
    ///
    /// `builtin_names` is the set of all command names currently in the
    /// registry.  `define-command!` checks against this to prevent shadowing.
    pub(crate) fn eval_init(
        &mut self,
        path: &Path,
        settings: &mut EditorSettings,
        keymap: &mut Keymap,
        builtin_names: std::collections::HashSet<String>,
    ) -> Result<Vec<SteelCmdDef>, String> {
        let source = match crate::os::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("reading {}: {e}", path.display())),
        };
        self.eval_source_raw(source, builtin_names, settings, keymap)
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
        settings: &mut EditorSettings,
        keymap: &mut Keymap,
    ) -> Result<Vec<SteelCmdDef>, String> {
        let budget_ms = settings.steel_init_budget_ms as u64;

        // Step 1: eval init.scm.  Collect plugin IDs queued for activation from
        // `pending_plugin_loads` — populated by `%load-plugin!` (eager) and by
        // `%declare-plugin!` + `%load-plugin!` (force-activate after bare-declare).
        let (eval_result, init_cmds, pending_plugin_loads) = {
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
                settings,
                keymap,
                builtin_names.clone(),
            );

            let result = run_steel(engine, &mut steel_ctx, source, budget_ms);
            (
                result,
                steel_ctx.pending_steel_cmds,
                steel_ctx.pending_plugin_loads,
            )
        };

        eval_result?;

        let mut all_cmds = self.process_pending_cmds(init_cmds);

        // Step 2: activate each queued plugin via the shared activate_plugin path.
        // Steel's module system mangles the plugin's private bindings
        // (e.g. `##mm<id>~helper`), so same-named helpers in different plugins
        // live in disjoint globals.  Command lambdas close over their mangled
        // helpers and dispatch correctly via the name-based `CommandRegistry`.
        for id in pending_plugin_loads {
            all_cmds.extend(
                self.activate_plugin(&id, settings, keymap, &builtin_names, budget_ms)?,
            );
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
    /// The plugin must be in [`PluginState::Declared`] in `self.lazy_registry`;
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
    pub(crate) fn activate_plugin(
        &mut self,
        id: &attribution::PluginId,
        settings: &mut EditorSettings,
        keymap: &mut Keymap,
        builtin_names: &std::collections::HashSet<String>,
        budget_ms: u64,
    ) -> Result<Vec<SteelCmdDef>, String> {
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

        let (plugin_result, plugin_cmds, requires) = {
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
                settings,
                keymap,
                builtin_names.clone(),
            );

            let result = run_steel(engine, &mut steel_ctx, require_program, budget_ms);
            (result, steel_ctx.pending_steel_cmds, steel_ctx.pending_plugin_loads)
        };

        self.plugin_stack.pop();

        match plugin_result {
            Ok(()) => {
                self.lazy_registry
                    .plugins
                    .insert(id.clone(), PluginState::Loaded);
                // Drop all trigger-map entries for this plugin — the real SteelBacked
                // commands are registered by the caller after this returns, and the
                // stub (Lazy) entry is overwritten by register_steel_cmds.  Any
                // trigger names the body did NOT define are cleaned up by
                // activate_lazy_plugin's loop guard.
                self.lazy_registry.drop_triggers_for(id);
                let mut defs = self.process_pending_cmds(plugin_cmds);
                // Drain transitive `(load-plugin …)` calls made by the body
                // (queued in pending_plugin_loads). The Loading/Loaded guards
                // in activate_plugin prevent cycles.
                for req in requires {
                    defs.extend(
                        self.activate_plugin(&req, settings, keymap, builtin_names, budget_ms)?,
                    );
                }
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
    /// The caller (`SteelBacked` dispatch arm in `editor/mappings.rs`) executes
    /// the returned commands and, if a wait-char was requested, enters WaitChar
    /// mode for that command.
    ///
    /// A watchdog thread enforces `settings.steel_command_budget_ms`.  If the
    /// script runs past the budget, `(hume/yield!)` calls abort it (cooperative
    /// interruption).
    ///
    /// No rollback on error: `is_init` is `false` during this call, so
    /// `(set-option!)`, `(bind-key!)`, and similar init-only builtins raise a
    /// Steel error when called from a command body.  Commands that queue further
    /// Rust commands via `(call! …)` dispatch those after returning `Ok`; on
    /// error the queue is dropped, so no further dispatch occurs.
    pub(crate) fn call_steel_cmd<'a>(
        &'a mut self,
        steel_proc: &str,
        pending_char: Option<char>,
        args: Vec<SteelVal>,
        refs: EditorSteelRefs<'a>,
    ) -> Result<SteelCmdResult, String> {
        let budget_ms = refs.settings.steel_command_budget_ms as u64;

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

        let (result, cmd_queue, wait_char_request, pending_language_sets) = {
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
                refs,
                pending_char,
            );

            let result = run_steel(engine, &mut steel_ctx, invocation, budget_ms);
            (result, steel_ctx.cmd_queue, steel_ctx.wait_char_request, steel_ctx.pending_language_sets)
        };

        // Null out arg globals — releases any Arc references and prevents stale
        // values leaking into later calls.
        for i in 0..args.len() {
            self.engine.update_value(&cmd_arg_global_name(i), SteelVal::Void);
        }

        result?;
        Ok(SteelCmdResult { cmd_queue, wait_char_request, pending_language_sets })
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
    pub(crate) fn fire_hook<'a>(
        &'a mut self,
        hook_id: hooks::HookId,
        args: &[SteelVal],
        refs: EditorSteelRefs<'a>,
    ) -> Result<HookResult, String> {
        // Collect handler procs before borrowing self mutably for the SteelCtx.
        let handler_procs: Vec<SteelVal> = self.hooks.handlers_for(hook_id).to_vec();
        if handler_procs.is_empty() {
            return Ok(HookResult { cmd_queue: vec![], pending_language_sets: vec![] });
        }

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

        let budget_ms = refs.settings.steel_command_budget_ms as u64;

        let (result, cmd_queue, pending_language_sets) = {
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
                refs,
                None,
            );

            let result = run_steel(engine, &mut steel_ctx, program, budget_ms);
            (result, steel_ctx.cmd_queue, steel_ctx.pending_language_sets)
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
        Ok(HookResult { cmd_queue, pending_language_sets })
    }
}

// ── Test helpers ──────────────────────────────────────────────────────────────

#[cfg(test)]
impl ScriptingHost {
    /// Like [`eval_source`] but also arms a real [`EvalWatchdog`] with the
    /// given budget.  Used by watchdog-specific tests that need to verify the
    /// watchdog actually fires rather than pre-setting the interrupt flag.
    ///
    /// Sets `settings.steel_init_budget_ms` for the duration and restores it
    /// afterwards so other settings state is not polluted.
    pub(crate) fn eval_source_watchdog(
        &mut self,
        source: &str,
        budget: std::time::Duration,
        settings: &mut EditorSettings,
        keymap: &mut Keymap,
    ) -> Result<(), String> {
        let saved_budget = settings.steel_init_budget_ms;
        settings.steel_init_budget_ms = budget.as_millis() as usize;
        let result = self.eval_source_raw(source.to_owned(), Default::default(), settings, keymap);
        settings.steel_init_budget_ms = saved_budget;
        result.map(|_| ())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
