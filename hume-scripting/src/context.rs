use std::sync::{Arc, atomic::AtomicBool};

use steel::gc::unsafe_erased_pointers::CustomReference;

use hume_engine::pipeline::{BufferId, PaneId};

use super::attribution::PluginStack;
use super::host::EditorHost;
use super::log::LogLevel;
use super::types::Effect;
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
    /// In `EvalSession::Init`, `host.buffers()`'s methods are gated by the
    /// `cmd` kind in `builtins!`'s registration table and never called; the
    /// init-only methods (`host.settings().set_global_option`,
    /// `host.keymap().bind_key`, `host.settings().configure_statusline`) are
    /// always safe.
    pub(crate) host: &'a mut dyn EditorHost,
    // ── Persistent state borrowed from ScriptingHost ──────────────────────────
    /// Plugin attribution stack; identifies whose mutation is being recorded.
    pub(crate) plugin_stack: &'a mut PluginStack,
    /// The four persistent registries: cmd_owners, hooks, lazy_registry,
    /// declared_plugins. Borrowed as a unit, disjoint from `steel`.
    pub(crate) registries: &'a mut ScriptingRegistries,
    /// Log messages accumulated by `(log! …)`.
    pub(crate) pending_messages: &'a mut Vec<(LogLevel, String)>,
    /// Side effects queued by Steel builtins this eval (and any eval this one
    /// is nested inside), in the exact order they were pushed. Persistent on
    /// `ScriptingHost`, borrowed here; each eval entry point drains its own
    /// contribution back out (see `types::Effect`).
    pub(crate) effects: &'a mut Vec<Effect>,
    /// Data/runtime directories (raw + display form) and the install-lock
    /// root, computed once by `ScriptingHost::new`.
    pub(crate) dirs: &'a crate::builtins::dirs::ScriptDirs,
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
    /// Pending char from a WaitChar keymap node.
    pub(crate) pending_char: Option<char>,
    // ── Mode discriminant ────────────────────────────────────────────────────
    /// Which entry point started this eval session. Set once at construction;
    /// see [`EvalMode`] for the effective legality context builtins gate on,
    /// which also depends on the live `plugin_stack`.
    pub(crate) session: EvalSession,
    /// True when it is safe to write directly to stdout: either during init
    /// (before the alt-screen TUI is up) or inside an `#:inline-output` command
    /// body (alt-screen temporarily left). See `EditorHost::is_inline_output_command`.
    /// Gates all ten steel-core print shims (`displayln`/`display`/`print`/
    /// `println`/`newline`/`write`/`write-string`/`write-char`/
    /// `simple-display`/`simple-displayln`) via `%stdout-gate!`
    /// (`builtins::io::stdout_gate`) — see that module's doc comment.
    pub(crate) is_inline_output: bool,
    // ── Multi-buffer focus snapshot ──────────────────────────────────────────
    pub(crate) focused_pane_id: PaneId,
    pub(crate) focused_buffer_id: BufferId,
    /// Tracks the live focused buffer across mutations within one command call.
    /// Starts equal to `focused_buffer_id`; updated by `switch-to-buffer!` and
    /// `close-buffer!` so subsequent builtins see the new current buffer.
    pub(crate) live_focused_buffer_id: BufferId,
    /// Effect-log length snapshots, one per currently-nested plugin body
    /// (`begin_lazy_activation` pushes, `finish_lazy_activation` pops — LIFO,
    /// matching `plugin_stack`). Lets a failed body's queued effects be
    /// rolled back (truncating `effects` back to the mark) without touching
    /// whatever the enclosing eval already queued before the nested
    /// activation began.
    pub(crate) activation_effect_marks: Vec<usize>,
    /// Set for the duration of a `manifest.scm` eval driven by a zero-trigger
    /// `(declare-plugin "id")` — the id being resolved. `%begin-manifest-declare!`
    /// sets it, `%finish-manifest-declare!` clears it. Guards against a manifest
    /// declaring a different plugin than the one it was resolved for, and against
    /// a manifest whose own `declare-plugin` is itself zero-trigger (which would
    /// otherwise recurse into manifest resolution forever).
    pub(crate) manifest_resolving: Option<crate::attribution::PluginId>,
}

