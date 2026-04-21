//! Hook registry for the Steel scripting layer.
//!
//! Plugins register handlers via `(register-hook! 'hook-name proc)`. When the
//! editor fires a lifecycle event, all registered handlers for that event are
//! called in registration order inside a single `with_mut_reference` session.

use std::collections::HashMap;

use steel::rvals::SteelVal;

use crate::scripting::ledger::{Owner, PluginId};

// ── HookId ────────────────────────────────────────────────────────────────────

/// Identifier for each editor lifecycle event plugins can observe.
// The `On` prefix is intentional — matches the `on-buffer-open` Steel naming convention.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum HookId {
    OnBufferOpen,
    OnBufferClose,
    OnBufferSave,
    OnEdit,
    OnModeChange,
}

impl HookId {
    /// Map a Steel symbol name to a `HookId`.
    pub(crate) fn from_symbol(s: &str) -> Option<Self> {
        match s {
            "on-buffer-open"  => Some(Self::OnBufferOpen),
            "on-buffer-close" => Some(Self::OnBufferClose),
            "on-buffer-save"  => Some(Self::OnBufferSave),
            "on-edit"         => Some(Self::OnEdit),
            "on-mode-change"  => Some(Self::OnModeChange),
            _ => None,
        }
    }

    /// All valid hook names, for error messages.
    pub(crate) fn all_names() -> &'static [&'static str] {
        &[
            "on-buffer-open", "on-buffer-close", "on-buffer-save",
            "on-edit", "on-mode-change",
        ]
    }
}

// ── HookRegistry ──────────────────────────────────────────────────────────────

/// Persistent per-hook handler lists, held on [`super::ScriptFacingCtx`].
///
/// Each entry pairs an owner (for ledger-attribution teardown) with the Steel
/// proc value.  `Clone` is required by [`super::EvalSnapshot`].
#[derive(Debug, Clone, Default)]
pub(crate) struct HookRegistry {
    handlers: HashMap<HookId, Vec<(Owner, SteelVal)>>,
}

impl HookRegistry {
    /// Append `proc` to the handler list for `hook_id`, attributed to `owner`.
    pub(crate) fn register(&mut self, hook_id: HookId, owner: Owner, proc: SteelVal) {
        self.handlers.entry(hook_id).or_default().push((owner, proc));
    }

    /// Return the handlers for `hook_id` in registration order.
    pub(crate) fn handlers_for(&self, hook_id: &HookId) -> &[(Owner, SteelVal)] {
        self.handlers.get(hook_id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// `true` if no handlers are registered for `hook_id` (fast early-exit path).
    pub(crate) fn is_empty_for(&self, hook_id: &HookId) -> bool {
        self.handlers.get(hook_id).is_none_or(Vec::is_empty)
    }

    /// Remove all handlers attributed to `plugin_id` (called from teardown).
    pub(crate) fn purge_plugin(&mut self, plugin_id: &PluginId) {
        for handlers in self.handlers.values_mut() {
            handlers.retain(|(owner, _)| {
                !matches!(owner, Owner::Plugin(pid) if pid == plugin_id)
            });
        }
    }
}
