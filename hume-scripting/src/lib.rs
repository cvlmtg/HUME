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
mod command_procs;
mod context;
mod eval;
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
pub use eval::HeldHandlers;
pub use host::PendingCompletionSource;
pub use keys::parse_key_stream;
pub use lazy::PluginStatus;
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
pub(crate) use context::SteelCtx;

/// Steel global name under which the live [`SteelCtx`] reference is visible to
/// builtins during an eval (injected by `run_steel_session`, reset to `#void`
/// after).
pub(crate) const HUME_CTX: &str = "*hume.ctx*";

// ── Internal imports ──────────────────────────────────────────────────────────

use std::path::Path;
use std::sync::{Arc, atomic::AtomicBool};

use steel::steel_vm::engine::Engine;

use attribution::PluginStack;
use hooks::HookRegistry;
use lazy::{LazyRegistry, PluginState};

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

    /// Plugin ids that should be activated when `language` is set on a buffer:
    /// those keyed by `language`, then those keyed by the wildcard `"*"`, each
    /// once.
    pub fn activation_language_plugins(&self, language: &str) -> Vec<attribution::EntryId> {
        self.registries.lazy_registry.language_plugins(language)
    }

    /// Status of a plugin in the lazy registry.
    pub fn plugin_status(&self, id: &attribution::EntryId) -> Option<PluginStatus> {
        self.registries
            .lazy_registry
            .plugins
            .get(id)
            .map(PluginState::status)
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
        host: &mut dyn host::EditorHost,
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
        host: &mut dyn host::EditorHost,
    ) -> Result<Vec<Effect>, String> {
        self.eval_source_with_budget(source, 10_000, host)
    }

    /// Evaluates `source` and returns its last value, under
    /// [`call_for_value`](Self::call_for_value)'s read-only rules.
    #[cfg(any(test, feature = "test-util"))]
    pub fn eval_source_for_value(
        &mut self,
        source: &str,
        host: &mut dyn host::EditorHost,
    ) -> Result<steel::rvals::SteelVal, String> {
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
        host: &mut dyn host::EditorHost,
    ) -> Result<(), String> {
        self.eval_source_with_budget(source, budget.as_millis() as u64, host)
            .map(|_| ())
    }
}

/// Pins the assumption the full-trust plugin model rests on: Steel's
/// `steel/process`/`steel/filesystem`/`steel/ports` globals are reachable
/// from plugin code with no `require-builtin` (they ride in via `steel/base`,
/// required by the auto-loaded prelude). PLUM depends on this directly; a
/// steel-core upgrade that changes it must fail these tests before it
/// silently breaks every core plugin.
#[cfg(test)]
mod tests;
