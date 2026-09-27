//! `EditorEvent`: the editor's own vocabulary of "something happened worth
//! telling subscribers about". SSOT for which events exist, their typed Rust
//! payloads, and their Steel-facing names/arg shapes; `hume-scripting` never
//! compiles in this type, only the `&str` names and `SteelVal` args produced
//! here (see `hume_scripting::host::EventHost`).

use hume_engine::pipeline::BufferId;
use hume_scripting::json::JsonHandle;
use hume_scripting::{PaneHandle, SteelPane};
use steel::rvals::SteelVal;

use super::Mode;

/// Something that happened in the editor, carrying whatever payload its
/// Steel handlers (and, for a growing subset, Rust-side reactions) need.
///
/// `steel_args` is the single place a variant's fields become `SteelVal`s;
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
    /// a derived join of `state.focus` and `pane.buffer_id`, each written by
    /// its own chokepoint (`focus_pane`, `switch_pane_to_buffer`), but no
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
    /// it: once per already-attached buffer at that moment, and again for
    /// any buffer that attaches later while the server stays Running.
    OnLspAttach {
        buffer: BufferId,
        server: String,
    },
    /// Fires once per buffer detached by `:lsp-stop`/`:lsp-restart`, right
    /// after `buf.lsp_server` is cleared: the counterpart to `OnLspAttach`,
    /// so a plugin holding buffer-scoped state derived from that server
    /// (e.g. inlay hints) can clear it instead of leaving it to drift with
    /// no server left to keep it in sync.
    OnLspDetach {
        buffer: BufferId,
        server: String,
    },
    /// Fires once per drain batch that ingested at least one
    /// `publishDiagnostics` for `buffer`, payload-free signal by design;
    /// pull via `(diagnostics-for-buffer bid …)`.
    OnDiagnosticsChanged {
        buffer: BufferId,
    },
    /// Fires after scroll/resize resolves a pane's viewport, debounced
    /// (`lsp.viewport-debounce-ms`) so a scroll burst fires once. `first_line`
    /// / `end_line` are the visible range, end-exclusive (matching
    /// `viewport-range`'s convention). No registered handler currently reads
    /// either arg (each re-reads live state via `(viewport-range pane)`
    /// instead), so this is a payload shape, not a behavior guarantee.
    /// `target`'s pane is the pane whose viewport actually scrolled, not
    /// necessarily the focused one.
    OnViewportChange {
        target: PaneHandle,
        first_line: hume_rope::line::ContentLine,
        end_line: hume_rope::line::ContentLine,
    },
    /// Fires in Insert mode after a registered trigger char (see
    /// `register-trigger-chars!`) has been inserted into the buffer, once
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
    /// Fires when a buffer's text changes: user edits, undo, redo, `:e!`
    /// reload, and read-only view refreshes (`:messages`, `:ls`,
    /// `:plugin-status`) alike, all of which bump `Buffer::text_gen`. Raised
    /// by diffing `text_gen` against a per-buffer `announced_text_gen`
    /// baseline at a drain observation point (`Editor::detect_text_changed`,
    /// `BufferStore::take_text_changed`), not from `Buffer::set_text` itself,
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
    /// (`Buffer::apply_edit*`, `commit_edit_group`, and `reload_from_text` all
    /// skip the mutation (or the revision that would make undo replay one)
    /// entirely in that case, specifically so this doesn't fire for one).
    /// Does fire, unconditionally and with no identity check,
    /// for every `:messages`/`:ls`/`:plugin-status` refresh of an
    /// already-open view buffer, even a byte-identical one; a handler that
    /// resolves the buffer's path must handle `#f` (these buffers have none).
    /// See `Buffer::announced_text_gen`'s doc.
    OnTextChanged {
        buffer: BufferId,
    },
    /// Fires after a successful `:set global`/`set-option!`/`:theme` write.
    /// `settings::ops::apply_global` is the single production path every one
    /// of those funnels through, so this is the one place to raise it.
    /// Buffer-scoped overrides (`:set`/`set-buffer-option!` without
    /// `global`) don't raise this: the payload has no `BufferId` to name,
    /// and `apply_buffer` has no per-key resync effects to piggyback on (see
    /// its doc). `value` is the raw string `:set`/`set-option!` was given,
    /// not its parsed/coerced form (`write_global` discards the parsed value
    /// after validating it); a plugin owning one setting's policy (e.g. the
    /// LSP inlay-hints plugin reacting to `lsp.inlay-hints`) should re-read
    /// `(get-option key)` for a typed value rather than pattern-match this
    /// string.
    OnOptionChange {
        key: String,
        value: String,
    },
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
            | EditorEvent::OnTextChanged { buffer } => Some(PaneHandle::buffer_only(*buffer)),
            EditorEvent::OnBufferEnter { target }
            | EditorEvent::OnViewportChange { target, .. }
            | EditorEvent::OnTriggerChar { target, .. }
            | EditorEvent::OnCompletionAccept { target, .. } => Some(*target),
            EditorEvent::OnFocusGained
            | EditorEvent::OnModeChange { .. }
            | EditorEvent::OnOptionChange { .. } => None,
        }
    }
}

