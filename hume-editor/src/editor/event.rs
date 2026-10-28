//! `EditorEvent`: the editor's own vocabulary of "something happened worth
//! telling subscribers about". SSOT for which events exist, their typed Rust
//! payloads, and their Steel-facing names/arg shapes; `hume-scripting` never
//! compiles in this type, only the `&str` names and `SteelVal` args produced
//! here (see `hume_scripting::host::EventHost`).

use hume_core::{BufferId, PaneHandle};
use hume_editing::hunk::ChangeHunk;
use hume_scripting::host::{EventShape, OptionValue};
use hume_scripting::json::JsonHandle;
use hume_scripting::{ServerRef, SteelPane};
use steel::rvals::SteelVal;

use super::Mode;

/// Something that happened in the editor, carrying whatever payload its
/// Steel handlers (and, for a growing subset, Rust-side reactions) need.
///
/// `into_steel_args` is the single place a variant's fields become `SteelVal`s;
/// see its doc for why arg construction lives there and not at the raise
/// site.
// All variants share the `On` prefix, matching the `on-buffer-open` Steel naming
// convention. The lint wants dissimilar prefixes; we intentionally override it.
#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
pub(in crate::editor) enum EditorEvent {
    OnBufferOpen {
        buffer: BufferId,
    },
    OnBufferClose {
        buffer: BufferId,
    },
    OnBufferSave {
        buffer: BufferId,
    },
    /// Fires when the focused pane's buffer changes: a diff taken inside
    /// `Editor::settle()`'s fixpoint against `EditorState::last_entered_buffer`,
    /// not a hook on any individual switch primitive. `focused_buffer_id()` is
    /// a derived join of `view.focused_pane()` and `pane.buffer_id`, each
    /// written by its own chokepoint (`focus`, `switch_pane_to_buffer`), but no
    /// single one of those covers both, so neither alone can host the raise.
    /// Fires once at startup (the initial buffer entering focus) and once
    /// more per subsequent switch, coalescing a pane-focus move and a buffer
    /// switch in the same `settle()` pass into a single event. `target`'s
    /// pane is the focused pane at raise time; a buffer only "enters" by
    /// way of some pane showing it.
    OnBufferEnter {
        target: PaneHandle,
    },
    /// Fires when the terminal regains focus, or the editor otherwise regains
    /// control of it (return from an inline shell command): every open
    /// buffer may have changed while the editor wasn't watching, not just the
    /// focused one. Payload-free by design: contrast `OnBufferEnter`, which
    /// names the one buffer that changed focus.
    OnFocusGained,
    OnModeChange {
        from: Mode,
        to: Mode,
    },
    /// Fires on every language transition (including round-trips and clears).
    /// `language` is the resolved name at raise time, not a `LanguageId`, since
    /// the id is a registry index and `:reload-config` can rebuild the
    /// registry before this drains, so resolving early is the only
    /// consistent read.
    ///
    /// **For lazy-loading:** use `#:languages` in `declare-plugin!` instead.
    /// `#:languages` *activates* the plugin on the *first* matching transition;
    /// the body then registers an `on-language-set` *hook* to react on every
    /// subsequent transition. Using `on-language-set` as a `#:events` activation
    /// entry would activate the plugin on *any* language transition, not just the
    /// ones it cares about.
    OnLanguageSet {
        buffer: BufferId,
        language: Option<String>,
    },
    /// Fires when a server is `Running` for a buffer attached to it: for
    /// every attached buffer when the server's handshake completes, for
    /// each buffer that attaches later while it stays Running, again when
    /// the attachment's feature filter changes, and again after a
    /// `:reload-config` that keeps the attachment.
    OnLspAttach {
        buffer: BufferId,
        server: ServerRef,
    },
    /// Fires once per (buffer, server) detach: `:lsp-stop`/`:lsp-restart`,
    /// a server list or language change that drops the server. Closing the
    /// buffer fires `OnBufferClose` instead. The counterpart to
    /// `OnLspAttach`, so a plugin holding buffer-scoped state derived from
    /// that server (e.g. inlay hints) can refresh it from the servers that
    /// remain.
    OnLspDetach {
        buffer: BufferId,
        server: ServerRef,
    },
    /// Fires when the diagnostics stored for `buffer` may have changed: a
    /// drain batch that ingested a `publishDiagnostics` for it, a pull
    /// report, a server detaching or its filter changing, `:e!` dropping
    /// them, and a config reload replaying them. Payload-free by design;
    /// pull via `(diagnostics-for-buffer bid …)`.
    OnDiagnosticsChanged {
        buffer: BufferId,
    },
    /// Fires when a pane's visible range changes (scroll, resize, an edit
    /// that moves the EOF-clamped end, a buffer switch), debounced
    /// (`hook-viewport-debounce-ms`) so a burst fires once. `lines` is the range
    /// `viewport-range` returns, end-exclusive, and reaches the hook as the
    /// same value; it covers every line that could be visible and is exact
    /// unless soft wrap
    /// makes lines span several rows.
    /// `target`'s pane is the pane whose viewport actually scrolled, not
    /// necessarily the focused one.
    OnViewportChange {
        target: PaneHandle,
        lines: hume_rope::offset::ExclusiveRange<hume_rope::line::ContentLine>,
    },
    /// Fires in Insert mode after a registered trigger char (see
    /// `set-hook-triggers!`) has been inserted into the buffer, once
    /// per source registered for that char under the buffer's language, so
    /// two sources sharing a char each get their own fire. `target`'s pane
    /// is the focused pane at raise time (Insert mode only ever types into
    /// it).
    OnTriggerChar {
        target: PaneHandle,
        ch: char,
        source: String,
    },
    /// Fires after `completion-accept!` applies the item's main `textEdit`
    /// (or `insertText` fallback), `additionalTextEdits`, and (if needed)
    /// `completionItem/resolve`. Rust owns all three atomically, so this is
    /// a plain extension point for anything the completion store doesn't
    /// itself parse (e.g. `command`), not a place that needs to apply edits.
    /// `item` is the accepted `CompletionItem`'s raw JSON as a `JsonHandle`
    /// (built at the one queue site, `session/accept.rs`) rather than a bare
    /// `Arc<Value>`: a handle shares the same underlying `Arc` just as
    /// cheaply, and keeps the item's `WireOrigin` tag alive for a handler
    /// that reads a position back out of it. `target`'s pane is the
    /// completion session's own pane, not necessarily the focused one at
    /// fire time.
    OnCompletionAccept {
        target: PaneHandle,
        item: JsonHandle,
    },
    /// Fires for a server notification HUME doesn't handle itself (it
    /// handles `window/logMessage`, `window/showMessage`, `$/progress`, and
    /// `publishDiagnostics`). `server` is the sending server. `params`
    /// crosses through `to_steel_handle` tagged with `origin` (the server
    /// and its position encoding), so a handler can read a wire position
    /// out of it. A `register-hook!` handler sees every method; a
    /// `register-lsp-notification-hook!` one only the methods it lists.
    OnLspNotification {
        server: ServerRef,
        method: String,
        params: std::sync::Arc<serde_json::Value>,
        origin: hume_scripting::json::WireOrigin,
    },
    /// Fires when a buffer's text changes: user edits, undo, redo, `:e!`
    /// reload, and read-only view refreshes (`:messages`, `:ls`,
    /// `:plugin-status`) alike, all of which bump the buffer text's generation. Raised
    /// by diffing the text generation against a per-buffer `announced_generation`
    /// baseline at a drain observation point (`Editor::detect_text_changed`,
    /// `BufferStore::take_text_changed`), not from `Buffer::install` itself,
    /// since `Buffer` has no path to the event queue, the same reason
    /// `OnBufferEnter` is raised via a diff rather than a raise site.
    /// Consequently this
    /// **coalesces**: several mutations to the same buffer observed by one
    /// pass of `drain_pending_work`'s fixpoint (detection runs at the top of
    /// every pass, so at least once per `settle()`) fire exactly one event,
    /// not one per mutation.
    ///
    /// Never fires for: a no-op undo at the history root; an edit refused by
    /// the read-only guard; an edit, insert/paste session, or `:e!` reload
    /// whose composed `ChangeSet` is the identity transform
    /// (`Buffer::apply_edit*`, `commit_edit_group`, and `replace_text_recorded` all
    /// skip the mutation (or the revision that would make undo replay one)
    /// entirely in that case, specifically so this doesn't fire for one).
    /// Does fire, unconditionally and with no identity check,
    /// for every `:messages`/`:ls`/`:plugin-status` refresh of an
    /// already-open view buffer, even a byte-identical one; a handler that
    /// resolves the buffer's path must handle `#f` (these buffers have none).
    /// See `Buffer::announced_generation`'s doc.
    ///
    /// `changes` is `Buffer::changes_since` the text the buffer's previous
    /// event announced: hunks with that text on the old side and the text at
    /// detection on the new side, `None` when the buffer cannot say (a view
    /// refresh, or a change log that keeps no step that far back). A
    /// handler that edits the buffer moves it past the new side for the
    /// handlers after it, and the next event reports that edit.
    OnTextChanged {
        buffer: BufferId,
        changes: Option<Vec<ChangeHunk>>,
    },
    /// Fires when a buffer's undo history changes: a revision recorded (an
    /// edit, an Insert or paste session ending, an `:e!` reload), an undo,
    /// redo or jump to another revision, an `undo-levels` trim, or a write that
    /// moves the saved revision. Unlike `OnTextChanged` it also fires for a walk whose net change to the text
    /// is nothing, such as undoing an insert together with its later delete,
    /// and never fires for text changes that record no revision (a view
    /// buffer's refresh). Raised by diffing a per-buffer baseline at the same drain observation point as
    /// `OnTextChanged` (`Editor::detect_history_changed`,
    /// `BufferStore::take_history_changed`, over a `HistoryMark` of the
    /// `change_seq` and the saved revision), so it coalesces the same way.
    OnUndoHistoryChanged {
        buffer: BufferId,
    },
    /// Fires after a successful `:set global`/`set-option!`/`:theme` write.
    /// `settings::ops::apply_global` is the single production path every one
    /// of those funnels through, so this is the one place to raise it.
    /// Buffer-scoped overrides (`:set`/`set-buffer-option!` without
    /// `global`) don't raise this: the payload has no `BufferId` to name,
    /// and `apply_buffer` has no per-key resync effects to piggyback on (see
    /// its doc). `value` is the setting's new value as `(get-option key)`
    /// reports it: a bool, integer, string, or list of strings.
    OnOptionChange {
        key: String,
        value: OptionValue,
    },
}

