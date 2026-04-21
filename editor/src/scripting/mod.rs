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
//! - `ledger.rs`: plugin ownership ledger + attribution stack for teardown.
//! - `hooks.rs`: `HookRegistry` + typed `HookId` enum.
//! - `builtins/`: `set-option!`, `bind-key!`, `define-command!`, multi-buffer ops,
//!   `(configure-statusline! …)`, `(hume/yield!)` step-budget interruption.

pub(crate) mod builtins;
pub(crate) mod hooks;
pub(crate) mod keys;
pub(crate) mod ledger;

use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use steel::steel_vm::engine::Engine;
use steel::gc::unsafe_erased_pointers::CustomReference;
use steel::rvals::SteelVal;

use std::borrow::Cow;

use engine::pipeline::{BufferId, EngineView, PaneId};
use slotmap::SecondaryMap;

use crate::core::jump_list::JumpList;
use crate::editor::buffer_store::BufferStore;
use crate::editor::keymap::{BindMode, Keymap};
use crate::editor::pane_state::PaneBufferState;
use crate::settings::{apply_setting, BufferOverrides, EditorSettings, SettingScope};

use hooks::HookRegistry;
use ledger::{LedgerStack, PluginId, PluginStack};

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
fn hook_arg_name(i: usize) -> String { format!("*hume.ha{i}*") }

/// Internal Steel global name for the i-th handler proc bound during a hook fire.
fn hook_proc_name(i: usize) -> String { format!("*hume.hp{i}*") }

/// Build the composite hook invocation program for `handler_count` handlers
/// and `arg_count` arguments.  The result is deterministic and cacheable.
fn build_hook_program(arg_count: usize, handler_count: usize) -> String {
    // 14 = len("*hume.ha99* ") worst-case per arg; 18 = len("(*hume.hp99*)\n") per handler.
    let mut arg_exprs = String::with_capacity(arg_count * 14);
    for i in 0..arg_count {
        if i > 0 { arg_exprs.push(' '); }
        arg_exprs.push_str(&hook_arg_name(i));
    }
    let mut program = String::with_capacity(handler_count * (18 + arg_exprs.len()));
    for i in 0..handler_count {
        if i > 0 { program.push('\n'); }
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
            let flag   = Arc::clone(&flag);
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
    /// Attribution owner at call time (for ledger recording).
    pub(crate) current_owner: ledger::Owner,
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
}

/// Context struct borrowed into the Steel engine for the duration of each eval
/// or command call via Steel's `with_mut_reference` API.
///
/// All persistent scripting state (plugin ledger, hooks, etc.) is held directly
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
    /// Ordered ledger of all plugin mutations, used for unload/reload teardown.
    pub(crate) ledger_stack: &'a mut LedgerStack,
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
            plugin_stack:           host.plugin_stack,
            ledger_stack:           host.ledger_stack,
            cmd_owners:             host.cmd_owners,
            hooks:                  host.hooks,
            pending_messages:       host.pending_messages,
            data_dir:               host.data_dir,
            runtime_dir:            host.runtime_dir,
            declared_plugins:       Vec::new(),
            loaded_plugins:         Vec::new(),
            builtin_cmd_names,
            pending_steel_cmds:     Vec::new(),
            interrupt_flag:         host.interrupt_flag,
            cmd_queue:              Vec::new(),
            wait_char_request:      None,
            pending_char:           None,
            cmd_arg:                None,
            is_init:                true,
            focused_pane_id:        PaneId::default(),
            focused_buffer_id:      BufferId::default(),
            live_focused_buffer_id: BufferId::default(),
            buffers:                None,
            engine_view:            None,
            pane_state:             None,
            pane_jumps:             None,
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
            settings:               refs.settings,
            keymap:                 refs.keymap,
            plugin_stack:           host.plugin_stack,
            ledger_stack:           host.ledger_stack,
            cmd_owners:             host.cmd_owners,
            hooks:                  host.hooks,
            pending_messages:       host.pending_messages,
            data_dir:               host.data_dir,
            runtime_dir:            host.runtime_dir,
            declared_plugins:       Vec::new(),
            loaded_plugins:         Vec::new(),
            builtin_cmd_names:      std::collections::HashSet::new(),
            pending_steel_cmds:     Vec::new(),
            interrupt_flag:         host.interrupt_flag,
            cmd_queue:              Vec::new(),
            wait_char_request:      None,
            pending_char,
            cmd_arg,
            is_init:                false,
            focused_pane_id:        refs.focused_pane_id,
            focused_buffer_id:      fid,
            live_focused_buffer_id: fid,
            buffers:                refs.buffers,
            engine_view:            refs.engine_view,
            pane_state:             refs.pane_state,
            pane_jumps:             refs.pane_jumps,
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
    pub(crate) settings:         EditorSettings,
    pub(crate) keymap:           Keymap,
    pub(crate) plugin_stack:     PluginStack,
    pub(crate) ledger_stack:     LedgerStack,
    pub(crate) cmd_owners:       std::collections::HashMap<String, String>,
    pub(crate) hooks:            HookRegistry,
    pub(crate) pending_messages: Vec<(crate::editor::Severity, String)>,
    pub(crate) data_dir:         Option<PathBuf>,
    pub(crate) runtime_dir:      Option<PathBuf>,
    pub(crate) interrupt_flag:   Arc<AtomicBool>,
}

#[cfg(test)]
impl SteelCtxTestHarness {
    pub(crate) fn new() -> Self {
        Self {
            settings:         EditorSettings::default(),
            keymap:           Keymap::default(),
            plugin_stack:     PluginStack::default(),
            ledger_stack:     LedgerStack::default(),
            cmd_owners:       std::collections::HashMap::new(),
            hooks:            HookRegistry::default(),
            pending_messages: Vec::new(),
            data_dir:         None,
            runtime_dir:      None,
            interrupt_flag:   Arc::new(AtomicBool::new(false)),
        }
    }

