use std::path::{Path, PathBuf};

use hume_engine::pipeline::{BufferId, PaneId};

use crate::attribution::PluginId;
use crate::types::SteelCmdDef;

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

/// A setting's effective value, typed just enough for `(get-option key)` to
/// build the right `SteelVal` — `hume-scripting` has no dependency on
/// `hume-editor`'s settings types, so the editor impl converts its own
/// per-key parser kind (`bool`/`usize`/`from_str`/…) down to one of these
/// three shapes at the trait boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum OptionValue {
    Bool(bool),
    Int(i64),
    Str(String),
}

/// "X: not supported by this host" — the single source for capability-absence
/// errors, replacing the trait-default bodies the capability-trait split removed.
pub fn unsupported(builtin: &str) -> String {
    format!("{builtin}: not supported by this host")
}

/// How an open popup reacts to key events — `show-popup!`'s `#:kind` symbol,
/// decoded once at the builtin boundary (`builtins::ui::show_popup`) and
/// carried as-is into the editor's own popup state, so there is exactly one
/// definition of the two dismiss behaviors, not a bool pair mapped to a
/// second enum on the other side of the trait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupKind {
    /// Untouched by keys; closed only by the `on-mode-change` Steel hook and
    /// the next `show-popup!`. Default — `#:kind` omitted, or `'sticky`.
    Sticky,
    /// Ctrl+u/Ctrl+d scroll the content and are consumed *when it overflows
    /// one screenful*; every other key — and Ctrl+u/d with nothing to scroll
    /// — closes the popup and falls through to normal dispatch (`#:kind
    /// 'scrollable`). Covers both scrollable hover and the dismiss-on-any-key
    /// `gn`/`gp` diagnostic overlay: the two collapse to the same behavior
    /// once content fits on screen, and a long diagnostic gets scrolling for
    /// free instead of a hard height cap.
    Scrollable,
}

/// The editor interface exposed to scripting builtins during a Steel eval, as
/// a capability directory: every domain method lives on one of the 12
/// capability traits in this module (`BufferHost`, `SettingsHost`,
/// `LanguageHost`, `CommandHost`, `CursorHost`, `UiHost`, `LspHost`,
/// `EditHost`, `DecorationHost`, `CompletionHost`, `TimerHost`,
/// `OutputHost`), reached through an accessor on this trait — `EditorHost`
/// itself declares no domain methods.
///
/// Implemented by `EditorHostImpl<'a>` in the editor crate (or `MockHost` in
/// tests). `SteelCtx` holds `host: &'a mut dyn EditorHost`; builtins call
/// `ctx.host.<accessor>().<method>(...)` rather than borrowing editor-domain
/// fields directly.
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
/// Five accessors are required — `buffers`, `settings`, `language`,
/// `commands`, `cursor` — because every host has *some* notion of them, even
/// if minimal (an empty buffer list, a rejecting command registry). The other
/// seven are optional, returning `Option<&mut dyn CapabilityTrait>`: `None`
/// means the host has no such capability, and the one call site per method
/// maps that to the same behavior the pre-split trait-default body produced
/// (a `"not supported by this host"` error, or a benign empty/no-op value).
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
    /// `(after …)` / `(cancel-timer! …)` scheduling — `None` for hosts with
    /// no timer wheel (test stubs).
    fn timers(&mut self) -> Option<&mut dyn TimerHost> {
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
}

/// Live cursor/selection reads — accessed through [`EditorHost::cursor`].
pub trait CursorHost {
    /// Line number (1-indexed) of the primary cursor in the focused buffer.
    ///
    /// Returns `None` when the focused (pane, buffer) has no seeded pane state
    /// (stale or never-focused ids).
    fn current_line_number(&self) -> Option<usize>;

    /// All selections in the focused buffer as `(anchor, head, primary)` triples —
    /// raw 0-indexed char offsets, inclusive model (anchor == head is a 1-char
    /// selection), direction preserved (anchor > head for backward selections),
    /// sorted by selection start, with exactly one triple flagged primary.
    ///
    /// Returns `None` when the focused (pane, buffer) has no seeded pane state.
    fn current_selections(&self) -> Option<Vec<(usize, usize, bool)>>;

