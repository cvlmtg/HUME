//! Test support types for scripting crate unit tests.
//!
//! Lives in the scripting crate because it needs access to private types
//! (`SteelCtx`, `HostBundle`).

use crate::attribution::AttributionStack;
use crate::builtins::dirs::ScriptDirs;
use crate::context::SteelCtx;
use crate::null_host::NullHost;
use crate::types::QueuedEffect;
use crate::{HostBundle, ScriptingRegistries};
use hume_core::Severity;

/// A stand-in bid for a builtin test that doesn't care which buffer it
/// names: [`NullHost`]/[`crate::testing`] mocks don't track real buffers,
/// so any value type-checks the same. A direct Rust call to a builtin
/// function (every caller of this) never runs `LivePane`'s liveness check at
/// all (that only happens inside the `builtins!`-registered Steel
/// closure), so this needs no "live" counterpart.
pub(crate) fn default_bid() -> hume_core::BufferId {
    hume_core::BufferId::default()
}

/// [`default_bid`]'s `PaneHandle` counterpart: a pane-less handle, for a
/// direct Rust call into a builtin now typed for `PaneHandle`/`ArgPane`/
/// `LivePane` rather than a bare `BufferId`.
pub(crate) fn default_pane() -> hume_core::PaneHandle {
    hume_core::PaneHandle::buffer_only(default_bid())
}

/// [`default_pane`], but with a pane half too, for a builtin whose own
/// contract needs one present (`#:require-focus`'s "needs a pane" check,
/// a kind-A/B builtin exercised only for its decode, never resolved against
/// real editor state through `NullHost`).
pub(crate) fn default_pane_with_pane() -> hume_core::PaneHandle {
    hume_core::PaneHandle::with_pane(default_bid(), hume_core::PaneId::default())
}

/// The Steel symbol `name`.
pub(crate) fn sym(name: &str) -> steel::rvals::SteelVal {
    steel::rvals::SteelVal::SymbolV(name.into())
}

/// The Steel string `s`.
pub(crate) fn str_val(s: &str) -> steel::rvals::SteelVal {
    steel::rvals::SteelVal::StringV(s.into())
}

/// Backing storage for [`SteelCtx`] in scripting-crate unit tests.
///
/// Uses [`NullHost`], sufficient for tests that only need to check that
/// scripting guards (`EvalMode`, `errors::require_cmd`/`require_config`,
/// etc.) fire correctly, without real editor state.
pub(crate) struct SteelCtxTestHarness {
    pub(crate) host: NullHost,
    pub(crate) attribution: AttributionStack,
    pub(crate) registries: ScriptingRegistries,
    pub(crate) pending_messages: Vec<(Severity, String)>,
    pub(crate) effects: Vec<QueuedEffect>,
    pub(crate) dirs: ScriptDirs,
    watchdog: crate::watchdog::EvalWatchdog,
}

/// Builds a [`HostBundle`] from the harness's persistent fields: the shared
/// tail every `ctx_*` constructor below ends with. A free function (not a
/// `&mut self` method) because the constructors also need `&mut self.host`
/// borrowed independently in the same call, the same NLL field-split pattern as
/// `ScriptingHost::steel_and_bundle`.
fn bundle<'a>(
    registries: &'a mut ScriptingRegistries,
    attribution: &'a mut AttributionStack,
    pending_messages: &'a mut Vec<(Severity, String)>,
    effects: &'a mut Vec<QueuedEffect>,
    dirs: &'a ScriptDirs,
    watchdog: &'a crate::watchdog::EvalWatchdog,
) -> HostBundle<'a> {
    HostBundle {
        registries,
        attribution,
        pending_messages,
        effects,
        dirs,
        budget: watchdog.clock(),
    }
}

impl SteelCtxTestHarness {
    pub(crate) fn new() -> Self {
        Self {
            host: NullHost,
            attribution: AttributionStack::default(),
            registries: ScriptingRegistries::default(),
            pending_messages: Vec::new(),
            effects: Vec::new(),
            dirs: ScriptDirs::new(&hume_platform::dirs::Dirs::none()),
            watchdog: crate::watchdog::EvalWatchdog::new(),
        }
    }

    /// Build a `SteelCtx` in init mode (`session = EvalSession::Init`), as
    /// `init.scm`.
    pub(crate) fn ctx_init(&mut self) -> SteelCtx<'_> {
        let Self {
            host,
            attribution,
            registries,
            pending_messages,
            effects,
            dirs,
            watchdog,
        } = self;
        SteelCtx::new_init(
            host,
            bundle(
                registries,
                attribution,
                pending_messages,
                effects,
                dirs,
                watchdog,
            ),
            Default::default(),
            crate::attribution::Owner::User,
        )
    }

    /// Build an init-mode `SteelCtx` over a caller-supplied host instead of the
    /// harness's `NullHost`, for tests that need specific host behaviour
    /// (e.g. [`crate::null_host::CommandStoreHost`]).
    pub(crate) fn ctx_init_with_host<'a>(
        &'a mut self,
        host: &'a mut dyn crate::host::EditorHost,
    ) -> SteelCtx<'a> {
        let Self {
            attribution,
            registries,
            pending_messages,
            effects,
            dirs,
            watchdog,
            ..
        } = self;
        SteelCtx::new_init(
            host,
            bundle(
                registries,
                attribution,
                pending_messages,
                effects,
                dirs,
                watchdog,
            ),
            Default::default(),
            crate::attribution::Owner::User,
        )
    }

    /// Build a `SteelCtx` in activation mode (`session = EvalSession::Runtime`,
    /// plugin body context).
    pub(crate) fn ctx_activation(&mut self) -> SteelCtx<'_> {
        let Self {
            host,
            attribution,
            registries,
            pending_messages,
            effects,
            dirs,
            watchdog,
        } = self;
        SteelCtx::new_activation(
            host,
            bundle(
                registries,
                attribution,
                pending_messages,
                effects,
                dirs,
                watchdog,
            ),
            Default::default(),
        )
    }

    /// Build a `SteelCtx` in command mode (`session = EvalSession::Runtime`),
    /// running a proc `init.scm` defined.
    pub(crate) fn ctx(&mut self) -> SteelCtx<'_> {
        self.ctx_as(user())
    }

    /// Build a command-mode `SteelCtx` running a proc credited as `base`.
    pub(crate) fn ctx_as(&mut self, base: crate::attribution::Provenance) -> SteelCtx<'_> {
        let Self {
            host,
            attribution,
            registries,
            pending_messages,
            effects,
            dirs,
            watchdog,
        } = self;
        SteelCtx::new_command(
            host,
            bundle(
                registries,
                attribution,
                pending_messages,
                effects,
                dirs,
                watchdog,
            ),
            base,
        )
    }

    /// Build a command-mode `SteelCtx` over a caller-supplied host instead of
    /// the harness's `NullHost`, for tests that need specific host behaviour
    /// (e.g. [`crate::null_host::RecordingInlineOutputHost`]).
    pub(crate) fn ctx_with_host<'a>(
        &'a mut self,
        host: &'a mut dyn crate::host::EditorHost,
    ) -> SteelCtx<'a> {
        let Self {
            attribution,
            registries,
            pending_messages,
            effects,
            dirs,
            watchdog,
            ..
        } = self;
        SteelCtx::new_command(
            host,
            bundle(
                registries,
                attribution,
                pending_messages,
                effects,
                dirs,
                watchdog,
            ),
            user(),
        )
    }
}

/// Who a harness session is credited to: the user's own code.
fn user() -> crate::attribution::Provenance {
    crate::attribution::Provenance::own(crate::attribution::Owner::User)
}
