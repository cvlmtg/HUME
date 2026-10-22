//! Steel scripting integration for HUME.
//!
//! [`ScriptingHost`] owns the Steel [`Engine`] and runs on the main event-loop
//! thread: the Engine is `!Send`, and edit commands are synchronous hot-path
//! operations where an IPC round-trip per keystroke would cost more.
//!
//! ## Plugin loading
//! - `load-plugin!`: init.scm/`:reload-config` only. A plugin with a
//!   `manifest.scm` is lazy: the manifest's `declare-plugin!` calls record the
//!   entries. One without runs `plugin.scm` at once via
//!   `%activate-plugin-inline!`.
//! - `declare-plugin!`: records `Declared` plus activation entries in
//!   `LazyRegistry`, from a manifest or from init.scm (a `./file.scm` beside
//!   init.scm, or an installed plugin with custom triggers). The first entry
//!   exercised runs the body, marks it `Loaded`, and drops all of that
//!   entry's triggers.
//! - States: `Declared -> Loading -> Loaded | Failed`. `Loading` guards
//!   re-entrant cycles; `Failed` doesn't retry until `:reload-config`.
//! - `#:config` on `load-plugin!` is opaque per-plugin data read back via
//!   `(plugin-config)`. Both forms record an installed plugin's name in its
//!   plugin record up front for PLUM; a local file is not listed.

#![deny(rustdoc::broken_intra_doc_links)]
#![deny(rustdoc::private_intra_doc_links)]

// ── Public submodules ─────────────────────────────────────────────────────────
pub mod attribution;
pub(crate) mod builtins;
pub mod command_name;
pub(crate) mod hooks;
pub mod host;
pub mod json;
pub(crate) mod keys;
pub(crate) mod lazy;
mod log;
mod plugin_record;
// ── Private implementation details ────────────────────────────────────────────
mod activation;
mod command_procs;
mod context;
#[cfg(test)]
mod null_host;
mod read_only_host;
#[cfg(any(test, feature = "test-util"))]
mod test_scm;
#[cfg(test)]
mod test_support;
#[cfg(any(test, feature = "test-util"))]
pub use test_scm::eager_load_scm;
mod types;
pub(crate) mod watchdog;

// ── Public API re-exports ─────────────────────────────────────────────────────
// Types the editor and editor tests use directly.
pub use builtins::args::line_range_value;
pub use builtins::args::symbol_hash;
pub use builtins::ids::{DocPos, DocRange, ServerRef, SteelPane};
pub use command_procs::{CommandProc, MappableProc, TypedProc};
pub use host::PendingCompletionSource;
pub use keys::parse_key_stream;
pub use log::LogLevel;
pub use types::{
    CapabilityQuery, ConfigOrigin, DocSpan, Effect, EvalError, FeatureFilter, GrammarReg,
    ListEntry, ListLayer, LspFeature, LspServerStatusEntry, LspServerTarget, PaneHandle, Params,
    PendingLanguageReg, PendingLspNotify, PendingLspRequest, PendingLspServerOp,
    PendingLspServerReg, RequestMode, RequestParams, RouteSpec, ServerName, SteelCmdBody,
    SteelCmdDef, SteelTypedCmdBody, SteelTypedCmdDef, TriggerKind, TriggerScope, VirtualLineSpec,
    WhenUnavailable,
};
// Test-only external visibility: the editor's own test suite arms/cancels a
// real watchdog directly (hume-editor/tests/scripting.rs) rather than
// through a ScriptingHost eval entry point. Outside test/test-util builds,
// `EvalWatchdog` is still needed unqualified below (the `watchdog` field,
// `steel_and_bundle`'s signature), just not re-exported past this crate.
#[cfg(any(test, feature = "test-util"))]
pub use watchdog::EvalWatchdog;
#[cfg(not(any(test, feature = "test-util")))]
use watchdog::EvalWatchdog;
// Test-only external visibility: every test crate that builds `.scm` source
// by interpolating a real filesystem path (`open-buffer!`, `picker!`, …)
// needs this same Windows-safe escaping. See `steel_path_literal`'s own doc
// comment for why it isn't hand-rolled per call site.
#[cfg(any(test, feature = "test-util"))]
pub use builtins::plugins::steel_path_literal;

// ── Internal re-exports (within-crate use) ────────────────────────────────────
pub(crate) use activation::run_steel_session;
pub(crate) use context::SteelCtx;