/// An event whose raiser waits for its handlers. Kept out of `EditorEvent`
/// so it cannot be queued: only `ScriptingHost::fire_held_hook` delivers it.
#[derive(Debug)]
pub(in crate::editor) enum HeldEvent {
    /// Fires before a write to the buffer's own file, with the write parked
    /// until every handler is done (`before_save`). A handler taking
    /// `(pane)` is done when it returns; one taking `(pane done)` is done
    /// when it calls `(done)`. Not raised for a write that cannot reach
    /// the buffer's own file. `target` carries the pane that asked for the
    /// write when it still shows the buffer as the event is raised, and no
    /// pane otherwise (`:wa` on a buffer it is not showing).
    OnBeforeBufferSave { target: PaneHandle },
}

impl HeldEvent {
    /// The args each handler gets ahead of `done`.
    pub(in crate::editor) fn into_steel_args(self) -> Vec<SteelVal> {
        match self {
            HeldEvent::OnBeforeBufferSave { target } => {
                vec![SteelPane::new(target).into_steel_val()]
            }
        }
    }
}

impl EditorEvent {
    /// The buffer (and, where the variant carries one, pane) this event
    /// concerns, if any: `None` for a buffer-less event (`OnFocusGained`,
    /// `OnModeChange`, `OnOptionChange`). A buffer-only variant wraps its
    /// `buffer` in [`PaneHandle::buffer_only`] rather than exposing it bare,
    /// so every caller checking event staleness (`Editor::run_pending_batch`)
    /// runs the same liveness check regardless of which shape a given
    /// variant happens to carry. Exhaustive match, no `_` arm: a future
    /// variant must be added here explicitly or this fails to compile, the same
    /// discipline `Editor::react_to_event`'s own match uses, for the same
    /// reason (a forgotten variant should be a compile error, not a silent
    /// `None`).
    pub(in crate::editor) fn handle(&self) -> Option<PaneHandle> {
        match self {
            EditorEvent::OnBufferOpen { buffer }
            | EditorEvent::OnBufferClose { buffer }
            | EditorEvent::OnBufferSave { buffer }
            | EditorEvent::OnLanguageSet { buffer, .. }
            | EditorEvent::OnLspAttach { buffer, .. }
            | EditorEvent::OnLspDetach { buffer, .. }
            | EditorEvent::OnDiagnosticsChanged { buffer }
            | EditorEvent::OnTextChanged { buffer, .. }
            | EditorEvent::OnUndoHistoryChanged { buffer } => {
                Some(PaneHandle::buffer_only(*buffer))
            }
            EditorEvent::OnBufferEnter { target }
            | EditorEvent::OnViewportChange { target, .. }
            | EditorEvent::OnTriggerChar { target, .. }
            | EditorEvent::OnCompletionAccept { target, .. } => Some(*target),
            EditorEvent::OnFocusGained
            | EditorEvent::OnModeChange { .. }
            | EditorEvent::OnOptionChange { .. }
            | EditorEvent::OnLspNotification { .. } => None,
        }
    }

