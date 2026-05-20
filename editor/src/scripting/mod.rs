//! Steel scripting integration for HUME.
//!
//! The [`ScriptingHost`] owns the Steel [`Engine`] and runs entirely on the
//! main event-loop thread — Steel's Engine is `!Send` by design (internal
//! `Rc`/`RefCell`, non-atomic `im-rs` lists). This is a deliberate choice:
//! edit commands are synchronous `(Buffer, SelectionSet) → (Buffer, SelectionSet)`
//! operations on the hot-key path; an IPC round-trip per keystroke would be
//! strictly worse than a direct function call.
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

use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use steel::gc::unsafe_erased_pointers::CustomReference;
use steel::rvals::SteelVal;
use steel::steel_vm::engine::Engine;

use engine::pipeline::{BufferId, EngineView, PaneId};
use slotmap::SecondaryMap;

use crate::core::jump_list::JumpList;
use crate::editor::buffer_store::BufferStore;
use crate::editor::keymap::Keymap;
use crate::editor::pane_state::PaneBufferState;
use crate::settings::EditorSettings;

use hooks::HookRegistry;
use attribution::PluginStack;

// ── HUME_CTX global name ──────────────────────────────────────────────────────

/// Name of the Steel global that holds the `&mut SteelCtx` reference during
/// each eval or command call.  Builtins registered with
/// `register_fn_with_ctx(HUME_CTX, …)` receive this value as their first arg.
pub(crate) const HUME_CTX: &str = "*hume.ctx*";

/// Internal Steel global name for the lambda of a Steel-backed command.
fn cmd_proc_name(name: &str) -> String {
    format!("%hume-cmd-{name}")
}

/// Internal Steel global name for the i-th argument bound during a hook fire.
fn hook_arg_name(i: usize) -> String {
    format!("*hume.ha{i}*")
}

/// Internal Steel global name for the i-th handler proc bound during a hook fire.
fn hook_proc_name(i: usize) -> String {
    format!("*hume.hp{i}*")
}

/// Build the composite hook invocation program for `handler_count` handlers
/// and `arg_count` arguments.  The result is deterministic and cacheable.
fn build_hook_program(arg_count: usize, handler_count: usize) -> String {
    // 14 = len("*hume.ha99* ") worst-case per arg; 18 = len("(*hume.hp99*)\n") per handler.
    let mut arg_exprs = String::with_capacity(arg_count * 14);
    for i in 0..arg_count {
        if i > 0 {
            arg_exprs.push(' ');
        }
        arg_exprs.push_str(&hook_arg_name(i));
    }
    let mut program = String::with_capacity(handler_count * (18 + arg_exprs.len()));
    for i in 0..handler_count {
        if i > 0 {
            program.push('\n');
        }
        program.push('(');
        program.push_str(&hook_proc_name(i));
        if arg_count > 0 {
            program.push(' ');
            program.push_str(&arg_exprs);
        }
        program.push(')');
    }
    program
}

// ── EvalWatchdog ──────────────────────────────────────────────────────────────

/// Arms a wall-clock budget for a single Steel eval.
///
/// When the budget expires the interrupt flag is set to `true`, signalling
/// `(hume/yield!)` calls inside the script to abort.  Interruption is
/// cooperative only — Steel 0.8.2 has no op-callback for involuntary stop.
///
/// Use `park_timeout` so [`EvalWatchdog::cancel`] wakes the thread
/// immediately on the happy path rather than sleeping out the full budget.
pub(crate) struct EvalWatchdog {
    cancel: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}

impl EvalWatchdog {
    /// Spawn the watchdog.  Will flip `flag` to `true` after `budget` unless
    /// cancelled first.
    fn arm(flag: Arc<AtomicBool>, budget: std::time::Duration) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let thread = {
            let flag = Arc::clone(&flag);
            let cancel = Arc::clone(&cancel);
            std::thread::spawn(move || {
                // park_timeout wakes either when unpark() is called (cancel path)
                // or when the budget elapses — whichever comes first.
                std::thread::park_timeout(budget);
                if !cancel.load(Ordering::Relaxed) {
                    flag.store(true, Ordering::Relaxed);
                }
            })
        };
        Self { cancel, thread }
    }

    /// Defuse: signal cancellation, wake the thread, and join.
    /// Always called after eval returns — on both success and error paths.
    fn cancel(self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.thread.thread().unpark();
        // Propagate panics from the watchdog thread; otherwise ignore join errors.
        let _ = self.thread.join();
    }
}

// ── SteelCtx ──────────────────────────────────────────────────────────────────

