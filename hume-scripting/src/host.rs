//! The editor interface exposed to scripting builtins, as a capability
//! directory (`EditorHost`) whose domain methods live one-per-file below,
//! one module per accessor. Each child is private and re-exported here so
//! every `hume_scripting::host::X` path a caller uses keeps resolving
//! unchanged, and no second public path (`host::ui::UiHost`) is minted
//! alongside it.

// ── Capability modules ────────────────────────────────────────────────────────
mod async_process;
mod buffers;
mod commands;
mod completion;
mod cursor;
mod decorations;
mod diff;
mod edits;
mod events;
mod language;
mod lsp;
mod output;
mod registers;
mod settings;
mod timers;
mod ui;

pub use async_process::AsyncProcessHost;
pub use buffers::BufferHost;
pub use commands::CommandHost;
pub use completion::{CompletionHost, Interaction, MatchKind};
pub use cursor::CursorHost;
pub use decorations::DecorationHost;
pub use diff::{DiffHost, DiffHunk, WordDiffHunk};
pub use edits::{EditHost, WireTextEdit};
pub use events::EventHost;
pub use language::LanguageHost;
pub use lsp::{LocationDisplay, LspHost};
pub use output::OutputHost;
pub use registers::RegisterHost;
pub use settings::{OptionValue, SettingsHost};
pub use timers::TimerHost;
pub use ui::{LivePickerOpts, PickerFeedMode, PickerOpts, PickerSourceOpts, PopupKind, UiHost};

/// Key-binding mode, as recognised by `bind-key!`/`unbind-key!`.
///
/// Defined here (scripting layer) so builtins do not depend on the editor's
/// internal `crate::editor::keymap::BindMode`.  Travels to the editor inside
/// [`crate::Effect::BindKey`]/[`BindWaitChar`](crate::Effect::BindWaitChar)/
/// [`UnbindKey`](crate::Effect::UnbindKey), which maps it to the editor's own
/// `BindMode` as it applies them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindMode {
    Normal,
    Extend,
    Insert,
}

/// "X: not supported by this host" — the single source for capability-absence
/// errors.
pub(crate) fn unsupported(builtin: &str) -> String {
    format!("{builtin}: not supported by this host")
}