    /// The key a keyed hook handler filters on: an LSP notification's
    /// method, matched against `register-lsp-notification-hook!`'s methods.
    /// `None` for every other event, which only unkeyed handlers receive.
    /// Exhaustive match for the same reason as `handle`.
    pub(in crate::editor) fn hook_key(&self) -> Option<&str> {
        match self {
            EditorEvent::OnLspNotification { method, .. } => Some(method),
            EditorEvent::OnBufferOpen { .. }
            | EditorEvent::OnBufferClose { .. }
            | EditorEvent::OnBufferSave { .. }
            | EditorEvent::OnBufferEnter { .. }
            | EditorEvent::OnFocusGained
            | EditorEvent::OnModeChange { .. }
            | EditorEvent::OnLanguageSet { .. }
            | EditorEvent::OnLspAttach { .. }
            | EditorEvent::OnLspDetach { .. }
            | EditorEvent::OnDiagnosticsChanged { .. }
            | EditorEvent::OnViewportChange { .. }
            | EditorEvent::OnTriggerChar { .. }
            | EditorEvent::OnCompletionAccept { .. }
            | EditorEvent::OnOptionChange { .. }
            | EditorEvent::OnTextChanged { .. }
            | EditorEvent::OnUndoHistoryChanged { .. } => None,
        }
    }