/// A `(define-command! …)` call captured during `eval_init`, to be processed
/// after the eval completes.
pub(crate) struct PendingSteelCmd {
    pub(crate) name: String,
    pub(crate) doc: String,
    /// The Steel lambda, captured at `define-command!` call time.
    pub(crate) proc: steel::rvals::SteelVal,
    /// Attribution owner at call time — stored in `cmd_owners` for `(command-plugin …)`.
    pub(crate) current_owner: attribution::Owner,
    /// Whether this command participates in sticky-Ctrl extend.
    /// Set by `(define-command-extend! …)`.
    pub(crate) extendable: bool,
}

/// A Steel command that has been fully registered in the engine and is ready
/// to be inserted into the `CommandRegistry`.
///
/// Returned by [`ScriptingHost::eval_init`] and
/// [`ScriptingHost::eval_plugin_with_attribution`]; the editor layer registers
/// the commands after a successful eval.
pub(crate) struct SteelCmdDef {
    pub(crate) name: String,
    pub(crate) doc: String,
    /// Name under which the lambda is bound in Steel's global namespace
    /// (e.g. `"%hume-cmd-my-command"`).  Used by
    /// [`crate::scripting::ScriptingHost::call_steel_cmd`] at dispatch time.
    pub(crate) steel_proc: String,
    pub(crate) extendable: bool,
}

/// Context struct borrowed into the Steel engine for the duration of each eval
/// or command call via Steel's `with_mut_reference` API.
///
/// All persistent scripting state (hooks, attribution, etc.) is held directly
/// on [`ScriptingHost`] and borrowed here by reference — no `mem::take`/put-back
/// needed.  Transient per-eval state (accumulators, mode flags, multi-buffer
/// borrows) is owned.
///
/// Builtins registered with `register_fn_with_ctx(HUME_CTX, …)` receive
/// `&mut SteelCtx` as their first argument, injected automatically by Steel.
pub(crate) struct SteelCtx<'a> {
    // ── Persistent state borrowed from ScriptingHost ──────────────────────────
    /// Editor settings — mutated by `(set-option! …)` during init.
    pub(crate) settings: &'a mut EditorSettings,
    /// Keymap — mutated by `(bind-key! …)` during init.
    pub(crate) keymap: &'a mut Keymap,
    /// Plugin attribution stack; identifies whose mutation is being recorded.
    pub(crate) plugin_stack: &'a mut PluginStack,
    /// Command-owner index; read by `(command-plugin …)`, written by
    /// [`ScriptingHost::process_pending_cmds`].
    pub(crate) cmd_owners: &'a mut std::collections::HashMap<String, String>,
    /// Hook registry; `(register-hook! …)` writes directly.
    pub(crate) hooks: &'a mut HookRegistry,
    /// Log messages accumulated by `(log! …)`.
    pub(crate) pending_messages: &'a mut Vec<(crate::editor::Severity, String)>,
    /// Where PLUM installs third-party plugins (`$XDG_DATA_HOME/hume/`).
    pub(crate) data_dir: Option<&'a std::path::Path>,
    /// Where core plugins, themes, and docs live.
    pub(crate) runtime_dir: Option<&'a std::path::Path>,
    // ── Transient per-eval state (owned) ──────────────────────────────────────
    /// Every plugin name passed to `(load-plugin …)`, including absent ones.
    pub(crate) declared_plugins: Vec<String>,
    /// Plugins that were both declared and successfully located on disk.
    pub(crate) loaded_plugins: Vec<String>,
    /// Built-in command names known at eval start.  `define-command!` checks
    /// against this to prevent shadowing core commands.
    pub(crate) builtin_cmd_names: std::collections::HashSet<String>,
    /// `(define-command! …)` calls accumulated during this eval.
    pub(crate) pending_steel_cmds: Vec<PendingSteelCmd>,
    /// Interrupt flag shared with the `EvalWatchdog`.
    pub(crate) interrupt_flag: Arc<AtomicBool>,
    // ── Command-mode fields (meaningful only when is_init = false) ────────────
    /// Commands queued by `(call! …)`.
    pub(crate) cmd_queue: Vec<String>,
    /// WaitChar command requested by `(request-wait-char! …)`.
    pub(crate) wait_char_request: Option<String>,
    /// Pending char from a WaitChar keymap node.
    pub(crate) pending_char: Option<char>,
    /// Command-line argument from `:cmd arg` invocation.
    pub(crate) cmd_arg: Option<String>,
    // ── Mode discriminant ────────────────────────────────────────────────────
    /// `true` during `eval_source_raw` (init.scm / plugin load);
    /// `false` during `call_steel_cmd` (command dispatch).
    /// Builtins that mutate config (`set-option!`, `bind-key!`, etc.) check
    /// this and raise a Steel error when called from command bodies.
    pub(crate) is_init: bool,
    // ── Multi-buffer focus snapshot ──────────────────────────────────────────
    pub(crate) focused_pane_id: PaneId,
    pub(crate) focused_buffer_id: BufferId,
    /// Tracks the live focused buffer across mutations within one command call.
    /// Starts equal to `focused_buffer_id`; updated by `switch-to-buffer!` and
    /// `close-buffer!` so subsequent builtins see the new current buffer.
    pub(crate) live_focused_buffer_id: BufferId,
    pub(crate) buffers: Option<&'a mut BufferStore>,
    pub(crate) engine_view: Option<&'a mut EngineView>,
    pub(crate) pane_state:
        Option<&'a mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>>,
    pub(crate) pane_jumps: Option<&'a mut SecondaryMap<PaneId, JumpList>>,
}

