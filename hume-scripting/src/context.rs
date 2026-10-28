use std::collections::HashMap;
use std::path::PathBuf;

use steel::gc::unsafe_erased_pointers::CustomReference;
use steel::rerrs::SteelErr;

use std::rc::Rc;

use super::attribution::{AttributionStack, EntryId, Owner, PluginId, Provenance};
use super::host::EditorHost;
use super::types::{Effect, QueuedEffect};
use super::{HostBundle, ScriptingRegistries};
use hume_core::Severity;

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
    /// init-only method (`host.settings().set_global_option`) is always safe.
    pub(crate) host: &'a mut dyn EditorHost,
    // ── Persistent state borrowed from ScriptingHost ──────────────────────────
    /// The frames of attributed code running: plugin bodies and `call!`ed
    /// commands. See [`Self::owner`].
    pub(crate) attribution: &'a mut AttributionStack,
    /// The persistent registries (`hooks`, `lazy_registry`, `plugin_records`).
    /// Borrowed as a unit, disjoint from `steel`.
    pub(crate) registries: &'a mut ScriptingRegistries,
    /// Log messages accumulated by `(log! …)`.
    pub(crate) pending_messages: &'a mut Vec<(Severity, String)>,
    /// Side effects queued by Steel builtins this eval (and any eval this one
    /// is nested inside), in the exact order they were pushed. Persistent on
    /// `ScriptingHost`, borrowed here; each eval entry point drains its own
    /// contribution back out (see `types::Effect`, `push_effect`).
    pub(crate) effects: &'a mut Vec<QueuedEffect>,
    /// Data/runtime directories (raw + display form), computed once by
    /// `ScriptingHost::new`.
    pub(crate) dirs: &'a crate::builtins::dirs::ScriptDirs,
    // ── Transient per-eval state (owned) ──────────────────────────────────────
    /// Stops the session's budget while a builtin waits on the OS.
    budget: crate::watchdog::BudgetClock<'a>,
    // ── Input state and command side-effects ─────────────────────────────────
    /// Register prefix set by `(set-register-prefix! …)` and inherited by
    /// subsequent `(call! …)` calls until changed.  Resets per invocation
    /// (SteelCtx is rebuilt each time).
    pub(crate) current_register_prefix: Option<char>,
    // ── Mode discriminant ────────────────────────────────────────────────────
    /// Which entry point started this eval session. Set once at construction;
    /// see [`EvalMode`] for the effective legality context builtins gate on,
    /// which also depends on whether a plugin body is running.
    pub(crate) session: EvalSession,
    /// Who the code this session was started to run is credited to: a
    /// file, a command, a hook handler or a callback. [`Self::owner`] and
    /// [`Self::registration_origin`] answer from it while no frame this
    /// session pushed overrides it.
    base: Provenance,
    /// Effect-log length snapshots, one per currently-nested plugin body
    /// (`begin_lazy_activation` pushes, `finish_lazy_activation` pops; LIFO,
    /// matching the bodies on `attribution`). Lets a failed body's own queued effects be
    /// rolled back (while keeping any entries a nested successful activation
    /// already committed) without touching whatever the enclosing eval
    /// queued before this activation began. See `pop_effect_marks`.
    pub(crate) activation_effect_marks: Vec<usize>,
    /// Set for the duration of a `manifest.scm` eval driven by
    /// `(load-plugin! "id")`: the id being evaluated. `%load-plugin!` sets it,
    /// `%finish-manifest-load!` clears it. Guards against a manifest declaring
    /// a different plugin than its own, and against a manifest calling
    /// `load-plugin!` (which would otherwise recurse into manifest evaluation).
    pub(crate) manifest_resolving: Option<PluginId>,
    /// Plugin activations contained mid-session: `finish_lazy_activation`/
    /// `finish_manifest_load` push here instead of letting the body's
    /// error propagate (see their own docs). Drained and reported by
    /// `run_steel_session` once the session ends, in emission order.
    pub(crate) failed_activations: Vec<(EntryId, SteelErr)>,
    /// The file locks `try-lock-file!` took this session, by resolved path.
    /// `run_steel_session` drops what is left when the session ends, so a
    /// lock never outlives the session that took it.
    pub(crate) file_locks: HashMap<PathBuf, crate::builtins::fs::HeldLock>,
}