/// Pairs each `EditorEvent` variant with its Steel-facing name, once, and
/// generates both `EditorEvent::name`'s match and `EVENT_NAMES` from that one
/// list. The alternative (a hand-written match plus a hand-written const
/// array) is the exact kind of two-places-say-the-same-thing drift a test can
/// catch but not prevent. Still an explicit table of string literals, not a
/// PascalCase→kebab-case computation: writing each name out means a variant
/// rename can never silently rename the Steel-facing event too; this macro
/// only removes writing each pair twice.
///
/// A variant not listed here would make `name`'s match non-exhaustive over
/// `EditorEvent` and fail to compile, so with every variant Steel-visible,
/// this is an exhaustive match written once. A future internal-only
/// (Rust-only, no Steel-facing name)
/// variant is out of scope for this macro as written; give `name` an
/// `Option` return and extend it with a `$variant:ident` arm (no `=> $name`)
/// mapping to `None` if one appears.
macro_rules! editor_event_names {
    ($($variant:ident => $name:literal),+ $(,)?) => {
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

        /// Every Steel-visible event name. Backs `EventHost::known_event_names`,
        /// consulted by `register-hook!` and `declare-plugin`'s `#:events` to
        /// validate names without `hume-scripting` compiling in `EditorEvent`.
        const EVENT_NAMES: &[&str] = &[$($name),+];
    };
}

editor_event_names! {
    OnBufferOpen => "on-buffer-open",
    OnBufferClose => "on-buffer-close",
    OnBufferSave => "on-buffer-save",
    OnBufferEnter => "on-buffer-enter",
    OnFocusGained => "on-focus-gained",
    OnModeChange => "on-mode-change",
    OnLanguageSet => "on-language-set",
    OnLspAttach => "on-lsp-attach",
    OnLspDetach => "on-lsp-detach",
    OnDiagnosticsChanged => "on-diagnostics-changed",
    OnViewportChange => "on-viewport-change",
    OnTriggerChar => "on-trigger-char",
    OnCompletionAccept => "on-completion-accept",
    OnOptionChange => "on-option-change",
    OnTextChanged => "on-text-changed",
}

