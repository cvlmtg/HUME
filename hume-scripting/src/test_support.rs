//! Test support types for scripting crate unit tests.
//!
//! Lives in the scripting crate because it needs access to private types
//! (`SteelCtx`, `HostState`).

use crate::HostState;
use crate::builtins::dirs::ScriptDirs;
use crate::context::SteelCtx;
use crate::null_host::NullHost;

/// A stand-in bid for a builtin test that doesn't care which buffer it
/// names: [`NullHost`]/[`crate::testing`] mocks don't track real buffers,
/// so any value type-checks the same. A direct Rust call to a builtin
/// function (every caller of this) never runs its pane arguments' checks
/// (those only run inside the `builtins!`-registered Steel closure), so this
/// needs no "live" counterpart.
pub(crate) fn default_bid() -> hume_core::BufferId {
    hume_core::BufferId::default()
}

/// [`default_bid`]'s `PaneHandle` counterpart: a pane-less handle, for a
/// direct Rust call into a builtin typed for `PaneHandle`/`ArgPane`.
pub(crate) fn default_pane() -> hume_core::PaneHandle {
    hume_core::PaneHandle::buffer_only(default_bid())
}

/// [`default_pane`], but with a pane half too, for a builtin whose own
/// contract needs one present (`#:require-focus`'s "needs a pane" check).
pub(crate) fn default_pane_with_pane() -> hume_core::PaneHandle {
    hume_core::PaneHandle::with_pane(default_bid(), hume_core::PaneId::default())
}

/// [`default_pane_with_pane`] as a shown pane, for a direct Rust call into
/// a builtin that acts through one.
pub(crate) fn default_shown() -> crate::host::ShownHandle {
    crate::host::ShownHandle::for_test(default_pane_with_pane())
}

/// [`default_pane_with_pane`] as a focused pane, for a direct Rust call
/// into a builtin that acts through one.
pub(crate) fn default_focused() -> crate::host::FocusedHandle {
    crate::host::FocusedHandle::for_test(default_pane_with_pane())
}

/// The Steel symbol `name`.
pub(crate) fn sym(name: &str) -> steel::rvals::SteelVal {
    steel::rvals::SteelVal::SymbolV(name.into())
}

/// The value under `'key` in the record hash `record`; panics when
/// `record` is not a hash or has no such key.
pub(crate) fn hash_field(record: &steel::rvals::SteelVal, key: &str) -> steel::rvals::SteelVal {
    match record {
        steel::rvals::SteelVal::HashMapV(map) => map
            .get(&sym(key))
            .cloned()
            .unwrap_or_else(|| panic!("no '{key} in {record:?}")),
        other => panic!("expected a hash, got {other:?}"),
    }
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
    pub(crate) state: HostState,
}

impl SteelCtxTestHarness {
    pub(crate) fn new() -> Self {
        Self {
            host: NullHost,
            state: HostState::new(
                ScriptDirs::new(&hume_platform::dirs::Dirs::none()),
                crate::watchdog::EvalWatchdog::new(),
            ),
        }
    }

    /// Build a `SteelCtx` in init mode (`session = EvalSession::Init`), as
    /// `init.scm`.
    pub(crate) fn ctx_init(&mut self) -> SteelCtx<'_> {
        let (_, bundle) = self.state.split();
        SteelCtx::new_init(&mut self.host, bundle, crate::attribution::Owner::User)
    }

    /// Build an init-mode `SteelCtx` over a caller-supplied host instead of the
    /// harness's `NullHost`, for tests that need specific host behaviour
    /// (e.g. [`crate::null_host::CommandStoreHost`]).
    pub(crate) fn ctx_init_with_host<'a>(
        &'a mut self,
        host: &'a mut dyn crate::host::EditorHost,
    ) -> SteelCtx<'a> {
        let (_, bundle) = self.state.split();
        SteelCtx::new_init(host, bundle, crate::attribution::Owner::User)
    }

    /// Build a `SteelCtx` in activation mode (`session = EvalSession::Runtime`,
    /// plugin body context).
    pub(crate) fn ctx_activation(&mut self) -> SteelCtx<'_> {
        let (_, bundle) = self.state.split();
        SteelCtx::new_activation(&mut self.host, bundle)
    }

    /// Build a `SteelCtx` in command mode (`session = EvalSession::Runtime`),
    /// running a proc `init.scm` defined.
    pub(crate) fn ctx(&mut self) -> SteelCtx<'_> {
        self.ctx_as(user())
    }

    /// Build a command-mode `SteelCtx` running a proc credited as `base`.
    pub(crate) fn ctx_as(&mut self, base: crate::attribution::Provenance) -> SteelCtx<'_> {
        let (_, bundle) = self.state.split();
        SteelCtx::new_command(&mut self.host, bundle, base)
    }

    /// Build a command-mode `SteelCtx` over a caller-supplied host instead of
    /// the harness's `NullHost`, for tests that need specific host behaviour
    /// (e.g. [`crate::null_host::RecordingInlineOutputHost`]).
    pub(crate) fn ctx_with_host<'a>(
        &'a mut self,
        host: &'a mut dyn crate::host::EditorHost,
    ) -> SteelCtx<'a> {
        let (_, bundle) = self.state.split();
        SteelCtx::new_command(host, bundle, user())
    }
}

/// Who a harness session is credited to: the user's own code.
fn user() -> crate::attribution::Provenance {
    crate::attribution::Provenance::own(crate::attribution::Owner::User)
}