/// Which entry point started this eval session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvalSession {
    /// `eval_source_raw`: init.scm and the plugins it loads, at startup or
    /// on `:reload-config`. Whether a write may reach the terminal is
    /// `OutputHost::claim_terminal`'s question, not this one.
    Init,
    /// `call_steel_cmd` / `fire_hook` / `run_steel_calls` / `activate_plugin_inline`.
    Runtime,
}

/// Effective legality context that builtins gate on, derived per call from
/// `session` × whether a plugin body is executing (possibly nested inside a
/// command or init eval). Bodies are pushed/popped onto `attribution`
/// mid-eval by `begin_lazy_activation`/`finish_lazy_activation`
/// (`builtins/plugins/activation.rs`), so this is derived fresh via [`SteelCtx::mode`]
/// rather than stored: a single stored 3-variant enum can't distinguish the
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
    /// init.scm top level: `EvalSession::Init`, no plugin body running.
    Init,
    /// Inside an eager `load-plugin!` body during init: `EvalSession::Init`,
    /// a plugin body running.
    PluginLoad,
    /// Inside a lazily-activated plugin body at runtime: `EvalSession::Runtime`,
    /// a plugin body running.
    PluginActivation,
    /// Plain command / hook body: `EvalSession::Runtime`, no plugin body running.
    Command,
}

