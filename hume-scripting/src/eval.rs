//! Every way Rust runs Steel code: the [`ScriptingHost`] entry points, each
//! built on one or more sessions (`session`) and each returning the effects
//! its eval queued.

use std::path::Path;
use std::sync::Arc;

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;
use steel::steel_vm::engine::Engine;

use crate::command_name::CommandNameSet;
use crate::command_procs::CommandProc;
use crate::context::SteelCtx;
use crate::host::{EditorHost, HostToken};
use crate::log::LogLevel;
use crate::read_only_host::ReadOnlyHost;
use crate::types::{Effect, EvalError};
use crate::watchdog::EvalWatchdog;
use crate::{HostBundle, ScriptingHost, attribution, builtins};

mod session;

use session::{SessionError, run_steel_session};

/// The handlers of a held event at the moment it is raised, so the raiser
/// can open one hold per handler that takes `done` before any of them runs.
pub struct HeldHandlers {
    handlers: Vec<(SteelVal, bool)>,
}

impl HeldHandlers {
    /// How many holds [`ScriptingHost::fire_held_hook`] needs: one per
    /// handler that takes `done`.
    pub fn holds_needed(&self) -> usize {
        self.handlers
            .iter()
            .filter(|(_, takes_done)| *takes_done)
            .count()
    }
}

/// Which [`SteelCtx`] constructor a session's context comes from.
enum CtxKind {
    Init(CommandNameSet),
    Activation(CommandNameSet),
    Command,
}

impl CtxKind {
    fn build<'a>(self, host: &'a mut dyn EditorHost, bundle: HostBundle<'a>) -> SteelCtx<'a> {
        match self {
            Self::Init(names) => SteelCtx::new_init(host, bundle, names),
            Self::Activation(names) => SteelCtx::new_activation(host, bundle, names),
            Self::Command => SteelCtx::new_command(host, bundle),
        }
    }
}