/// Steel global name under which the live [`SteelCtx`] reference is visible to
/// builtins during an eval (injected by `run_steel_session`, reset to `#void`
/// after).
pub(crate) const HUME_CTX: &str = "*hume.ctx*";

// ── Internal imports ──────────────────────────────────────────────────────────

use std::path::Path;
use std::sync::{Arc, atomic::AtomicBool};

use steel::rvals::SteelVal;
use steel::steel_vm::engine::Engine;

use attribution::PluginStack;
use hooks::HookRegistry;
use host::{EditorHost, HostToken};
use lazy::{LazyRegistry, PluginState};
use read_only_host::ReadOnlyHost;

/// The persistent registry fields bundled as a unit so they can be
/// borrowed as a single `&mut ScriptingRegistries`, disjoint from the
/// Steel VM (`steel`) and the rest of `ScriptingHost`.
#[derive(Default)]
pub(crate) struct ScriptingRegistries {
    /// Persistent hook registry: handlers registered by `(register-hook! …)`.
    pub(crate) hooks: HookRegistry,
    /// Lazy plugin registry: populated by `%declare-plugin!` during init;
    /// activation maps consulted by command dispatch, event firing, and language-set.
    pub(crate) lazy_registry: LazyRegistry,
    /// Plugin-level state of every plugin named by `(load-plugin! …)` or
    /// `(declare-plugin! …)`, including plugins absent on disk.
    pub(crate) plugin_records: plugin_record::PluginRecords,
}

/// Borrows of [`ScriptingHost`] fields needed to populate [`SteelCtx`].
///
/// Built by [`ScriptingHost::steel_and_bundle`] and passed to
/// [`SteelCtx::new_init`] or [`SteelCtx::new_command`]. Private to this module.
pub(crate) struct HostBundle<'a> {
    pub(crate) registries: &'a mut ScriptingRegistries,
    plugin_stack: &'a mut PluginStack,
    pending_messages: &'a mut Vec<(LogLevel, String)>,
    effects: &'a mut Vec<types::QueuedEffect>,
    dirs: &'a builtins::dirs::ScriptDirs,
    /// Owned `Arc` clone: `new_init`/`new_command` consume it via move into
    /// `SteelCtx::interrupt_flag`, avoiding a second clone at eval time.
    interrupt_flag: Arc<AtomicBool>,
}

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

/// The embedded Steel scripting host.
///
/// Owns the Steel `Engine` and all persistent scripting state.  Each eval or
/// command call constructs a `SteelCtx` that borrows the persistent fields
/// directly.
///
/// Constructed once during `Editor::init_scripting()` and held for the
/// lifetime of the process.
pub struct ScriptingHost {
    /// The Scheme VM, always called `steel` (never bare "engine", which refers to the `engine/` crate).
    steel: Engine,
    /// The persistent registries borrowed as a unit into `SteelCtx`, disjoint
    /// from `steel` so the VM and command/hook state can be borrowed
    /// simultaneously (NLL field-split).
    pub(crate) registries: ScriptingRegistries,
    /// Attribution stack: `stack.last()` is the plugin currently executing.
    /// Empty → top-level `init.scm` → `Owner::User`.
    plugin_stack: PluginStack,
    /// Log messages accumulated by `(log! …)` since the last drain.
    /// Drained by the editor via `take_pending_messages()`.
    pending_messages: Vec<(LogLevel, String)>,
    /// Side effects queued by Steel builtins, in push order. Each eval entry
    /// point (`call_steel_cmd`, `fire_hook`, `run_steel_calls`, `eval_init`,
    /// `activate_plugin_inline`) drains exactly what it pushed back out on
    /// success (`Vec<Effect>`). On error it drains only the entries committed
    /// by a nested successful plugin activation (`QueuedEffect::committed`);
    /// see `take_eval_effects` and `types::EvalError`.
    effects: Vec<types::QueuedEffect>,
    /// Data/runtime directories (raw + display form) and the install-lock
    /// root, computed once at construction.
    dirs: builtins::dirs::ScriptDirs,
    /// Shared interrupt flag.  Set to `true` by the watchdog to signal that
    /// `(hume/yield!)` calls should abort the running script.  Reset to
    /// `false` after every eval (command dispatch, hook fires, and plugin
    /// activation, not just `eval_init`).
    interrupt_flag: Arc<AtomicBool>,
    /// Persistent budget-enforcement thread, re-armed around every eval.
    watchdog: EvalWatchdog,
}