impl EditorEvent {
    /// The single definition of every event's Steel arg shape: the SSOT
    /// `user-manual/docs/plugins.md`'s hook table is checked against, and
    /// the only place `IntoSteelVal`/`to_steel_handle` is invoked for events.
    /// Called at drain, after the `has_hook_handlers` early-exit, so an
    /// event nobody subscribes to never allocates a `SteelVal`.
    pub(in crate::editor) fn steel_args(&self) -> Vec<SteelVal> {
        match self {
            EditorEvent::OnBufferOpen { buffer }
            | EditorEvent::OnBufferClose { buffer }
            | EditorEvent::OnBufferSave { buffer }
            | EditorEvent::OnDiagnosticsChanged { buffer }
            | EditorEvent::OnTextChanged { buffer } => {
                vec![SteelPane::new(PaneHandle::buffer_only(*buffer)).into_steel_val()]
            }
            EditorEvent::OnBufferEnter { target } => {
                vec![SteelPane::new(*target).into_steel_val()]
            }
            EditorEvent::OnFocusGained => vec![],
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
                vec![
                    SteelPane::new(PaneHandle::buffer_only(*buffer)).into_steel_val(),
                    lang_val,
                ]
            }
            EditorEvent::OnLspAttach { buffer, server }
            | EditorEvent::OnLspDetach { buffer, server } => {
                vec![
                    SteelPane::new(PaneHandle::buffer_only(*buffer)).into_steel_val(),
                    SteelVal::StringV(server.as_str().into()),
                ]
            }
            EditorEvent::OnViewportChange {
                target,
                first_line,
                end_line,
            } => {
                vec![
                    SteelPane::new(*target).into_steel_val(),
                    SteelVal::IntV(first_line.index() as isize),
                    SteelVal::IntV(end_line.index() as isize),
                ]
            }
            EditorEvent::OnTriggerChar { target, ch, source } => {
                vec![
                    SteelPane::new(*target).into_steel_val(),
                    SteelVal::StringV(ch.to_string().into()),
                    SteelVal::StringV(source.as_str().into()),
                ]
            }
            EditorEvent::OnCompletionAccept { target, item } => {
                vec![
                    SteelPane::new(*target).into_steel_val(),
                    item.clone().into_steel_val(),
                ]
            }
            EditorEvent::OnOptionChange { key, value } => {
                vec![
                    SteelVal::StringV(key.as_str().into()),
                    SteelVal::StringV(value.as_str().into()),
                ]
            }
        }
    }
}

/// One item of deferred Steel work, queued by a raise site and drained by
/// `Editor::settle()` in FIFO order. One merged queue closes the stranded-
/// events bug (see `tests/events.rs`'s
/// `event_raised_from_async_work_fires_on_settle_with_no_input`): a `Call`
/// and an `Event` queued in the same batch drain in insertion order, in one
/// fixpoint, instead of two queues drained at two different points of the
/// run loop.
///
/// Hooks always route through here rather than firing inline. This is a
/// semantic guarantee of the hook model ("when X happens, then do Y"), not a
/// consequence of the borrow architecture: a hook must run *after* the
/// command that triggers it completes, never mid-command. Even if re-entrancy
/// were fully solved mechanically, this stays queued. **Do not optimize hooks
/// to fire inline**: this decision is locked.
#[derive(Debug)]
pub(in crate::editor) enum PendingWork {
    /// A specific Steel closure already captured by the raise site: an
    /// `lsp-request` callback, a timer thunk, a prompt/menu/drawer/picker
    /// callback. Delivered to exactly that closure, not to every handler for
    /// a name.
    ///
    /// `anchor`: `Some` only for an `lsp-request` callback, carrying the same
    /// `ResponseAnchor` already checked once at LSP drain time
    /// (`Editor::anchor_admits`), re-checked here, at dequeue, because
    /// arbitrary other queued work (an earlier `Call` in the same batch, a
    /// hook) can run first and change the state the drain-time check saw.
    /// `None` for every other source (a timer thunk, a picker callback, …),
    /// which never had an anchor to begin with.
    Call {
        proc: SteelVal,
        args: Vec<SteelVal>,
        anchor: Option<super::lsp::ResponseAnchor>,
        /// A dot-capture handed off from a picker's own `PickerSession` (see
        /// [`super::edit_session::DotCapture`]'s own doc). `Some` only for
        /// a picker's `on_select`/dismiss call, and only when the dispatch
        /// that opened it was itself under one. `Editor::run_pending_batch`
        /// re-arms it on whatever session is current when this call actually
        /// runs, which forces this call to run alone rather than batched
        /// with any sibling `Call`: a fresh arm per call, never per batch.
        dot_capture: Option<super::edit_session::DotCapture>,
    },
    /// An editor event to fire by name at drain time. Args are built by
    /// `steel_args()` only if a handler is actually registered.
    Event(EditorEvent),
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

#[cfg(test)]
mod tests;
