//! `EditorEvent` — the editor's own vocabulary of "something happened worth
//! telling subscribers about". SSOT for which events exist, their typed Rust
//! payloads, and their Steel-facing names/arg shapes; `hume-scripting` never
//! compiles in this type, only the `&str` names and `SteelVal` args produced
//! here (see `hume_scripting::host::EventHost`).

use hume_engine::pipeline::BufferId;
use hume_scripting::SteelBufferId;
use hume_scripting::json::json_to_steel;
use steel::rvals::SteelVal;

use super::Mode;

/// Something that happened in the editor, carrying whatever payload its
/// Steel handlers (and, for a growing subset, Rust-side reactions) need.
///
/// `steel_args` is the single place a variant's fields become `SteelVal`s —
/// see its doc for why arg construction lives there and not at the raise
/// site.
// All variants share the `On` prefix, matching the `on-buffer-open` Steel naming
// convention. The lint wants dissimilar prefixes; we intentionally override it.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum EditorEvent {
    OnBufferOpen {
        buffer: BufferId,
    },
    OnBufferClose {
        buffer: BufferId,
    },
    OnBufferSave {
        buffer: BufferId,
    },
    OnModeChange {
        from: Mode,
        to: Mode,
    },
    /// Fires on every language transition (including round-trips and clears).
    /// `language` is the resolved name at raise time, not a `LanguageId` —
    /// the id is a registry index and `:reload-config` can rebuild the
    /// registry before this drains, so resolving early is the only
    /// consistent read.
    ///
    /// **For lazy-loading:** use `#:languages` in `declare-plugin` instead.
    /// `#:languages` *activates* the plugin on the *first* matching transition;
    /// the body then registers an `on-language-set` *hook* to react on every
    /// subsequent transition. Using `on-language-set` as a `#:events` activation
    /// entry would activate the plugin on *any* language transition, not just the
    /// ones it cares about.
    OnLanguageSet {
        buffer: BufferId,
        language: Option<String>,
    },
    /// Fires when an LSP client reaches `Running` for a buffer attached to
    /// it — once per already-attached buffer at that moment, and again for
    /// any buffer that attaches later while the server stays Running.
    OnLspAttach {
        buffer: BufferId,
        server: String,
    },
    /// Fires once per buffer detached by `:lsp-stop`/`:lsp-restart`, right
    /// after `buf.lsp_server` is cleared — the counterpart to `OnLspAttach`,
    /// so a plugin holding buffer-scoped state derived from that server
    /// (e.g. inlay hints) can clear it instead of leaving it to drift with
    /// no server left to keep it in sync.
    OnLspDetach {
        buffer: BufferId,
        server: String,
    },
    /// Fires once per drain batch that ingested at least one
    /// `publishDiagnostics` for `buffer` — payload-free signal by design;
    /// pull via `(diagnostics-for-buffer bid …)`.
    OnDiagnosticsChanged {
        buffer: BufferId,
    },
    /// Fires after scroll/resize resolves a pane's viewport, debounced
    /// (`lsp.viewport-debounce-ms`) so a scroll burst fires once.
    OnViewportChange {
        buffer: BufferId,
        first_line: usize,
        last_line: usize,
    },
    /// Fires in Insert mode after a registered trigger char (see
    /// `register-trigger-chars!`) has been inserted into the buffer — once
    /// per source registered for that char under the buffer's language, so
    /// two sources sharing a char each get their own fire.
    OnTriggerChar {
        buffer: BufferId,
        ch: char,
        source: String,
    },
    /// Fires after `completion-accept!` applies the item's main `textEdit`
    /// (or `insertText` fallback), `additionalTextEdits`, and (if needed)
    /// `completionItem/resolve` — Rust owns all three atomically, so this is
    /// a plain extension point for anything the completion store doesn't
    /// itself parse (e.g. `command`), not a place that needs to apply edits.
    /// `item` is the accepted `CompletionItem`'s raw JSON.
    OnCompletionAccept {
        buffer: BufferId,
        item: serde_json::Value,
    },
    /// Fires from the Insert-mode per-keystroke refilter path, but only when
    /// the open session's `isIncomplete` flag is set — a bounded,
    /// user-intent-adjacent window, not an unconditional per-keystroke hook.
    OnCompletionRefilter {
        buffer: BufferId,
        filter_text: String,
    },
}

