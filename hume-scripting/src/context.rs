use std::sync::{Arc, atomic::AtomicBool};

use steel::gc::unsafe_erased_pointers::CustomReference;

use hume_engine::pipeline::{BufferId, PaneId};

use super::attribution::PluginStack;
use super::host::EditorHost;
use super::log::LogLevel;
use super::types::{
    HookResult, PendingLanguageReg, PendingLanguageSets, PendingLspNotify, PendingLspRequest,
    PendingLspServerOp,
};
use super::{HostBundle, ScriptingRegistries};

/// Context struct borrowed into the Steel engine for the duration of each eval
/// or command call via Steel's `with_mut_reference` API.
///
/// All persistent scripting state (hooks, attribution, etc.) is held directly
/// on [`super::ScriptingHost`] and borrowed here by reference.  Transient
/// per-eval state (accumulators, mode flags) is owned.
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
    /// LSP server registrations/unregistrations queued by
    /// `(register-lsp-server! …)` / `(unregister-lsp-server! …)` during this
    /// eval, applied at the end-of-eval drain (see `types::PendingLspServerOp`).
    pub(crate) pending_lsp_server_ops: &'a mut Vec<PendingLspServerOp>,
    /// Where PLUM installs third-party plugins (`$XDG_DATA_HOME/hume/`).
    pub(crate) data_dir: Option<&'a std::path::Path>,
    /// Where core plugins, themes, and docs live.
    pub(crate) runtime_dir: Option<&'a std::path::Path>,
    // ── Transient per-eval state (owned) ──────────────────────────────────────
    /// Built-in command names known at eval start.  `define-command!` checks
    /// against this to prevent shadowing core commands.
    pub(crate) builtin_cmd_names: std::collections::HashSet<String>,
    /// Interrupt flag shared with the `EvalWatchdog`.
    pub(crate) interrupt_flag: Arc<AtomicBool>,
    // ── Input state and command side-effects ─────────────────────────────────
    /// Register prefix set by `(set-register-prefix! …)` and inherited by
    /// subsequent `(call! …)` calls until changed.  Resets per invocation
    /// (SteelCtx is rebuilt each time).
    pub(crate) current_register_prefix: Option<char>,
    /// WaitChar command requested by `(request-wait-char! …)`.
    pub(crate) wait_char_request: Option<String>,
    /// `set-buffer-language!` calls deferred during this eval; drained by the
    /// consumer (mappings.rs / fire_hook_silent) after the eval returns.
    pub(crate) pending_language_sets: PendingLanguageSets,
    /// Pending char from a WaitChar keymap node.
    pub(crate) pending_char: Option<char>,
    // ── Mode discriminant ────────────────────────────────────────────────────
    /// `true` during `eval_source_raw` (init.scm); `false` during
    /// `call_steel_cmd` (command dispatch) and `activate_plugin_inline`
    /// (runtime-activated plugin bodies, which use `SteelCtx::new_activation`).
    ///
    /// Config builtins (`set-option!`, `bind-key!`, etc.) gate on
    /// `!is_init && plugin_stack.is_empty()`: permitted during plugin
    /// activation (plugin_stack non-empty) even when `is_init` is `false`,
    /// but blocked from plain command bodies (plugin_stack empty, is_init false).
    ///
    /// Registration verbs (`load-plugin`, `declare-plugin`) use the stricter
    /// `!is_init || !plugin_stack.is_empty()` gate: both must be false, i.e.
    /// only the init.scm top level (is_init=true, stack empty) is allowed.
    pub(crate) is_init: bool,
    /// True when it is safe to write directly to stdout: either during init
    /// (before the alt-screen TUI is up) or inside an `#:inline-output` command
    /// body (alt-screen temporarily left). See `EditorHost::is_inline_output_command`.
    /// Gates the print shims (`displayln`/`display`/`print`/`println`/
    /// `newline`) via `%stdout-gate!` (`builtins::io::stdout_gate`).
    pub(crate) is_inline_output: bool,
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
    /// `(lsp-request …)` calls queued this eval; flushed into
    /// `SteelCmdResult`/`HookResult.pending_lsp_requests` and sent by
    /// `Editor::flush_pending_lsp_requests` right after.
    pub(crate) pending_lsp_requests: Vec<PendingLspRequest>,
    /// `(lsp-notify …)` calls queued this eval; flushed the same way.
    pub(crate) pending_lsp_notifies: Vec<PendingLspNotify>,
    /// Effect-queue length snapshots, one per currently-nested plugin body
    /// (`begin_lazy_activation` pushes, `finish_lazy_activation` pops — LIFO,
    /// matching `plugin_stack`). Lets a failed body's queued effects be
    /// rolled back without touching whatever the enclosing eval already
    /// queued before the nested activation began.
    pub(crate) activation_effect_marks: Vec<EffectMarks>,
    /// Set for the duration of a `manifest.scm` eval driven by a zero-trigger
    /// `(declare-plugin "id")` — the id being resolved. `%begin-manifest-declare!`
    /// sets it, `%finish-manifest-declare!` clears it. Guards against a manifest
    /// declaring a different plugin than the one it was resolved for, and against
    /// a manifest whose own `declare-plugin` is itself zero-trigger (which would
    /// otherwise recurse into manifest resolution forever).
    pub(crate) manifest_resolving: Option<crate::attribution::PluginId>,
}

