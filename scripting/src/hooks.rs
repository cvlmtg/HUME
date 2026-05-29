//! Hook registry for the Steel scripting layer.
//!
//! Plugins register handlers via `(register-hook! 'hook-name proc)`. When the
//! editor fires a lifecycle event, all registered handlers for that event are
//! called in registration order inside a single `with_mut_reference` session.

use std::collections::HashMap;

use steel::rvals::SteelVal;

// ── HookId ────────────────────────────────────────────────────────────────────

/// Identifier for each editor lifecycle event plugins can observe.
// All variants share the `On` prefix, matching the `on-buffer-open` Steel naming
// convention.  The lint wants dissimilar prefixes; we intentionally override it.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookId {
    OnBufferOpen,
    OnBufferClose,
    OnBufferSave,
    OnEdit,
    OnModeChange,
    OnLanguageSet,
}

/// Single source of truth: `(HookId variant, Steel symbol name)` pairs.
const HOOKS: &[(HookId, &str)] = &[
    (HookId::OnBufferOpen, "on-buffer-open"),
    (HookId::OnBufferClose, "on-buffer-close"),
    (HookId::OnBufferSave, "on-buffer-save"),
    (HookId::OnEdit, "on-edit"),
    (HookId::OnModeChange, "on-mode-change"),
    (HookId::OnLanguageSet, "on-language-set"),
];

impl HookId {
    /// Map a Steel symbol name to a `HookId`.
    pub fn from_symbol(s: &str) -> Option<Self> {
        HOOKS.iter().find(|(_, name)| *name == s).map(|(id, _)| *id)
    }

    /// All valid hook names as an iterator, for error messages.
    pub fn all_names() -> impl Iterator<Item = &'static str> {
        HOOKS.iter().map(|(_, name)| *name)
    }

    /// The Steel symbol name for this hook, e.g. `"on-buffer-save"`.
    pub fn symbol(self) -> &'static str {
        HOOKS
            .iter()
            .find(|(id, _)| *id == self)
            .map(|(_, name)| *name)
            .expect("all HookId variants covered in HOOKS")
    }
}

// ── HookRegistry ──────────────────────────────────────────────────────────────

/// Persistent per-hook handler lists, held on [`super::ScriptingHost`].
#[derive(Debug, Default)]
pub struct HookRegistry {
    handlers: HashMap<HookId, Vec<SteelVal>>,
}

impl HookRegistry {
    /// Append `proc` to the handler list for `hook_id`.
    pub fn register(&mut self, hook_id: HookId, proc: SteelVal) {
        self.handlers.entry(hook_id).or_default().push(proc);
    }

    /// Return the handlers for `hook_id` in registration order.
    pub fn handlers_for(&self, hook_id: HookId) -> &[SteelVal] {
        self.handlers
            .get(&hook_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// `true` if no handlers are registered for `hook_id` (fast early-exit path).
    pub fn is_empty_for(&self, hook_id: HookId) -> bool {
        self.handlers.get(&hook_id).is_none_or(Vec::is_empty)
    }
}