    /// 1-indexed line number containing the 0-indexed char offset `idx` in the
    /// focused buffer.
    ///
    /// Returns `None` when the focused buffer id is stale (buffer no longer
    /// exists) or when `idx` is out of range (> `len_chars()`).
    fn char_index_to_line(&self, idx: usize) -> Option<usize>;

    /// `(symbol-under-cursor bid)` — the word at the primary cursor head,
    /// `""` on whitespace/punctuation.
    fn symbol_under_cursor(&self, bid: BufferId) -> String;

    /// `(selection-spans-full-line? bid)`.
    fn selection_spans_full_line(&self, bid: BufferId) -> bool;
}

/// Command registry queries, synchronous native dispatch, and Steel command
/// registration — accessed through [`EditorHost::commands`].
pub trait CommandHost {
    /// Returns `Ok(true)` if `name` is a native (Rust-registered) command —
    /// `Motion`, `Selection`, `Edit`, or `EditorCmd` — whose only valid `call!`
    /// args are `count` and `extend`. Returns `Ok(false)` for Steel-defined
    /// commands (`SteelBacked`, `Lazy`) that accept arbitrary positional args.
    /// Returns `Err(msg)` if the name is unknown.
    ///
    /// Read-only: never executes the command. Hosts without a registry (test
    /// stubs) return `Ok(false)` to treat all commands as Steel/forward-raw.
    fn command_is_native(&self, name: &str) -> Result<bool, String>;

    /// Execute a named native command synchronously.
    ///
    /// All four native variants (`Motion`, `Selection`, `Edit`, `EditorCmd`) apply
    /// their effect immediately; a subsequent read in the same eval sees the new
    /// state. Non-native names (`SteelBacked`, `Lazy`) return `Err` — the
    /// implementation self-guards, so the caller need not pre-check via
    /// `command_is_native` (though doing so avoids a wasted lookup).
    ///
    /// `count`: `None` means "as if no count was typed" — for `move-down`/`move-up`
    /// this selects visual-row movement instead of buffer-line movement (every other
    /// native command treats `None` the same as `Some(1)`). `parse_count_extend`
    /// decodes a Steel-side count of `0` to `None`.
    ///
    /// `register` arms `state.register_prefix` before dispatch so register-aware
    /// commands (`yank`, `delete`, `paste-after`, etc.) route to the right
    /// destination. Pass `None` when no explicit register was set.
    ///
    /// Returns `Ok(())` on success (includes `EditorCmd` errors, which are reported
    /// to the user and treated as success for the Steel caller).
    /// Returns `Err(msg)` when the name is not found or is not a native command.
    ///
    /// Valid only in command mode; gated by the caller's `cmd`-kind registration.
    fn run_command_sync(
        &mut self,
        name: &str,
        count: Option<usize>,
        extend: bool,
        register: Option<char>,
    ) -> Result<(), String>;

    /// Register a Steel command in the editor's `CommandRegistry`.
    ///
    /// Called inline from `define-command!` during init or plugin load.
    /// Overwrites a `Lazy` stub for the same name (expected path: a lazy plugin
    /// body's `define-command!` replaces the activation command stub).
    /// Returns `Err(msg)` if the name conflicts with any non-Lazy existing command.
    fn register_command(&mut self, def: SteelCmdDef) -> Result<(), String>;

    /// Remove a previously registered Steel command from the `CommandRegistry`.
    ///
    /// Called by `finish_lazy_activation` on the failure path to roll back
    /// commands that a partially-evaluated plugin body registered before erroring.
    /// No-op if the name is not present.
    fn unregister_command(&mut self, name: &str);

    /// Whether `ch` names a valid register (`0`–`9`, `k`, `c`, `b`).
    fn is_valid_register_name(&self, ch: char) -> bool;

    /// Register a `Lazy` activation stub for `name`, owned by `plugin`.
    ///
    /// Called from `declare-plugin`'s `#:commands` processing, once per
    /// accepted command name, so the editor's `CommandRegistry` is the single
    /// place a name is claimed — no separate scripting-side activation map.
    ///
    /// Returns `Err(msg)` if `name` is already claimed by any existing
    /// command (native, `SteelBacked`, or another plugin's `Lazy` stub); the
    /// message names the conflicting owner for a specific declare-time log.
    fn register_lazy_command(&mut self, name: &str, plugin: &PluginId) -> Result<(), String>;