/// Snapshot of every effect-queue length at the point a plugin body begins
/// evaluating (`begin_lazy_activation`). On a failed activation,
/// `finish_lazy_activation` truncates each queue back to its mark — undoing
/// whatever the partially-evaluated body queued — the same way D2 rolls back
/// `define-command!` calls. `pending_messages` is deliberately excluded: a
/// failed plugin's `log!` output stays visible for debugging.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EffectMarks {
    pending_language_regs: usize,
    pending_lsp_server_ops: usize,
    pending_language_sets: usize,
    pending_grammar_sweeps: usize,
    pending_lsp_requests: usize,
    pending_lsp_notifies: usize,
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
            pending_lsp_server_ops: host_bundle.pending_lsp_server_ops,
            data_dir: host_bundle.data_dir,
            runtime_dir: host_bundle.runtime_dir,
            builtin_cmd_names,
            interrupt_flag: host_bundle.interrupt_flag,
            current_register_prefix: None,
            wait_char_request: None,
            pending_language_sets: Vec::new(),
            pending_char: None,
            is_init: true,
            is_inline_output: false,
            focused_pane_id: PaneId::default(),
            focused_buffer_id: BufferId::default(),
            live_focused_buffer_id: BufferId::default(),
            pending_grammar_sweeps: Vec::new(),
            pending_lsp_requests: Vec::new(),
            pending_lsp_notifies: Vec::new(),
            activation_effect_marks: Vec::new(),
            manifest_resolving: None,
        }
    }

    /// For Rust-side runtime plugin activation (lazy command/event/language activations).
    ///
    /// Identical to `new_init` but with `is_init = false`: native `(call! …)` calls
    /// inside the plugin body are allowed (they run synchronously via `run_command_sync`),
    /// while `(load-plugin …)` and `(declare-plugin …)` are rejected (registration
    /// verbs are init.scm top-level only; a plugin can never load another plugin).
    pub(super) fn new_activation(
        host: &'a mut dyn EditorHost,
        host_bundle: HostBundle<'a>,
        builtin_cmd_names: std::collections::HashSet<String>,
    ) -> Self {
        Self {
            is_init: false,
            ..Self::new_init(host, host_bundle, builtin_cmd_names)
        }
    }

    /// Push a log message — prefer this over direct `pending_messages.push` so
    /// any future severity filter is applied uniformly.
    pub(crate) fn log(&mut self, level: LogLevel, msg: String) {
        self.pending_messages.push((level, msg));
    }

    /// Drain the four per-eval side-effect accumulators into a [`HookResult`],
    /// leaving empty `Vec`s behind. Shared tail for `call_steel_cmd`,
    /// `fire_hook`, and `run_steel_calls` — the single place that knows which
    /// fields make up "this eval's side effects".
    pub(crate) fn take_side_effects(&mut self) -> HookResult {
        HookResult {
            pending_language_sets: std::mem::take(&mut self.pending_language_sets),
            grammar_sweeps: std::mem::take(&mut self.pending_grammar_sweeps),
            pending_lsp_requests: std::mem::take(&mut self.pending_lsp_requests),
            pending_lsp_notifies: std::mem::take(&mut self.pending_lsp_notifies),
        }
    }

    /// Snapshot every effect queue's current length and push it — called by
    /// `begin_lazy_activation` right after it pushes `plugin_stack`, so the
    /// two stacks stay in lockstep (LIFO, one mark per currently-nested body).
    pub(crate) fn mark_effects(&mut self) {
        self.activation_effect_marks.push(EffectMarks {
            pending_language_regs: self.pending_language_regs.len(),
            pending_lsp_server_ops: self.pending_lsp_server_ops.len(),
            pending_language_sets: self.pending_language_sets.len(),
            pending_grammar_sweeps: self.pending_grammar_sweeps.len(),
            pending_lsp_requests: self.pending_lsp_requests.len(),
            pending_lsp_notifies: self.pending_lsp_notifies.len(),
        });
    }

    /// Pop the most recent mark and, on `success == false`, truncate every
    /// effect queue back to it — discarding whatever the failed body queued
    /// before it errored. Called by `finish_lazy_activation` right after it
    /// pops `plugin_stack`. `pending_messages` is untouched: a failed
    /// plugin's `log!` output stays visible for debugging.
    pub(crate) fn pop_effect_marks(&mut self, success: bool) {
        let Some(marks) = self.activation_effect_marks.pop() else {
            return;
        };
        if success {
            return;
        }
        self.pending_language_regs
            .truncate(marks.pending_language_regs);
        self.pending_lsp_server_ops
            .truncate(marks.pending_lsp_server_ops);
        self.pending_language_sets
            .truncate(marks.pending_language_sets);
        self.pending_grammar_sweeps
            .truncate(marks.pending_grammar_sweeps);
        self.pending_lsp_requests
            .truncate(marks.pending_lsp_requests);
        self.pending_lsp_notifies
            .truncate(marks.pending_lsp_notifies);
    }

    pub(crate) fn new_command(
        host: &'a mut dyn EditorHost,
        host_bundle: HostBundle<'a>,
        focused_pane_id: PaneId,
        focused_buffer_id: BufferId,
        pending_char: Option<char>,
    ) -> Self {
        // Read before `host` is moved into the struct below.
        let is_inline_output = host.is_inline_output_command();
        Self {
            host,
            plugin_stack: host_bundle.plugin_stack,
            registries: host_bundle.registries,
            pending_messages: host_bundle.pending_messages,
            pending_language_regs: host_bundle.pending_language_regs,
            pending_lsp_server_ops: host_bundle.pending_lsp_server_ops,
            data_dir: host_bundle.data_dir,
            runtime_dir: host_bundle.runtime_dir,
            builtin_cmd_names: std::collections::HashSet::new(),
            interrupt_flag: host_bundle.interrupt_flag,
            current_register_prefix: None,
            wait_char_request: None,
            pending_language_sets: Vec::new(),
            pending_char,
            is_init: false,
            is_inline_output,
            focused_pane_id,
            focused_buffer_id,
            live_focused_buffer_id: focused_buffer_id,
            pending_grammar_sweeps: Vec::new(),
            pending_lsp_requests: Vec::new(),
            pending_lsp_notifies: Vec::new(),
            activation_effect_marks: Vec::new(),
            manifest_resolving: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::LogLevel;
    use crate::test_support::SteelCtxTestHarness;

    // ── Mode discriminant ─────────────────────────────────────────────────────

    /// `new_init` sets `is_init = true`.
    ///
    /// Fail oracle: swap `is_init: true` → `false` in `new_init` → assert fires.
    #[test]
    fn new_init_has_is_init_true() {
        let mut h = SteelCtxTestHarness::new();
        let ctx = h.ctx_init();
        assert!(ctx.is_init, "new_init must set is_init = true");
    }

    /// `new_command` sets `is_init = false`.
    ///
    /// Fail oracle: swap `is_init: false` → `true` in `new_command` → assert fires.
    #[test]
    fn new_command_has_is_init_false() {
        let mut h = SteelCtxTestHarness::new();
        let ctx = h.ctx();
        assert!(!ctx.is_init, "new_command must set is_init = false");
    }

    /// `new_activation` sets `is_init = false` (same as command mode).
    ///
    /// Runtime-activated plugin bodies use `new_activation` so `(call! …)` is
    /// allowed inside them.  Fail oracle: set `is_init: true` → plugin bodies
    /// would be blocked from calling native commands.
    #[test]
    fn new_activation_has_is_init_false() {
        let mut h = SteelCtxTestHarness::new();
        let ctx = h.ctx_activation();
        assert!(!ctx.is_init, "new_activation must set is_init = false");
    }

    // ── Terminal safety ───────────────────────────────────────────────────────

    /// `new_command` reads `is_inline_output` off the host rather than
    /// hardcoding it — `NullHost` (default) reports `false`.
    ///
    /// Fail oracle: hardcode `is_inline_output: false` in `new_command` →
    /// this assert fires even though the host says `true`.
    #[test]
    fn new_command_reads_inline_output_true_from_host() {
        use crate::null_host::InlineOutputHost;
        let mut host = InlineOutputHost;
        let mut h = SteelCtxTestHarness::new();
        let ctx = h.ctx_with_host(&mut host);
        assert!(
            ctx.is_inline_output,
            "new_command must read is_inline_output_command() from the host"
        );
    }

    /// The harness's default `NullHost` reports `is_inline_output_command() ==
    /// false`, so a plain `ctx()` must carry `is_inline_output == false`.
    #[test]
    fn new_command_defaults_inline_output_false() {
        let mut h = SteelCtxTestHarness::new();
        let ctx = h.ctx();
        assert!(!ctx.is_inline_output, "NullHost must default to false");
    }

    // ── Focus snapshot (new_command) ──────────────────────────────────────────

    /// `new_command` stores the focus IDs passed in; `live_focused_buffer_id`
    /// starts equal to `focused_buffer_id`.
    #[test]
    fn new_command_stores_focus_ids() {
        let mut h = SteelCtxTestHarness::new();
        // ctx() uses PaneId::default() and BufferId::default() as the focus IDs.
        let ctx = h.ctx();
        assert_eq!(ctx.focused_pane_id, PaneId::default());
        assert_eq!(ctx.focused_buffer_id, BufferId::default());
        assert_eq!(
            ctx.live_focused_buffer_id, ctx.focused_buffer_id,
            "live_focused_buffer_id must start equal to focused_buffer_id"
        );
    }

    /// `new_command` stores `pending_char` correctly.
    ///
    /// The harness passes `None`; test `new_command` directly for the `Some` case.
    #[test]
    fn new_command_stores_pending_char() {
        // Use ctx() (None) and verify the field is None.
        let mut h = SteelCtxTestHarness::new();
        let ctx = h.ctx();
        assert_eq!(ctx.pending_char, None, "default ctx has no pending_char");
    }

    // ── init mode: focus IDs are zeroed ───────────────────────────────────────

    /// `new_init` leaves focus IDs at their defaults (not real buffer/pane IDs).
    #[test]
    fn new_init_focus_ids_are_default() {
        let mut h = SteelCtxTestHarness::new();
        let ctx = h.ctx_init();
        assert_eq!(ctx.focused_pane_id, PaneId::default());
        assert_eq!(ctx.focused_buffer_id, BufferId::default());
    }

    // ── log helper ────────────────────────────────────────────────────────────

    /// `ctx.log(…)` appends to `pending_messages`.
    ///
    /// Fail oracle: make `log` a no-op → pending_messages stays empty → assert fires.
    #[test]
    fn log_pushes_to_pending_messages() {
        let mut h = SteelCtxTestHarness::new();
        {
            let mut ctx = h.ctx_init();
            ctx.log(LogLevel::Info, "hello".into());
            ctx.log(LogLevel::Warning, "world".into());
        }
        assert_eq!(h.pending_messages.len(), 2);
        assert_eq!(h.pending_messages[0].0, LogLevel::Info);
        assert_eq!(h.pending_messages[1].0, LogLevel::Warning);
    }
}