impl ScriptingHost {
    /// Create a new scripting host with the Steel standard library and all HUME
    /// builtins loaded.
    ///
    /// `dirs` supplies the data and runtime directories builtins expose, and
    /// the grammar compiler, so no builtin reads environment variables on a
    /// call.
    pub fn new(dirs: &hume_platform::dirs::Dirs) -> Self {
        let dirs = builtins::dirs::ScriptDirs::new(dirs);
        let mut steel = Engine::new();
        builtins::register_all(&mut steel);
        Self {
            steel,
            registries: ScriptingRegistries::default(),
            plugin_stack: PluginStack::default(),
            pending_messages: Vec::new(),
            effects: Vec::new(),
            dirs,
            interrupt_flag: Arc::new(AtomicBool::new(false)),
            watchdog: EvalWatchdog::new(),
        }
    }
}

impl ScriptingHost {
    /// Split `self` into the Steel engine, the watchdog, and the [`HostBundle`]
    /// of persistent borrows needed to build a [`SteelCtx`].
    ///
    /// The returned borrows are disjoint fields of `self` (NLL field-split), so
    /// the VM can be run while the bundle is lent to the eval context. Shared
    /// by `eval_source_raw`, `activate_plugin_inline`, `call_steel_cmd`, and
    /// `fire_hook`.
    pub(crate) fn steel_and_bundle(&mut self) -> (&mut Engine, &EvalWatchdog, HostBundle<'_>) {
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
}

/// `true` for a name plugin code never calls directly, so
/// [`ScriptingHost::host_global_names`] leaves it out of the generated
/// `steel-language-server` host-globals file:
/// - `%`-prefixed: HUME's private primitives, each wrapped by a public name
///   (`%register-lsp-server!` by `register-lsp-server!`).
/// - `*earmuffed*`: Scheme's convention for internal state (`*hume.ctx*`).
/// - `hm.`-prefixed: `steel/meta`, aliased by `bootstrap.scm` for HUME's glue.
/// - `#`-prefixed: steel-core's internal markers, including the
///   nondeterministically numbered `###ctx-funcN` builtin wrappers.
/// - `struct:NAME` and `___NAME-options___`: the type descriptor and the
///   options hash steel-core's `struct` macro binds beside every struct.
///   A plugin builds and reads a struct through its constructor, predicate
///   and accessors.
#[cfg(any(test, feature = "test-util"))]
fn is_internal_name(name: &str) -> bool {
    name.starts_with('%')
        || name.starts_with('#')
        || name.starts_with("hm.")
        || (name.starts_with('*') && name.ends_with('*'))
        || name.starts_with("struct:")
        || (name.starts_with("___") && name.ends_with("-options___"))
}

impl ScriptingHost {
    // ── Outward API (clean; no direct field access outside this module) ────────

    /// Pre-register native command names as callable Steel bindings.
    ///
    /// For each name, evaluates `(define name (lambda args (%dispatch-command!
    /// "name" args)))`. This makes bare `(move-left pane)` callable without
    /// `(call! "move-left" pane)`, and variadic, so `(move-down pane 3)` /
    /// `(move-down pane 0)` work too (count `0` = "no count typed", see
    /// `parse_native_args`). `pane` is always the first arg, same as
    /// `%call-native!`'s own contract: this binding is a thin wrapper over
    /// the same dispatcher, not a separate calling convention.
    ///
    /// Calls `%dispatch-command!` directly rather than the public `call!` macro
    /// (which desugars to exactly this): the variadic lambda's args are
    /// already the list `%dispatch-command!` expects, no intermediate
    /// `(list ...)` needed.
    ///
    /// Called from `Editor::init_scripting`, after `ScriptingHost::new` and
    /// before any `eval_init`, so init.scm/plugins can use bare command names
    /// without a `FreeIdentifier` compile error.
    pub fn register_command_names(&mut self, names: &[&str]) {
        if names.is_empty() {
            return;
        }
        // Build one compound source string: one define per command.
        let mut source = String::with_capacity(names.len() * 64);
        for &name in names {
            source.push_str("(define ");
            source.push_str(name);
            source.push_str(" (lambda args (%dispatch-command! \"");
            source.push_str(name);
            source.push_str("\" args)))\n");
        }
        self.steel
            .compile_and_run_raw_program(source)
            .expect("command name pre-registration failed: this is a bug");
    }