    /// The plugin that owns `name`'s `Lazy` stub, or `None` if `name` is not a
    /// pending lazy activation entry (already activated, never declared, or a
    /// non-lazy command).
    fn lazy_command_owner(&self, name: &str) -> Option<PluginId>;

    /// Remove every remaining `Lazy` stub owned by `plugin`.
    ///
    /// Called by `finish_lazy_activation` on both the success and failure
    /// path: on success, any stub the plugin body didn't itself replace via
    /// `define-command!` is dead weight (the plugin is now `Loaded` and will
    /// never re-run its body); on failure, every stub the plugin ever claimed
    /// must be freed so a later plugin can claim the name. Never removes a
    /// `SteelBacked` command — only `Lazy` entries.
    fn unregister_lazy_stubs_of(&mut self, plugin: &PluginId);
}

/// Grammar attachment and trigger-char registration — accessed through
/// [`EditorHost::language`].
pub trait LanguageHost {
    fn attach_grammar(
        &mut self,
        name: &str,
        grammar_path: &Path,
        symbol: &str,
        highlights_path: &Path,
        injections_path: Option<&Path>,
    ) -> Result<(), String>;

    fn has_grammar(&self, language: &str) -> bool;

    /// `(register-trigger-chars! source language chars)` — registers `chars`
    /// as `OnTriggerChar`-firing chars for `(source, language)`, replacing
    /// that exact pair's previous set (a plugin's own reload doesn't
    /// accumulate duplicates; a second language attaching under the same
    /// source doesn't clobber the first's). An empty `chars` removes the
    /// entry.
    fn register_trigger_chars(&mut self, source: String, language: String, chars: Vec<char>);
}

/// Global settings, statusline config, and the Steel eval budget —
/// accessed through [`EditorHost::settings`].
pub trait SettingsHost {
    /// Init-only; only `Global` scope from scripts.
    fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String>;

    /// `(get-option key)` — the effective value of `key`: `bid`'s buffer
    /// override if one is set, else the global default. `Err` for an
    /// unknown key. Callable from any context (no init/plugin-load gate,
    /// unlike `set_global_option`).
    fn get_option(&self, key: &str, bid: BufferId) -> Result<OptionValue, String>;

    /// Init-only; the editor parses element names into `StatusElement`.
    fn configure_statusline(
        &mut self,
        left: Vec<String>,
        center: Vec<String>,
        right: Vec<String>,
    ) -> Result<(), String>;

    /// Steel eval budget in milliseconds for command / hook execution.
    fn steel_command_budget_ms(&self) -> u64;
}

/// Buffer/pane enumeration, reads, lifecycle, and viewport geometry —
/// accessed through [`EditorHost::buffers`].
pub trait BufferHost {
    /// All open buffer ids in open-order.
    fn buffer_ids(&self) -> Vec<BufferId>;
    /// All open pane ids.
    fn pane_ids(&self) -> Vec<PaneId>;

    // ── Buffer reads (None ⇒ unknown/stale id) ──────────────────────────────
    fn buffer_exists(&self, id: BufferId) -> bool;
    fn buffer_path(&self, id: BufferId) -> Option<PathBuf>;
    fn buffer_display_name(&self, id: BufferId) -> Option<String>;
    fn buffer_is_dirty(&self, id: BufferId) -> Option<bool>;
    /// Language stored on the buffer (not accounting for pending `set-buffer-language!`).
    fn buffer_stored_language(&self, id: BufferId) -> Option<String>;

    // ── Buffer lifecycle ─────────────────────────────────────────────────────
    /// Open a file at `path`, deduplicating if already open.
    /// Returns the `BufferId` (new or existing).
    fn open_buffer(&mut self, path: &Path) -> Result<BufferId, String>;
    /// Close `id`.  Returns the new live focused buffer id, or `Err` when `id`
    /// does not name an open buffer.
    fn close_buffer(&mut self, id: BufferId) -> Result<BufferId, String>;
    /// Switch the focused pane to `target`, recording a jump entry.
    fn switch_to_buffer(&mut self, current: BufferId, target: BufferId) -> Result<(), String>;

