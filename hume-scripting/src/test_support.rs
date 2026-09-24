//! Test support types for scripting crate unit tests.
//!
//! Lives in the scripting crate because it needs access to private types
//! (`SteelCtx`, `HostBundle`).

use std::sync::{Arc, atomic::AtomicBool};

use crate::attribution::PluginStack;
use crate::builtins::dirs::ScriptDirs;
use crate::context::SteelCtx;
use crate::log::LogLevel;
use crate::null_host::NullHost;
use crate::types::QueuedEffect;
use crate::{HostBundle, ScriptingRegistries};

/// A stand-in bid for a builtin test that doesn't care which buffer it
/// names — [`NullHost`]/[`crate::testing`] mocks don't track real buffers,
/// so any value type-checks the same. A direct Rust call to a builtin
/// function (every caller of this) never runs `LivePane`'s liveness check at
/// all — that only happens inside the `builtins!`-registered Steel
/// closure — so this needs no "live" counterpart.
pub(crate) fn default_bid() -> hume_engine::pipeline::BufferId {
    hume_engine::pipeline::BufferId::default()
}

/// [`default_bid`]'s `PaneHandle` counterpart — a pane-less handle, for a
/// direct Rust call into a builtin now typed for `PaneHandle`/`ArgPane`/
/// `LivePane` rather than a bare `BufferId`.
pub(crate) fn default_pane() -> crate::types::PaneHandle {
    crate::types::PaneHandle::buffer_only(default_bid())
}

/// [`default_pane`], but with a pane half too — for a builtin whose own
/// contract needs one present (`#:require-focus`'s "needs a pane" check,
/// a kind-A/B builtin exercised only for its decode, never resolved against
/// real editor state through `NullHost`).
pub(crate) fn default_pane_with_pane() -> crate::types::PaneHandle {
    crate::types::PaneHandle::with_pane(default_bid(), hume_engine::pipeline::PaneId::default())
}

/// Backing storage for [`SteelCtx`] in scripting-crate unit tests.
///
/// Uses [`NullHost`] — sufficient for tests that only need to check that
/// scripting guards (`EvalMode`, `errors::require_cmd`/`require_config`,
/// etc.) fire correctly, without real editor state.
pub(crate) struct SteelCtxTestHarness {
    pub(crate) host: NullHost,
    pub(crate) plugin_stack: PluginStack,
    pub(crate) registries: ScriptingRegistries,
    pub(crate) pending_messages: Vec<(LogLevel, String)>,
    pub(crate) effects: Vec<QueuedEffect>,
    pub(crate) dirs: ScriptDirs,
    pub(crate) interrupt_flag: Arc<AtomicBool>,
}

/// Builds a [`HostBundle`] from the harness's persistent fields — the shared
/// tail every `ctx_*` constructor below ends with. A free function (not a
/// `&mut self` method) because the constructors also need `&mut self.host`
/// borrowed independently in the same call — same NLL field-split pattern as
/// `ScriptingHost::steel_and_bundle`.
fn bundle<'a>(
    registries: &'a mut ScriptingRegistries,
    plugin_stack: &'a mut PluginStack,
    pending_messages: &'a mut Vec<(LogLevel, String)>,
    effects: &'a mut Vec<QueuedEffect>,
    dirs: &'a ScriptDirs,
    interrupt_flag: &Arc<AtomicBool>,
) -> HostBundle<'a> {
    HostBundle {
        registries,
        plugin_stack,
        pending_messages,
        effects,
        dirs,
        interrupt_flag: Arc::clone(interrupt_flag),
    }
}

impl SteelCtxTestHarness {
    pub(crate) fn new() -> Self {
        Self {
            host: NullHost,
            plugin_stack: PluginStack::default(),
            registries: ScriptingRegistries::default(),
            pending_messages: Vec::new(),
            effects: Vec::new(),
            dirs: ScriptDirs::new(None, None),
            interrupt_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Build a `SteelCtx` in init mode (`session = EvalSession::Init`).
    pub(crate) fn ctx_init(&mut self) -> SteelCtx<'_> {
        let Self {
            host,
            plugin_stack,
            registries,
            pending_messages,
            effects,
            dirs,
            interrupt_flag,
        } = self;
        SteelCtx::new_init(
            host,
            bundle(
                registries,
                plugin_stack,
                pending_messages,
                effects,
                dirs,
                interrupt_flag,
            ),
            Default::default(),
        )
    }

    /// Build an init-mode `SteelCtx` over a caller-supplied host instead of the
    /// harness's `NullHost` — for tests that need specific host behaviour
    /// (e.g. [`crate::null_host::FailingRegisterHost`]).
    pub(crate) fn ctx_init_with_host<'a>(
        &'a mut self,
        host: &'a mut dyn crate::host::EditorHost,
    ) -> SteelCtx<'a> {
        let Self {
            plugin_stack,
            registries,
            pending_messages,
            effects,
            dirs,
            interrupt_flag,
            ..
        } = self;
        SteelCtx::new_init(
            host,
            bundle(
                registries,
                plugin_stack,
                pending_messages,
                effects,
                dirs,
                interrupt_flag,
            ),
            Default::default(),
        )
    }

    /// Build a `SteelCtx` in activation mode (`session = EvalSession::Runtime`,
    /// plugin body context).
    pub(crate) fn ctx_activation(&mut self) -> SteelCtx<'_> {
        let Self {
            host,
            plugin_stack,
            registries,
            pending_messages,
            effects,
            dirs,
            interrupt_flag,
        } = self;
        SteelCtx::new_activation(
            host,
            bundle(
                registries,
                plugin_stack,
                pending_messages,
                effects,
                dirs,
                interrupt_flag,
            ),
            Default::default(),
        )
    }

    /// Build a `SteelCtx` in command mode (`session = EvalSession::Runtime`).
    pub(crate) fn ctx(&mut self) -> SteelCtx<'_> {
        let Self {
            host,
            plugin_stack,
            registries,
            pending_messages,
            effects,
            dirs,
            interrupt_flag,
        } = self;
        SteelCtx::new_command(
            host,
            bundle(
                registries,
                plugin_stack,
                pending_messages,
                effects,
                dirs,
                interrupt_flag,
            ),
            None,
        )
    }

    /// Build a command-mode `SteelCtx` over a caller-supplied host instead of
    /// the harness's `NullHost` — for tests that need specific host behaviour
    /// (e.g. [`crate::null_host::RecordingInlineOutputHost`]).
    pub(crate) fn ctx_with_host<'a>(
        &'a mut self,
        host: &'a mut dyn crate::host::EditorHost,
    ) -> SteelCtx<'a> {
        let Self {
            plugin_stack,
            registries,
            pending_messages,
            effects,
            dirs,
            interrupt_flag,
            ..
        } = self;
        SteelCtx::new_command(
            host,
            bundle(
                registries,
                plugin_stack,
                pending_messages,
                effects,
                dirs,
                interrupt_flag,
            ),
            None,
        )
    }
}
