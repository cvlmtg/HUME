use std::sync::{
    Arc,
    atomic::AtomicBool,
};

use steel::gc::unsafe_erased_pointers::CustomReference;
use steel::rvals::SteelVal;

use engine::pipeline::{BufferId, EngineView, PaneId};
use slotmap::SecondaryMap;

use crate::core::jump_list::JumpList;
use crate::editor::buffer_store::BufferStore;
use crate::editor::keymap::Keymap;
use crate::editor::pane_state::PaneBufferState;
use crate::settings::EditorSettings;

use super::attribution::PluginStack;
use super::hooks::HookRegistry;
use super::lazy::LazyRegistry;
use super::types::{EditorSteelRefs, PendingLanguageReg, PendingLanguageSets, PendingSteelCmd};
use super::HostBundle;

/// Context struct borrowed into the Steel engine for the duration of each eval
/// or command call via Steel's `with_mut_reference` API.
///
/// All persistent scripting state (hooks, attribution, etc.) is held directly
/// on [`super::ScriptingHost`] and borrowed here by reference — no `mem::take`/put-back
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
    /// [`super::ScriptingHost::process_pending_cmds`].
    pub(crate) cmd_owners: &'a mut std::collections::HashMap<String, String>,
    /// Hook registry; `(register-hook! …)` writes directly.
    pub(crate) hooks: &'a mut HookRegistry,
    /// Lazy plugin registry; `%declare-plugin!` writes directly.
    pub(crate) lazy_registry: &'a mut LazyRegistry,
    /// Log messages accumulated by `(log! …)`.
    pub(crate) pending_messages: &'a mut Vec<(crate::editor::Severity, String)>,
    /// Language identity registrations queued by `(define-language! …)` during init.
    pub(crate) pending_language_regs: &'a mut Vec<PendingLanguageReg>,
    /// Where PLUM installs third-party plugins (`$XDG_DATA_HOME/hume/`).
    pub(crate) data_dir: Option<&'a std::path::Path>,
    /// Where core plugins, themes, and docs live.
    pub(crate) runtime_dir: Option<&'a std::path::Path>,
    // ── Transient per-eval state (owned) ──────────────────────────────────────
    /// Every plugin name passed to `(load-plugin …)`, including absent ones.
    pub(crate) declared_plugins: Vec<String>,
    /// Plugins queued for activation at the end of this eval (init.scm or plugin
    /// body).  Populated by `%declare-plugin!` for eager plugins and by
    /// `(require-plugin …)` for explicit loads; drained by `eval_source_raw`
    /// (init.scm) and by `activate_plugin` (plugin body).
    pub(crate) pending_plugin_loads: Vec<super::attribution::PluginId>,
    /// Built-in command names known at eval start.  `define-command!` checks
    /// against this to prevent shadowing core commands.
    pub(crate) builtin_cmd_names: std::collections::HashSet<String>,
    /// `(define-command! …)` calls accumulated during this eval.
    pub(crate) pending_steel_cmds: Vec<PendingSteelCmd>,
    /// Interrupt flag shared with the `EvalWatchdog`.
    pub(crate) interrupt_flag: Arc<AtomicBool>,
    // ── Command-mode fields (meaningful only when is_init = false) ────────────
    /// Commands queued by `(call! …)`, with their positional args.
    pub(crate) cmd_queue: Vec<(String, Vec<SteelVal>)>,
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
    pub(crate) buffers: Option<&'a mut BufferStore>,
    pub(crate) engine_view: Option<&'a mut EngineView>,
    pub(crate) pane_state:
        Option<&'a mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>>,
    pub(crate) pane_jumps: Option<&'a mut SecondaryMap<PaneId, JumpList>>,
}

impl CustomReference for SteelCtx<'_> {}
steel::custom_reference!(SteelCtx<'a>);

impl<'a> SteelCtx<'a> {
    pub(super) fn new_init(
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
            lazy_registry: host.lazy_registry,
            pending_messages: host.pending_messages,
            pending_language_regs: host.pending_language_regs,
            data_dir: host.data_dir,
            runtime_dir: host.runtime_dir,
            declared_plugins: Vec::new(),
            pending_plugin_loads: Vec::new(),
            builtin_cmd_names,
            pending_steel_cmds: Vec::new(),
            interrupt_flag: host.interrupt_flag,
            cmd_queue: Vec::new(),
            wait_char_request: None,
            pending_language_sets: Vec::new(),
            pending_char: None,
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

    pub(crate) fn new_command(
        host: HostBundle<'a>,
        refs: EditorSteelRefs<'a>,
        pending_char: Option<char>,
    ) -> Self {
        let fid = refs.focused_buffer_id;
        Self {
            settings: refs.settings,
            keymap: refs.keymap,
            plugin_stack: host.plugin_stack,
            cmd_owners: host.cmd_owners,
            hooks: host.hooks,
            lazy_registry: host.lazy_registry,
            pending_messages: host.pending_messages,
            pending_language_regs: host.pending_language_regs,
            data_dir: host.data_dir,
            runtime_dir: host.runtime_dir,
            declared_plugins: Vec::new(),
            pending_plugin_loads: Vec::new(),
            builtin_cmd_names: std::collections::HashSet::new(),
            pending_steel_cmds: Vec::new(),
            interrupt_flag: host.interrupt_flag,
            cmd_queue: Vec::new(),
            wait_char_request: None,
            pending_language_sets: Vec::new(),
            pending_char,
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