    /// Steel-side staleness token for buffer `id` (its `text_gen`, bumped by
    /// every mutation) — `None` if `id` is unknown. Not LSP-specific (any
    /// script can compare a saved value against a live read), but the LSP
    /// bridge's own `#:allow-stale` staleness check is what motivated it.
    fn buffer_generation(&self, id: BufferId) -> Option<u64>;

    /// `(viewport-range bid)` — the `(first_line . last_line)` char-line span
    /// currently visible for `id` (the focused pane's if shown there, else
    /// the first pane showing it), or `None` if `id` isn't open in any pane.
    /// Pane geometry, not LSP state — doesn't need an attached server.
    fn viewport_range(&self, id: BufferId) -> Option<(usize, usize)>;
}

/// Completion session orchestration — accessed through
/// [`EditorHost::completions`].
pub trait CompletionHost {
    /// `(completion-begin! bid items #:incomplete f)` — `items` is a list of
    /// decoded `CompletionItem` hashmaps (JSON already converted by the
    /// caller). Starting a session replaces any session already open.
    fn completion_begin(
        &mut self,
        bid: BufferId,
        items: Vec<serde_json::Value>,
        incomplete: bool,
    ) -> Result<(), String>;

    /// `(completion-update-filter! text)` — re-ranks the open session
    /// against `text`; Rust-side work only, safe to call every keystroke.
    fn completion_update_filter(&mut self, text: String) -> Result<(), String>;

    /// `(completion-top n)` — up to `n` ranked items as hashmaps, `[]` with
    /// no open session.
    fn completion_top(&self, n: usize) -> Vec<serde_json::Value>;

    /// `(completion-accept! idx)` — applies `idx`'s item (an index into the
    /// ranked/filtered list, not the raw response order) and ends the
    /// session, success or failure.
    fn completion_accept(&mut self, idx: usize) -> Result<(), String>;

    /// `(completion-dismiss!)` — clears any open session; no-op if none.
    fn completion_dismiss(&mut self);
}

/// Inlay hints, signs, virtual lines, extra highlights, inline diagnostics,
/// and the diagnostic pull/count reads — accessed through
/// [`EditorHost::decorations`].
pub trait DecorationHost {
    /// `(set-inlay-hints! bid hints)` — replaces `bid`'s inlay hints
    /// wholesale. Each entry is `(wire_position, text, before)`; the wire
    /// position (raw decoded `{"line" "character"}`) is converted to a char
    /// offset using `bid`'s attached server's negotiated encoding.
    fn set_inlay_hints(&mut self, bid: BufferId, hints: Vec<(serde_json::Value, String, bool)>);

    /// `(set-signs! source bid signs)` — replaces `source`'s signs for `bid`
    /// wholesale. Each entry is `(line, text, scope, priority)`.
    fn set_signs(
        &mut self,
        source: String,
        bid: BufferId,
        signs: Vec<(usize, String, String, i64)>,
    );

    /// `(set-virtual-lines! source bid lines)` — replaces `source`'s virtual
    /// lines for `bid` wholesale. Each entry is `(line, text)` or `(line
    /// text scope)` — `scope` styles the whole line (`ui.virtual` fallback
    /// when absent).
    fn set_virtual_lines(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<(usize, String, Option<String>)>,
    );

    /// `(set-extra-highlights! source bid spans)` — replaces `source`'s
    /// extra highlights for `bid` wholesale. Each entry is `(start, end,
    /// scope)`, char offsets.
    fn set_extra_highlights(
        &mut self,
        source: String,
        bid: BufferId,
        spans: Vec<(usize, usize, String)>,
    );

    /// `(set-inline-diagnostics! bid lines)` — replaces `bid`'s inline
    /// diagnostic text wholesale (one owner, the diagnostics plugin — no
    /// `source` multiplexing, unlike `set_virtual_lines`). Each entry is
    /// `(line, text, scope)`; `text` is spliced in at the end of `line`.
    fn set_inline_diagnostics(&mut self, bid: BufferId, lines: Vec<(usize, String, String)>);