    /// The Trace line reported when no handler takes this event, for the
    /// events where that is worth surfacing. Exhaustive match for the same
    /// reason as `handle`.
    pub(in crate::editor) fn unhandled_trace(&self) -> Option<String> {
        match self {
            EditorEvent::OnLspNotification { server, method, .. } => {
                Some(format!("{}: unhandled notification {method}", server.name))
            }
            EditorEvent::OnBufferOpen { .. }
            | EditorEvent::OnBufferClose { .. }
            | EditorEvent::OnBufferSave { .. }
            | EditorEvent::OnBufferEnter { .. }
            | EditorEvent::OnFocusGained
            | EditorEvent::OnModeChange { .. }
            | EditorEvent::OnLanguageSet { .. }
            | EditorEvent::OnLspAttach { .. }
            | EditorEvent::OnLspDetach { .. }
            | EditorEvent::OnDiagnosticsChanged { .. }
            | EditorEvent::OnViewportChange { .. }
            | EditorEvent::OnTriggerChar { .. }
            | EditorEvent::OnCompletionAccept { .. }
            | EditorEvent::OnOptionChange { .. }
            | EditorEvent::OnTextChanged { .. }
            | EditorEvent::OnUndoHistoryChanged { .. } => None,
        }
    }
}

/// Pairs each `EditorEvent` and `HeldEvent` variant with its Steel-facing
/// name and the number of args its handlers get, once, and generates both
/// `name` matches, `EVENT_NAMES` and `EVENT_SHAPES` from those lists.
/// Still an explicit table of names (string literals, or a const for one
/// the write path asks for without an event), not a PascalCase→kebab-case
/// computation: writing each name out means a variant rename can never
/// silently rename the Steel-facing event too; this macro only removes
/// writing each pair twice. A held entry's count is of the args its
/// handlers get ahead of `done`.
///
/// A variant not listed here would make `name`'s match non-exhaustive and
/// fail to compile, so with every variant Steel-visible, this is an
/// exhaustive match written once. A future internal-only (Rust-only, no
/// Steel-facing name) variant is out of scope for this macro as written;
/// give `name` an `Option` return and extend it with a `$variant:ident` arm
/// (no `=> $name`) mapping to `None` if one appears.
macro_rules! editor_event_names {
    (
        queued { $($variant:ident => $name:tt ($args:literal)),+ $(,)? }
        held { $($held_variant:ident => $held_name:tt ($held_args:literal)),+ $(,)? }
    ) => {
        impl EditorEvent {
            /// The Steel symbol name for this event. An exhaustive match
            /// (generated by `editor_event_names!`) rather than a table
            /// lookup: every variant's name is compiler-checked, not just
            /// checked by a test.
            pub(in crate::editor) fn name(&self) -> &'static str {
                match self {
                    $(EditorEvent::$variant { .. } => $name,)+
                }
            }
        }

        impl HeldEvent {
            /// The Steel symbol name for this event.
            pub(in crate::editor) fn name(&self) -> &'static str {
                match self {
                    $(HeldEvent::$held_variant { .. } => $held_name,)+
                }
            }
        }

        /// Every Steel-visible event name. Backs `EventHost::known_event_names`,
        /// consulted by `register-hook!` and `declare-plugin!`'s `#:events` to
        /// validate names without `hume-scripting` compiling in `EditorEvent`.
        const EVENT_NAMES: &[&str] = &[$($name,)+ $($held_name,)+];

        /// What each event hands its handlers, by name.
        const EVENT_SHAPES: &[(&str, EventShape)] = &[
            $(($name, EventShape { args: $args, held: false }),)+
            $(($held_name, EventShape { args: $held_args, held: true }),)+
        ];
    };
}