/// Pairs each `EditorEvent` variant with its Steel-facing name, once, and
/// generates both `EditorEvent::name`'s match and `EVENT_NAMES` from that one
/// list — the alternative (a hand-written match plus a hand-written const
/// array) is the exact kind of two-places-say-the-same-thing drift a test can
/// catch but not prevent. Still an explicit table of string literals, not a
/// PascalCase→kebab-case computation (SPEC.md §2 rejected deriving the string
/// itself as too magic) — this only removes writing each pair twice.
///
/// A variant not listed here would make `name`'s match non-exhaustive over
/// `EditorEvent` and fail to compile — so today, with every variant
/// Steel-visible, this is equivalent to the old exhaustive match, just
/// written once. An internal-only variant (§1a's "N+1 Rust-only events") is
/// out of scope for this macro as written; extend it with a
/// `$variant:ident` arm (no `=> $name`) mapping to `None` if one appears.
macro_rules! editor_event_names {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        impl EditorEvent {
            /// The Steel symbol name for this event, or `None` if it's
            /// internal-only (raised and reacted to entirely on the Rust
            /// side). An exhaustive match (generated by
            /// `editor_event_names!`) rather than a table lookup: every
            /// variant's name is compiler-checked, not just checked by a
            /// test.
            pub(crate) fn name(&self) -> Option<&'static str> {
                Some(match self {
                    $(EditorEvent::$variant { .. } => $name,)+
                })
            }
        }

        /// Every Steel-visible event name — backs `EventHost::known_event_names`,
        /// consulted by `register-hook!` and `declare-plugin`'s `#:events` to
        /// validate names without `hume-scripting` compiling in `EditorEvent`.
        const EVENT_NAMES: &[&str] = &[$($name),+];
    };
}

editor_event_names! {
    OnBufferOpen => "on-buffer-open",
    OnBufferClose => "on-buffer-close",
    OnBufferSave => "on-buffer-save",
    OnModeChange => "on-mode-change",
    OnLanguageSet => "on-language-set",
    OnLspAttach => "on-lsp-attach",
    OnLspDetach => "on-lsp-detach",
    OnDiagnosticsChanged => "on-diagnostics-changed",
    OnViewportChange => "on-viewport-change",
    OnTriggerChar => "on-trigger-char",
    OnCompletionAccept => "on-completion-accept",
    OnCompletionRefilter => "on-completion-refilter",
}

impl EditorEvent {
    /// The single definition of every event's Steel arg shape — the SSOT
    /// `user-manual/docs/plugins.md`'s hook table is checked against, and
    /// the only place `IntoSteelVal`/`json_to_steel` is invoked for events.
    /// Called at drain, after the `has_hook_handlers` early-exit, so an
    /// event nobody subscribes to never allocates a `SteelVal`.
    pub(crate) fn steel_args(&self) -> Vec<SteelVal> {
        match self {
            EditorEvent::OnBufferOpen { buffer }
            | EditorEvent::OnBufferClose { buffer }
            | EditorEvent::OnBufferSave { buffer }
            | EditorEvent::OnDiagnosticsChanged { buffer } => {
                vec![SteelBufferId::new(*buffer).into_steel_val()]
            }
            EditorEvent::OnModeChange { from, to } => {
                vec![
                    SteelVal::StringV(mode_name(*from).into()),
                    SteelVal::StringV(mode_name(*to).into()),
                ]
            }
            EditorEvent::OnLanguageSet { buffer, language } => {
                let lang_val = match language {
                    Some(name) => SteelVal::StringV(name.as_str().into()),
                    None => SteelVal::BoolV(false),
                };
                vec![SteelBufferId::new(*buffer).into_steel_val(), lang_val]
            }
            EditorEvent::OnLspAttach { buffer, server }
            | EditorEvent::OnLspDetach { buffer, server } => {
                vec![
                    SteelBufferId::new(*buffer).into_steel_val(),
                    SteelVal::StringV(server.as_str().into()),
                ]
            }
            EditorEvent::OnViewportChange {
                buffer,
                first_line,
                last_line,
            } => {
                vec![
                    SteelBufferId::new(*buffer).into_steel_val(),
                    SteelVal::IntV(*first_line as isize),
                    SteelVal::IntV(*last_line as isize),
                ]
            }
            EditorEvent::OnTriggerChar { buffer, ch, source } => {
                vec![
                    SteelBufferId::new(*buffer).into_steel_val(),
                    SteelVal::StringV(ch.to_string().into()),
                    SteelVal::StringV(source.as_str().into()),
                ]
            }
            EditorEvent::OnCompletionAccept { buffer, item } => {
                vec![
                    SteelBufferId::new(*buffer).into_steel_val(),
                    json_to_steel(item),
                ]
            }
            EditorEvent::OnCompletionRefilter {
                buffer,
                filter_text,
            } => {
                vec![
                    SteelBufferId::new(*buffer).into_steel_val(),
                    SteelVal::StringV(filter_text.as_str().into()),
                ]
            }
        }
    }
}

fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Normal => "normal",
        Mode::Insert => "insert",
        Mode::Extend => "extend",
        Mode::Command => "command",
        Mode::Search => "search",
        Mode::Select => "select",
    }
}

pub(crate) fn known_event_names() -> Vec<&'static str> {
    EVENT_NAMES.to_vec()
}

#[cfg(test)]
mod tests;