    /// `(diagnostics-for-buffer bid #:severity floor #:range (start end))` —
    /// decoded `{"start" "end" "line" "col" "severity" "message" "code"
    /// "source"}` hashmaps, filtered then capped at 1000. `severity_floor`
    /// is `None` for "no floor" (everything); `range` is `None` for the
    /// whole buffer. `Err` on an unknown `#:severity` name.
    fn diagnostics_for_buffer(
        &self,
        bid: BufferId,
        severity_floor: Option<&str>,
        range: Option<(usize, usize)>,
    ) -> Result<Vec<serde_json::Value>, String>;

    /// `(diagnostic-counts bid)` → `(errors . warnings)`.
    fn diagnostic_counts(&self, bid: BufferId) -> (usize, usize);
}

/// LSP server introspection — accessed through [`EditorHost::lsp`].
pub trait LspHost {
    /// Decoded `ServerCapabilities` for `server` (a registered language name,
    /// or `None` for the focused buffer's attached server) — `None` if
    /// unresolvable or the server hasn't finished its handshake yet.
    fn lsp_capabilities(&self, server: Option<&str>) -> Option<serde_json::Value>;

    /// One entry per running (language, root) server.
    fn lsp_server_status(&self) -> Vec<crate::types::LspServerStatusEntry>;

    /// The registered language for the server attached to buffer `id`, or
    /// `None` if `id` is unknown or has no attached server.
    fn lsp_server_for_buffer(&self, id: BufferId) -> Option<String>;

    /// Whether `language` currently has a `register-lsp-server!` config
    /// (registered, not necessarily attached/running) — used by the
    /// `on-language-set` missing-server hint to distinguish "not installed"
    /// from "still starting". Reports state *as of the last completed
    /// drain* — the `lsp-registered-for-language?` builtin overlays this
    /// with the `Effect::LspServerOp` entries queued this eval/init before
    /// falling back here, so same-eval visibility is handled at the builtin
    /// layer, not this trait method.
    fn lsp_registered_for_language(&self, language: &str) -> bool;

    /// Ready-made `{"textDocument" {"uri"} "position" {"line" "character"}}`
    /// params for `id`'s primary cursor head, in its attached server's
    /// negotiated encoding — `None` if `id` has no path, no attached server,
    /// or isn't currently shown in any pane.
    fn lsp_position_params(&self, id: BufferId) -> Option<serde_json::Value>;

    /// Same as [`lsp_position_params`](Self::lsp_position_params) but a
    /// `{"textDocument" {"uri"} "range" {"start" "end"}}` shape from the
    /// primary selection.
    fn lsp_range_params(&self, id: BufferId) -> Option<serde_json::Value>;
}

/// Timer scheduling — accessed through [`EditorHost::timers`].
pub trait TimerHost {
    /// Schedules `thunk` — opaque to this trait, a raw Steel closure — to
    /// fire after `ms` milliseconds. Returns the new timer id, or `None` if
    /// this host has no timer wheel to schedule onto right now (`EditorHostImpl`
    /// only carries one at three call sites — command dispatch, hook fire,
    /// queued-call drain — not during init).
    fn schedule_timer(&mut self, ms: u64, thunk: steel::rvals::SteelVal) -> Option<u64>;

    /// Cancels a previously scheduled timer. A no-op if `id` already fired,
    /// was already cancelled, or this host has no timer wheel right now.
    fn cancel_timer(&mut self, id: u64);
}

/// Cursor-anchored popup, selection menu, bottom drawer, and minibuffer
/// prompt — accessed through [`EditorHost::ui`]. `None` from that accessor
/// means "no UI surface to drive" (test stubs); every method here is
/// required once a host does provide `UiHost`.
pub trait UiHost {
    /// `(prompt! label #:prefill text on-confirm)` — opens a one-shot
    /// Command-mode minibuffer session. `callback` fires exactly once, with
    /// the confirmed text or `#f` on cancel — queued through the same
    /// drained-at-frame-boundary path as every other Rust→Steel call, never
    /// invoked inline. Errors if a minibuffer session is already open.
    fn prompt(
        &mut self,
        label: String,
        prefill: String,
        callback: steel::rvals::SteelVal,
    ) -> Result<(), String>;