/// Which entry point started this eval session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvalSession {
    /// `eval_source_raw` — init.scm, before the TUI is up.
    Init,
    /// `call_steel_cmd` / `fire_hook` / `run_steel_calls` / `activate_plugin_inline`.
    Runtime,
}

/// Effective legality context that builtins gate on, derived per call from
/// `session` × whether `plugin_stack` is currently non-empty (a plugin body
/// is executing, possibly nested inside a command or init eval). `plugin_stack`
/// is pushed/popped mid-eval by `begin_lazy_activation`/`finish_lazy_activation`
/// (`builtins/plugins.rs`), so this is derived fresh via [`SteelCtx::mode`]
/// rather than stored — a single stored 3-variant enum can't distinguish the
/// init-top-level state from the eager-plugin-load-during-init state, which
/// have different gate outcomes (see the table below).
///
/// | State                              | `require_cmd` (`cmd` kind) | `require_config` (`config` kind) | `ensure_top_level` |
/// |-------------------------------------|:---:|:---:|:---:|
/// | `Init` (init.scm top level)         | ✗ | ✓ | ✓ |
/// | `PluginLoad` (eager, during init)   | ✗ | ✓ | ✗ |
/// | `PluginActivation` (lazy, runtime)  | ✓ | ✓ | ✗ |
/// | `Command` (plain command/hook body) | ✓ | ✗ | ✗ |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvalMode {
    /// init.scm top level: `EvalSession::Init`, `plugin_stack` empty.
    Init,
    /// Inside an eager `load-plugin` body during init: `EvalSession::Init`,
    /// `plugin_stack` non-empty.
    PluginLoad,
    /// Inside a lazily-activated plugin body at runtime: `EvalSession::Runtime`,
    /// `plugin_stack` non-empty.
    PluginActivation,
    /// Plain command / hook body: `EvalSession::Runtime`, `plugin_stack` empty.
    Command,
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
            effects: host_bundle.effects,
            dirs: host_bundle.dirs,
            builtin_cmd_names,
            interrupt_flag: host_bundle.interrupt_flag,
            current_register_prefix: None,
            wait_char_request: None,
            pending_char: None,
            session: EvalSession::Init,
            is_inline_output: false,
            focused_pane_id: PaneId::default(),
            focused_buffer_id: BufferId::default(),
            live_focused_buffer_id: BufferId::default(),
            activation_effect_marks: Vec::new(),
            manifest_resolving: None,
        }
    }

    /// For Rust-side runtime plugin activation (lazy command/event/language activations).
    ///
    /// Identical to `new_init` but with `session = EvalSession::Runtime`: native
    /// `(call! …)` calls inside the plugin body are allowed (they run synchronously
    /// via `run_command_sync`), while `(load-plugin …)` and `(declare-plugin …)` are
    /// rejected (registration verbs are init.scm top-level only; a plugin can never
    /// load another plugin).
    pub(super) fn new_activation(
        host: &'a mut dyn EditorHost,
        host_bundle: HostBundle<'a>,
        builtin_cmd_names: std::collections::HashSet<String>,
    ) -> Self {
        Self {
            session: EvalSession::Runtime,
            ..Self::new_init(host, host_bundle, builtin_cmd_names)
        }
    }

    /// The effective legality context builtins gate on — see [`EvalMode`].
    pub(crate) fn mode(&self) -> EvalMode {
        match (self.session, self.plugin_stack.is_empty()) {
            (EvalSession::Init, true) => EvalMode::Init,
            (EvalSession::Init, false) => EvalMode::PluginLoad,
            (EvalSession::Runtime, false) => EvalMode::PluginActivation,
            (EvalSession::Runtime, true) => EvalMode::Command,
        }
    }

    /// Push a log message — prefer this over direct `pending_messages.push` so
    /// any future severity filter is applied uniformly.
    pub(crate) fn log(&mut self, level: LogLevel, msg: String) {
        self.pending_messages.push((level, msg));
    }

    /// Snapshot the effect log's current length and push it — called by
    /// `begin_lazy_activation` right after it pushes `plugin_stack`, so the
    /// two stacks stay in lockstep (LIFO, one mark per currently-nested body).
    pub(crate) fn mark_effects(&mut self) {
        self.activation_effect_marks.push(self.effects.len());
    }

    /// Pop the most recent mark and, on `success == false`, truncate the
    /// effect log back to it — discarding whatever the failed body queued
    /// before it errored. Called by `finish_lazy_activation` right after it
    /// pops `plugin_stack`. `pending_messages` is untouched: a failed
    /// plugin's `log!` output stays visible for debugging.
    pub(crate) fn pop_effect_marks(&mut self, success: bool) {
        let Some(mark) = self.activation_effect_marks.pop() else {
            return;
        };
        if success {
            return;
        }
        self.effects.truncate(mark);
    }

    pub(crate) fn new_command(
        host: &'a mut dyn EditorHost,
        host_bundle: HostBundle<'a>,
        focused_pane_id: PaneId,
        focused_buffer_id: BufferId,
        pending_char: Option<char>,
    ) -> Self {
        // Read before `host` is moved into the struct below.
        let is_inline_output = host
            .output()
            .is_some_and(|output| output.is_inline_output_command());
        Self {
            host,
            plugin_stack: host_bundle.plugin_stack,
            registries: host_bundle.registries,
            pending_messages: host_bundle.pending_messages,
            effects: host_bundle.effects,
            dirs: host_bundle.dirs,
            builtin_cmd_names: std::collections::HashSet::new(),
            interrupt_flag: host_bundle.interrupt_flag,
            current_register_prefix: None,
            wait_char_request: None,
            pending_char,
            session: EvalSession::Runtime,
            is_inline_output,
            focused_pane_id,
            focused_buffer_id,
            live_focused_buffer_id: focused_buffer_id,
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

    /// `new_init` sets `session = EvalSession::Init`.
    ///
    /// Fail oracle: swap `EvalSession::Init` → `Runtime` in `new_init` → assert fires.
    #[test]
    fn new_init_has_init_session() {
        let mut h = SteelCtxTestHarness::new();
        let ctx = h.ctx_init();
        assert_eq!(
            ctx.session,
            EvalSession::Init,
            "new_init must set session = EvalSession::Init"
        );
    }

    /// `new_command` sets `session = EvalSession::Runtime`.
    ///
    /// Fail oracle: swap `EvalSession::Runtime` → `Init` in `new_command` → assert fires.
    #[test]
    fn new_command_has_runtime_session() {
        let mut h = SteelCtxTestHarness::new();
        let ctx = h.ctx();
        assert_eq!(
            ctx.session,
            EvalSession::Runtime,
            "new_command must set session = EvalSession::Runtime"
        );
    }

    /// `new_activation` sets `session = EvalSession::Runtime` (same as command mode).
    ///
    /// Runtime-activated plugin bodies use `new_activation` so `(call! …)` is
    /// allowed inside them.  Fail oracle: set `session: EvalSession::Init` →
    /// plugin bodies would be blocked from calling native commands.
    #[test]
    fn new_activation_has_runtime_session() {
        let mut h = SteelCtxTestHarness::new();
        let ctx = h.ctx_activation();
        assert_eq!(
            ctx.session,
            EvalSession::Runtime,
            "new_activation must set session = EvalSession::Runtime"
        );
    }

    /// `mode()` derives the correct `EvalMode` for all four `(session,
    /// plugin_stack)` states. Independent oracle: expected variants come from
    /// the truth table in `EvalMode`'s doc, not from `mode()`'s own logic.
    ///
    /// Fail oracle: swap any two arms in `SteelCtx::mode`'s match → one of
    /// these four assertions fires.
    #[test]
    fn mode_derives_from_session_and_plugin_stack() {
        use crate::attribution::PluginId;

        let mut h = SteelCtxTestHarness::new();
        assert_eq!(h.ctx_init().mode(), EvalMode::Init);
        assert_eq!(h.ctx().mode(), EvalMode::Command);

        h.plugin_stack
            .push(PluginId::parse("core:test-plugin").unwrap());
        assert_eq!(h.ctx_init().mode(), EvalMode::PluginLoad);
        assert_eq!(h.ctx_activation().mode(), EvalMode::PluginActivation);
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
        let mut host = InlineOutputHost::default();
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
