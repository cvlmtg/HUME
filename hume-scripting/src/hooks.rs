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

use std::rc::Rc;

use rustc_hash::FxHashMap;

use crate::Callback;
use crate::attribution::EntryId;

// ── HookRegistry ──────────────────────────────────────────────────────────────

/// A single hook handler, run as the code that registered it.
///
/// `activation` is the plugin body that was running when it was registered
/// (`None`: none was), which removes it if that body fails; see
/// `remove_registered_in`.
///
/// `keys` narrows the entry to events carrying one of those keys (the
/// editor decides what an event's key is: an LSP notification's method).
/// `None` fires for every event of the name. The filter lives on the entry
/// so rollback removes it together with the handler.
///
/// `takes_done` is set for a handler of a held event
/// (`EventHost::held_event_arg_count`) that has one parameter more than the
/// event has args: it takes `done` last.
#[derive(Debug)]
pub(crate) struct HookEntry {
    pub(crate) handler: Callback,
    pub(crate) activation: Option<Rc<EntryId>>,
    pub(crate) keys: Option<Box<[String]>>,
    pub(crate) takes_done: bool,
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
    /// Append `handler` (narrowed to `keys`, registered while
    /// `activation`'s body ran) to the handler list for `name`.
    pub(crate) fn register(
        &mut self,
        name: &str,
        handler: Callback,
        activation: Option<Rc<EntryId>>,
        keys: Option<Box<[String]>>,
        takes_done: bool,
    ) {
        self.handlers
            .entry(name.to_string())
            .or_default()
            .push(HookEntry {
                handler,
                activation,
                keys,
                takes_done,
            });
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

    /// Remove every handler registered while `activation`'s body ran,
    /// across all hook names. Called when that body fails, so a `Failed`
    /// plugin's hooks stop firing.
    pub(crate) fn remove_registered_in(&mut self, activation: &EntryId) {
        for entries in self.handlers.values_mut() {
            entries.retain(|e| e.activation.as_deref() != Some(activation));
        }
    }
}

#[cfg(test)]
mod tests;