impl CustomReference for SteelCtx<'_> {}
steel::custom_reference!(SteelCtx<'a>);

impl<'a> SteelCtx<'a> {
    /// For a file's evaluation, run as `owner` on its own behalf:
    /// `init.scm` the user's, or one of HUME's own runtime files.
    pub(super) fn new_init(
        host: &'a mut dyn EditorHost,
        host_bundle: HostBundle<'a>,
        owner: Owner,
    ) -> Self {
        Self {
            host,
            base: Provenance::own(owner),
            attribution: host_bundle.attribution,
            registries: host_bundle.registries,
            pending_messages: host_bundle.pending_messages,
            effects: host_bundle.effects,
            dirs: host_bundle.dirs,
            budget: host_bundle.budget,
            current_register_prefix: None,
            session: EvalSession::Init,
            activation_effect_marks: Vec::new(),
            manifest_resolving: None,
            failed_activations: Vec::new(),
            file_locks: HashMap::new(),
        }
    }

    /// For Rust-side runtime plugin activation (lazy command/event/language activations).
    ///
    /// Identical to `new_init` but with `session = EvalSession::Runtime`: native
    /// `(call! …)` calls inside the plugin body are allowed (they run synchronously
    /// via `run_command_sync`), while `(load-plugin! …)` and `(declare-plugin! …)` are
    /// rejected (registration verbs are init.scm top-level only; a plugin can never
    /// load another plugin). The session's own code is HUME's activation
    /// wrapper; the body it runs is the plugin's.
    pub(super) fn new_activation(
        host: &'a mut dyn EditorHost,
        host_bundle: HostBundle<'a>,
    ) -> Self {
        Self {
            session: EvalSession::Runtime,
            ..Self::new_init(host, host_bundle, Owner::Core)
        }
    }

    /// The effective legality context builtins gate on. See [`EvalMode`].
    pub(crate) fn mode(&self) -> EvalMode {
        match (self.session, self.attribution.in_body()) {
            (EvalSession::Init, false) => EvalMode::Init,
            (EvalSession::Init, true) => EvalMode::PluginLoad,
            (EvalSession::Runtime, true) => EvalMode::PluginActivation,
            (EvalSession::Runtime, false) => EvalMode::Command,
        }
    }

    /// How a registration the running code makes ranks: a plugin body's as
    /// its plugin's, whoever made it run, since a body sets its plugin up;
    /// otherwise as made on the session's behalf. A `call!`ed command runs
    /// on its caller's.
    pub(crate) fn registration_origin(&self) -> hume_core::ConfigOrigin {
        if self.attribution.in_body() {
            hume_core::ConfigOrigin::Plugin
        } else {
            self.base.behalf
        }
    }

    /// Whose code is running: the innermost plugin body or `call!`ed
    /// command this session entered, else the code the session was started
    /// for.
    pub(crate) fn owner(&self) -> &Rc<Owner> {
        self.attribution.top_owner().unwrap_or(&self.base.owner)
    }

    /// The plugin body a registration made now belongs to: rolled back with
    /// it if that body fails, whoever wrote the code making it.
    pub(crate) fn activation(&self) -> Option<Rc<EntryId>> {
        self.attribution.innermost_body().cloned()
    }

    /// The running code's [`Provenance`]: what a proc it hands over is
    /// credited as.
    pub(crate) fn provenance(&self) -> Provenance {
        Provenance {
            owner: Rc::clone(self.owner()),
            behalf: self.registration_origin(),
        }
    }

    /// Push a log message. Prefer this over direct `pending_messages.push` so
    /// any future severity filter is applied uniformly.
    pub(crate) fn log(&mut self, severity: Severity, msg: String) {
        self.pending_messages.push((severity, msg));
    }

    /// Queue `effect` (uncommitted). All builtins push through here so the
    /// `committed` flag has a single origin; see `pop_effect_marks`.
    pub(crate) fn push_effect(&mut self, effect: Effect) {
        self.effects.push(QueuedEffect {
            effect,
            committed: false,
        });
    }

    /// Snapshot the effect log's current length and push it. Called by
    /// `begin_lazy_activation` right after it pushes the body onto
    /// `attribution`, so the marks stay in lockstep with the bodies (LIFO,
    /// one mark per currently-nested body).
    pub(crate) fn mark_effects(&mut self) {
        self.activation_effect_marks.push(self.effects.len());
    }

    /// Pop the most recent mark.
    ///
    /// On `success == true`, marks every entry at `[mark..]` as committed,
    /// including entries already committed by an activation nested deeper
    /// inside this one. Committed effects survive a later enclosing failure
    /// (see `ScriptingHost::take_eval_effects`), because activation itself
    /// (the `PluginState::Loaded` transition, commands registered inline) is
    /// never rolled back once this body finishes successfully.
    ///
    /// On `success == false`, discards whatever this body queued, except
    /// entries already committed by a nested successful activation, which are
    /// kept in place (order preserved). Marks are LIFO and both branches only
    /// touch `[mark..]`, so outer marks and any `effects_start` snapshot
    /// taken before this activation began stay valid indices.
    ///
    /// Called by `finish_lazy_activation` right after it pops the body off
    /// `attribution`.
    /// `pending_messages` is untouched: a failed plugin's `log!` output stays
    /// visible for debugging.
    pub(crate) fn pop_effect_marks(&mut self, success: bool) {
        let Some(mark) = self.activation_effect_marks.pop() else {
            return;
        };
        if success {
            for entry in &mut self.effects[mark..] {
                entry.committed = true;
            }
            return;
        }
        let tail = self.effects.split_off(mark);
        self.effects
            .extend(tail.into_iter().filter(|e| e.committed));
    }

    /// Runs `wait`, a call that blocks on the OS (a child process, a tree
    /// delete), with the session's budget stopped: the budget measures the
    /// time spent running Scheme.
    pub(crate) fn while_waiting<R>(&self, wait: impl FnOnce() -> R) -> R {
        self.budget.waiting(wait)
    }

    /// For a session that runs one proc a script handed over (a command, a
    /// hook handler, a callback), credited as `base`.
    pub(crate) fn new_command(
        host: &'a mut dyn EditorHost,
        host_bundle: HostBundle<'a>,
        base: Provenance,
    ) -> Self {
        Self {
            host,
            base,
            attribution: host_bundle.attribution,
            registries: host_bundle.registries,
            pending_messages: host_bundle.pending_messages,
            effects: host_bundle.effects,
            dirs: host_bundle.dirs,
            budget: host_bundle.budget,
            current_register_prefix: None,
            session: EvalSession::Runtime,
            activation_effect_marks: Vec::new(),
            manifest_resolving: None,
            failed_activations: Vec::new(),
            file_locks: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests;
