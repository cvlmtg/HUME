//! Hook registry for the Steel scripting layer.
//!
//! Plugins register handlers via `(register-hook! 'hook-name proc)`, or a
//! keyed one via `register-lsp-notification-hook!`. When the editor fires an
//! event, every handler for that name matching the event's key is called in
//! registration order, each in its own `with_mut_reference` session.
//!
//! Name-keyed, not enum-keyed: this crate has no compiled-in knowledge of
//! which event names exist. That's `hume-editor`'s `EditorEvent`, reached
//! only through `EditorHost::events().known_event_names()` for validation.
//! See `builtins::hooks::register_hook` and `builtins::plugins::declare_plugin`.

use rustc_hash::FxHashMap;

use steel::rvals::SteelVal;

use crate::attribution::PluginId;

// ── HookRegistry ──────────────────────────────────────────────────────────────

/// A single hook handler plus the plugin whose body registered it (`None`:
/// top-level `init.scm`/user config, never rolled back). The owner drives
/// per-plugin rollback when a plugin activation fails; see `remove_owned_by`.
///
/// `keys` narrows the entry to events carrying one of those keys (the
/// editor decides what an event's key is: an LSP notification's method).
/// `None` fires for every event of the name. The filter lives on the entry
/// so rollback removes it together with the handler.
#[derive(Debug)]
pub(crate) struct HookEntry {
    pub(crate) owner: Option<PluginId>,
    pub(crate) proc: SteelVal,
    pub(crate) keys: Option<Box<[String]>>,
}

impl HookEntry {
    fn matches(&self, key: Option<&str>) -> bool {
        match &self.keys {
            None => true,
            Some(keys) => key.is_some_and(|k| keys.iter().any(|x| x == k)),
        }
    }
}

/// Persistent per-hook handler lists, held on [`super::ScriptingHost`].
#[derive(Debug, Default)]
pub(crate) struct HookRegistry {
    handlers: FxHashMap<String, Vec<HookEntry>>,
}

impl HookRegistry {
    /// Append `proc` (attributed to `owner`, narrowed to `keys`) to the
    /// handler list for `name`.
    pub(crate) fn register(
        &mut self,
        name: &str,
        owner: Option<PluginId>,
        proc: SteelVal,
        keys: Option<Box<[String]>>,
    ) {
        self.handlers
            .entry(name.to_string())
            .or_default()
            .push(HookEntry { owner, proc, keys });
    }

    /// Every entry for `name`, keyed or not, in registration order.
    #[cfg(test)]
    pub(crate) fn handlers_for(&self, name: &str) -> &[HookEntry] {
        self.handlers.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The entries for `name` that fire for an event carrying `key`, in
    /// registration order.
    pub(crate) fn matching<'a>(
        &'a self,
        name: &str,
        key: Option<&'a str>,
    ) -> impl Iterator<Item = &'a HookEntry> {
        self.handlers
            .get(name)
            .into_iter()
            .flatten()
            .filter(move |e| e.matches(key))
    }

    /// `true` if at least one entry for `name` fires for `key`.
    pub(crate) fn has_match(&self, name: &str, key: Option<&str>) -> bool {
        self.matching(name, key).next().is_some()
    }

    /// Remove every handler owned by `owner`, across all hook names. Called
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
