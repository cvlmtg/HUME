//! Hook registry for the Steel scripting layer.
//!
//! Plugins register handlers via `(register-hook! 'hook-name proc)`. When the
//! editor fires a lifecycle event, all registered handlers for that event are
//! called in registration order inside a single `with_mut_reference` session.

use std::collections::HashMap;

use steel::rvals::SteelVal;

use crate::attribution::PluginId;

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
    OnModeChange,
    /// Fires on every language transition (including round-trips and clears).
    ///
    /// **For lazy-loading:** use `#:languages` in `declare-plugin` instead.
    /// `#:languages` *activates* the plugin on the *first* matching transition;
    /// the body then registers an `on-language-set` *hook* to react on every
    /// subsequent transition.  Using `on-language-set` as a `#:events` activation
    /// entry would activate the plugin on *any* language transition, not just the
    /// ones it cares about.
    OnLanguageSet,
    /// Fires when an LSP client reaches `Running` for a buffer attached to
    /// it — once per already-attached buffer at that moment, and again for
    /// any buffer that attaches later while the server stays Running.
    /// Args: `(bid server-name)`.
    OnLspAttach,
    /// Fires once per buffer detached by `:lsp-stop`/`:lsp-restart`, right
    /// after `buf.lsp_server` is cleared — the counterpart to `OnLspAttach`,
    /// so a plugin holding buffer-scoped state derived from that server
    /// (e.g. inlay hints) can clear it instead of leaving it to drift with
    /// no server left to keep it in sync. Args: `(bid server-name)`.
    OnLspDetach,
    /// Fires once per drain batch that ingested at least one
    /// `publishDiagnostics` for `bid` — payload-free signal by design; pull
    /// via `(diagnostics-for-buffer bid …)`. Args: `(bid)`.
    OnDiagnosticsChanged,
    /// Fires after scroll/resize resolves a pane's viewport, debounced
    /// (`lsp.viewport-debounce-ms`) so a scroll burst fires once. Args:
    /// `(bid first-line last-line)`.
    OnViewportChange,
    /// Fires in Insert mode after a registered trigger char (see
    /// `register-trigger-chars!`) has been inserted into the buffer — once
    /// per source registered for that char under the buffer's language, so
    /// two sources sharing a char each get their own fire. Args: `(bid
    /// char-string source)`.
    OnTriggerChar,
    /// Fires after `completion-accept!` applies the item's main `textEdit`
    /// (or `insertText` fallback) — Steel handles `additionalTextEdits` and
    /// `completionItem/resolve` from here, since Rust only ever applies
    /// the primary edit. Args: `(bid item)`, `item` the accepted
    /// `CompletionItem`'s raw JSON decoded via `json_to_steel`.
    OnCompletionAccept,
    /// Fires from the Insert-mode per-keystroke refilter path, but only when
    /// the open session's `isIncomplete` flag is set — a bounded,
    /// user-intent-adjacent window, not an unconditional
    /// per-keystroke hook. Args: `(bid filter-text)`.
    OnCompletionRefilter,
}

/// Single source of truth: `(HookId variant, Steel symbol name)` pairs.
const HOOKS: &[(HookId, &str)] = &[
    (HookId::OnBufferOpen, "on-buffer-open"),
    (HookId::OnBufferClose, "on-buffer-close"),
    (HookId::OnBufferSave, "on-buffer-save"),
    (HookId::OnModeChange, "on-mode-change"),
    (HookId::OnLanguageSet, "on-language-set"),
    (HookId::OnLspAttach, "on-lsp-attach"),
    (HookId::OnLspDetach, "on-lsp-detach"),
    (HookId::OnDiagnosticsChanged, "on-diagnostics-changed"),
    (HookId::OnViewportChange, "on-viewport-change"),
    (HookId::OnTriggerChar, "on-trigger-char"),
    (HookId::OnCompletionAccept, "on-completion-accept"),
    (HookId::OnCompletionRefilter, "on-completion-refilter"),
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
    handlers: HashMap<HookId, Vec<HookEntry>>,
}

impl HookRegistry {
    /// Append `proc` (attributed to `owner`) to the handler list for `hook_id`.
    pub(crate) fn register(&mut self, hook_id: HookId, owner: Option<PluginId>, proc: SteelVal) {
        self.handlers
            .entry(hook_id)
            .or_default()
            .push(HookEntry { owner, proc });
    }

    /// Return the handler entries for `hook_id` in registration order.
    pub(crate) fn handlers_for(&self, hook_id: HookId) -> &[HookEntry] {
        self.handlers
            .get(&hook_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// `true` if no handlers are registered for `hook_id` (fast early-exit path).
    pub(crate) fn is_empty_for(&self, hook_id: HookId) -> bool {
        self.handlers.get(&hook_id).is_none_or(Vec::is_empty)
    }

    /// Remove every handler owned by `owner`, across all hook ids — called by
    /// `finish_lazy_activation` on activation failure so a `Failed` plugin's
    /// hooks stop firing. Entries with `owner: None` (top-level) are never
    /// matched.
    pub(crate) fn remove_owned_by(&mut self, owner: &PluginId) {
        for entries in self.handlers.values_mut() {
            entries.retain(|e| e.owner.as_ref() != Some(owner));
        }
    }
}

#[cfg(test)]
mod tests;