    /// Runtime directory for core plugins, themes, and docs.
    pub fn runtime_dir(&self) -> Option<&Path> {
        self.dirs.runtime_dir.as_deref()
    }

    /// Data directory for user/third-party plugins.
    pub fn data_dir(&self) -> Option<&Path> {
        self.dirs.data_dir.as_deref()
    }

    /// Every top-level global and macro name HUME's own layers add on top of
    /// a pristine Steel engine, minus HUME's own internal-only names: the
    /// payload for the generated `steel-language-server` host-globals file
    /// (see `runtime/plugins/core/steel-server/lsp-home/hume-globals.scm`'s
    /// generator).
    ///
    /// Diffs against a fresh `Engine::new()` so upstream Steel stdlib names
    /// never appear in the output (the server already knows those from its
    /// own internal engine), then drops anything `is_internal_name` flags:
    /// HUME's own naming conventions for "never called by plugin code",
    /// checked against a full, real-world generated list to confirm no
    /// legitimate public name matches any of them.
    ///
    /// Sorted and deduped for a stable, reviewable diff in the checked-in
    /// generated file.
    ///
    /// Reads two guards on the same `RwLock` sequentially (never held
    /// simultaneously, since `parking_lot`'s `RwLock` is not reentrant): globals
    /// first, then macros, each collected into an owned `Vec` before the
    /// guard drops.
    #[cfg(any(test, feature = "test-util"))]
    pub fn host_global_names(&self) -> Vec<String> {
        let baseline = Engine::new();
        let baseline_globals: rustc_hash::FxHashSet<String> = baseline
            .globals()
            .iter()
            .map(|s| s.resolve().to_string())
            .collect();
        let baseline_macros: rustc_hash::FxHashSet<String> = baseline
            .in_scope_macros()
            .keys()
            .map(|s| s.resolve().to_string())
            .collect();

        let mut names: Vec<String> = {
            let globals = self.steel.globals();
            globals
                .iter()
                .map(|s| s.resolve().to_string())
                .filter(|n| !is_internal_name(n) && !baseline_globals.contains(n))
                .collect()
        };
        names.extend({
            let macros = self.steel.in_scope_macros();
            macros
                .keys()
                .map(|s| s.resolve().to_string())
                .filter(|n| !is_internal_name(n) && !baseline_macros.contains(n))
                .collect::<Vec<_>>()
        });
        names.sort_unstable();
        names.dedup();
        names
    }

    /// Drain all accumulated log messages since the last drain.
    pub fn take_pending_messages(&mut self) -> Vec<(LogLevel, String)> {
        std::mem::take(&mut self.pending_messages)
    }

    /// Returns `true` if at least one handler for `name` fires for an event
    /// carrying `key`: an unkeyed handler always does, a keyed one only when
    /// `key` is among its keys.
    pub fn has_hook_handlers(&self, name: &str, key: Option<&str>) -> bool {
        self.registries.hooks.has_match(name, key)
    }

    /// A snapshot of the language activation entries declared during init (language → plugins).
    pub fn activation_languages(&self) -> rustc_hash::FxHashMap<String, Vec<attribution::EntryId>> {
        self.registries.lazy_registry.activation_languages.clone()
    }

    /// Plugin ids that should be activated when the event named `name` fires.
    pub fn activation_event_plugins(&self, name: &str) -> Vec<attribution::EntryId> {
        self.registries
            .lazy_registry
            .activation_events
            .get(name)
            .cloned()
            .unwrap_or_default()
    }

    /// Plugin ids that should be activated when `language` is set on a buffer.
    ///
    /// Unions entries keyed by `language` with entries keyed by the wildcard
    /// `"*"` (a manifest that can't enumerate every language it might ever
    /// support), deduped so a plugin listed under both activates once.
    pub fn activation_language_plugins(&self, language: &str) -> Vec<attribution::EntryId> {
        let activation_languages = &self.registries.lazy_registry.activation_languages;
        let specific = activation_languages.get(language);
        let wildcard = activation_languages.get("*");
        let mut plugins: Vec<attribution::EntryId> = specific.cloned().unwrap_or_default();
        if let Some(wildcard) = wildcard {
            for id in wildcard {
                if !plugins.contains(id) {
                    plugins.push(id.clone());
                }
            }
        }
        plugins
    }