impl ScriptingHost {
    /// Split `self` into the Steel engine, the watchdog, and the [`HostBundle`]
    /// of persistent borrows needed to build a [`SteelCtx`].
    ///
    /// The returned borrows are disjoint fields of `self` (NLL field-split), so
    /// the VM can be run while the bundle is lent to the eval context.
    fn steel_and_bundle(&mut self) -> (&mut Engine, &EvalWatchdog, HostBundle<'_>) {
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

    /// Runs `body` in one session on a `kind` context and takes the effects
    /// it queued, by [`Self::take_eval_effects`]'s contract.
    fn run_session(
        &mut self,
        host: &mut dyn EditorHost,
        kind: CtxKind,
        budget_ms: u64,
        body: impl FnOnce(&mut Engine) -> Result<(), SteelErr>,
    ) -> Result<Vec<Effect>, EvalError> {
        let effects_start = self.effects.len();
        let result = {
            let (steel, watchdog, bundle) = self.steel_and_bundle();
            let mut steel_ctx = kind.build(host, bundle);
            run_steel_session(steel, watchdog, &mut steel_ctx, budget_ms, body)
                .map_err(|e| e.message)
        };
        self.take_eval_effects(effects_start, result)
    }

    /// Core eval machinery.
    ///
    /// Evaluates `source` (init.scm) synchronously.  `(load-plugin! …)` calls
    /// inside the source activate their plugin bodies inline via the BOOTSTRAP
    /// `%activate-plugin-inline!` helper (VM-aware `hm.eval-string`, no
    /// `&mut Engine` borrow).  `(define-command! …)` calls register commands
    /// directly into the editor's `CommandRegistry` via `host.register_command`.
    ///
    /// Returns the effects this eval queued (atomically on success; on error,
    /// only effects committed by a nested successful plugin activation; see
    /// `ScriptingHost::take_eval_effects`).
    pub(crate) fn eval_source_raw(
        &mut self,
        source: String,
        builtin_names: crate::command_name::CommandNameSet,
        budget_ms: u64,
        host: &mut dyn EditorHost,
    ) -> Result<Vec<Effect>, EvalError> {
        self.run_session(host, CtxKind::Init(builtin_names), budget_ms, |steel| {
            steel.compile_and_run_raw_program(source).map(|_| ())
        })
    }

    /// Activate a plugin inline by a direct call to `%activate-plugin-inline!`.
    ///
    /// Used by event- and language-activation paths that
    /// fire outside any running eval and need their own watchdog.  The plugin body
    /// runs inside `hm.eval-string` (VM-aware, no `&mut Engine` borrow), sharing
    /// the same `ctx.registries` as any concurrent eval.  `define-command!` calls
    /// inside the body register directly into `host.register_command` inline.
    ///
    /// Returns the activating body's queued effects (`register-lsp-server!`,
    /// `set-buffer-option!`'s `"language"`, etc.) so the caller can apply them immediately.
    /// Otherwise they'd sit unapplied until some unrelated later drain, which can
    /// skip attaching the very buffer that triggered this activation.
    ///
    /// A failed activation's own effects are already rolled back by the
    /// BOOTSTRAP `%activate-plugin-inline!` wrapper's `%begin-lazy-activation!`/
    /// `%finish-lazy-activation!` mark/pop (`ctx.pop_effect_marks`) before the
    /// error reaches here, except any effects committed by a nested
    /// successful plugin activation, which `pop_effect_marks` keeps and which
    /// `take_eval_effects` salvages onto the returned `EvalError`.
    pub fn activate_plugin_inline(
        &mut self,
        id: &attribution::EntryId,
        budget_ms: u64,
        host: &mut dyn EditorHost,
        builtin_names: &crate::command_name::CommandNameSet,
    ) -> Result<Vec<Effect>, EvalError> {
        let args = vec![
            SteelVal::StringV(id.plugin.to_string().into()),
            SteelVal::StringV(id.file.as_str().into()),
        ];
        let kind = CtxKind::Activation(builtin_names.clone());
        self.run_session(host, kind, budget_ms, |steel| {
            steel
                .call_function_by_name_with_args("%activate-plugin-inline!", args)
                .map(|_| ())
        })
    }

    /// Evaluate `init.scm` at `path`, giving builtins access to editor state
    /// (settings, keymap) via `host` for the duration of the call.
    ///
    /// - Returns `Ok(effects)` if the file does not exist (missing config is
    ///   normal: `effects` is empty) or if eval succeeds, with every effect
    ///   this file's eval queued, in emission order.  Commands defined during
    ///   eval are registered into the `CommandRegistry` inline via
    ///   `host.register_command`. A plugin (`load-plugin!`/`declare-plugin!`)
    ///   body raising is contained here too, not a cause of `Err`: its own
    ///   activation is atomically rolled back (see
    ///   `builtins::plugins::activation::finish_lazy_activation`), reported by name and
    ///   location as an `Error`-severity pending message (drained by
    ///   `take_pending_messages`; queued from `run_steel_session`),
    ///   and the rest of the file keeps running. A broken third-party
    ///   plugin never blocks a `core:plum` declared later in the same file
    ///   from being reached.
    /// - Returns `Err(EvalError)` if the file exists but fails to parse, or
    ///   a top-level form outside any plugin/manifest body raises; the
    ///   failed eval's own queued effects are discarded, except any
    ///   committed by a nested successful plugin activation, which are
    ///   salvaged onto the error (see `take_eval_effects`).  The caller is
    ///   responsible for applying `EvalError::effects` and surfacing the
    ///   error.
    ///
    /// Atomicity is not all-or-nothing across the whole file even for a
    /// top-level raise: only deferred effects (keybinds, LSP
    /// registration, …) roll back on error. `define-command!` and
    /// `register-hook!` write the editor's command registry / `HookRegistry`
    /// inline the instant the builtin runs (see `builtins::commands`/`builtins::hooks`),
    /// so a `define-command!` or `register-hook!` that ran before the
    /// failing form stays live in the degraded session. A config error
    /// already surfaces to the user, and `:reload-config` rebuilds a fresh
    /// `ScriptingHost`, so the half-applied state never accumulates across
    /// reloads.
    pub fn eval_init(
        &mut self,
        path: &Path,
        budget_ms: u64,
        host: &mut dyn EditorHost,
        builtin_names: crate::command_name::CommandNameSet,
    ) -> Result<Vec<Effect>, EvalError> {
        self.dirs.init_dir = path.parent().map(Path::to_path_buf);
        self.eval_runtime(path, budget_ms, host, builtin_names)
    }

    /// Evaluate a bundled runtime Scheme file at `path` with `eval_init`'s
    /// contract, except that it does not set the local-plugin directory: only
    /// `eval_init` does, so a host resolves `./file.scm` plugins only after
    /// the user's `init.scm` has been evaluated on it.
    pub fn eval_runtime(
        &mut self,
        path: &Path,
        budget_ms: u64,
        host: &mut dyn EditorHost,
        builtin_names: crate::command_name::CommandNameSet,
    ) -> Result<Vec<Effect>, EvalError> {
        let source = match std::fs::read_to_string(path) {
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
    /// (`QueuedEffect::committed`, set by `SteelCtx::pop_effect_marks`).
    /// Those effects are salvaged into the returned `EvalError` so the caller
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

    /// Invoke a Steel command's proc and return the effects it queued, for
    /// the caller to apply. `proc` is the one on the registry entry the
    /// editor resolved, so no second lookup can disagree with it.
    ///
    /// A watchdog thread enforces `host.steel_command_budget_ms()`.  If the
    /// script runs past the budget, `(hume/yield!)` calls abort it (cooperative
    /// interruption).
    ///
    /// # Design note: Steel in the signature
    /// `args` is `Vec<SteelVal>` rather than an opaque owned type because the
    /// caller (editor dispatch) constructs the args by wrapping already-resolved
    /// Rust values via `IntoSteelVal` and passing them straight in. Introducing
    /// an intermediate arg type would add conversion with no practical benefit:
    /// the editor crate already depends on `steel-core` for `SteelPane`.
    /// Encapsulating Steel on this side of the API is not
    /// cost-free; the trade-off is accepted intentionally.
    pub fn call_steel_cmd<'a>(
        &'a mut self,
        proc: CommandProc,
        args: Vec<SteelVal>,
        host: &'a mut dyn EditorHost,
    ) -> Result<Vec<Effect>, EvalError> {
        let budget_ms = host.settings().steel_command_budget_ms();
        self.run_session(host, CtxKind::Command, budget_ms, |steel| {
            proc.call(steel, args)?;
            Ok(())
        })
    }

    /// [`Self::call_steel_cmd`] on the proc `host` holds for `name`, for tests
    /// that drive a command without the editor's dispatch. Panics if `host`
    /// has no Steel command named `name`.
    #[cfg(any(test, feature = "test-util"))]
    pub fn call_steel_cmd_by_name<'a>(
        &'a mut self,
        name: &str,
        args: Vec<SteelVal>,
        host: &'a mut dyn EditorHost,
    ) -> Result<Vec<Effect>, EvalError> {
        let proc = host
            .commands()
            .defined_command(name)
            .unwrap_or_else(|| panic!("host has no Steel command named '{name}'"));
        self.call_steel_cmd(proc, args, host)
    }

    /// Fire every handler for `name` that matches `key`, passing `args` to
    /// each.
    ///
    /// Handlers are called in registration order, each in its own
    /// `with_mut_reference` session (see [`Self::run_steel_calls`]) so they
    /// have full access to HUME builtins (`focused-pane`, `call!`, etc.).
    ///
    /// Returns immediately (no Steel engine call, no watchdog) if no handler
    /// for `name` matches `key`.
    ///
    /// # Panics
    ///
    /// If a matching handler takes `done`: a held event is raised through
    /// [`Self::fire_held_hook`].
    pub fn fire_hook<'a>(
        &'a mut self,
        name: &str,
        key: Option<&str>,
        args: &[SteelVal],
        host: &'a mut dyn EditorHost,
    ) -> Result<Vec<Effect>, EvalError> {
        // Every handler gets the same args: pair them up and hand the batch
        // to run_steel_calls, which already is the general "run these
        // (proc, args) pairs in one session, first error aborts the rest"
        // machinery this needs.
        let calls: Vec<(SteelVal, Vec<SteelVal>)> = self
            .registries
            .hooks
            .matching(name, key)
            .map(|e| {
                assert!(
                    !e.takes_done,
                    "'{name} has a handler taking `done`; raise it through fire_held_hook"
                );
                (e.proc.clone(), args.to_vec())
            })
            .collect();
        self.run_steel_calls(calls, host)
    }

