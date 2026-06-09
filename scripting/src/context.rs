use std::sync::{
    Arc,
    atomic::AtomicBool,
};

use steel::gc::unsafe_erased_pointers::CustomReference;

use engine::pipeline::{BufferId, PaneId};

use super::attribution::PluginStack;
use super::host::EditorHost;
use super::log::LogLevel;
use super::types::{PendingLanguageReg, PendingLanguageSets, PendingSteelCmd, QueuedCommand};
use super::{HostBundle, ScriptingRegistries};

/// Context struct borrowed into the Steel engine for the duration of each eval
/// or command call via Steel's `with_mut_reference` API.
///
/// All persistent scripting state (hooks, attribution, etc.) is held directly
/// on [`super::ScriptingHost`] and borrowed here by reference — no `mem::take`/put-back
/// needed.  Transient per-eval state (accumulators, mode flags) is owned.
///
/// Editor-domain state is accessed through [`EditorHost`], which is implemented
/// by `EditorHostImpl<'a>` in the editor crate (or `MockHost` in tests).  This
/// removes the direct dependency from the scripting layer onto editor-crate types.
///
/// Builtins registered with `register_fn_with_ctx(HUME_CTX, …)` receive
/// `&mut SteelCtx` as their first argument, injected automatically by Steel.
pub(crate) struct SteelCtx<'a> {
    // ── Editor interface ───────────────────────────────────────────────────────
    /// Access to all live editor state during evaluation.
    ///
    /// In init mode (`is_init = true`), the host's buffer/pane methods are
    /// guarded by `require_cmd_ctx!` and never called; the init-only methods
    /// (`set_global_option`, `bind_key`, `configure_statusline`) are always safe.
    pub(crate) host: &'a mut dyn EditorHost,
    // ── Persistent state borrowed from ScriptingHost ──────────────────────────
    /// Plugin attribution stack; identifies whose mutation is being recorded.
    pub(crate) plugin_stack: &'a mut PluginStack,
    /// The four persistent registries: cmd_owners, hooks, lazy_registry,
    /// declared_plugins. Borrowed as a unit, disjoint from `steel`.
    pub(crate) registries: &'a mut ScriptingRegistries,
    /// Log messages accumulated by `(log! …)`.
    pub(crate) pending_messages: &'a mut Vec<(LogLevel, String)>,
    /// Language identity registrations queued by `(define-language! …)` during init.
    pub(crate) pending_language_regs: &'a mut Vec<PendingLanguageReg>,
    /// Where PLUM installs third-party plugins (`$XDG_DATA_HOME/hume/`).
    pub(crate) data_dir: Option<&'a std::path::Path>,
    /// Where core plugins, themes, and docs live.
    pub(crate) runtime_dir: Option<&'a std::path::Path>,
    // ── Transient per-eval state (owned) ──────────────────────────────────────
    /// Plugins queued for activation at the end of this eval (init.scm or plugin
    /// body).  Populated by `%load-plugin!` (eager) or by force-activating a
    /// declared plugin; drained by `eval_source_raw` (init.scm) and by
    /// `activate_plugin` (plugin body).
    pub(crate) pending_plugin_loads: Vec<super::attribution::PluginId>,
    /// Built-in command names known at eval start.  `define-command!` checks
    /// against this to prevent shadowing core commands.
    pub(crate) builtin_cmd_names: std::collections::HashSet<String>,
    /// `(define-command! …)` calls accumulated during this eval.
    pub(crate) pending_steel_cmds: Vec<PendingSteelCmd>,
    /// Interrupt flag shared with the `EvalWatchdog`.
    pub(crate) interrupt_flag: Arc<AtomicBool>,
    // ── Queued commands and input state ──────────────────────────────────────
    /// Commands queued by `(call! …)`, with their positional args.
    /// In init mode, drained by the host into `pending_startup_commands` after
    /// each eval; in command mode, returned in `SteelCmdResult`.
    pub(crate) cmd_queue: Vec<QueuedCommand>,
    /// Register prefix set by `(set-register-prefix! …)` and inherited by
    /// subsequent `(call! …)` calls until changed.  Resets per invocation
    /// (SteelCtx is rebuilt each time).
    pub(crate) current_register_prefix: Option<char>,
    /// WaitChar command requested by `(request-wait-char! …)`.
    pub(crate) wait_char_request: Option<String>,
    /// `set-buffer-language!` calls deferred during this eval; drained by the
    /// consumer (mappings.rs / fire_hook_silent) before cmd_queue dispatch.
    pub(crate) pending_language_sets: PendingLanguageSets,
    /// Pending char from a WaitChar keymap node.
    pub(crate) pending_char: Option<char>,
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
    /// Language names for which a grammar was just attached; flushed into
    /// `SteelCmdResult.grammar_sweeps` / `HookResult.grammar_sweeps`.
    pub(crate) pending_grammar_sweeps: Vec<String>,
}

impl CustomReference for SteelCtx<'_> {}
steel::custom_reference!(SteelCtx<'a>);

impl<'a> SteelCtx<'a> {
    pub(super) fn new_init(
        host: &'a mut dyn EditorHost,
        host_bundle: HostBundle<'a>,
        builtin_cmd_names: std::collections::HashSet<String>,
    ) -> Self {
        Self {
            host,
            plugin_stack: host_bundle.plugin_stack,
            registries: host_bundle.registries,
            pending_messages: host_bundle.pending_messages,
            pending_language_regs: host_bundle.pending_language_regs,
            data_dir: host_bundle.data_dir,
            runtime_dir: host_bundle.runtime_dir,
            pending_plugin_loads: Vec::new(),
            builtin_cmd_names,
            pending_steel_cmds: Vec::new(),
            interrupt_flag: host_bundle.interrupt_flag,
            cmd_queue: Vec::new(),
            current_register_prefix: None,
            wait_char_request: None,
            pending_language_sets: Vec::new(),
            pending_char: None,
            is_init: true,
            focused_pane_id: PaneId::default(),
            focused_buffer_id: BufferId::default(),
            live_focused_buffer_id: BufferId::default(),
            pending_grammar_sweeps: Vec::new(),
        }
    }

    /// Push a log message — prefer this over direct `pending_messages.push` so
    /// any future severity filter is applied uniformly.
    pub(crate) fn log(&mut self, level: LogLevel, msg: String) {
        self.pending_messages.push((level, msg));
    }

    pub(crate) fn new_command(
        host: &'a mut dyn EditorHost,
        host_bundle: HostBundle<'a>,
        focused_pane_id: PaneId,
        focused_buffer_id: BufferId,
        pending_char: Option<char>,
    ) -> Self {
        Self {
            host,
            plugin_stack: host_bundle.plugin_stack,
            registries: host_bundle.registries,
            pending_messages: host_bundle.pending_messages,
            pending_language_regs: host_bundle.pending_language_regs,
            data_dir: host_bundle.data_dir,
            runtime_dir: host_bundle.runtime_dir,
            pending_plugin_loads: Vec::new(),
            builtin_cmd_names: std::collections::HashSet::new(),
            pending_steel_cmds: Vec::new(),
            interrupt_flag: host_bundle.interrupt_flag,
            cmd_queue: Vec::new(),
            current_register_prefix: None,
            wait_char_request: None,
            pending_language_sets: Vec::new(),
            pending_char,
            is_init: false,
            focused_pane_id,
            focused_buffer_id,
            live_focused_buffer_id: focused_buffer_id,
            pending_grammar_sweeps: Vec::new(),
        }
    }
}
