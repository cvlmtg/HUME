//! `EditorEvent` — the editor's own vocabulary of "something happened worth
//! telling subscribers about". SSOT for which events exist and their
//! Steel-facing names; `hume-scripting` never compiles in this type, only
//! the `&str` names produced here (see `hume_scripting::host::EventHost`).

/// Identifier for each editor event plugins can observe.
// All variants share the `On` prefix, matching the `on-buffer-open` Steel naming
// convention. The lint wants dissimilar prefixes; we intentionally override it.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorEvent {
    OnBufferOpen,
    OnBufferClose,
    OnBufferSave,
    OnModeChange,
    /// Fires on every language transition (including round-trips and clears).
    ///
    /// **For lazy-loading:** use `#:languages` in `declare-plugin` instead.
    /// `#:languages` *activates* the plugin on the *first* matching transition;
    /// the body then registers an `on-language-set` *hook* to react on every
    /// subsequent transition. Using `on-language-set` as a `#:events` activation
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
    /// (or `insertText` fallback), `additionalTextEdits`, and (if needed)
    /// `completionItem/resolve` — Rust owns all three atomically, so this is
    /// a plain extension point for anything the completion store doesn't
    /// itself parse (e.g. `command`), not a place that needs to apply edits.
    /// Args: `(bid item)`, `item` the accepted `CompletionItem`'s raw JSON
    /// decoded via `json_to_steel`.
    OnCompletionAccept,
    /// Fires from the Insert-mode per-keystroke refilter path, but only when
    /// the open session's `isIncomplete` flag is set — a bounded,
    /// user-intent-adjacent window, not an unconditional
    /// per-keystroke hook. Args: `(bid filter-text)`.
    OnCompletionRefilter,
}

/// Single source of truth: `(EditorEvent variant, Steel symbol name)` pairs.
/// Non-exhaustive over variants by construction — a variant with no entry
/// here is internal-only, never reaching Steel (see `EditorEvent::name`).
const EDITOR_EVENT_NAMES: &[(EditorEvent, &str)] = &[
    (EditorEvent::OnBufferOpen, "on-buffer-open"),
    (EditorEvent::OnBufferClose, "on-buffer-close"),
    (EditorEvent::OnBufferSave, "on-buffer-save"),
    (EditorEvent::OnModeChange, "on-mode-change"),
    (EditorEvent::OnLanguageSet, "on-language-set"),
    (EditorEvent::OnLspAttach, "on-lsp-attach"),
    (EditorEvent::OnLspDetach, "on-lsp-detach"),
    (EditorEvent::OnDiagnosticsChanged, "on-diagnostics-changed"),
    (EditorEvent::OnViewportChange, "on-viewport-change"),
    (EditorEvent::OnTriggerChar, "on-trigger-char"),
    (EditorEvent::OnCompletionAccept, "on-completion-accept"),
    (EditorEvent::OnCompletionRefilter, "on-completion-refilter"),
];

impl EditorEvent {
    /// The Steel symbol name for this event, or `None` if it's internal-only
    /// (raised and reacted to entirely on the Rust side — no `#[allow]`
    /// variant exists yet, but the drain loop already handles the case).
    pub(crate) fn name(self) -> Option<&'static str> {
        EDITOR_EVENT_NAMES
            .iter()
            .find(|(e, _)| *e == self)
            .map(|(_, name)| *name)
    }
}

/// Every Steel-visible event name — backs `EventHost::known_event_names`,
/// consulted by `register-hook!` and `declare-plugin`'s `#:events` to
/// validate names without `hume-scripting` compiling in `EditorEvent`.
///
/// Returns an owned `Vec` rather than a `&'static` slice: deriving a
/// name-only static slice from `EDITOR_EVENT_NAMES` needs const-eval
/// gymnastics, and a second parallel const would itself be a SSOT
/// violation. Both callers are config-time only, so one small alloc per
/// validation is free.
pub(crate) fn known_event_names() -> Vec<&'static str> {
    EDITOR_EVENT_NAMES.iter().map(|(_, name)| *name).collect()
}

#[cfg(test)]
mod tests;
