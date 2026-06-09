//! Plugin activation and core eval machinery for [`super::ScriptingHost`].
//!
//! ## Activation state machine
//!
//! ```text
//! Declared ──── activate_plugin ────► Loading ──┬──► Loaded
//!                                               └──► Failed
//!
//! Loaded / Failed / Loading / absent  ──► Ok(vec![])  (no-op)
//! ```
//!
//! `eval_source_raw` drives `init.scm` evaluation and then calls
//! `activate_plugin` for every `(load-plugin …)` call discovered inside the
//! source.  `activate_plugin` is also the lazy-activation entry point invoked
//! by the editor when a command trigger, event trigger, or language trigger
//! fires at runtime.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use steel::rvals::SteelVal;

use crate::attribution;
use crate::codegen::{HUME_CTX, cmd_proc_name};
use crate::context::SteelCtx;
use crate::host::EditorHost;
use crate::lazy::PluginState;
use crate::types::{PendingSteelCmd, SteelCmdDef};
use crate::watchdog::EvalWatchdog;
use crate::{HostBundle, ScriptingHost};

// ── run_steel ──────────────────────────────────��──────────────────────────────

/// Arm the watchdog, run `program` inside `engine` with `ctx` visible as
/// `*hume.ctx*`, then cancel the watchdog and reset the interrupt flag.
///
/// Used by `eval_source_raw`, `call_steel_cmd`, and `fire_hook` to avoid
/// repeating the same arm / eval / cancel / reset ceremony in each entry point.
pub(crate) fn run_steel<'a>(
    steel: &mut steel::steel_vm::engine::Engine,
    ctx: &mut SteelCtx<'a>,
    program: String,
    budget_ms: u64,
) -> Result<(), String> {
    let watchdog = EvalWatchdog::arm(
        Arc::clone(&ctx.interrupt_flag),
        std::time::Duration::from_millis(budget_ms),
    );
    let result = steel
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

// ── ScriptingHost — activation impl ─────────���─────────────────────────────���──

impl ScriptingHost {
    /// Core eval machinery used by [`ScriptingHost::eval_init`].
    ///
    /// Evaluates `source` (init.scm) then, for each plugin queued by
    /// `(load-plugin …)` or `(declare-plugin …)` + explicit `(load-plugin …)`,
    /// submits `(require "<abs-path>")` on the same engine.  Each plugin is its
    /// own Steel module, so private helpers with the same name in different
    /// plugins are mangled to distinct globals and never collide.  Commands are
    /// drained between plugins so that a later plugin can bind keys to commands
    /// defined by an earlier one.
    pub(crate) fn eval_source_raw(
        &mut self,
        source: String,
        builtin_names: HashSet<String>,
        budget_ms: u64,
        host: &mut dyn EditorHost,
    ) -> Result<Vec<SteelCmdDef>, String> {

        // Step 1: eval init.scm.  Collect plugin IDs queued for activation from
        // `pending_plugin_loads` — populated by `%load-plugin!` (eager) and by
        // `%declare-plugin!` + `%load-plugin!` (force-activate after bare-declare).
        let (eval_result, init_cmds, pending_plugin_loads, startup_cmds) = {
            let Self {
                steel,
                registries,
                plugin_stack,
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
                    registries,
                    plugin_stack,
                    pending_messages,
                    pending_language_regs,
                    data_dir: data_dir.as_deref(),
                    runtime_dir: runtime_dir.as_deref(),
                    interrupt_flag: Arc::clone(interrupt_flag),
                },
                builtin_names.clone(),
            );

            let result = run_steel(steel, &mut steel_ctx, source, budget_ms);
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
            all_cmds.extend(self.activate_plugin(&id, budget_ms, host, &builtin_names)?);
        }

        Ok(all_cmds)
    }

    /// Process `PendingSteelCmd`s collected during an eval:
    /// register each lambda in the engine's global namespace and record the
    /// owner in `cmd_owners`.  Returns the `SteelCmdDef`s for the caller to
    /// register in the `CommandRegistry`.
    pub(crate) fn process_pending_cmds(&mut self, pending: Vec<PendingSteelCmd>) -> Vec<SteelCmdDef> {
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
            self.steel.register_value(&steel_proc, cmd.proc);
            // Record the owner string for `(command-plugin …)` introspection.
            self.registries.cmd_owners
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
    /// The plugin must be in the `Declared` state in `self.registries.lazy_registry`;
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
        budget_ms: u64,
        host: &mut dyn EditorHost,
        builtin_names: &HashSet<String>,
    ) -> Result<Vec<SteelCmdDef>, String> {
        // Extract path from Declared state; short-circuit all other states.
        let path = match self.registries.lazy_registry.plugins.get(id) {
            Some(PluginState::Declared { path }) => path.clone(),
            Some(PluginState::Loaded | PluginState::Failed | PluginState::Loading) | None => {
                return Ok(vec![]);
            }
        };

        let abs_str = path.to_string_lossy();
        if abs_str.contains('"') {
            self.registries.lazy_registry
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
        self.registries.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Loading);

        // Attribution: push before the require eval, pop after.
        self.plugin_stack.push(id.clone());

        let (plugin_result, plugin_cmds, requires, plugin_startup_cmds) = {
            let Self {
                steel,
                registries,
                plugin_stack,
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
                    registries,
                    plugin_stack,
                    pending_messages,
                    pending_language_regs,
                    data_dir: data_dir.as_deref(),
                    runtime_dir: runtime_dir.as_deref(),
                    interrupt_flag: Arc::clone(interrupt_flag),
                },
                builtin_names.clone(),
            );

            let result = run_steel(steel, &mut steel_ctx, require_program, budget_ms);
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
                    match self.activate_plugin(&req, budget_ms, host, builtin_names) {
                        Ok(d) => defs.extend(d),
                        Err(e) => {
                            self.registries.lazy_registry
                                .plugins
                                .insert(id.clone(), PluginState::Failed);
                            self.registries.lazy_registry.drop_triggers_for(id);
                            return Err(format!("loading plugin '{id}': transitive dep failed: {e}"));
                        }
                    }
                }
                self.registries.lazy_registry
                    .plugins
                    .insert(id.clone(), PluginState::Loaded);
                // Drop all trigger-map entries — the real SteelBacked commands
                // are registered by the caller after this returns, and any Lazy
                // stub is overwritten by register_steel_cmds.  Trigger names the
                // body did NOT define are cleaned up by activate_lazy_plugin's
                // loop guard.
                self.registries.lazy_registry.drop_triggers_for(id);
                Ok(defs)
            }
            Err(e) => {
                self.registries.lazy_registry
                    .plugins
                    .insert(id.clone(), PluginState::Failed);
                // Drop trigger-map entries on failure so a spent trigger never
                // re-fires for a non-retrying plugin.
                self.registries.lazy_registry.drop_triggers_for(id);
                Err(format!("loading plugin '{id}': {e}"))
            }
        }
    }
}

