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
#[derive(Debug)]
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
    /// Fires when the focused pane's buffer changes — a diff taken inside
    /// `Editor::settle()`'s fixpoint against `EditorState::last_entered_buffer`,
    /// not a hook on any individual switch primitive. `focused_buffer_id()` is
    /// a derived join of `focused_pane_id` (5 write sites) and `pane.buffer_id`
    /// (1 write site), so no write site can serve as a chokepoint to hang a
    /// raise on. Fires once at startup (the initial buffer entering focus)
    /// and once more per subsequent switch, coalescing a pane-focus move and a
    /// buffer switch in the same `settle()` pass into a single event.
    OnBufferEnter {
        buffer: BufferId,
    },
    /// Fires when the terminal regains focus, or the editor otherwise regains
    /// control of it (return from an inline shell command) — every open
    /// buffer may have changed while the editor wasn't watching, not just the
    /// focused one. Payload-free by design: contrast `OnBufferEnter`, which
    /// names the one buffer that changed focus.
    OnFocusGained,
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
    /// (`lsp.viewport-debounce-ms`) so a scroll burst fires once. `first_line`
    /// / `end_line` are the visible range, end-exclusive (matching
    /// `viewport-range`'s convention) — no registered handler currently reads
    /// either arg (each re-reads live state via `(viewport-range bid)`
    /// instead), so this is a payload shape, not a behavior guarantee.
    OnViewportChange {
        buffer: BufferId,
        first_line: usize,
        end_line: usize,
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
    /// Fires when a buffer's text changes — user edits, undo, redo, `:e!`
    /// reload, and read-only view refreshes (`:messages`, `:ls`,
    /// `:plugin-status`) alike, all of which bump `Buffer::text_gen`. Raised
    /// by diffing `text_gen` against a per-buffer `announced_text_gen`
    /// baseline at a drain observation point (`Editor::detect_text_changed`,
    /// `BufferStore::take_text_changed`), not from `Buffer::set_text` itself
    /// — `Buffer` has no path to the event queue, the same reason
    /// `OnBufferEnter` is raised via a diff rather than a raise site.
    /// Consequently this
    /// **coalesces**: several mutations to the same buffer observed by one
    /// pass of `drain_pending_work`'s fixpoint — detection runs at the top of
    /// every pass, so at least once per `settle()` — fire exactly one event,
    /// not one per mutation.
    ///
    /// Never fires for: a no-op undo at the history root; an edit refused by
    /// the read-only guard; an edit, insert/paste session, or `:e!` reload
    /// whose composed `ChangeSet` is the identity transform
    /// (`Buffer::apply_edit*`, `commit_edit_group`, and `reload_from_text` all
    /// skip the mutation — or the revision that would make undo replay one —
    /// entirely in that case, specifically so this doesn't fire for one).
    /// Does fire, unconditionally and with no identity check,
    /// for every `:messages`/`:ls`/`:plugin-status` refresh of an
    /// already-open view buffer, even a byte-identical one — a handler that
    /// resolves the buffer's path must handle `#f` (these buffers have none).
    /// Also fires, exactly once, for a buffer replaced in place under a
    /// surviving `BufferId` (`close_buffer`'s last-buffer scratch swap) —
    /// see `Buffer::announced_text_gen`'s doc.
    OnTextChanged {
        buffer: BufferId,
    },
    /// Fires after a successful `:set global`/`set-option!`/`:theme` write —
    /// `settings_ops::apply_global` is the single production path every one
    /// of those funnels through, so this is the one place to raise it.
    /// Buffer-scoped overrides (`:set`/`set-buffer-option!` without
    /// `global`) don't raise this: the payload has no `BufferId` to name,
    /// and `apply_buffer` has no per-key resync effects to piggyback on (see
    /// its doc). `value` is the raw string `:set`/`set-option!` was given,
    /// not its parsed/coerced form (`write_global` discards the parsed value
    /// after validating it) — a plugin owning one setting's policy (e.g. the
    /// LSP inlay-hints plugin reacting to `lsp.inlay-hints`) should re-read
    /// `(get-option key)` for a typed value rather than pattern-match this
    /// string.
    OnOptionChange {
        key: String,
        value: String,
    },
}

/// Pairs each `EditorEvent` variant with its Steel-facing name, once, and
/// generates both `EditorEvent::name`'s match and `EVENT_NAMES` from that one
/// list — the alternative (a hand-written match plus a hand-written const
/// array) is the exact kind of two-places-say-the-same-thing drift a test can
/// catch but not prevent. Still an explicit table of string literals, not a
/// PascalCase→kebab-case computation: writing each name out means a variant
/// rename can never silently rename the Steel-facing event too — this macro
/// only removes writing each pair twice.
///
/// A variant not listed here would make `name`'s match non-exhaustive over
/// `EditorEvent` and fail to compile — so today, with every variant
/// Steel-visible, this is equivalent to the old exhaustive match, just
/// written once. A future internal-only (Rust-only, no Steel-facing name)
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
            pub(crate) fn name(&self) -> &'static str {
                match self {
                    $(EditorEvent::$variant { .. } => $name,)+
                }
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
    OnCompletionRefilter => "on-completion-refilter",
    OnOptionChange => "on-option-change",
    OnTextChanged => "on-text-changed",
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
            | EditorEvent::OnBufferEnter { buffer }
            | EditorEvent::OnDiagnosticsChanged { buffer }
            | EditorEvent::OnTextChanged { buffer } => {
                vec![SteelBufferId::new(*buffer).into_steel_val()]
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
                end_line,
            } => {
                vec![
                    SteelBufferId::new(*buffer).into_steel_val(),
                    SteelVal::IntV(*first_line as isize),
                    SteelVal::IntV(*end_line as isize),
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
/// `Editor::settle()` in FIFO order — one merged queue closes the stranded-
/// events bug (see `tests/events.rs`'s
/// `event_raised_from_async_work_fires_on_settle_with_no_input`): a `Call`
/// and an `Event` queued in the same batch drain in insertion order, in one
/// fixpoint, instead of two queues drained at two different points of the
/// run loop.
///
/// Hooks always route through here rather than firing inline — this is a
/// semantic guarantee of the hook model ("when X happens, then do Y"), not a
/// consequence of the borrow architecture: a hook must run *after* the
/// command that triggers it completes, never mid-command. Even if re-entrancy
/// were fully solved mechanically, this stays queued. **Do not optimize hooks
/// to fire inline** — this decision is locked.
#[derive(Debug)]
pub(crate) enum PendingWork {
    /// A specific Steel closure already captured by the raise site — an
    /// `lsp-request` callback, a timer thunk, a prompt/menu/drawer/picker
    /// callback. Delivered to exactly that closure, not to every handler for
    /// a name.
    Call(SteelVal, Vec<SteelVal>),
    /// An editor event to fire by name at drain time. Args are built by
    /// `steel_args()` only if a handler is actually registered.
    Event(EditorEvent),
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

pub(crate) fn known_event_names() -> &'static [&'static str] {
    EVENT_NAMES
}

#[cfg(test)]
mod tests;