/// Named so the write path can ask for handlers without an event in hand.
pub(in crate::editor) const BEFORE_BUFFER_SAVE: &str = "on-before-buffer-save";

editor_event_names! {
    queued {
        OnBufferOpen => "on-buffer-open" (1),
        OnBufferClose => "on-buffer-close" (1),
        OnBufferSave => "on-buffer-save" (1),
        OnBufferEnter => "on-buffer-enter" (1),
        OnFocusGained => "on-focus-gained" (0),
        OnModeChange => "on-mode-change" (2),
        OnLanguageSet => "on-language-set" (2),
        OnLspAttach => "on-lsp-attach" (2),
        OnLspDetach => "on-lsp-detach" (2),
        OnDiagnosticsChanged => "on-diagnostics-changed" (1),
        OnViewportChange => "on-viewport-change" (2),
        OnTriggerChar => "on-trigger-char" (3),
        OnCompletionAccept => "on-completion-accept" (2),
        OnLspNotification => "on-lsp-notification" (3),
        OnOptionChange => "on-option-change" (2),
        OnTextChanged => "on-text-changed" (2),
        OnUndoHistoryChanged => "on-undo-history-changed" (1),
    }
    held {
        OnBeforeBufferSave => BEFORE_BUFFER_SAVE (1),
    }
}