    /// `(show-popup! text #:anchor 'cursor #:kind 'sticky #:lang #f)` — shows
    /// `text` in a popup panel. Geometry (wrap width, flip/clamp position, or
    /// the docked band's size) is resolved fresh every frame by the host, not
    /// here — this just stores the raw content. Replaces any popup already
    /// showing (no stacking).
    ///
    /// `kind`: see [`PopupKind`] for the two dismiss behaviors. `docked`:
    /// `#:anchor 'bottom` — renders as a full-width chrome band directly
    /// above the statusline (reserving pane space, like the drawer) instead
    /// of floating near the cursor. `lang`: when `Some(name)` and a grammar
    /// named `name` is registered, `text` is syntax-highlighted like a real
    /// buffer; otherwise (no grammar by that name, or `None`) it renders as
    /// plain text.
    fn show_popup(
        &mut self,
        text: String,
        kind: PopupKind,
        docked: bool,
        lang: Option<String>,
    ) -> Result<(), String>;

    /// `(close-popup!)` — dismisses the popup. Idempotent: closing when none
    /// is showing is not an error (only an unsupported *host* errors).
    fn close_popup(&mut self) -> Result<(), String>;

    /// `(show-menu! items on-select)` — opens a selection menu near the
    /// cursor. `on-select` fires exactly once: the chosen index, or `#f` on
    /// dismissal — queued, never invoked inline. Replaces any menu already
    /// open (no stacking). Hosts should reject this from Insert mode — a
    /// menu that can't be driven is worse than no menu (note: a command
    /// triggered via `:name` still runs with the *previous* mode active, so
    /// this must be an Insert-specific rejection, not a Normal/Extend-only
    /// allowlist).
    fn show_menu(
        &mut self,
        items: Vec<String>,
        callback: steel::rvals::SteelVal,
    ) -> Result<(), String>;

    /// `(close-menu!)` — dismisses the menu *without* invoking its callback
    /// (caller-initiated close, distinct from the key-driven dismissal paths
    /// which do call back with `#f`).
    fn close_menu(&mut self) -> Result<(), String>;

    /// `(show-drawer-list! items on-select)` — opens a scrolling pick-list
    /// in the bottom chrome band. `items` are pre-formatted display strings;
    /// the drawer never interprets their content — the jump (if any) is the
    /// caller's job, typically `(goto-location! ...)` inside `on-select`.
    /// `on-select` receives the chosen index and, unlike the popup/menu's
    /// one-shot callback, may fire more than once: the drawer stays open
    /// across `Enter` (Helix-style browse) until `Esc` or `close-drawer!`.
    /// Replaces any drawer already open (no stacking).
    fn show_drawer_list(
        &mut self,
        items: Vec<String>,
        callback: steel::rvals::SteelVal,
    ) -> Result<(), String>;

    /// `(close-drawer!)` — dismisses the drawer *without* invoking its
    /// callback (caller-initiated close, distinct from `Esc`, which does
    /// call back with `#f`).
    fn close_drawer(&mut self) -> Result<(), String>;

    /// `(picker! items on-select #:prompt "…")` — opens the fuzzy-finder
    /// panel (`docs/FUZZY-FINDERS.md`). `items` are `(display . payload)`
    /// pairs; `payload` is handed back to `on-select` verbatim, never
    /// interpreted by Rust. Returns a token that scopes later
    /// `picker-push!` calls to this session. Unlike the menu/drawer, the
    /// picker is allowed from any mode (Q-B7) but closes any live
    /// completion session first, since only one modal owner may be active
    /// at a time. `on-select` fires exactly once, queued (never invoked
    /// inline): the selected payload on `Enter`, or `#f` on `Esc`,
    /// `picker-close!`, or being replaced by a second `picker!` call.
    fn open_picker(
        &mut self,
        items: Vec<(String, steel::rvals::SteelVal)>,
        prompt: String,
        on_select: steel::rvals::SteelVal,
    ) -> Result<u64, String>;

