//! Hook registry for the Steel scripting layer.
//!
//! Plugins register handlers via `(register-hook! 'hook-name proc)`. When the
//! editor fires an event, all registered handlers for that name are called
//! in registration order inside a single `with_mut_reference` session.
//!
//! Name-keyed, not enum-keyed: this crate has no compiled-in knowledge of
//! which event names exist — that's `hume-editor`'s `EditorEvent`, reached
//! only through `EditorHost::events().known_event_names()` for validation.
//! See `builtins::hooks::register_hook` and `builtins::plugins::declare_plugin`.

use rustc_hash::FxHashMap;

use steel::rvals::SteelVal;

use crate::attribution::PluginId;

// ── HookRegistry ──────────────────────────────────────────────────────────────

/// A single hook handler plus the plugin whose body registered it (`None` —
/// top-level `init.scm`/user config, never rolled back). The owner drives
/// per-plugin rollback when a plugin activation fails — see `remove_owned_by`.
#[derive(Debug)]
pub(crate) struct HookEntry {
    pub(crate) owner: Option<PluginId>,
    pub(crate) proc: SteelVal,
}

/// Persistent per-hook handler lists, held on [`super::ScriptingHost`].
#[derive(Debug, Default)]
pub(crate) struct HookRegistry {
    handlers: FxHashMap<String, Vec<HookEntry>>,
}

impl HookRegistry {
    /// Append `proc` (attributed to `owner`) to the handler list for `name`.
    pub(crate) fn register(&mut self, name: &str, owner: Option<PluginId>, proc: SteelVal) {
        self.handlers
            .entry(name.to_string())
            .or_default()
            .push(HookEntry { owner, proc });
    }

    /// Return the handler entries for `name` in registration order.
    pub(crate) fn handlers_for(&self, name: &str) -> &[HookEntry] {
        self.handlers.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    /// `true` if no handlers are registered for `name` (fast early-exit path).
    pub(crate) fn is_empty_for(&self, name: &str) -> bool {
        self.handlers.get(name).is_none_or(Vec::is_empty)
    }

    /// Remove every handler owned by `owner`, across all hook names — called
    /// by `finish_lazy_activation` on activation failure so a `Failed`
    /// plugin's hooks stop firing. Entries with `owner: None` (top-level) are
    /// never matched.
    pub(crate) fn remove_owned_by(&mut self, owner: &PluginId) {
        for entries in self.handlers.values_mut() {
            entries.retain(|e| e.owner.as_ref() != Some(owner));
        }
    }
}

#[cfg(test)]
mod tests;