    /// Status of a plugin in the lazy registry.
    pub fn plugin_status(&self, id: &attribution::EntryId) -> Option<PluginStatus> {
        self.registries
            .lazy_registry
            .plugins
            .get(id)
            .map(|state| match state {
                PluginState::Declared { .. } => PluginStatus::Declared,
                PluginState::Loading => PluginStatus::Loading,
                PluginState::Loaded => PluginStatus::Loaded,
                PluginState::Failed => PluginStatus::Failed,
            })
    }

    /// Returns `true` if any plugin in the registry has transitioned to `Loaded`.
    #[cfg(any(test, feature = "test-util"))]
    pub fn has_any_loaded_plugin(&self) -> bool {
        self.registries
            .lazy_registry
            .plugins
            .values()
            .any(|s| matches!(s, PluginState::Loaded))
    }

    /// All plugin names ever passed to `(load-plugin! …)` or `(declare-plugin! …)`.
    #[cfg(any(test, feature = "test-util"))]
    pub fn declared_plugins(&self) -> Vec<String> {
        self.registries.plugin_records.plum_names().collect()
    }

    /// Format a human-readable plugin status table for `:plugin-status`.
    ///
    /// `lazy_cmds` is the editor's current `Lazy`-stub list (`name`, owning
    /// plugin, `is_typed`). This crate doesn't track pending command
    /// activations itself, so the caller supplies its live registry snapshot.
    pub fn lazy_status_string(&self, lazy_cmds: &[(String, attribution::EntryId, bool)]) -> String {
        self.registries
            .lazy_registry
            .format_status(self.registries.plugin_records.entryless_rows(), lazy_cmds)
    }

    /// Peek at pending messages without draining.  Only for test assertions.
    #[cfg(any(test, feature = "test-util"))]
    pub fn peek_pending_messages(&self) -> &[(LogLevel, String)] {
        &self.pending_messages
    }