/// The editor interface exposed to scripting builtins during a Steel eval, as
/// a capability directory: every domain method lives on one of the capability
/// traits in this module (`BufferHost`, `SettingsHost`, `LanguageHost`,
/// `CommandHost`, `CursorHost`, `EventHost`, `UiHost`, `LspHost`, `EditHost`,
/// `DecorationHost`, `CompletionHost`, `TimerHost`, `AsyncProcessHost`,
/// `OutputHost`, `DiffHost`), reached through an accessor on this trait —
/// `EditorHost` itself declares no domain methods.
///
/// Implemented by `EditorHostImpl<'a>` in the editor crate (or `MockHost` in
/// tests). Builtins call `ctx.host.<accessor>().<method>(...)` rather than
/// borrowing editor-domain fields directly.
///
/// All methods (on `EditorHost` and every capability trait) take/return only
/// `'static` types (owned `String`/`PathBuf`/`Vec`, `Copy` ids, scripting-owned
/// enums), since `SteelCtx<'static>` is the type projection Steel's
/// `with_mut_reference` requires.
///
/// `BufferHost` methods (`open_buffer`, `close_buffer`, `switch_to_buffer`,
/// reads/enumeration) are command-mode only, gated per-builtin by the `cmd`
/// kind in `builtins!`'s registration table (`errors::require_cmd`);
/// init-only methods (`SettingsHost::set_global_option`,
/// `SettingsHost::configure_statusline`) use the `config` kind
/// (`errors::require_config`), the reverse guard.
///
/// Focused buffer/pane ids are passed as explicit constructor args to
/// `call_steel_cmd`/`fire_hook` rather than queried through this trait, so a
/// builtin always sees the pre-command snapshot, not a value that can change
/// mid-eval (e.g. after `switch-to-buffer!`).
///
/// Six accessors are required — `buffers`, `settings`, `language`,
/// `commands`, `cursor`, `events` — since every host has *some* notion of
/// them, even if minimal (an empty buffer list, a rejecting command
/// registry). The rest are optional (`Option<&mut dyn CapabilityTrait>`):
/// `None` means the host has no such capability. A mutating builtin maps
/// `None` to the `"not supported by this host"` error via `errors::require_cap` —
/// silently discarding the write would report success for a mutation that
/// never happened. A silent no-op is reserved for calls whose own contract
/// is already idempotent regardless of host support (e.g.
/// `cancel-timer!`/`cancel-async!` on an id that was never scheduled).
///
/// The trait exists — rather than builtins reaching into `EditorState`
/// directly — for two reasons: the crate cycle `hume-editor → hume-scripting
/// → {hume-engine, hume-platform}` is a hard wall, so dissolving it would mean
/// moving `EditorState` into a crate below `hume-scripting`, re-layering most
/// of the editor; and it keeps scripting tests mockable (`NullHost`,
/// `MockHost`) behind a curated API boundary instead of the full state surface.
pub trait EditorHost {
    // ── Optional capability accessors ────────────────────────────────────────
    /// Cursor-anchored popup / selection menu / bottom drawer / minibuffer
    /// prompt — `None` for hosts with no UI surface to drive (test stubs).
    fn ui(&mut self) -> Option<&mut dyn UiHost> {
        None
    }
    /// LSP-driven text edits, workspace edits, and go-to-location — `None`
    /// for hosts with no editable buffers/panes to route them to.
    fn edits(&mut self) -> Option<&mut dyn EditHost> {
        None
    }
    /// Completion session orchestration — `None` for hosts with no
    /// completion popup to drive.
    fn completions(&mut self) -> Option<&mut dyn CompletionHost> {
        None
    }
    /// Inlay hints / signs / virtual lines / extra highlights / inline
    /// diagnostics / diagnostic pull — `None` for hosts with no decoration
    /// stores to write into.
    fn decorations(&mut self) -> Option<&mut dyn DecorationHost> {
        None
    }
    /// LSP server introspection (capabilities, status, attachment,
    /// registration, position/range params) — `None` for hosts with no LSP
    /// bridge.
    fn lsp(&mut self) -> Option<&mut dyn LspHost> {
        None
    }
    /// Register content reads/writes (`read-register`/`write-register!`) —
    /// `None` for hosts with no register store (test stubs).
    fn registers(&mut self) -> Option<&mut dyn RegisterHost> {
        None
    }
    /// `(after …)` / `(cancel-timer! …)` scheduling — `None` for hosts with
    /// no timer wheel (test stubs).
    fn timers(&mut self) -> Option<&mut dyn TimerHost> {
        None
    }
    /// `(spawn-async! …)` / `(cancel-async! …)` — `None` for hosts with no
    /// job registry to spawn onto (test stubs).
    fn async_process(&mut self) -> Option<&mut dyn AsyncProcessHost> {
        None
    }
    /// `(diff-lines …)` / `(diff-buffer-lines …)` — `None` for hosts with no
    /// text-diffing backend (test stubs).
    fn diff(&mut self) -> Option<&mut dyn DiffHost> {
        None
    }
    /// Terminal-safety state around `#:inline-output` commands — `None` for
    /// hosts with no live TUI to protect (test stubs): `is_inline_output_command`
    /// reads false and `ensure_inline_output_screen` is a no-op success.
    fn output(&mut self) -> Option<&mut dyn OutputHost> {
        None
    }
    /// Live cursor/selection reads — required: every host has some notion
    /// (even if only "nothing is focused") of the focused buffer's cursor.
    fn cursor(&mut self) -> &mut dyn CursorHost;
    /// Command registry queries, synchronous native dispatch, and
    /// registration — required: every host has some notion of its command
    /// set, even if empty.
    fn commands(&mut self) -> &mut dyn CommandHost;
    /// Grammar attachment and trigger-char registration — required: every
    /// host has some notion (even if empty) of its language/grammar set.
    fn language(&mut self) -> &mut dyn LanguageHost;
    /// Global settings, statusline config, and the Steel eval budget —
    /// required: every host has some notion (even if minimal defaults) of
    /// its settings.
    fn settings(&mut self) -> &mut dyn SettingsHost;
    /// Buffer/pane enumeration, reads, lifecycle, and viewport geometry —
    /// required: every host has some notion (even if empty) of open buffers.
    fn buffers(&mut self) -> &mut dyn BufferHost;
    /// Event-name introspection — required: every host has some notion (even
    /// if empty) of which event names it can raise.
    fn events(&mut self) -> &mut dyn EventHost;
}