    /// `(picker-push! token items)` — appends `items` to the open picker's
    /// store and reranks, but only if `token` matches the session the
    /// caller opened (returned by `open_picker`). A stale token — the
    /// picker was closed or replaced since — is expected-normal for an
    /// async source racing the user, so it is a silent no-op, not an error:
    /// returns whether the push was applied.
    fn picker_push(&mut self, token: u64, items: Vec<(String, steel::rvals::SteelVal)>) -> bool;

    /// `(picker-source-spawn! token cmd args #:cwd dir #:nul flag)` —
    /// attaches a streaming external-command source to the open picker
    /// (direct argv spawn, no shell). Its stdout lines flow directly into
    /// the store, never through Steel. Replaces (killing) any source
    /// already attached to the same session — a second spawn is a
    /// re-spawn, not a second concurrent source.
    ///
    /// `Ok(false)` — same "expected-normal race, not an error" contract as
    /// `picker_push` — means a stale token or no open picker; nothing was
    /// spawned. `Err` means the process itself failed to spawn (missing
    /// binary, bad `#:cwd`).
    fn picker_source_spawn(
        &mut self,
        token: u64,
        cmd: &str,
        args: Vec<String>,
        cwd: Option<std::path::PathBuf>,
        nul: bool,
    ) -> Result<bool, String>;

    /// `(picker-close!)` — ends the open picker, if any, firing its
    /// `on-select` with `#f` (unlike `close-menu!`/`close-drawer!`, which
    /// drop the callback without invoking it — the picker's callback
    /// lifecycle guarantees exactly one fire per session no matter how it
    /// ends). Idempotent: closing when none is open is not an error.
    fn picker_close(&mut self);
}

/// LSP-driven text edits, workspace edits, and go-to-location — accessed
/// through [`EditorHost::edits`].
pub trait EditHost {
    /// `(apply-text-edits! bid edits #:expect-generation gen)` — `edits` is
    /// `(start_line, start_char, end_line, end_char, new_text)` tuples in
    /// wire coordinates. Applied as one undo step.
    fn apply_text_edits(
        &mut self,
        bid: BufferId,
        edits: Vec<(usize, usize, usize, usize, String)>,
        expect_gen: Option<u64>,
    ) -> Result<(), String>;

    /// `(apply-workspace-edit! edit)` — `edit` is a decoded LSP
    /// `WorkspaceEdit` JSON blob. Returns the number of buffers modified.
    fn apply_workspace_edit(&mut self, edit: serde_json::Value) -> Result<usize, String>;

    /// `(goto-location! target)`, raw `Location`/`LocationLink` shape —
    /// `uri` a wire URI string, `line`/`character` wire coordinates.
    fn goto_location_wire(
        &mut self,
        uri: String,
        line: usize,
        character: usize,
    ) -> Result<(), String>;

    /// `(goto-location! target)`, `(list target line col)` shape with a
    /// path or `file://` URI string target — already char-indexed.
    fn goto_location_path(
        &mut self,
        path_or_uri: String,
        line: usize,
        col: usize,
    ) -> Result<(), String>;

    /// `(goto-location! target)`, `(list target line col)` shape with a
    /// `bid` target — already char-indexed.
    fn goto_location_buffer(
        &mut self,
        bid: BufferId,
        line: usize,
        col: usize,
    ) -> Result<(), String>;
}

/// Terminal-safety state around `#:inline-output` commands — accessed
/// through [`EditorHost::output`].
pub trait OutputHost {
    /// True while the command currently being dispatched is `#:inline-output`
    /// (raw stdout writes are safe — either the alt-screen has been left for
    /// the duration of its body, or there is no TUI to protect at all).
    fn is_inline_output_command(&self) -> bool;

    /// Called by a builtin just before it writes its first byte of terminal
    /// output (`displayln`, a subprocess with inherited stdio, …). Enters the
    /// inline-output alt-screen bracket lazily — on the first real output,
    /// not eagerly at dispatch — so a command whose body only logs never
    /// flashes an empty screen or blocks on an unnecessary keypress. Safe to
    /// call more than once per command body: only the first call (per
    /// dispatch) does anything.
    fn ensure_inline_output_screen(&mut self) -> Result<(), String>;
}