// ── Tests ─────���───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::io::Write as _;

    use tempfile::TempDir;

    use crate::ScriptingHost;
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
        host.registries.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Declared { path });

        let defs = host.activate_plugin(&id, 10_000, &mut NullHost, &no_builtins()).unwrap();

        assert_eq!(defs.len(), 1, "expected exactly one SteelCmdDef");
        assert_eq!(defs[0].name, "test-cmd");
        assert!(
            matches!(host.registries.lazy_registry.plugins.get(&id), Some(PluginState::Loaded)),
            "plugin must be in Loaded state after successful activation"
        );
    }

    // ── Case 2: Syntax error → Failed, Err returned ────��─────────────────────

    #[test]
    fn syntax_error_transitions_to_failed() {
        let dir = TempDir::new().unwrap();
        let path = write_plugin(&dir, "bad.scm", "(((invalid syntax");
        let id = plugin_id("core:bad");
        let mut host = ScriptingHost::new();
        host.registries.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Declared { path });

        let result = host.activate_plugin(&id, 10_000, &mut NullHost, &no_builtins());

        assert!(result.is_err(), "must return Err on syntax error");
        assert!(
            matches!(host.registries.lazy_registry.plugins.get(&id), Some(PluginState::Failed)),
            "plugin must be in Failed state after syntax error"
        );
    }

    // ── Case 3: Idempotent no-ops for non-Declared states ────────────────────

    #[test]
    fn already_loaded_is_noop() {
        let id = plugin_id("core:loaded");
        let mut host = ScriptingHost::new();
        host.registries.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Loaded);

        let defs = host.activate_plugin(&id, 10_000, &mut NullHost, &no_builtins()).unwrap();

        assert!(defs.is_empty(), "Loaded plugin must be a no-op");
        assert!(
            matches!(host.registries.lazy_registry.plugins.get(&id), Some(PluginState::Loaded)),
            "state must remain Loaded"
        );
    }

    #[test]
    fn already_failed_is_noop() {
        let id = plugin_id("core:failed");
        let mut host = ScriptingHost::new();
        host.registries.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Failed);

        let defs = host.activate_plugin(&id, 10_000, &mut NullHost, &no_builtins()).unwrap();

        assert!(defs.is_empty(), "Failed plugin must be a no-op");
        assert!(
            matches!(host.registries.lazy_registry.plugins.get(&id), Some(PluginState::Failed)),
            "state must remain Failed"
        );
    }

    #[test]
    fn absent_plugin_is_noop() {
        let id = plugin_id("core:absent");
        let mut host = ScriptingHost::new();
        // Do not seed anything in lazy_registry.

        let defs = host.activate_plugin(&id, 10_000, &mut NullHost, &no_builtins()).unwrap();

        assert!(defs.is_empty(), "absent plugin must be a no-op");
        assert!(
            !host.registries.lazy_registry.plugins.contains_key(&id),
            "absent plugin must not appear in registry after no-op"
        );
    }

    // ── Case 4: Loading re-entrancy guard → no-op ────────────────���───────────

    #[test]
    fn loading_reentrancy_guard_is_noop() {
        let id = plugin_id("core:cycling");
        let mut host = ScriptingHost::new();
        host.registries.lazy_registry
            .plugins
            .insert(id.clone(), PluginState::Loading);

        let defs = host.activate_plugin(&id, 10_000, &mut NullHost, &no_builtins()).unwrap();

        assert!(defs.is_empty(), "Loading plugin must be a no-op (re-entrancy guard)");
        assert!(
            matches!(host.registries.lazy_registry.plugins.get(&id), Some(PluginState::Loading)),
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
        host.registries.lazy_registry
            .plugins
            .insert(id_a.clone(), PluginState::Declared { path: path_a });
        host.registries.lazy_registry
            .plugins
            .insert(id_b.clone(), PluginState::Declared { path: path_b });

        let result = host.activate_plugin(&id_a, 10_000, &mut NullHost, &no_builtins());

        assert!(result.is_err(), "transitive failure must propagate as Err");
        assert!(
            matches!(host.registries.lazy_registry.plugins.get(&id_a), Some(PluginState::Failed)),
            "parent plugin A must be Failed when its dep B fails"
        );
        assert!(
            matches!(host.registries.lazy_registry.plugins.get(&id_b), Some(PluginState::Failed)),
            "dep plugin B must itself be Failed"
        );
    }

    // ── Case 6: Path containing '"' rejected before any eval ─────────────────

    #[test]
    fn path_with_quote_char_transitions_to_failed() {
        let id = plugin_id("core:quoted");
        let mut host = ScriptingHost::new();
        host.registries.lazy_registry.plugins.insert(
            id.clone(),
            PluginState::Declared {
                path: std::path::PathBuf::from("/some/path\"with/quote/plugin.scm"),
            },
        );

        let result = host.activate_plugin(&id, 10_000, &mut NullHost, &no_builtins());

        assert!(result.is_err(), "path with '\"' must be rejected");
        assert!(
            matches!(host.registries.lazy_registry.plugins.get(&id), Some(PluginState::Failed)),
            "plugin must be Failed after path-with-quote rejection"
        );
    }
}