impl CustomReference for SteelCtx<'_> {}
steel::custom_reference!(SteelCtx<'a>);

impl<'a> SteelCtx<'a> {
    fn new_init(
        host: HostBundle<'a>,
        settings: &'a mut EditorSettings,
        keymap: &'a mut Keymap,
        builtin_cmd_names: std::collections::HashSet<String>,
    ) -> Self {
        Self {
            settings,
            keymap,
            plugin_stack: host.plugin_stack,
            cmd_owners: host.cmd_owners,
            hooks: host.hooks,
            pending_messages: host.pending_messages,
            data_dir: host.data_dir,
            runtime_dir: host.runtime_dir,
            declared_plugins: Vec::new(),
            loaded_plugins: Vec::new(),
            builtin_cmd_names,
            pending_steel_cmds: Vec::new(),
            interrupt_flag: host.interrupt_flag,
            cmd_queue: Vec::new(),
            wait_char_request: None,
            pending_char: None,
            cmd_arg: None,
            is_init: true,
            focused_pane_id: PaneId::default(),
            focused_buffer_id: BufferId::default(),
            live_focused_buffer_id: BufferId::default(),
            buffers: None,
            engine_view: None,
            pane_state: None,
            pane_jumps: None,
        }
    }

    /// Push a log message — prefer this over direct `pending_messages.push` so
    /// any future severity filter is applied uniformly.
    pub(crate) fn log(&mut self, severity: crate::editor::Severity, msg: String) {
        self.pending_messages.push((severity, msg));
    }

    fn new_command(
        host: HostBundle<'a>,
        refs: EditorSteelRefs<'a>,
        pending_char: Option<char>,
        cmd_arg: Option<String>,
    ) -> Self {
        let fid = refs.focused_buffer_id;
        Self {
            settings: refs.settings,
            keymap: refs.keymap,
            plugin_stack: host.plugin_stack,
            cmd_owners: host.cmd_owners,
            hooks: host.hooks,
            pending_messages: host.pending_messages,
            data_dir: host.data_dir,
            runtime_dir: host.runtime_dir,
            declared_plugins: Vec::new(),
            loaded_plugins: Vec::new(),
            builtin_cmd_names: std::collections::HashSet::new(),
            pending_steel_cmds: Vec::new(),
            interrupt_flag: host.interrupt_flag,
            cmd_queue: Vec::new(),
            wait_char_request: None,
            pending_char,
            cmd_arg,
            is_init: false,
            focused_pane_id: refs.focused_pane_id,
            focused_buffer_id: fid,
            live_focused_buffer_id: fid,
            buffers: refs.buffers,
            engine_view: refs.engine_view,
            pane_state: refs.pane_state,
            pane_jumps: refs.pane_jumps,
        }
    }
}

/// Backing storage for [`SteelCtx`] in unit tests.
///
/// Because `SteelCtx<'a>` borrows all persistent state by reference, tests
/// need owned storage to borrow from.  Create one of these, then call
/// [`SteelCtxTestHarness::ctx`] to get a `SteelCtx<'_>` that borrows from it.
#[cfg(test)]
pub(crate) struct SteelCtxTestHarness {
    pub(crate) settings: EditorSettings,
    pub(crate) keymap: Keymap,
    pub(crate) plugin_stack: PluginStack,
    pub(crate) cmd_owners: std::collections::HashMap<String, String>,
    pub(crate) hooks: HookRegistry,
    pub(crate) pending_messages: Vec<(crate::editor::Severity, String)>,
    pub(crate) data_dir: Option<PathBuf>,
    pub(crate) runtime_dir: Option<PathBuf>,
    pub(crate) interrupt_flag: Arc<AtomicBool>,
}

#[cfg(test)]
impl SteelCtxTestHarness {
    pub(crate) fn new() -> Self {
        Self {
            settings: EditorSettings::default(),
            keymap: Keymap::default(),
            plugin_stack: PluginStack::default(),
            cmd_owners: std::collections::HashMap::new(),
            hooks: HookRegistry::default(),
            pending_messages: Vec::new(),
            data_dir: None,
            runtime_dir: None,
            interrupt_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Build a `SteelCtx` in command mode (`is_init = false`) borrowing from
    /// this harness.  Inspect harness fields after the call to read side-effects.
    pub(crate) fn ctx(&mut self) -> SteelCtx<'_> {
        let Self {
            settings,
            keymap,
            plugin_stack,
            cmd_owners,
            hooks,
            pending_messages,
            data_dir,
            runtime_dir,
            interrupt_flag,
        } = self;
        SteelCtx::new_command(
            HostBundle {
                plugin_stack,
                cmd_owners,
                hooks,
                pending_messages,
                data_dir: data_dir.as_deref(),
                runtime_dir: runtime_dir.as_deref(),
                interrupt_flag: Arc::clone(interrupt_flag),
            },
            EditorSteelRefs {
                settings,
                keymap,
                focused_pane_id: PaneId::default(),
                focused_buffer_id: BufferId::default(),
                buffers: None,
                engine_view: None,
                pane_state: None,
                pane_jumps: None,
            },
            None,
            None,
        )
    }
}

// ── EditorSteelRefs / HostBundle ─────────────────────────────────────────────

/// Editor-side references bundled for a single Steel eval in command mode.
///
/// Passed to [`ScriptingHost::call_steel_cmd`] and [`ScriptingHost::fire_hook`]
/// to replace the previous 8-parameter sprawl.  All fields have the same
/// lifetime `'a` so a single `'a` annotation on those methods suffices.
pub(crate) struct EditorSteelRefs<'a> {
    pub(crate) settings: &'a mut EditorSettings,
    pub(crate) keymap: &'a mut Keymap,
    pub(crate) focused_pane_id: PaneId,
    pub(crate) focused_buffer_id: BufferId,
    pub(crate) buffers: Option<&'a mut BufferStore>,
    pub(crate) engine_view: Option<&'a mut EngineView>,
    pub(crate) pane_state:
        Option<&'a mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>>,
    pub(crate) pane_jumps: Option<&'a mut SecondaryMap<PaneId, JumpList>>,
}

/// Borrows of [`ScriptingHost`] fields needed to populate [`SteelCtx`].
///
/// Built from a `let Self { engine, plugin_stack, … } = &mut *self` destructure
/// and passed to [`SteelCtx::new_init`] or [`SteelCtx::new_command`].
/// Private to this module.
struct HostBundle<'a> {
    plugin_stack: &'a mut PluginStack,
    cmd_owners: &'a mut std::collections::HashMap<String, String>,
    hooks: &'a mut HookRegistry,
    pending_messages: &'a mut Vec<(crate::editor::Severity, String)>,
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
    /// Log messages accumulated by `(log! …)` since the last drain.
    /// Drained by the editor after each `eval_init` / `call_steel_cmd` call.
    pub(crate) pending_messages: Vec<(crate::editor::Severity, String)>,
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
            pending_messages: Vec::new(),
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
    /// `(load-plugin …)`, submits `(require "<abs-path>")` on the same engine.
    /// Each plugin is its own Steel module, so private helpers with the same
    /// name in different plugins are mangled to distinct globals and never
    /// collide.  Commands are drained between plugins so that a later plugin
    /// can bind keys to commands defined by an earlier one.
    fn eval_source_raw(
        &mut self,
        source: String,
        builtin_names: std::collections::HashSet<String>,
        settings: &mut EditorSettings,
        keymap: &mut Keymap,
    ) -> Result<Vec<SteelCmdDef>, String> {
        let budget_ms = settings.steel_init_budget_ms as u64;

        // Phase 1: eval init.scm.  Collect queued plugin names from
        // `loaded_plugins` — populated by `(push-loaded-plugin! …)` inside
        // the Scheme `load-plugin` wrapper.
        let (eval_result, init_cmds, loaded_plugins) = {
            let Self {
                engine,
                plugin_stack,
                cmd_owners,
                hooks,
                pending_messages,
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
                    pending_messages,
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
                steel_ctx.loaded_plugins,
            )
        };

        eval_result?;

        let mut all_cmds = self.process_pending_cmds(init_cmds);

        // Phase 2: load each plugin as a separate Steel module.
        // `(require "<abs>")` is submitted as a fresh top-level program on the
        // same engine.  Steel's module system mangles the plugin's private
        // bindings (e.g. `##mm<id>~helper`), so same-named helpers in
        // different plugins live in disjoint globals.  Command lambdas close
        // over their mangled helpers and dispatch correctly via the name-based
        // `CommandRegistry`.
        for plugin_name in loaded_plugins {
            let plugin_id = attribution::PluginId::parse(&plugin_name)
                .map_err(|e| format!("plugin '{plugin_name}': {e}"))?;

            let abs_path = match builtins::plugins::resolve_path_for_name(
                &plugin_name,
                self.runtime_dir.as_deref(),
                self.data_dir.as_deref(),
            ) {
                Ok(Some(p)) => p,
                // Plugin was on disk during init.scm but has since vanished — skip.
                Ok(None) => continue,
                Err(e) => return Err(format!("plugin '{plugin_name}': {e}")),
            };

            let abs_str = abs_path.to_string_lossy();
            // Paths from resolve_path_for_name come from our own dirs and
            // plugin IDs (validated by PluginId::parse), so double-quotes
            // are not expected.  Error loudly rather than silently corrupt
            // the require string.
            if abs_str.contains('"') {
                return Err(format!(
                    "plugin path contains '\"' — cannot embed in require: {}",
                    abs_path.display()
                ));
            }
            let require_program = format!("(require \"{abs_str}\")");

            // Attribution: push before the require eval, pop after.
            // Rust drives push/pop instead of Scheme's dynamic-wind so that
            // we control the stack across the module boundary.
            self.plugin_stack.push(plugin_id);

            let (plugin_result, plugin_cmds) = {
                let Self {
                    engine,
                    plugin_stack,
                    cmd_owners,
                    hooks,
                    pending_messages,
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
                        pending_messages,
                        data_dir: data_dir.as_deref(),
                        runtime_dir: runtime_dir.as_deref(),
                        interrupt_flag: Arc::clone(interrupt_flag),
                    },
                    settings,
                    keymap,
                    builtin_names.clone(),
                );

                let result = run_steel(engine, &mut steel_ctx, require_program, budget_ms);
                (result, steel_ctx.pending_steel_cmds)
            };

            self.plugin_stack.pop();

            match plugin_result {
                Ok(()) => all_cmds.extend(self.process_pending_cmds(plugin_cmds)),
                Err(e) => return Err(format!("loading plugin '{plugin_name}': {e}")),
            }
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
            });
        }
        defs
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
        cmd_arg: Option<String>,
        refs: EditorSteelRefs<'a>,
    ) -> Result<(Vec<String>, Option<String>), String> {
        let budget_ms = refs.settings.steel_command_budget_ms as u64;
        let invocation = format!("({steel_proc})");

        let (result, cmd_queue, wait_char_request) = {
            let Self {
                engine,
                plugin_stack,
                cmd_owners,
                hooks,
                pending_messages,
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
                    pending_messages,
                    data_dir: data_dir.as_deref(),
                    runtime_dir: runtime_dir.as_deref(),
                    interrupt_flag: Arc::clone(interrupt_flag),
                },
                refs,
                pending_char,
                cmd_arg,
            );

            let result = run_steel(engine, &mut steel_ctx, invocation, budget_ms);
            (result, steel_ctx.cmd_queue, steel_ctx.wait_char_request)
        };

        result?;
        Ok((cmd_queue, wait_char_request))
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
    ) -> Result<Vec<String>, String> {
        // Collect handler procs before borrowing self mutably for the SteelCtx.
        let handler_procs: Vec<SteelVal> =
            self.hooks.handlers_for(hook_id).iter().cloned().collect();
        if handler_procs.is_empty() {
            return Ok(vec![]);
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

        let (result, cmd_queue) = {
            let Self {
                engine,
                plugin_stack,
                cmd_owners,
                hooks,
                pending_messages,
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
                    pending_messages,
                    data_dir: data_dir.as_deref(),
                    runtime_dir: runtime_dir.as_deref(),
                    interrupt_flag: Arc::clone(interrupt_flag),
                },
                refs,
                None,
                None,
            );

            let result = run_steel(engine, &mut steel_ctx, program, budget_ms);
            (result, steel_ctx.cmd_queue)
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
        Ok(cmd_queue)
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