    /// Peek at the effect log without draining.  Only for test assertions:
    /// production code always goes through an eval entry point's own
    /// atomic drain (`take_eval_effects`).
    #[cfg(any(test, feature = "test-util"))]
    pub fn effects_for_test(&self) -> Vec<&Effect> {
        self.effects.iter().map(|e| &e.effect).collect()
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn interrupt_flag_for_test(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        std::sync::Arc::clone(&self.interrupt_flag)
    }

    /// Current plugin activation nesting depth (number of bodies on the call
    /// stack).
    #[cfg(test)]
    pub(crate) fn plugin_stack_depth_for_test(&self) -> usize {
        self.plugin_stack.len()
    }

    /// Push a fake plugin id onto the attribution stack.
    #[cfg(test)]
    pub(crate) fn push_plugin_for_test(&mut self, id: attribution::EntryId) {
        self.plugin_stack.push(id);
    }

    // ── Eval machinery ────────────────────────────────────────────────────────

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
    ///   `builtins::plugins::finish_lazy_activation`), reported by name and
    ///   location as an `Error`-severity pending message (drained by
    ///   `take_pending_messages`; queued from `activation::run_steel_session`),
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
    /// Atomicity is **not** all-or-nothing across the whole file even for
    /// this genuine top-level case: only deferred effects (keybinds, LSP
    /// registration, …) roll back on error. `define-command!` and
    /// `register-hook!` write the editor's command registry / `HookRegistry`
    /// inline the instant the builtin runs (see `builtins::commands`/`builtins::hooks`),
    /// so a `define-command!` or `register-hook!` that ran before the
    /// failing form stays live in the degraded session. This is deliberate:
    /// a config error already surfaces to the user, and `:reload-config`
    /// rebuilds a fresh `ScriptingHost`, so the half-applied state never
    /// accumulates across reloads.
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
        let effects_start = self.effects.len();
        let result = {
            let (steel, watchdog, bundle) = self.steel_and_bundle();
            let mut steel_ctx = SteelCtx::new_command(host, bundle);
            run_steel_session(steel, watchdog, &mut steel_ctx, budget_ms, |steel| {
                proc.call(steel, args)?;
                Ok(())
            })
            .map_err(|e| e.message)
        };

        self.take_eval_effects(effects_start, result)
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
    /// aborts the rest, distinguished via [`activation::SessionError::
    /// interrupted`], read off `ctx.interrupt_flag` before that session
    /// resets it, rather than sniffing the interrupt's own fixed message
    /// text.
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
                Err(activation::SessionError {
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
    fn read_only_call(
        &mut self,
        host: &mut dyn EditorHost,
        call: impl FnOnce(&mut Engine) -> Result<SteelVal, steel::rerrs::SteelErr>,
    ) -> Result<SteelVal, String> {
        let budget_ms = host.settings().steel_command_budget_ms();
        let effects_start = self.effects.len();
        let mut read_only = ReadOnlyHost::new(host);
        let mut value = None;
        let result = {
            let (steel, watchdog, bundle) = self.steel_and_bundle();
            let mut steel_ctx = SteelCtx::new_command(&mut read_only, bundle);
            run_steel_session(steel, watchdog, &mut steel_ctx, budget_ms, |steel| {
                value = Some(call(steel)?);
                Ok(())
            })
            .map_err(|e| e.message)
        };
        let queued = self.effects.split_off(effects_start);
        result?;
        if let Some(refused) = read_only.refused() {
            return Err(refused);
        }
        if !queued.is_empty() {
            return Err("commands cannot be queued during a read-only call".to_string());
        }
        Ok(value.expect("a session that succeeded ran the call"))
    }
}

// ── Test helpers ──────────────────────────────────────────────────────────────

#[cfg(any(test, feature = "test-util"))]
impl ScriptingHost {
    /// Shared tail for [`eval_source`](Self::eval_source) and
    /// [`eval_source_watchdog`](Self::eval_source_watchdog): delegates to
    /// `eval_source_raw` with empty `builtin_names`, at the given budget.
    fn eval_source_with_budget(
        &mut self,
        source: &str,
        budget_ms: u64,
        host: &mut dyn EditorHost,
    ) -> Result<Vec<Effect>, String> {
        self.eval_source_raw(source.to_owned(), Default::default(), budget_ms, host)
            .map_err(|e| e.message)
    }

    /// Evaluate a Steel source string directly, without a file.
    ///
    /// Convenience wrapper for testing, at the default 10-second init budget
    /// (harmless for normal tests that complete quickly). Returns the
    /// effects the eval queued, in emission order, same contract as
    /// `eval_init`, so a test can assert on what a builtin marshalled across
    /// the boundary.
    pub fn eval_source(
        &mut self,
        source: &str,
        host: &mut dyn EditorHost,
    ) -> Result<Vec<Effect>, String> {
        self.eval_source_with_budget(source, 10_000, host)
    }

    /// Evaluates `source` and returns its last value, under
    /// [`call_for_value`](Self::call_for_value)'s read-only rules.
    #[cfg(any(test, feature = "test-util"))]
    pub fn eval_source_for_value(
        &mut self,
        source: &str,
        host: &mut dyn EditorHost,
    ) -> Result<SteelVal, String> {
        let source = source.to_owned();
        self.read_only_call(host, |steel| {
            steel
                .compile_and_run_raw_program(source)?
                .pop()
                .ok_or_else(|| builtins::errors::generic_err("the source produced no value"))
        })
    }

    /// Like [`eval_source`](Self::eval_source) but arms a real
    /// [`EvalWatchdog`] with the given budget.  Used by watchdog-specific
    /// tests that need to verify the watchdog actually fires rather than
    /// pre-setting the interrupt flag.
    pub fn eval_source_watchdog(
        &mut self,
        source: &str,
        budget: std::time::Duration,
        host: &mut dyn EditorHost,
    ) -> Result<(), String> {
        self.eval_source_with_budget(source, budget.as_millis() as u64, host)
            .map(|_| ())
    }
}

// ── Public status enum ────────────────────────────────────────────────────────

/// Payload-free public view of a plugin's lifecycle state.
///
/// Maps from the private `PluginState` enum (which carries a `PathBuf` for
/// `Declared`) without leaking internal details.  Used by `plugin_status()` so
/// the editor-crate test suite can assert on states without reaching into
/// `ScriptingHost` fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginStatus {
    /// Declared but not yet loaded.
    Declared,
    /// Currently being loaded (re-entrancy guard).
    Loading,
    /// Loaded successfully.
    Loaded,
    /// Load failed (will not retry until `:reload-config`).
    Failed,
}

/// Pins the assumption the full-trust plugin model rests on: Steel's
/// `steel/process`/`steel/filesystem`/`steel/ports` globals are reachable
/// from plugin code with no `require-builtin` (they ride in via `steel/base`,
/// required by the auto-loaded prelude). PLUM depends on this directly; a
/// steel-core upgrade that changes it must fail these tests before it
/// silently breaks every core plugin.
#[cfg(test)]
mod tests;