    /// Build a `SteelCtx` in command mode (`is_init = false`) borrowing from
    /// this harness.  Inspect harness fields after the call to read side-effects.
    pub(crate) fn ctx(&mut self) -> SteelCtx<'_> {
        let Self { settings, keymap, plugin_stack, ledger_stack, cmd_owners, hooks,
                   pending_messages, data_dir, runtime_dir, interrupt_flag } = self;
        SteelCtx::new_command(
            HostBundle {
                plugin_stack,
                ledger_stack,
                cmd_owners,
                hooks,
                pending_messages,
                data_dir:       data_dir.as_deref(),
                runtime_dir:    runtime_dir.as_deref(),
                interrupt_flag: Arc::clone(interrupt_flag),
            },
            EditorSteelRefs {
                settings,
                keymap,
                focused_pane_id:   PaneId::default(),
                focused_buffer_id: BufferId::default(),
                buffers:           None,
                engine_view:       None,
                pane_state:        None,
                pane_jumps:        None,
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
    pub(crate) settings:          &'a mut EditorSettings,
    pub(crate) keymap:            &'a mut Keymap,
    pub(crate) focused_pane_id:   PaneId,
    pub(crate) focused_buffer_id: BufferId,
    pub(crate) buffers:           Option<&'a mut BufferStore>,
    pub(crate) engine_view:       Option<&'a mut EngineView>,
    pub(crate) pane_state:        Option<&'a mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>>,
    pub(crate) pane_jumps:        Option<&'a mut SecondaryMap<PaneId, JumpList>>,
}

/// Borrows of [`ScriptingHost`] fields needed to populate [`SteelCtx`].
///
/// Built from a `let Self { engine, plugin_stack, … } = &mut *self` destructure
/// and passed to [`SteelCtx::new_init`] or [`SteelCtx::new_command`].
/// Private to this module.
struct HostBundle<'a> {
    plugin_stack:     &'a mut PluginStack,
    ledger_stack:     &'a mut LedgerStack,
    cmd_owners:       &'a mut std::collections::HashMap<String, String>,
    hooks:            &'a mut HookRegistry,
    pending_messages: &'a mut Vec<(crate::editor::Severity, String)>,
    data_dir:         Option<&'a std::path::Path>,
    runtime_dir:      Option<&'a std::path::Path>,
    /// Owned `Arc` clone: `new_init`/`new_command` consume it via move into
    /// `SteelCtx::interrupt_flag`, avoiding a second clone at eval time.
    interrupt_flag:   Arc<AtomicBool>,
}

// ── run_steel ─────────────────────────────────────────────────────────────────

/// Arm the watchdog, run `program` inside `engine` with `ctx` visible as
/// `*hume.ctx*`, then cancel the watchdog and reset the interrupt flag.
///
/// Used by `eval_source_raw`, `call_steel_cmd`, and `fire_hook` to avoid
/// repeating the same arm / eval / cancel / reset ceremony in each entry point.
fn run_steel<'a>(
    engine:     &mut Engine,
    ctx:        &mut SteelCtx<'a>,
    program:    String,
    budget_ms:  u64,
) -> Result<(), String> {
    let watchdog = EvalWatchdog::arm(
        Arc::clone(&ctx.interrupt_flag),
        std::time::Duration::from_millis(budget_ms),
    );
    let result = engine
        .with_mut_reference::<SteelCtx<'a>, SteelCtx<'static>>(ctx)
        .consume_once(|engine, args| {
            let ctx_val = args.into_iter().next().expect("with_mut_reference yields one arg");
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

// ── EvalSnapshot ─────────────────────────────────────────────────────────────

/// Captured state for all-or-nothing rollback of a Steel eval on error.
///
/// Constructed before an eval via [`EvalSnapshot::capture`]; on success the
/// snapshot is simply dropped (or ignored). On error, call
/// [`EvalSnapshot::restore`] to revert all reverted fields to their pre-eval
/// values.
///
/// Covers: settings, keymap, plugin_stack, ledger_stack, cmd_owners, hooks.
/// `pending_messages` is intentionally NOT reverted — messages from the
/// failed eval are preserved so the user can see what went wrong.
struct EvalSnapshot {
    settings:     EditorSettings,
    keymap:       Keymap,
    plugin_stack: PluginStack,
    ledger_stack: LedgerStack,
    cmd_owners:   std::collections::HashMap<String, String>,
    hooks:        HookRegistry,
    /// Version of `hooks` at capture time — used to skip the write-back in
    /// `restore` when no hooks were registered during the failed eval.
    hooks_version_at_capture: u32,
}

impl EvalSnapshot {
    fn capture(settings: &EditorSettings, keymap: &Keymap, host: &ScriptingHost) -> Self {
        Self {
            settings:     settings.clone(),
            keymap:       keymap.clone(),
            plugin_stack: host.plugin_stack.clone(),
            ledger_stack: host.ledger_stack.clone(),
            cmd_owners:   host.cmd_owners.clone(),
            hooks:        host.hooks.clone(),
            hooks_version_at_capture: host.hooks.version,
        }
    }

    fn restore(self, settings: &mut EditorSettings, keymap: &mut Keymap, host: &mut ScriptingHost) {
        *settings           = self.settings;
        *keymap             = self.keymap;
        host.plugin_stack   = self.plugin_stack;
        host.ledger_stack   = self.ledger_stack;
        host.cmd_owners     = self.cmd_owners;
        // Skip write-back when no hooks were registered during the failed eval.
        if host.hooks.version != self.hooks_version_at_capture {
            host.hooks = self.hooks;
        }
    }
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
    /// Ordered ledger of all plugin mutations, used for unload/reload teardown.
    pub(crate) ledger_stack: LedgerStack,
    /// Command-to-owner index: maps each Steel-registered command name to a
    /// display string (`"hume"`, `"user"`, or a plugin id like `"core:plum"`).
    /// Populated by `process_pending_cmds`; queried by `(command-plugin name)`.
    pub(crate) cmd_owners: std::collections::HashMap<String, String>,
    /// Persistent hook registry: handlers registered by `(register-hook! …)`
    /// across all evals.  Purged per-plugin on teardown.
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
        let data_dir    = crate::os::dirs::data_dir();
        let runtime_dir = crate::os::dirs::runtime_dir();
        // Initialize the fs builtin directory TLS before the engine registers
        // builtins — the `data-dir` / `runtime-dir` / sandbox functions read
        // from this TLS whenever they are called.
        builtins::fs::init_dirs(data_dir.clone(), runtime_dir.clone());
        let mut engine = Engine::new();
        builtins::register_all(&mut engine);
        Self {
            engine,
            plugin_stack:        PluginStack::default(),
            ledger_stack:        LedgerStack::default(),
            cmd_owners:          std::collections::HashMap::new(),
            hooks:               HookRegistry::default(),
            pending_messages:    Vec::new(),
            data_dir,
            runtime_dir,
            interrupt_flag:      Arc::new(AtomicBool::new(false)),
            hook_program_cache:  std::collections::HashMap::new(),
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

    // ── Plugin teardown / reload ───────────────────────────────────────────────

    /// Unload `plugin_name` by replaying its ledger entries in reverse:
    /// settings are restored via [`apply_setting`] and keybinds are restored
    /// (or removed) via [`Keymap::bind_user`] / [`Keymap::unbind_user`].
    ///
    /// Ledger entries with key `"cmd:<name>"` represent commands the plugin
    /// defined.  These are not restored here (there is no prior command to
    /// restore to); instead their names are returned so the caller can
    /// unregister them from the `CommandRegistry`.
    ///
    /// Returns `Ok(names)` where `names` is the list of command names to
    /// remove, or `Ok([])` if the plugin had no ledger — no-op for unknown
    /// plugins.
    pub(crate) fn teardown_plugin(
        &mut self,
        plugin_name: &str,
        settings: &mut EditorSettings,
        keymap: &mut Keymap,
    ) -> Result<Vec<String>, String> {
        let plugin_id = PluginId::parse(plugin_name)
            .map_err(|e| format!("teardown-plugin: {e}"))?;
        self.hooks.purge_plugin(&plugin_id);
        let to_restore = self.ledger_stack.unload(&plugin_id);

        let mut cmds_to_remove = Vec::new();
        for entry in to_restore {
            if let Some(cmd_name) = entry.key.strip_prefix("cmd:") {
                // Command defined by this plugin — caller removes it from registry;
                // also evict from the cmd_owners index.
                self.cmd_owners.remove(cmd_name);
                cmds_to_remove.push(cmd_name.to_string());
            } else {
                restore_ledger_entry(entry, settings, keymap)?;
            }
        }
        Ok(cmds_to_remove)
    }

    /// Reload `plugin_name`: tear it down then re-evaluate its file.
    ///
    /// Returns `(cmds_to_remove, new_cmds)`:
    /// - `cmds_to_remove`: command names the old plugin version defined
    ///   (caller calls `registry.unregister` for each).
    /// - `new_cmds`: Steel commands the new plugin version defines
    ///   (caller calls `registry.register` for each).
    ///
    /// If the plugin file is not found on disk (e.g. uninstalled), teardown
    /// still runs and an empty `new_cmds` list is returned — consistent with
    /// the `load-plugin` "not on disk → silently skipped" rule.
    pub(crate) fn reload_plugin(
        &mut self,
        plugin_name: &str,
        settings: &mut EditorSettings,
        keymap: &mut Keymap,
        builtin_names: std::collections::HashSet<String>,
    ) -> Result<(Vec<String>, Vec<SteelCmdDef>), String> {
        let cmds_to_remove = self.teardown_plugin(plugin_name, settings, keymap)?;

        let plugin_id = PluginId::parse(plugin_name)
            .map_err(|e| format!("reload-plugin: {e}"))?;
        let path = builtins::plugins::resolve_path_for_name(
            plugin_name,
            self.runtime_dir.as_deref(),
            self.data_dir.as_deref(),
        ).map_err(|e| format!("reload-plugin: {e}"))?;

        let new_cmds = match path {
            Some(p) => self.eval_plugin_with_attribution(&plugin_id, &p, settings, keymap, builtin_names)?,
            None    => Vec::new(),
        };
        Ok((cmds_to_remove, new_cmds))
    }

    /// Evaluate a plugin file with `plugin_id` on the attribution stack.
    ///
    /// Unlike [`eval_init`], this always evaluates (no early return on missing
    /// file), and wraps the eval in a plugin push/pop so mutations are correctly
    /// attributed to `plugin_id`.
    ///
    /// Used by [`reload_plugin`] to re-run a plugin after teardown.
    fn eval_plugin_with_attribution(
        &mut self,
        plugin_id: &PluginId,
        path: &std::path::Path,
        settings: &mut EditorSettings,
        keymap: &mut Keymap,
        builtin_names: std::collections::HashSet<String>,
    ) -> Result<Vec<SteelCmdDef>, String> {
        let source = crate::os::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;

        // Push the plugin attribution before the eval so that all mutations
        // are attributed to `plugin_id`.
        self.plugin_stack.push(plugin_id.clone());
        let result = self.eval_source_raw(source, builtin_names, settings, keymap);

        // Unconditionally pop the attribution we pushed above, even if eval
        // errored.  `eval_source_raw` snapshots the stack AFTER the push, so
        // on both success and error the stack still has `plugin_id` on top when
        // it returns — this pop is real work, not a no-op.
        self.plugin_stack.pop();

        result
    }

    /// Core eval machinery shared by [`eval_init`] and [`eval_plugin_with_attribution`].
    ///
    /// Snapshots all mutable state for all-or-nothing rollback on error, borrows
    /// state into a [`SteelCtx`] via Steel's `with_mut_reference` API, runs a
    /// watchdog thread (cooperative budget via `hume/yield!`), then evaluates
    /// `source` and drains log messages.
    fn eval_source_raw(
        &mut self,
        source: String,
        builtin_names: std::collections::HashSet<String>,
        settings: &mut EditorSettings,
        keymap: &mut Keymap,
    ) -> Result<Vec<SteelCmdDef>, String> {
        let budget_ms = settings.steel_init_budget_ms as u64;
        // Snapshot for all-or-nothing rollback on error.
        let snapshot = EvalSnapshot::capture(settings, keymap, self);

        // Build SteelCtx borrowing persistent fields directly from self.
        // The explicit block ensures steel_ctx is dropped (releasing all borrows)
        // before snapshot.restore() needs &mut self again.
        let (eval_result, pending_steel_cmds) = {
            let Self { engine, plugin_stack, ledger_stack, cmd_owners, hooks,
                       pending_messages, data_dir, runtime_dir, interrupt_flag, .. } = &mut *self;

            let mut steel_ctx = SteelCtx::new_init(
                HostBundle { plugin_stack, ledger_stack, cmd_owners, hooks, pending_messages,
                             data_dir: data_dir.as_deref(), runtime_dir: runtime_dir.as_deref(),
                             interrupt_flag: Arc::clone(interrupt_flag) },
                settings, keymap, builtin_names,
            );

            let result = run_steel(engine, &mut steel_ctx, source, budget_ms);
            (result, steel_ctx.pending_steel_cmds)
        };

        let steel_cmds = if eval_result.is_ok() {
            self.process_pending_cmds(pending_steel_cmds)
        } else {
            // Rollback: discard partial mutations and restore pre-eval snapshot.
            // Hooks written by register-hook! are reverted via snapshot.hooks.
            snapshot.restore(settings, keymap, self);
            Vec::new()
        };

        eval_result.map(|()| steel_cmds)
    }

    /// Process `PendingSteelCmd`s collected during an eval:
    /// register each lambda in the engine's global namespace and record a
    /// ledger entry.  Returns the `SteelCmdDef`s for the caller to register
    /// in the `CommandRegistry`.
    fn process_pending_cmds(&mut self, pending: Vec<PendingSteelCmd>) -> Vec<SteelCmdDef> {
        let mut defs = Vec::new();
        for cmd in pending {
            let steel_proc = cmd_proc_name(&cmd.name);
            // Register (or overwrite) the lambda under its internal name.
            self.engine.register_value(&steel_proc, cmd.proc);
            // Record a ledger entry so teardown knows to remove this command.
            let ledger_key = format!("cmd:{}", cmd.name);
            let prior_owner = self.ledger_stack.owner_of(&ledger_key);
            if let ledger::Owner::Plugin(ref plugin_id) = cmd.current_owner {
                self.ledger_stack.record(
                    plugin_id,
                    ledger_key,
                    prior_owner,
                    String::new(), // commands always start fresh (ownership rules prevent shadowing)
                );
            }
            // Record the owner string for `(command-plugin …)` introspection.
            self.cmd_owners.insert(cmd.name.clone(), cmd.current_owner.to_string());
            defs.push(SteelCmdDef {
                name: cmd.name,
                doc: cmd.doc,
                steel_proc,
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
            let Self { engine, plugin_stack, ledger_stack, cmd_owners, hooks,
                       pending_messages, data_dir, runtime_dir, interrupt_flag, .. } = &mut *self;

            let mut steel_ctx = SteelCtx::new_command(
                HostBundle { plugin_stack, ledger_stack, cmd_owners, hooks, pending_messages,
                             data_dir: data_dir.as_deref(), runtime_dir: runtime_dir.as_deref(),
                             interrupt_flag: Arc::clone(interrupt_flag) },
                refs, pending_char, cmd_arg,
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
        let handler_procs: Vec<SteelVal> = self.hooks
            .handlers_for(hook_id)
            .iter()
            .map(|(_, proc)| proc.clone())
            .collect();
        if handler_procs.is_empty() { return Ok(vec![]); }

        // Pre-bind each arg global.
        for (i, arg) in args.iter().enumerate() {
            self.engine.register_value(&hook_arg_name(i), arg.clone());
        }

        // Pre-bind each handler proc global.
        for (i, proc) in handler_procs.iter().enumerate() {
            self.engine.register_value(&hook_proc_name(i), proc.clone());
        }

        // Look up (or build once) the composite invocation program.
        let program = self.hook_program_cache
            .entry((args.len(), handler_procs.len()))
            .or_insert_with(|| build_hook_program(args.len(), handler_procs.len()))
            .clone();

        let budget_ms = refs.settings.steel_command_budget_ms as u64;

        let (result, cmd_queue) = {
            let Self { engine, plugin_stack, ledger_stack, cmd_owners, hooks,
                       pending_messages, data_dir, runtime_dir, interrupt_flag, .. } = &mut *self;

            let mut steel_ctx = SteelCtx::new_command(
                HostBundle { plugin_stack, ledger_stack, cmd_owners, hooks, pending_messages,
                             data_dir: data_dir.as_deref(), runtime_dir: runtime_dir.as_deref(),
                             interrupt_flag: Arc::clone(interrupt_flag) },
                refs, None, None,
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

// ── Ledger restoration helper ─────────────────────────────────────────────────

/// Apply one ledger entry's restoration to `settings` or `keymap`.
///
/// Setting keys are plain strings like `"tab-width"`.
/// Keymap keys are mode-prefixed: `"normal f"`, `"insert <ctrl-d>"`, etc.
///
/// For keybinds: if `prior_value` is empty the binding is removed
/// (it was unbound before the plugin set it); otherwise it is restored.
fn restore_ledger_entry(
    entry: crate::scripting::ledger::LedgerEntry,
    settings: &mut EditorSettings,
    keymap: &mut Keymap,
) -> Result<(), String> {
    if let Some(mode_and_seq) = BindMode::from_ledger_prefix(&entry.key) {
        let (mode, key_str) = mode_and_seq;
        let keys = keys::parse_key_sequence(key_str)?;
        if entry.prior_value.is_empty() {
            keymap.unbind_user(mode, &keys);
        } else {
            keymap.bind_user(mode, &keys, Cow::Owned(entry.prior_value));
        }
    } else if entry.key.contains(' ') {
        // A key with a space is unambiguously a keymap entry, but the mode
        // prefix didn't match any known BindMode — treat as corruption.
        return Err(format!(
            "ledger key '{}' has unknown mode prefix (expected 'normal ', 'extend ', or 'insert ')",
            entry.key
        ));
    } else {
        // Setting key — restore via apply_setting.
        if !entry.prior_value.is_empty() {
            let mut dummy = BufferOverrides::default();
            apply_setting(SettingScope::Global, &entry.key, &entry.prior_value, settings, &mut dummy)
                .map_err(|e| format!("restoring setting '{}': {e}", entry.key))?;
        }
    }
    Ok(())
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
mod tests {
    use super::*;
    use engine::pipeline::{BufferId, PaneId};
    use crate::settings::EditorSettings;
    use crate::editor::keymap::Keymap;

    fn host() -> ScriptingHost {
        ScriptingHost::new()
    }

    /// Build a minimal `EditorSteelRefs` for tests that don't exercise
    /// multi-buffer builtins (no `buffers` / `engine_view` / etc.).
    fn test_refs<'a>(s: &'a mut EditorSettings, km: &'a mut Keymap) -> EditorSteelRefs<'a> {
        test_refs_with_bid(s, km, BufferId::default())
    }

    fn test_refs_with_bid<'a>(
        s: &'a mut EditorSettings,
        km: &'a mut Keymap,
        bid: BufferId,
    ) -> EditorSteelRefs<'a> {
        EditorSteelRefs {
            settings:          s,
            keymap:            km,
            focused_pane_id:   PaneId::default(),
            focused_buffer_id: bid,
            buffers:           None,
            engine_view:       None,
            pane_state:        None,
            pane_jumps:        None,
        }
    }

    // ── set-option! ───────────────────────────────────────────────────────────

    #[test]
    fn set_option_tab_width_integer() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        assert_eq!(s.tab_width, 4);
        h.eval_source("(set-option! \"tab-width\" 2)", &mut s, &mut km).unwrap();
        assert_eq!(s.tab_width, 2);
    }

    #[test]
    fn set_option_tab_width_string() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        h.eval_source("(set-option! \"tab-width\" \"8\")", &mut s, &mut km).unwrap();
        assert_eq!(s.tab_width, 8);
    }

    #[test]
    fn set_option_bool_as_bool() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        assert!(s.mouse_enabled);
        h.eval_source("(set-option! \"mouse-enabled\" #f)", &mut s, &mut km).unwrap();
        assert!(!s.mouse_enabled);
    }

    #[test]
    fn set_option_unknown_key_errors() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(set-option! \"nonexistent\" \"val\")", &mut s, &mut km)
            .unwrap_err();
        assert!(err.contains("unknown setting"), "got: {err}");
    }

    #[test]
    fn set_option_settings_restored_on_error() {
        // On error, settings are rolled back to their pre-eval state (all-or-nothing).
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        // First set tab-width to 2...
        h.eval_source("(set-option! \"tab-width\" 2)", &mut s, &mut km).unwrap();
        assert_eq!(s.tab_width, 2);
        // Then run a script that errors mid-way: tab-width is set to 8, then a
        // bad setting that raises. The eval errors and the snapshot is restored:
        // tab-width goes back to 2, not left at the partial 8.
        let err = h.eval_source(
            "(set-option! \"tab-width\" 8)\n(set-option! \"bogus\" \"x\")",
            &mut s, &mut km,
        );
        assert!(err.is_err(), "expected eval to fail");
        assert_eq!(s.tab_width, 2, "snapshot should have been restored");
    }

    #[test]
    fn cmd_owners_rolled_back_on_error() {
        // A failing eval that defines a command mid-way must not leave a stale
        // entry in cmd_owners — the snapshot must be restored on error.
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        // Register a command successfully first so cmd_owners is non-empty.
        h.eval_source(
            r#"(push-current-plugin! "user/plugin-a")
               (define-command! "cmd-a" "a" (lambda () (+ 1 0)))
               (pop-current-plugin!)"#,
            &mut s, &mut km,
        ).unwrap();
        assert!(h.cmd_owners.contains_key("cmd-a"), "cmd-a should be registered");

        // Now run a script that defines a second command but then errors.
        // cmd-b must NOT appear in cmd_owners after rollback.
        let err = h.eval_source(
            r#"(push-current-plugin! "user/plugin-b")
               (define-command! "cmd-b" "b" (lambda () (+ 1 0)))
               (set-option! "bogus-key" "x")"#,
            &mut s, &mut km,
        );
        assert!(err.is_err(), "expected eval to fail");
        assert!(h.cmd_owners.contains_key("cmd-a"), "cmd-a should survive");
        assert!(!h.cmd_owners.contains_key("cmd-b"), "cmd-b must be rolled back");
    }

    // ── bind-key! ─────────────────────────────────────────────────────────────

    #[test]
    fn bind_key_does_not_error_on_valid_input() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        // A valid binding should succeed; the trie is verified via keymap's own tests.
        h.eval_source("(bind-key! \"normal\" \"z\" \"move-right\")", &mut s, &mut km).unwrap();
    }

    #[test]
    fn bind_key_multi_key_sequence_no_error() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        h.eval_source("(bind-key! \"normal\" \"g h\" \"move-right\")", &mut s, &mut km).unwrap();
    }

    #[test]
    fn bind_key_invalid_mode_errors() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(bind-key! \"visual\" \"f\" \"cmd\")", &mut s, &mut km)
            .unwrap_err();
        assert!(err.contains("mode"), "got: {err}");
    }

    #[test]
    fn bind_key_invalid_key_sequence_errors() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(bind-key! \"normal\" \"boguskey\" \"cmd\")", &mut s, &mut km)
            .unwrap_err();
        assert!(!err.is_empty(), "expected error for unknown key 'boguskey'");
    }

    // ── load-plugin path resolution ────────────────────────────────────────────

    #[test]
    fn load_plugin_missing_plugin_declared_not_loaded() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        // The plugin doesn't exist on disk — should be declared but not loaded.
        h.eval_source("(load-plugin \"user/nonexistent-repo\")", &mut s, &mut km).unwrap();

        // Inspect state via builtins.
        // declared-plugins filters out core:* — user/nonexistent should appear.
        let declared_result = h.eval_source("(declared-plugins)", &mut s, &mut km);
        // Even if we can't inspect the list directly here, the eval should not error.
        assert!(declared_result.is_ok(), "declared-plugins raised: {:?}", declared_result);
    }

    #[test]
    fn load_plugin_malformed_name_errors() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source("(load-plugin \"just-a-name\")", &mut s, &mut km)
            .unwrap_err();
        assert!(!err.is_empty(), "expected error for malformed plugin name");
    }

    // ── configure-statusline! ─────────────────────────────────────────────────

    #[test]
    fn configure_statusline_sets_left_section() {
        use crate::ui::statusline::StatusElement;
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        h.eval_source(
            r#"(configure-statusline! '("Mode" "FileName") '() '("Position"))"#,
            &mut s, &mut km,
        ).unwrap();

        assert_eq!(s.statusline.left,   vec![StatusElement::Mode, StatusElement::FileName]);
        assert_eq!(s.statusline.center, vec![]);
        assert_eq!(s.statusline.right,  vec![StatusElement::Position]);
    }

    #[test]
    fn configure_statusline_all_sections() {
        use crate::ui::statusline::StatusElement;
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        h.eval_source(
            r#"(configure-statusline!
                 '("Position" "FileName" "DirtyIndicator")
                 '("SearchMatches")
                 '("Separator" "Mode"))"#,
            &mut s, &mut km,
        ).unwrap();

        assert_eq!(s.statusline.left,
            vec![StatusElement::Position, StatusElement::FileName, StatusElement::DirtyIndicator]);
        assert_eq!(s.statusline.center, vec![StatusElement::SearchMatches]);
        assert_eq!(s.statusline.right,  vec![StatusElement::Separator, StatusElement::Mode]);
    }

    #[test]
    fn configure_statusline_empty_sections() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        h.eval_source("(configure-statusline! '() '() '())", &mut s, &mut km).unwrap();

        assert!(s.statusline.left.is_empty());
        assert!(s.statusline.center.is_empty());
        assert!(s.statusline.right.is_empty());
    }

    #[test]
    fn configure_statusline_unknown_element_errors() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        let err = h.eval_source(
            r#"(configure-statusline! '("NotAnElement") '() '())"#,
            &mut s, &mut km,
        ).unwrap_err();
        assert!(err.contains("NotAnElement"), "got: {err}");
    }

    #[test]
    fn configure_statusline_wrong_arity_errors() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        let err = h.eval_source("(configure-statusline! '())", &mut s, &mut km).unwrap_err();
        assert!(!err.is_empty(), "expected arity error");
    }

    // ── hume/yield! ───────────────────────────────────────────────────────────

    #[test]
    fn hume_yield_no_interrupt_is_noop() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        // With no interrupt flag set, (hume/yield!) is a transparent no-op.
        h.eval_source("(hume/yield!)", &mut s, &mut km).unwrap();
    }

    #[test]
    fn hume_yield_with_interrupt_errors() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        // Pre-set the interrupt flag before the eval.
        h.interrupt_flag.store(true, Ordering::Relaxed);
        let err = h.eval_source("(hume/yield!)", &mut s, &mut km).unwrap_err();
        assert!(err.contains("interrupted"), "expected 'interrupted' in error, got: {err}");

        // eval_source resets the flag after every call.
        assert!(!h.interrupt_flag.load(Ordering::Relaxed), "flag should be false after eval");
    }

    #[test]
    fn hume_yield_stops_loop_when_interrupted() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        // Pre-set so the loop aborts on the very first yield call.
        h.interrupt_flag.store(true, Ordering::Relaxed);
        let err = h.eval_source(
            // Without the interrupt flag this loop would run forever.
            "(let loop () (hume/yield!) (loop))",
            &mut s, &mut km,
        ).unwrap_err();
        assert!(err.contains("interrupted"), "got: {err}");
    }

    #[test]
    fn interrupt_flag_reset_after_eval() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        // Pre-set the flag; after eval_source it must be cleared.
        h.interrupt_flag.store(true, Ordering::Relaxed);
        h.eval_source("(hume/yield!)", &mut s, &mut km).unwrap_err(); // interrupted via pre-set flag
        assert!(!h.interrupt_flag.load(Ordering::Relaxed),
                "interrupt_flag must be false after eval_source returns");

        // Subsequent evals with no flag pre-set should succeed normally.
        h.eval_source("(hume/yield!)", &mut s, &mut km).unwrap();
    }

    // ── teardown_plugin / reload_plugin ───────────────────────────────────────

    /// Run a mini two-plugin scenario:
    ///   plugin A sets tab-width to 8 (prior: 4, core)
    ///   plugin B sets tab-width to 2 (prior: 8, A)
    /// Unloading A rewrites B's prior to (4, core).
    /// Unloading B restores tab-width to 4.
    #[test]
    fn teardown_restores_setting_when_plugin_is_live_owner() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        // Simulate plugin A setting tab-width to 8.
        // We drive via eval_source with the plugin on the attribution stack.
        h.eval_source(
            r#"(push-current-plugin! "user/a")
               (set-option! "tab-width" 8)
               (pop-current-plugin!)"#,
            &mut s, &mut km,
        ).unwrap();
        assert_eq!(s.tab_width, 8);

        // Tear down plugin A — tab-width should be restored to 4 (prior).
        h.teardown_plugin("user/a", &mut s, &mut km).unwrap();
        assert_eq!(s.tab_width, 4, "teardown should restore prior tab-width");
    }

    #[test]
    fn teardown_splices_chain_when_later_plugin_owns_key() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        // A sets tab-width 8, then B sets it to 2.
        h.eval_source(
            r#"(push-current-plugin! "user/a")
               (set-option! "tab-width" 8)
               (pop-current-plugin!)
               (push-current-plugin! "user/b")
               (set-option! "tab-width" 2)
               (pop-current-plugin!)"#,
            &mut s, &mut km,
        ).unwrap();
        assert_eq!(s.tab_width, 2);

        // Unload A — B still owns tab-width (live value = 2 unchanged).
        h.teardown_plugin("user/a", &mut s, &mut km).unwrap();
        assert_eq!(s.tab_width, 2, "B's live value must be preserved");

        // Now unload B — B's prior was rewritten by A's teardown to (4, core),
        // so restoring should give tab-width = 4.
        h.teardown_plugin("user/b", &mut s, &mut km).unwrap();
        assert_eq!(s.tab_width, 4, "after both unloads, core default restored");
    }

    #[test]
    fn teardown_restores_keybind() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        // The default normal keymap has 'h' bound to "move-left".
        // Plugin A rebinds 'h' to "move-right".
        h.eval_source(
            r#"(push-current-plugin! "user/a")
               (bind-key! "normal" "h" "move-right")
               (pop-current-plugin!)"#,
            &mut s, &mut km,
        ).unwrap();

        use crate::editor::keymap::BindMode;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let h_key = &[KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)];
        assert_eq!(km.lookup_command(BindMode::Normal, h_key).as_deref(), Some("move-right"));

        // Tear down plugin A — 'h' should go back to "move-left".
        h.teardown_plugin("user/a", &mut s, &mut km).unwrap();
        assert_eq!(km.lookup_command(BindMode::Normal, h_key).as_deref(), Some("move-left"),
                   "teardown should restore prior keybind");
    }

    #[test]
    fn teardown_unbinds_when_key_was_previously_unbound() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        // Bind an unused key (assume 'Q' is not in the default keymap).
        h.eval_source(
            r#"(push-current-plugin! "user/a")
               (bind-key! "normal" "Q" "move-right")
               (pop-current-plugin!)"#,
            &mut s, &mut km,
        ).unwrap();

        use crate::editor::keymap::BindMode;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let q_key = &[KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE)];
        assert!(km.lookup_command(BindMode::Normal, q_key).is_some());

        // Tear down — 'Q' was unbound before, so it should become unbound again.
        h.teardown_plugin("user/a", &mut s, &mut km).unwrap();
        assert!(km.lookup_command(BindMode::Normal, q_key).is_none(),
                "binding for unowned key must be removed on teardown");
    }

    #[test]
    fn teardown_unknown_plugin_is_noop() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        // No error, no state change.
        h.teardown_plugin("user/nonexistent", &mut s, &mut km).unwrap();
        assert_eq!(s.tab_width, 4);
    }

    // ── command-plugin ────────────────────────────────────────────────────────

    /// `(command-plugin name)` returns the owning plugin id for a Steel command.
    #[test]
    fn command_plugin_returns_plugin_owner_during_eval() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        // Register a command attributed to a plugin.
        h.eval_source(
            r#"(push-current-plugin! "user/myplugin")
               (define-command! "my-cmd" "test cmd" (lambda () (+ 1 0)))
               (pop-current-plugin!)"#,
            &mut s, &mut km,
        ).unwrap();

        // Verify the owner is queryable during a subsequent eval.
        // We can't call (command-plugin) from Rust directly at exec-time in
        // these unit tests, but we CAN call it during eval_source.
        let result = h.eval_source(
            r#"(command-plugin "my-cmd")"#,
            &mut s, &mut km,
        );
        assert!(result.is_ok(), "command-plugin should not error: {:?}", result);
        // The owner is recorded in cmd_owners; verify via the map directly.
        assert_eq!(h.cmd_owners.get("my-cmd").map(|s| s.as_str()), Some("user/myplugin"));
    }

    /// Unknown (built-in) commands return "hume".
    #[test]
    fn command_plugin_unknown_returns_hume() {
        let h = host();

        // "move-right" is a Rust built-in — not in cmd_owners.
        assert!(!h.cmd_owners.contains_key("move-right"));
    }

    /// Teardown removes the command from cmd_owners.
    #[test]
    fn command_plugin_cleared_on_teardown() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();

        h.eval_source(
            r#"(push-current-plugin! "user/myplugin")
               (define-command! "my-cmd" "test cmd" (lambda () (+ 1 0)))
               (pop-current-plugin!)"#,
            &mut s, &mut km,
        ).unwrap();
        assert_eq!(h.cmd_owners.get("my-cmd").map(|s| s.as_str()), Some("user/myplugin"));

        h.teardown_plugin("user/myplugin", &mut s, &mut km).unwrap();
        assert!(!h.cmd_owners.contains_key("my-cmd"), "teardown should remove from cmd_owners");
    }

    // ── EvalWatchdog ──────────────────────────────────────────────────────────

    /// Cancelling a watchdog with a long budget wakes the thread immediately.
    /// Without `park_timeout` + `unpark`, this would block for the full budget.
    #[test]
    fn watchdog_cancel_wakes_thread_immediately() {
        let flag   = Arc::new(AtomicBool::new(false));
        let budget = std::time::Duration::from_secs(10);
        let start  = std::time::Instant::now();
        let watchdog = EvalWatchdog::arm(Arc::clone(&flag), budget);
        watchdog.cancel();
        // cancel() must return well within the budget; 500 ms is generous.
        assert!(start.elapsed() < std::time::Duration::from_millis(500),
                "cancel() took too long: {:?}", start.elapsed());
        // Flag must not have been set (we cancelled before it fired).
        assert!(!flag.load(Ordering::Relaxed), "flag must stay false after cancel");
    }

    /// A watchdog with a tiny budget fires and causes (hume/yield!) to abort.
    #[test]
    fn eval_source_raw_watchdog_aborts_runaway() {
        let mut h  = host();
        let mut s  = EditorSettings::default();
        let mut km = Keymap::default();
        let budget = std::time::Duration::from_millis(50);
        let start  = std::time::Instant::now();

        let err = h.eval_source_watchdog(
            // This loop would run forever without the watchdog.
            "(let loop () (hume/yield!) (loop))",
            budget,
            &mut s,
            &mut km,
        ).unwrap_err();

        assert!(err.contains("interrupted"), "expected 'interrupted' in error, got: {err}");
        // Must abort well within a second — if not, the watchdog didn't fire.
        assert!(start.elapsed() < std::time::Duration::from_secs(1),
                "eval took too long: {:?}", start.elapsed());
        // Flag must be reset after eval_source_raw returns.
        assert!(!h.interrupt_flag.load(Ordering::Relaxed),
                "interrupt_flag must be false after eval returns");
    }

    /// When the watchdog fires during an eval that had already mutated a
    /// setting, the rollback must restore the original value.
    #[test]
    fn eval_source_raw_watchdog_rollback_on_abort() {
        let mut h  = host();
        let mut s  = EditorSettings::default();
        let mut km = Keymap::default();
        let budget = std::time::Duration::from_millis(50);

        // Confirm the starting value so the assertion is not vacuously true.
        assert_eq!(s.tab_width, 4, "precondition: default tab-width is 4");

        // Set the option then run forever — rollback must undo the set.
        let err = h.eval_source_watchdog(
            r#"(set-option! "tab-width" 99) (let loop () (hume/yield!) (loop))"#,
            budget,
            &mut s,
            &mut km,
        ).unwrap_err();

        assert!(err.contains("interrupted"), "expected 'interrupted' in error, got: {err}");
        assert_eq!(s.tab_width, 4, "rollback must restore tab-width to pre-eval value");
    }

    /// call_steel_cmd watchdog fires and aborts a runaway Steel command.
    #[test]
    fn call_steel_cmd_watchdog_aborts_runaway() {
        let mut h  = host();
        let mut s  = EditorSettings::default();
        let mut km = Keymap::default();

        // Register a command whose body loops forever.
        h.eval_source(
            r#"(define-command! "spin" "spin forever" (lambda () (let loop () (hume/yield!) (loop))))"#,
            &mut s, &mut km,
        ).unwrap();
        let steel_proc = "%hume-cmd-spin".to_string();

        // Use a tight command budget.
        s.steel_command_budget_ms = 50;

        let start = std::time::Instant::now();
        let err = h.call_steel_cmd(
            &steel_proc, None, None, test_refs(&mut s, &mut km),
        ).unwrap_err();

        assert!(err.contains("interrupted"), "expected 'interrupted', got: {err}");
        assert!(start.elapsed() < std::time::Duration::from_secs(1),
                "call_steel_cmd took too long: {:?}", start.elapsed());
        assert!(!h.interrupt_flag.load(Ordering::Relaxed),
                "interrupt_flag must be false after call_steel_cmd returns");
    }

    /// Command bodies cannot mutate settings/keymap (is_init = false during
    /// call_steel_cmd; init-only builtins raise Steel errors).  This test verifies
    /// that after a watchdog interrupt the settings remain at their pre-call values.
    /// Also verifies the budget is read from settings at call time.
    #[test]
    fn call_steel_cmd_interrupt_leaves_settings_unchanged() {
        let mut h  = host();
        let mut s  = EditorSettings::default();
        let mut km = Keymap::default();

        h.eval_source(
            r#"(define-command! "looper" "loop" (lambda () (let loop () (hume/yield!) (loop))))"#,
            &mut s, &mut km,
        ).unwrap();
        let steel_proc = "%hume-cmd-looper".to_string();

        assert_eq!(s.tab_width, 4, "precondition");
        s.steel_command_budget_ms = 50;

        let err = h.call_steel_cmd(
            &steel_proc, None, None, test_refs(&mut s, &mut km),
        ).unwrap_err();

        assert!(err.contains("interrupted"), "expected 'interrupted', got: {err}");
        assert_eq!(s.tab_width, 4, "tab-width must be unchanged after interrupt");
    }

    /// Calling an init-only builtin from a Steel command body must raise a Steel
    /// error (not panic).  `is_init = false` during call_steel_cmd, and init-only
    /// builtins check this flag.
    #[test]
    fn call_steel_cmd_set_option_from_body_returns_steel_error() {
        let mut h  = host();
        let mut s  = EditorSettings::default();
        let mut km = Keymap::default();

        h.eval_source(
            r#"(define-command! "try-set" "" (lambda () (set-option! "tab-width" 8)))"#,
            &mut s, &mut km,
        ).unwrap();

        let err = h.call_steel_cmd(
            "%hume-cmd-try-set", None, None, test_refs(&mut s, &mut km),
        ).unwrap_err();

        assert!(err.contains("set-option!"),
            "error must name the failing builtin; got: {err}");
        // Mutation never happened, so the setting is unchanged.
        assert_eq!(s.tab_width, 4, "tab-width must be untouched");
    }

    // ── call! alias ───────────────────────────────────────────────────────────

    /// Both `call!` and `call-command!` route to the same builtin.  Verify
    /// that commands defined with each spelling both queue their sub-commands.
    #[test]
    fn call_bang_and_call_command_both_dispatch() {
        let mut h  = host();
        let mut s  = EditorSettings::default();
        let mut km = Keymap::default();

        h.eval_source(
            r#"
(define-command! "use-call-bang"    "" (lambda () (call! "move-right")))
(define-command! "use-call-command" "" (lambda () (call-command! "move-left")))
"#,
            &mut s, &mut km,
        ).unwrap();

        let (q1, _) = h.call_steel_cmd(
            "%hume-cmd-use-call-bang", None, None, test_refs(&mut s, &mut km),
        ).unwrap();
        assert_eq!(q1, vec!["move-right"], "call! should queue the command");

        let (q2, _) = h.call_steel_cmd(
            "%hume-cmd-use-call-command", None, None, test_refs(&mut s, &mut km),
        ).unwrap();
        assert_eq!(q2, vec!["move-left"], "call-command! alias should queue the command");
    }

    // ── register-hook! / fire_hook ────────────────────────────────────────────

    use crate::scripting::hooks::HookId;
    use crate::scripting::builtins::ids::SteelBufferId;

    #[test]
    fn register_hook_fires_on_buffer_open() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        h.eval_source(
            r#"(register-hook! 'on-buffer-open (lambda (bid) (call! "move-right")))"#,
            &mut s, &mut km,
        ).unwrap();
        let bid = BufferId::default();
        let val = SteelBufferId(bid).into_steel_val();
        let queue = h.fire_hook(
            HookId::OnBufferOpen, &[val], test_refs_with_bid(&mut s, &mut km, bid),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"]);
    }

    #[test]
    fn register_hook_fires_on_buffer_close() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        h.eval_source(
            r#"(register-hook! 'on-buffer-close (lambda (bid) (call! "move-left")))"#,
            &mut s, &mut km,
        ).unwrap();
        let bid = BufferId::default();
        let val = SteelBufferId(bid).into_steel_val();
        let queue = h.fire_hook(
            HookId::OnBufferClose, &[val], test_refs_with_bid(&mut s, &mut km, bid),
        ).unwrap();
        assert_eq!(queue, vec!["move-left"]);
    }

    #[test]
    fn register_hook_fires_on_buffer_save() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        h.eval_source(
            r#"(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right")))"#,
            &mut s, &mut km,
        ).unwrap();
        let bid = BufferId::default();
        let val = SteelBufferId(bid).into_steel_val();
        let queue = h.fire_hook(
            HookId::OnBufferSave, &[val], test_refs_with_bid(&mut s, &mut km, bid),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"]);
    }

    #[test]
    fn register_hook_fires_on_mode_change() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        h.eval_source(
            r#"(register-hook! 'on-mode-change
                  (lambda (old new)
                    (when (equal? new "insert") (call! "move-right"))))"#,
            &mut s, &mut km,
        ).unwrap();
        use steel::rvals::IntoSteelVal as _;
        let old_val = "normal".into_steelval().unwrap();
        let new_val = "insert".into_steelval().unwrap();
        let queue = h.fire_hook(
            HookId::OnModeChange, &[old_val, new_val], test_refs(&mut s, &mut km),
        ).unwrap();
        assert_eq!(queue, vec!["move-right"]);
    }

    #[test]
    fn register_hook_no_fire_if_no_handlers() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let queue = h.fire_hook(
            HookId::OnBufferOpen, &[], test_refs(&mut s, &mut km),
        ).unwrap();
        assert!(queue.is_empty());
    }

    #[test]
    fn register_hook_multiple_handlers_all_fire() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        h.eval_source(
            r#"
(register-hook! 'on-buffer-save (lambda (bid) (call! "move-right")))
(register-hook! 'on-buffer-save (lambda (bid) (call! "move-left")))
"#,
            &mut s, &mut km,
        ).unwrap();
        let bid = BufferId::default();
        let val = SteelBufferId(bid).into_steel_val();
        let queue = h.fire_hook(
            HookId::OnBufferSave, &[val], test_refs_with_bid(&mut s, &mut km, bid),
        ).unwrap();
        assert_eq!(queue, vec!["move-right", "move-left"]);
    }

    #[test]
    fn teardown_removes_plugin_hooks() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        // Register a hook as part of a plugin.
        h.plugin_stack.push(ledger::PluginId::parse("user/myplugin").unwrap());
        h.eval_source(
            r#"(register-hook! 'on-buffer-open (lambda (bid) (call! "move-right")))"#,
            &mut s, &mut km,
        ).unwrap();
        h.plugin_stack.pop();
        // Hook is registered.
        assert!(!h.hooks.is_empty_for(HookId::OnBufferOpen));
        // Teardown removes it.
        h.teardown_plugin("user/myplugin", &mut s, &mut km).unwrap();
        assert!(h.hooks.is_empty_for(HookId::OnBufferOpen));
    }

    #[test]
    fn register_hook_errors_in_command_mode() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        // Define a command that tries to register a hook (not allowed in command mode).
        h.eval_source(
            r#"(define-command! "bad-cmd" "" (lambda ()
                 (register-hook! 'on-buffer-open (lambda (bid) #f))))"#,
            &mut s, &mut km,
        ).unwrap();
        let err = h.call_steel_cmd(
            "%hume-cmd-bad-cmd", None, None, test_refs(&mut s, &mut km),
        ).unwrap_err();
        assert!(err.contains("can only be called during init"), "got: {err}");
    }

    #[test]
    fn register_hook_unknown_name_errors() {
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let err = h.eval_source(
            r#"(register-hook! 'on-nonexistent (lambda () #f))"#,
            &mut s, &mut km,
        ).unwrap_err();
        assert!(err.contains("unknown hook"), "got: {err}");
    }

    #[test]
    fn fire_hook_globals_cleared_between_fires() {
        // After each fire_hook call, *hume.ha0* / *hume.hp0* … must be Void.
        // Leaking them keeps Arc references alive (e.g. to a closed buffer)
        // and may surface stale data in subsequent fires with fewer args.
        let mut h = host();
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        // Handler reads arg 0 and queues its string representation.
        h.eval_source(
            r#"(register-hook! 'on-mode-change (lambda (old new) (call! new)))"#,
            &mut s, &mut km,
        ).unwrap();
        use steel::rvals::IntoSteelVal as _;
        let old_val = "normal".into_steelval().unwrap();
        let new_val = "insert".into_steelval().unwrap();
        let q1 = h.fire_hook(
            HookId::OnModeChange, &[old_val.clone(), new_val], test_refs(&mut s, &mut km),
        ).unwrap();
        assert_eq!(q1, vec!["insert"]);

        // Second fire with different args — stale *hume.ha1* would give wrong result.
        let new_val2 = "normal".into_steelval().unwrap();
        let q2 = h.fire_hook(
            HookId::OnModeChange, &[old_val, new_val2], test_refs(&mut s, &mut km),
        ).unwrap();
        assert_eq!(q2, vec!["normal"], "second fire must not see stale globals from first");
    }

    #[test]
    fn restore_ledger_entry_rejects_unknown_mode_prefix() {
        use crate::scripting::ledger::{LedgerEntry, Owner};
        let mut s = EditorSettings::default();
        let mut km = Keymap::default();
        let entry = LedgerEntry {
            key: "bogus abc".to_string(),
            prior_value: String::new(),
            prior_owner: Owner::Core,
        };
        let err = restore_ledger_entry(entry, &mut s, &mut km).unwrap_err();
        assert!(err.contains("unknown mode prefix"), "got: {err}");
    }
}