impl EditorEvent {
    /// The single definition of every event's Steel arg shape: the SSOT
    /// `user-manual/docs/plugins.md`'s hook table is checked against, and
    /// the only place `IntoSteelVal`/`to_steel_handle` is invoked for events.
    /// Called at drain, after the `has_hook_handlers` early-exit, so an
    /// event nobody subscribes to never allocates a `SteelVal`.
    pub(in crate::editor) fn into_steel_args(self) -> Vec<SteelVal> {
        match self {
            EditorEvent::OnBufferOpen { buffer }
            | EditorEvent::OnBufferClose { buffer }
            | EditorEvent::OnBufferSave { buffer }
            | EditorEvent::OnDiagnosticsChanged { buffer }
            | EditorEvent::OnUndoHistoryChanged { buffer } => {
                vec![SteelPane::new(PaneHandle::buffer_only(buffer)).into_steel_val()]
            }
            EditorEvent::OnTextChanged { buffer, changes } => {
                vec![
                    SteelPane::new(PaneHandle::buffer_only(buffer)).into_steel_val(),
                    hume_scripting::hunk_list_value(changes),
                ]
            }
            EditorEvent::OnBufferEnter { target } => {
                vec![SteelPane::new(target).into_steel_val()]
            }
            EditorEvent::OnFocusGained => vec![],
            EditorEvent::OnModeChange { from, to } => {
                vec![
                    SteelVal::SymbolV(mode_name(from).into()),
                    SteelVal::SymbolV(mode_name(to).into()),
                ]
            }
            EditorEvent::OnLanguageSet { buffer, language } => {
                vec![
                    SteelPane::new(PaneHandle::buffer_only(buffer)).into_steel_val(),
                    SteelVal::StringV(language.unwrap_or_default().into()),
                ]
            }
            EditorEvent::OnLspAttach { buffer, server }
            | EditorEvent::OnLspDetach { buffer, server } => {
                vec![
                    SteelPane::new(PaneHandle::buffer_only(buffer)).into_steel_val(),
                    server.into_steel_val(),
                ]
            }
            EditorEvent::OnViewportChange { target, lines } => {
                vec![
                    SteelPane::new(target).into_steel_val(),
                    hume_scripting::line_range_value(lines),
                ]
            }
            EditorEvent::OnTriggerChar { target, ch, source } => {
                vec![
                    SteelPane::new(target).into_steel_val(),
                    SteelVal::CharV(ch),
                    SteelVal::StringV(source.into()),
                ]
            }
            EditorEvent::OnCompletionAccept { target, item } => {
                vec![
                    SteelPane::new(target).into_steel_val(),
                    item.into_steel_val(),
                ]
            }
            EditorEvent::OnOptionChange { key, value } => {
                vec![SteelVal::StringV(key.into()), SteelVal::from(value)]
            }
            EditorEvent::OnLspNotification {
                server,
                method,
                params,
                origin,
            } => {
                vec![
                    server.into_steel_val(),
                    SteelVal::StringV(method.into()),
                    hume_scripting::json::to_steel_handle(params, origin),
                ]
            }
        }
    }
}

pub(in crate::editor) fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Normal => "normal",
        Mode::Insert => "insert",
        Mode::Extend => "extend",
        Mode::Command => "command",
        Mode::Search => "search",
        Mode::Sift => "sift",
    }
}

pub(crate) fn known_event_names() -> &'static [&'static str] {
    EVENT_NAMES
}

pub(crate) fn event_shape(name: &str) -> Option<EventShape> {
    EVENT_SHAPES
        .iter()
        .find(|(event, _)| *event == name)
        .map(|&(_, shape)| shape)
}

mod pump;
mod queue;

pub(in crate::editor) use queue::{CallGuard, PendingWork};

#[cfg(test)]
mod tests;