    /// The handlers registered for the held event `name`
    /// (`EventHost::held_event_arg_count`).
    pub fn held_handlers(&self, name: &str) -> HeldHandlers {
        HeldHandlers {
            handlers: self
                .registries
                .hooks
                .matching(name, None)
                .map(|e| (e.proc.clone(), e.takes_done))
                .collect(),
        }
    }

    /// [`Self::fire_hook`] for a held event. A handler that takes `done`
    /// gets one of `holds`, in order, as the thunk's hold; a handler that
    /// raises, or never ran, releases its own through
    /// `EventHost::finish_hold`. Every other handler holds nothing.
    ///
    /// Panics if `holds` is not [`HeldHandlers::holds_needed`] long.
    pub fn fire_held_hook<'a>(
        &'a mut self,
        handlers: HeldHandlers,
        args: &[SteelVal],
        holds: Vec<HostToken>,
        host: &'a mut dyn EditorHost,
    ) -> Result<Vec<Effect>, EvalError> {
        const HELD_HANDLER: &str = "%call-held-handler";
        assert_eq!(
            holds.len(),
            handlers.holds_needed(),
            "one hold per handler taking `done`"
        );
        let helper = match self.steel.extract_value(HELD_HANDLER) {
            Ok(helper) => helper,
            Err(e) => {
                for hold in holds {
                    host.events().finish_hold(hold);
                }
                return Err(EvalError {
                    message: format!("{e}"),
                    effects: Vec::new(),
                });
            }
        };
        let args_list = SteelVal::ListV(args.to_vec().into());
        let mut holds = holds.into_iter();
        let mut held_at: Vec<Option<HostToken>> = Vec::with_capacity(handlers.handlers.len());
        let mut calls = Vec::with_capacity(handlers.handlers.len());
        for (proc, takes_done) in handlers.handlers {
            if takes_done {
                let hold = holds.next().expect("counted above");
                held_at.push(Some(hold));
                calls.push((
                    helper.clone(),
                    vec![proc, hold.to_steel(), args_list.clone()],
                ));
            } else {
                held_at.push(None);
                calls.push((proc, args.to_vec()));
            }
        }
        let (completed, result) = self.run_tracked_calls(calls, host);
        for (hold, completed) in held_at.into_iter().zip(completed) {
            if let (Some(hold), false) = (hold, completed) {
                host.events().finish_hold(hold);
            }
        }
        result
    }

    /// Calls each `(proc, args)` pair directly, in order: the shared
    /// mechanism behind the `lsp-request!` callback, timer thunks, and the
    /// prompt callback, and (via [`fire_hook`](Self::fire_hook)) every
    /// handler registered for one event.
    ///
    /// Each call runs in its *own* `with_mut_reference` session, isolated
    /// from its siblings: an ordinary Steel error (a raised exception, a
    /// stale-buffer `LivePane` raise, an arity mismatch) does not abort the
    /// rest of the batch, so one plugin's bug in one hook handler or queued
    /// callback can't silently drop every other one behind it in the same
    /// batch. A cooperative watchdog interrupt (`hume/yield!`, raised when
    /// the step budget is exceeded) is the one exception: it means the
    /// whole batch's time is spent, not just this call's, so it still
    /// aborts the rest, distinguished by the session's own `interrupted`
    /// flag, read off `ctx.interrupt_flag` before that session resets it,
    /// rather than sniffing the interrupt's own fixed message text.
    ///
    /// The *first* failure (interrupt or not) is returned as this function's
    /// own `Err`. Callers key cleanup on it and report it themselves. A
    /// *second* (or later) failure in the same batch would otherwise have no
    /// trace at all, since it doesn't abort anything. It is logged here
    /// directly, with the same `"steel call error: "` text, so it isn't
    /// silently dropped.
    ///
    /// Never called from inside a completion-detection borrow (LSP drain,
    /// timer drain, minibuffer key handling). The caller queues `(proc,
    /// args)` and this runs at the drain boundary.
    pub fn run_steel_calls<'a>(
        &'a mut self,
        calls: Vec<(SteelVal, Vec<SteelVal>)>,
        host: &'a mut dyn EditorHost,
    ) -> Result<Vec<Effect>, EvalError> {
        self.run_tracked_calls(calls, host).1
    }

    /// [`Self::run_steel_calls`] that also says, per call in order, whether
    /// it ran to completion: `false` for one that raised, and for one an
    /// interrupt kept from running at all.
    fn run_tracked_calls<'a>(
        &'a mut self,
        calls: Vec<(SteelVal, Vec<SteelVal>)>,
        host: &'a mut dyn EditorHost,
    ) -> (Vec<bool>, Result<Vec<Effect>, EvalError>) {
        let mut completed = vec![false; calls.len()];
        if calls.is_empty() {
            return (completed, Ok(Vec::new()));
        }

        let budget_ms = host.settings().steel_command_budget_ms();
        let effects_start = self.effects.len();
        let mut first_error: Option<String> = None;

        for (index, (proc, args)) in calls.into_iter().enumerate() {
            let (steel, watchdog, bundle) = self.steel_and_bundle();
            let mut steel_ctx = SteelCtx::new_command(host, bundle);
            let result = run_steel_session(steel, watchdog, &mut steel_ctx, budget_ms, |steel| {
                steel.call_function_with_args(proc, args).map(|_| ())
            });
            match result {
                Ok(()) => completed[index] = true,
                Err(SessionError {
                    message,
                    interrupted,
                }) => {
                    if first_error.is_none() {
                        first_error = Some(message);
                    } else {
                        steel_ctx.log(LogLevel::Error, format!("steel call error: {message}"));
                    }
                    if interrupted {
                        break;
                    }
                }
            }
        }

        let effects = self.take_eval_effects(effects_start, first_error.map_or(Ok(()), Err));
        (completed, effects)
    }

    /// Calls `proc` with `args` and returns its value: the one entry point
    /// for a proc whose result the editor uses, such as a drawer's row
    /// renderer. The proc sees `host` through `ReadOnlyHost`: it may read
    /// the editor but not change it, queue commands with `call!`, or start a
    /// declared plugin's activation. Any such attempt fails the call, even
    /// when the proc caught the error it raised, and whatever it queued is
    /// dropped. Runs under the
    /// same watchdog budget as a command.
    pub fn call_for_value(
        &mut self,
        proc: SteelVal,
        args: Vec<SteelVal>,
        host: &mut dyn EditorHost,
    ) -> Result<SteelVal, String> {
        self.read_only_call(host, |steel| steel.call_function_with_args(proc, args))
    }

    /// `name`'s bundled data, read by `bootstrap.scm`'s
    /// `%bundled-language-config` under [`call_for_value`](Self::call_for_value)'s
    /// read-only rules. `None` when there is no runtime directory or no file
    /// for `name`; a malformed file is an error naming it.
    pub fn bundled_language(
        &mut self,
        name: &str,
        host: &mut dyn EditorHost,
    ) -> Result<Option<hume_treesitter::registry::LanguageFields>, String> {
        const CONFIG: &str = "%bundled-language-config";
        let config = self.read_only_call(host, |steel| {
            steel.call_function_by_name_with_args(CONFIG, vec![SteelVal::StringV(name.into())])
        })?;
        builtins::syntax::optional_language_fields(config, CONFIG)
            .map_err(|e| format!("{name}.scm: {e}"))
    }

    /// Runs `call` in a session whose host is read-only and whose queued
    /// commands fail the call; the shared body of
    /// [`call_for_value`](Self::call_for_value) and
    /// [`bundled_language`](Self::bundled_language).
    pub(crate) fn read_only_call(
        &mut self,
        host: &mut dyn EditorHost,
        call: impl FnOnce(&mut Engine) -> Result<SteelVal, steel::rerrs::SteelErr>,
    ) -> Result<SteelVal, String> {
        let budget_ms = host.settings().steel_command_budget_ms();
        let mut read_only = ReadOnlyHost::new(host);
        let mut value = None;
        let queued = self
            .run_session(&mut read_only, CtxKind::Command, budget_ms, |steel| {
                value = Some(call(steel)?);
                Ok(())
            })
            .map_err(|e| e.message)?;
        if let Some(refused) = read_only.refused() {
            return Err(refused);
        }
        if !queued.is_empty() {
            return Err("commands cannot be queued during a read-only call".to_string());
        }
        Ok(value.expect("a session that succeeded ran the call"))
    }
}
