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
    /// via `(diagnostics-for-buffer bid …)` (B5). Args: `(bid)`.
    OnDiagnosticsChanged,
    /// Fires after scroll/resize resolves a pane's viewport, debounced
    /// (`lsp.viewport-debounce-ms`) so a scroll burst fires once. Args:
    /// `(bid first-line last-line)`.
    OnViewportChange,
    /// Fires in Insert mode after a registered trigger char (see
    /// `register-trigger-chars!`) has been inserted into the buffer. Args:
    /// `(bid char-string)`.
    OnTriggerChar,
    /// Fires after `completion-accept!` applies the item's main `textEdit`
    /// (or `insertText` fallback) — Steel handles `additionalTextEdits` and
    /// `completionItem/resolve` from here (F3), since Rust only ever applies
    /// the primary edit. Args: `(bid item)`, `item` the accepted
    /// `CompletionItem`'s raw JSON decoded via `json_to_steel`.
    OnCompletionAccept,
    /// Fires from the Insert-mode per-keystroke refilter path, but only when
    /// the open session's `isIncomplete` flag is set — a bounded,
    /// user-intent-adjacent window (see B10c), not an unconditional
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

/// Persistent per-hook handler lists, held on [`super::ScriptingHost`].
#[derive(Debug, Default)]
pub(crate) struct HookRegistry {
    handlers: HashMap<HookId, Vec<SteelVal>>,
}

impl HookRegistry {
    /// Append `proc` to the handler list for `hook_id`.
    pub(crate) fn register(&mut self, hook_id: HookId, proc: SteelVal) {
        self.handlers.entry(hook_id).or_default().push(proc);
    }

    /// Return the handlers for `hook_id` in registration order.
    pub(crate) fn handlers_for(&self, hook_id: HookId) -> &[SteelVal] {
        self.handlers
            .get(&hook_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// `true` if no handlers are registered for `hook_id` (fast early-exit path).
    pub(crate) fn is_empty_for(&self, hook_id: HookId) -> bool {
        self.handlers.get(&hook_id).is_none_or(Vec::is_empty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Never called — its only purpose is the exhaustive `match`: adding a
    /// `HookId` variant without extending this list is a compile error, not
    /// a runtime `.expect()` panic the first time something fires the new
    /// hook. Keep in lockstep with `ALL_VARIANTS` below.
    #[allow(dead_code)]
    fn _exhaustiveness_check(id: HookId) {
        match id {
            HookId::OnBufferOpen
            | HookId::OnBufferClose
            | HookId::OnBufferSave
            | HookId::OnModeChange
            | HookId::OnLanguageSet
            | HookId::OnLspAttach
            | HookId::OnLspDetach
            | HookId::OnDiagnosticsChanged
            | HookId::OnViewportChange
            | HookId::OnTriggerChar
            | HookId::OnCompletionAccept
            | HookId::OnCompletionRefilter => {}
        }
    }

    const ALL_VARIANTS: &[HookId] = &[
        HookId::OnBufferOpen,
        HookId::OnBufferClose,
        HookId::OnBufferSave,
        HookId::OnModeChange,
        HookId::OnLanguageSet,
        HookId::OnLspAttach,
        HookId::OnLspDetach,
        HookId::OnDiagnosticsChanged,
        HookId::OnViewportChange,
        HookId::OnTriggerChar,
        HookId::OnCompletionAccept,
        HookId::OnCompletionRefilter,
    ];

    /// Fail oracle: delete a HOOKS row for a variant still in `ALL_VARIANTS`
    /// → `symbol()` panics (caught as a normal test failure here, not a
    /// runtime surprise the first time the hook fires).
    #[test]
    fn every_hook_id_round_trips_through_symbol_and_from_symbol() {
        for &id in ALL_VARIANTS {
            let name = id.symbol();
            assert_eq!(
                HookId::from_symbol(name),
                Some(id),
                "round trip failed for {name}"
            );
        }
    }

    #[test]
    fn all_names_has_no_duplicates_and_matches_variant_count() {
        let names: Vec<&str> = HookId::all_names().collect();
        assert_eq!(names.len(), ALL_VARIANTS.len());
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "HOOKS has a duplicate symbol name");
    }
}
