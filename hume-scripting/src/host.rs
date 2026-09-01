use std::ops::Range;
use std::path::{Path, PathBuf};

use hume_engine::pipeline::{BufferId, PaneId};

use crate::attribution::PluginId;
use crate::types::{SteelCmdDef, VirtualLineSpec};

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionValue {
    Bool(bool),
    Int(i64),
    Str(String),
}

/// "X: not supported by this host" — the single source for capability-absence
/// errors.
pub(crate) fn unsupported(builtin: &str) -> String {
    format!("{builtin}: not supported by this host")
}

/// How an open popup reacts to key and mouse input — `show-popup!`'s
/// `#:kind` symbol, decoded once at the builtin boundary
/// (`builtins::ui::show_popup`) and carried as-is into the editor's own
/// popup state, so there is exactly one definition of the two dismiss
/// behaviors, not a bool pair mapped to a second enum on the other side of
/// the trait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupKind {
    /// Untouched by keys and mouse input alike; closed only by the
    /// `on-mode-change` Steel hook and the next `show-popup!`. Default —
    /// `#:kind` omitted, or `'sticky`.
    Sticky,
    /// Ctrl+u/Ctrl+d scroll the content and are consumed *when it overflows
    /// one screenful*; every other key or mouse event — and Ctrl+u/d with
    /// nothing to scroll — closes the popup and falls through to normal
    /// dispatch (`#:kind 'scrollable`). Covers both scrollable hover and the
    /// dismiss-on-any-key `gn`/`gp` diagnostic overlay: the two collapse to
    /// the same behavior once content fits on screen, and a long diagnostic
    /// gets scrolling for free instead of a hard height cap.
    Scrollable,
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

/// Event-name introspection — accessed through [`EditorHost::events`].
///
/// The name-based boundary this crate is built on: `hume-scripting` has no
/// compiled-in knowledge of which event names exist (that's the editor's
/// `EditorEvent`), so `register-hook!` and `declare-plugin`'s `#:events`
/// validate against this instead of a static match.
pub trait EventHost {
    /// Every Steel-visible event name this host can raise.
    fn known_event_names(&self) -> &'static [&'static str];
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

    /// `(symbol-under-cursor bid)` — the word at the primary cursor head in
    /// the pane currently showing `bid`, `""` on whitespace/punctuation or
    /// when `bid` isn't shown in any pane.
    fn symbol_under_cursor(&self, bid: BufferId) -> String;

    /// `(selections-linewise? bid)` — every *unambiguous* selection in
    /// `bid`'s state, as seen in the pane currently showing it, is linewise
    /// (spans whole lines, anchor to trailing `\n`). A selection collapsed
    /// onto a single empty line is ambiguous (see
    /// `hume_editing::selection::linewise_classification`) and carries no
    /// vote either way. `false` when every selection is ambiguous, matching
    /// an ordinary collapsed cursor's default, and `false` when `bid` isn't
    /// shown in any pane.
    ///
    /// Paired with [`Self::selections_charwise`] to express `:lsp-fmt`'s
    /// three-way verdict (all linewise / none linewise / mixed) as two
    /// booleans rather than a symbol on `lsp-linewise-ranges-params`'s wire
    /// params — every other `lsp-*-params` builtin returns a hash forwarded
    /// to `lsp-request` verbatim or with a *protocol* key inserted, and a
    /// non-protocol verdict key would break that. `(false, false)` from the
    /// pair means *mixed or not-shown*; an all-ambiguous set answers
    /// `(false, true)`, deliberately indistinguishable from all-charwise,
    /// which is the default it's meant to take.
    fn selections_linewise(&self, bid: BufferId) -> bool;

    /// `(selections-charwise? bid)` — no *unambiguous* selection in `bid`'s
    /// state, as seen in the pane currently showing it, is linewise. `true`
    /// when every selection is ambiguous (the complementary default to
    /// [`Self::selections_linewise`]'s `false` in that same case). `false`
    /// when `bid` isn't shown in any pane. See [`Self::selections_linewise`]
    /// for why this is a second predicate rather than a verdict field
    /// elsewhere.
    fn selections_charwise(&self, bid: BufferId) -> bool;
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

/// Register content reads/writes — accessed through [`EditorHost::registers`].
///
/// A register holds one string per selection captured at yank time. Macro
/// registers (recorded key sequences, not text) are out of scope: there is no
/// wire format yet for handing a `Vec<KeyEvent>` to Scheme, so [`Self::read_register`]
/// answers `None` for one, exactly as it would for an empty register.
pub trait RegisterHost {
    /// Contents of `name` as one string per selection, or `None` when the
    /// register is empty, is the black hole, or holds a macro.
    ///
    /// `&mut self` because the clipboard register (`'c'`) may need to read
    /// the live OS clipboard.
    fn read_register(&mut self, name: char) -> Option<Vec<String>>;

    /// Store `values` in register `name`. Routes exactly like a keyboard
    /// write: `'k'` pushes onto the kill ring (stamped for smart-paste, same
    /// as `"ky`), `'c'` mirrors to the OS clipboard, `'b'` discards silently.
    fn write_register(&mut self, name: char, values: Vec<String>);
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
    /// `(set-option! key value)` — only `Global` scope from scripts. No
    /// eval-mode gate (`open` kind): callable from `init.scm`, plugin load,
    /// plugin activation, or a plain command/hook body — the write already
    /// goes through the editor's validating chokepoint regardless of caller.
    fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String>;

    /// `(set-buffer-option! bid key value)` — writes `key`'s per-buffer
    /// override on `bid`. Command/hook context (`cmd` kind), unlike
    /// `set_global_option`. `Err` for a stale `bid`, a global-only key, or a
    /// bad value.
    fn set_buffer_option(&mut self, key: &str, value: &str, bid: BufferId) -> Result<(), String>;

    /// `(get-option [bid] key)` — the effective value of `key`:
    /// `bid`'s buffer override if one is set, else the global default. `Err`
    /// for an unknown key. No eval-mode gate (`open` kind): callable from
    /// `init.scm` too — a stale or default `bid` degrades gracefully to the
    /// global default rather than erroring.
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
    /// Fully display-ready path string (absolutized, lexically normalized,
    /// UNC-stripped, `~`-collapsed) — print verbatim. `None` for scratch/synthetic
    /// buffers, same as `buffer_path`.
    fn buffer_display_path(&self, id: BufferId) -> Option<String>;
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

    /// `(buffer-text bid)` — the buffer's full live (dirty) in-memory
    /// content, always ending with the structural trailing `\n`. `None` if
    /// `id` is unknown.
    fn buffer_text(&self, id: BufferId) -> Option<String>;

    /// Number of *content* lines in `id`'s live text — every HUME buffer
    /// ends with a structural `\n`, which ropey counts as one extra empty
    /// line (see [`hume_engine::pipeline`] invariants); this excludes that
    /// phantom line, matching what the statusline and `:w` report. `None`
    /// if `id` is unknown.
    fn buffer_line_count(&self, id: BufferId) -> Option<usize>;

    /// Content lines `range` (0-based, end-exclusive) of `id`'s live text,
    /// each with its trailing line break stripped. `range` is caller-
    /// validated against [`buffer_line_count`](Self::buffer_line_count) —
    /// this call itself does not clamp or bounds-check, and an out-of-range
    /// `range` is a caller bug: implementations may panic rather than
    /// return `None` (the editor implementation does, via the underlying
    /// rope's line lookup). `None` if `id` is unknown.
    fn buffer_lines(&self, id: BufferId, range: Range<usize>) -> Option<Vec<String>>;

    /// The 0-based char offset where content `line` (0-based) starts in
    /// `id`'s live text. `line` is caller-validated against
    /// [`buffer_line_count`](Self::buffer_line_count) — same contract as
    /// [`buffer_lines`](Self::buffer_lines): this call itself does not
    /// bounds-check, and an out-of-range `line` is a caller bug (the editor
    /// implementation panics, via the underlying rope's line lookup). `None`
    /// if `id` is unknown.
    ///
    /// Backs the Steel `(line->offset bid line)` builtin — the inverse
    /// direction of `char-index->line`, but not a drop-in inverse of it:
    /// `char-index->line` is 1-indexed and reads the focused buffer, this is
    /// 0-indexed and takes an explicit `id`.
    fn line_to_offset(&self, id: BufferId, line: usize) -> Option<usize>;

    /// The line range (0-based, end-exclusive) currently visible for `id`
    /// (the focused pane's if shown there, else the first pane showing it),
    /// or `None` if `id` isn't open in any pane. Backs the Steel
    /// `(viewport-range bid)` builtin. Pane geometry, not LSP state —
    /// doesn't need an attached server.
    fn viewport_range(&self, id: BufferId) -> Option<Range<usize>>;
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

/// Inlay hints, signs, virtual lines, extra highlights, EOL text, statusline
/// text, and the diagnostic pull/count reads — accessed through
/// [`EditorHost::decorations`].
pub trait DecorationHost {
    /// `(set-inlay-hints! source bid hints)` — replaces `source`'s inlay
    /// hints for `bid` wholesale. Each entry is `(offset, text, before)`,
    /// `offset` already a char offset — the Steel builtin no longer accepts
    /// LSP wire positions directly (see `lsp-position->offset`).
    fn set_inlay_hints(
        &mut self,
        source: String,
        bid: BufferId,
        hints: Vec<(usize, String, bool)>,
    ) -> Result<(), String>;

    /// `(register-sign-source! name bid priority)` — declares `name` a sign
    /// channel at `priority` *for `bid`*, replacing any prior registration
    /// under that name in that buffer (last wins, matching
    /// `register-lsp-server!`). Its gutter slot is its rank among every
    /// source registered for that same buffer, not a property of any one
    /// `set_signs` call, and not shared with any other buffer — a source
    /// claims its slot the first time it becomes relevant to a given
    /// buffer and holds it for that buffer's life; there is no withdrawal.
    fn register_sign_source(
        &mut self,
        name: String,
        bid: BufferId,
        priority: i64,
    ) -> Result<(), String>;

    /// `(set-signs! source bid signs)` — replaces `source`'s signs for `bid`
    /// wholesale. Each entry is `(line, text, scope)`; `line` converts to
    /// that line's line-start char offset at this boundary — `Err`, naming
    /// the builtin, if `line` is out of range or `source` isn't registered
    /// for `bid`.
    fn set_signs(
        &mut self,
        source: String,
        bid: BufferId,
        signs: Vec<(usize, String, String)>,
    ) -> Result<(), String>;

    /// `(set-virtual-lines! source bid lines)` — replaces `source`'s virtual
    /// lines for `bid` wholesale. Each `VirtualLineSpec`'s `segments` are
    /// **unvalidated** char ranges (the Steel boundary only decodes shape,
    /// see `VirtualLineSpec`'s doc) — this method is the sole enforcement
    /// point: it must sort, validate (bounds, ordering, non-overlap,
    /// grapheme-cluster alignment against `text`), and convert to byte
    /// offsets, `Err`ing with a message naming `set-virtual-lines!` on any
    /// violation rather than storing bad data.
    fn set_virtual_lines(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<VirtualLineSpec>,
    ) -> Result<(), String>;

    /// `(set-extra-highlights! source bid spans)` — replaces `source`'s
    /// extra highlights for `bid` wholesale. Each entry is `(start, end,
    /// scope)`, char offsets — `Err`, naming the builtin, if the range is
    /// empty or out of bounds.
    fn set_extra_highlights(
        &mut self,
        source: String,
        bid: BufferId,
        spans: Vec<(usize, usize, String)>,
    ) -> Result<(), String>;

    /// `(set-eol-text! source bid lines)` — replaces `source`'s EOL text for
    /// `bid` wholesale. Each entry is `(line, text, scope)`; `text` is
    /// spliced in at the end of `line`, which converts to that line's
    /// line-start char offset at this boundary — `Err`, naming the builtin,
    /// if `line` is out of range. Not diagnostics-specific — the diagnostics
    /// plugin is its first client, not its owner.
    fn set_eol_text(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<(usize, String, String)>,
    ) -> Result<(), String>;

    /// `(set-line-backgrounds! source bid entries)` — replaces `source`'s
    /// line backgrounds for `bid` wholesale. Each entry is `(line, scope)`;
    /// `line` converts to that line's line-start char offset at this
    /// boundary — `Err`, naming the builtin, if `line` is out of range.
    fn set_line_backgrounds(
        &mut self,
        source: String,
        bid: BufferId,
        entries: Vec<(usize, String)>,
    ) -> Result<(), String>;

    /// `(set-statusline-text! source bid text)` — replaces `source`'s
    /// statusline text for `bid` wholesale; an empty `text` clears it.
    /// Rendered by the `steel:<source>` statusline element, reading only the
    /// focused buffer's entry — a `bid` that isn't focused simply isn't
    /// shown, not an error. `Err` for an unknown `bid`.
    fn set_statusline_text(
        &mut self,
        source: String,
        bid: BufferId,
        text: String,
    ) -> Result<(), String>;

    /// `(diagnostics-for-buffer bid #:severity floor #:range (start end))` —
    /// decoded `{"start" "end" "line" "char-col" "grapheme-col" "severity"
    /// "message" "code" "source"}` hashmaps, filtered then capped at 1000.
    /// `char-col` is an addressing unit (feeds `goto-location!`);
    /// `grapheme-col` is the display unit (the one every HUME surface shows
    /// the user) — never render `char-col` directly. `severity_floor`
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
    /// primary selection alone.
    fn lsp_primary_range_params(&self, id: BufferId) -> Option<serde_json::Value>;

    /// `{"textDocument" {"uri"} "ranges" [...]}` — one wire range per
    /// *linewise* selection in `id`'s buffer, coalescing any run of
    /// selections that touch end-to-end into a single range (an LSP range
    /// is naturally contiguous, so a touching run needs no splitting). A
    /// non-linewise selection is skipped, not an error — `ranges` is simply
    /// empty when none of `id`'s selections are linewise. `None` (as
    /// opposed to an empty `ranges`) only for the same reasons
    /// `lsp_primary_range_params` returns `None`: no path, no attached
    /// server, or the buffer isn't shown in any pane.
    fn lsp_linewise_ranges_params(&self, id: BufferId) -> Option<serde_json::Value>;

    /// Wire `(line, character)` → char offset in `id`'s attached server's
    /// negotiated encoding — backs `lsp-range->offsets`. `None` if `id` is
    /// unknown or has no attached server (no negotiated encoding to convert
    /// with). Clamps rather than refuses an out-of-range `line`/`character`
    /// (a range's `end` can legitimately land at the buffer's char length);
    /// point-anchored callers want [`lsp_wire_point_to_char`](Self::lsp_wire_point_to_char).
    fn lsp_wire_to_char(&self, id: BufferId, line: usize, character: usize) -> Option<usize>;

    /// Same conversion as [`lsp_wire_to_char`](Self::lsp_wire_to_char), but
    /// backs `lsp-position->offset` specifically: refuses (`None`) rather
    /// than clamping when the wire position would land on the buffer's
    /// trailing phantom line, since every point-anchored decoration setter
    /// (`set-inlay-hints!`) rejects that offset outright — see
    /// `wire_point_to_char_for_buffer`'s doc for why the two must differ.
    fn lsp_wire_point_to_char(&self, id: BufferId, line: usize, character: usize) -> Option<usize>;

    /// Backs `(lsp-label-offsets->text bid label offsets)` — the
    /// `[start, end)` slice of `label` named by a
    /// `ParameterInformation.label` wire offset pair (unpacked from that
    /// builtin's `offsets` list), in the encoding `id`'s attached server
    /// negotiated. `None` if `id` is unknown or has no attached server.
    ///
    /// `label` is server-authored display text (a
    /// `SignatureInformation.label`), never document text, so no buffer
    /// holds it and the other converters here don't fit. `id` names the
    /// server, not the text being indexed.
    fn lsp_label_offsets_to_text(
        &self,
        id: BufferId,
        label: &str,
        start: usize,
        end: usize,
    ) -> Option<String>;

    /// `(lsp-locations->display-parts locs)` — the filesystem path, wire
    /// line, and column of each raw `Location`/`LocationLink` hashmap in
    /// `locs`, decoded through the same `hume_lsp::location::decode_location`
    /// `goto-location!` uses. Backs `lsp/location-display`'s drawer rows:
    /// `goto-location!` already converts wire positions correctly for the
    /// *jump*; this is the display-side counterpart.
    ///
    /// The column is an exact grapheme column when the target has an open
    /// buffer, `None` when it's an open buffer whose line is out of range,
    /// and otherwise the location's own wire `character` verbatim — see
    /// `LocationDisplay`'s `grapheme_col_or_wire` field doc for why that last case
    /// is the one sanctioned exception to "never render a wire unit
    /// directly".
    ///
    /// The path and line ride along with the column because all three come
    /// from one decode: reading `range.start.line` a second time in Scheme
    /// to render the row prefix is how a row ends up naming a position that
    /// doesn't match the one its column was read from. Every location shares
    /// the encoding negotiated by the currently focused buffer's attached
    /// server, same as `goto-location!`'s wire shape.
    ///
    /// `Err` — aborting the whole batch, not just one row — only for a
    /// location whose shape can't be decoded at all (missing `uri`/`range`,
    /// unparseable URI): such a location names no destination `goto-location!`
    /// could reach either, so a drawer row for it would be unselectable by
    /// construction. Degrading only this builtin wouldn't help either — the
    /// same malformed location would still abort three lines later inside
    /// `lsp/location-display`, which is why both routes decode through the
    /// one shared `decode_location` instead of tolerating a bad shape here.
    /// See `decode_location`'s doc for the full rule.
    fn lsp_locations_display_parts(
        &self,
        locs: Vec<serde_json::Value>,
    ) -> Result<Vec<LocationDisplay>, String>;
}

/// One `lsp-locations->display-parts` result row — see
/// [`LspHost::lsp_locations_display_parts`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationDisplay {
    /// The URI's decoded display path (`hume_lsp::uri::uri_to_display_string`).
    pub path: String,
    /// 0-based wire line, straight from the location's `range.start.line`.
    pub line: usize,
    /// 0-based column — a grapheme column when the location's target has an
    /// open buffer, `None` when it does and `line` is out of its range,
    /// otherwise the location's own wire `character` verbatim (the display
    /// companion never reads an unopened target's file to refine this
    /// number; see `location_display_parts`'s doc, `hume-editor`, for the
    /// full reasoning and the resulting unit divergence). Named for both
    /// possible units, not just the common one — see CLAUDE.md's "Column
    /// naming" invariant's one sanctioned exception.
    pub grapheme_col_or_wire: Option<usize>,
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

/// Async subprocess execution — accessed through
/// [`EditorHost::async_process`]. Backs `(spawn-async! cmd args cwd
/// callback)` / `(cancel-async! id)`.
pub trait AsyncProcessHost {
    /// Spawns `cmd` with `args` (direct argv, no shell) in `cwd` (`None` =
    /// the editor's own cwd), capturing its whole stdout/stderr to
    /// completion. Always returns a job id, even if the spawn itself fails
    /// — `callback` still fires exactly once either way, `(stdout stderr
    /// exit-code)`, `exit-code` `-1` for a signal-killed child, a status the
    /// OS never returned, or a spawn failure (missing binary, bad `cwd`).
    /// No error channel: a plugin holding a callback should never have to
    /// handle failure in two places, matching `lsp-request`/`prompt!`/
    /// `picker!`'s exactly-once contract.
    fn spawn_async(
        &mut self,
        cmd: &str,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        callback: steel::rvals::SteelVal,
    ) -> u64;

    /// Kills and reaps the job's child and drops its callback without
    /// firing it. A no-op if `id` already completed, was already
    /// cancelled, or never existed (a spawn failure that already fired its
    /// callback) — same idempotent contract as `cancel_timer`.
    fn cancel_async(&mut self, id: u64);
}

/// A single line-level change between two texts, 0-based and Steel-surface
/// ready — `set-signs!`/`set-virtual-lines!` are 0-indexed at the Steel
/// boundary, so no arithmetic is needed to feed a hunk into either. The
/// count each side covers is `old_lines.len()`/`new_lines.len()` — there is
/// no separate count field to keep in sync. A zero-length side needs no
/// special anchoring case: its empty line list already sits exactly at the
/// insertion/deletion point (`old_lines` empty for a pure insert, `new_lines`
/// empty for a pure deletion). `Equal` runs are never represented —
/// `DiffHost` methods drop them before returning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    /// Line index in the old text where this hunk starts.
    pub old_start: usize,
    /// Line index in the new text where this hunk starts.
    pub new_start: usize,
    /// The covered old-side lines, trailing newlines stripped.
    pub old_lines: Vec<String>,
    /// The covered new-side lines, trailing newlines stripped.
    pub new_lines: Vec<String>,
}

/// A single word-level change between two texts — e.g. a single changed
/// line's old/new text, as passed from a `diff-lines`/`diff-buffer-lines`
/// `Replace` hunk. Ranges are 0-based **char offsets**, not byte offsets,
/// matching `WordHunk`/`ExtraHighlightEntry`/`set-virtual-lines!`'s
/// `'segments`. `Equal` runs are dropped, same as [`DiffHunk`].
///
/// Unlike [`DiffHunk`] (line-index `start` into a rebuilt line list), a
/// word hunk is one contiguous span of text per side, so it carries `end`
/// (an exclusive char offset) and one `String` per side rather than a line
/// list — reusing `DiffHunk`'s shape here would force a fake
/// single-element `Vec<String>` that doesn't mean the same thing.
///
/// A zero-width side (`start == end`) needs no special case, same
/// rationale as `DiffHunk`'s empty-line-list side: it already sits exactly
/// at the insertion/deletion point (pure insert: `old_start == old_end`,
/// pure delete: `new_start == new_end`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordDiffHunk {
    pub old_start: usize,
    pub old_end: usize,
    pub new_start: usize,
    pub new_end: usize,
    pub old_text: String,
    pub new_text: String,
}

/// BufferText diffing — accessed through [`EditorHost::diff`]. Backs
/// `(diff-lines old-text new-text)` / `(diff-buffer-lines bid ref-text)` /
/// `(diff-words old-text new-text)`.
///
/// `diff_lines`/`diff_buffer_lines` treat their string inputs as buffer
/// text: every line ending becomes LF and a trailing newline is added if
/// missing, matching how HUME would load them from disk. This is a
/// deliberate divergence from `git diff`'s raw byte comparison — a file
/// missing its final newline reports no change on that line, since nothing
/// would change about it on save either. `diff_words` does **no** such
/// normalization: its inputs are
/// single lines already extracted from `BufferText`-normalized content (typically
/// one side of a `Replace` hunk), so wrapping them again would be a no-op
/// at best.
pub trait DiffHost {
    /// Line-level hunks between `old` and `new`, `Equal` runs dropped.
    fn diff_lines(&self, old: &str, new: &str) -> Vec<DiffHunk>;

    /// As [`diff_lines`](DiffHost::diff_lines), diffing `ref_text` against
    /// `bid`'s live (dirty) in-memory text — avoids materializing the whole
    /// buffer as a Steel string on every debounced call. `None` for an
    /// unknown/stale `bid` — the single liveness check this call needs
    /// (looking up the buffer's text also answers "does it exist"), so the
    /// Steel boundary maps `None` straight to an error rather than checking
    /// liveness a second time first.
    fn diff_buffer_lines(&self, bid: BufferId, ref_text: &str) -> Option<Vec<DiffHunk>>;

    /// Word-level hunks between `old` and `new`, `Equal` runs dropped. The
    /// returned `bool` mirrors `WordDiff::deadline_hit()`: `true` means the
    /// underlying Myers pass could not finish within its deadline and
    /// returned a coarse (Replace-all) result — unlike line-diff's Myers
    /// fallback (still a correct partition), a word-diff timeout result
    /// should be treated as a fallback, not a precise diff (skip word
    /// highlighting, fall back to a whole-line scope).
    fn diff_words(&self, old: &str, new: &str) -> (Vec<WordDiffHunk>, bool);
}

/// Which end of an over-long picker row is dropped — `picker!`'s and
/// `live-picker!`'s `#:truncate` symbol, decoded once at the builtin
/// boundary (`builtins::ui`) and carried as-is into the panel's paint-time
/// clip (`hume-editor`'s `PickerViewState::truncate`). A path's
/// distinguishing part (the basename) sits at the end, so cutting the head
/// is the default; a row whose distinguishing part sits at the front (a
/// grep match's file path, say, before the line preview) wants `'tail`
/// instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TruncateEnd {
    #[default]
    Head,
    Tail,
}

/// Grouped `picker!` open-time keyword options. [`UiHost::open_picker`] takes
/// the Scheme call's positional arguments (`items`, `on_select`) directly;
/// every `#:`-prefixed one rides here instead — the same split
/// [`PickerSourceOpts`] uses for `picker_source_spawn`. `Default` is every
/// keyword's own default (empty prompt, not pending, empty query) — the
/// shape a test that doesn't care about any of them wants.
#[derive(Default)]
pub struct PickerOpts {
    /// `#:prompt` — label painted before the query in the input line.
    pub prompt: String,
    /// `#:pending` — see [`UiHost::open_picker`]'s doc.
    pub pending: bool,
    /// `#:query` — the query the picker opens with, applied (as a fuzzy
    /// filter) to the item list at construction.
    pub query: String,
    /// `#:truncate` — see [`TruncateEnd`].
    pub truncate: TruncateEnd,
}

/// Grouped `live-picker!` open-time keyword options — the live counterpart
/// of [`PickerOpts`]. No `Default`: `on_query_change` is always
/// `live-picker!`'s own internal requery lambda (never a caller-supplied
/// value directly), so a default that silently opened a non-live session
/// would be a footgun, not a convenience.
pub struct LivePickerOpts {
    /// `#:prompt` — as [`PickerOpts::prompt`].
    pub prompt: String,
    /// `#:query` — the query the picker opens with. Unlike `PickerOpts`'s
    /// (which only filters the already-known item list), a non-empty value
    /// here also has the `live-picker!` Scheme wrapper spawn once,
    /// undebounced, right after this call returns — see
    /// [`UiHost::open_live_picker`]'s doc. `open_live_picker` itself never
    /// fires `on_query_change` for it; that lambda is reserved for
    /// per-keystroke requeries.
    pub query: String,
    /// Fired with `(token query)` on every query-changing keystroke instead
    /// of driving a local fuzzy filter — see
    /// `PickerSession::rebuild_filtered`'s doc for why a live session skips
    /// it. Always `live-picker!`'s own stop-and-clear-then-debounce
    /// wrapper around the caller's `#:command` builder, never the builder
    /// itself.
    pub on_query_change: steel::rvals::SteelVal,
    /// `#:truncate` — see [`TruncateEnd`].
    pub truncate: TruncateEnd,
}

/// Grouped `picker-source-spawn!` keyword options — the same split
/// [`PickerOpts`] uses for `open_picker`: [`UiHost::picker_source_spawn`]'s
/// positional arguments (`token`, `cmd`, `args`) stay directly on the trait
/// method. No `Default`: `#:ok-exit-codes` defaults to `'(0)`, not
/// `Vec::default()`'s empty list, so a caller must always supply it
/// explicitly rather than get a silently wrong allowlist.
pub struct PickerSourceOpts {
    /// `#:cwd` — working directory for the spawned process; `None` inherits
    /// the caller's.
    pub cwd: Option<std::path::PathBuf>,
    /// `#:nul` — split stdout on NUL bytes instead of newlines.
    pub nul: bool,
    /// `#:ok-exit-codes` — the complete set of exit codes that count as a
    /// normal outcome. It *replaces* the success check rather than
    /// extending it — nothing is implied, `0` included — so a caller
    /// overriding the `'(0)` default lists `0` alongside whatever it adds:
    /// `'(0 1)` for `rg`, which exits `1` on "no matches". `'(1)` alone
    /// would report every successful run as a failure. Explicit over
    /// convenient: the list is the whole contract.
    pub ok_exit_codes: Vec<i32>,
}

/// How [`UiHost::picker_feed`] merges a batch into the open picker's item
/// list — `picker-push!` (`Append`) vs `picker-replace!` (`Replace`).
pub enum PickerFeedMode {
    Append,
    Replace,
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

    /// `(picker! items on-select #:prompt "…" #:pending [#f] #:query [""])`
    /// — opens the fuzzy-finder panel, always fuzzy-filtered over `items`
    /// (query-change never leaves the local filter — for a source whose
    /// query drives an external command instead, see
    /// [`open_live_picker`](Self::open_live_picker)). `items` are
    /// `(display . payload)` pairs; `payload` is handed back to `on-select`
    /// verbatim, never interpreted by Rust. Returns a token that scopes
    /// later `picker-push!`/`picker-replace!`/`picker-source-spawn!` calls
    /// to this session. Unlike the menu/drawer, the picker is allowed from
    /// any mode but closes any live completion session first, since only
    /// one modal owner may be active at a time. `on-select` fires exactly
    /// once, queued (never invoked inline): the selected payload on
    /// `Enter`, or `#f` on `Esc`, `picker-close!`, or being replaced by a
    /// second `picker!`/`live-picker!` call. `pending`: set when a caller
    /// opens empty and expects more results via `spawn-async!` rather than
    /// `picker-source-spawn!` (which already implies "still populating" on
    /// its own) — surfaced to the UI as a "results still arriving"
    /// indicator, cleared by the first `push!`/`replace!` that actually
    /// applies. `query`: see [`PickerOpts`].
    fn open_picker(
        &mut self,
        items: Vec<(String, steel::rvals::SteelVal)>,
        on_select: steel::rvals::SteelVal,
        opts: PickerOpts,
    ) -> Result<u64, String>;

    /// `(live-picker! on-select #:command command #:prompt "…" #:query [""]
    /// #:debounce-ms [150] #:cwd [#f] #:nul [#f] #:ok-exit-codes ['(0)])` —
    /// opens the fuzzy-finder panel with the query driving an external
    /// source instead of the local fuzzy filter: `filtered` is always the
    /// identity permutation over whatever `items` currently holds (see
    /// `PickerSession::rebuild_filtered`'s doc). No `items`/`pending`
    /// parameter — a live session starts empty and is populated entirely by
    /// its own `on_query_change` callback (via `picker-push!`/
    /// `picker-replace!`/`picker-source-spawn!`, exactly as a `picker!`
    /// session's async sources are). Same return, exactly-once `on-select`,
    /// and modal-ownership contract as `open_picker`. `opts`: see
    /// [`LivePickerOpts`] — this method itself never fires
    /// `on_query_change`; the `live-picker!` Scheme wrapper spawns for a
    /// non-empty seed `query` itself, undebounced, right after this
    /// returns, so a bad `#:command` raise on that seed leaves the session
    /// this call already opened in place (Esc still closes it) rather than
    /// tearing it down — the wrapper deliberately doesn't catch-and-reraise
    /// around it, since a call sourced from a native builtin re-raised
    /// through a nested Steel handler corrupts the VM's continuation stack.
    fn open_live_picker(
        &mut self,
        on_select: steel::rvals::SteelVal,
        opts: LivePickerOpts,
    ) -> Result<u64, String>;

    /// `(picker-push! token items)` / `(picker-replace! token items)` —
    /// appends to, or wholesale replaces, the open picker's item list and
    /// reranks, but only if `token` matches the session the caller opened
    /// (returned by `open_picker`). One method for both: the token guard
    /// and "no open picker"/stale-token no-op contract are identical, only
    /// the merge policy ([`PickerFeedMode`]) differs. A stale token — the
    /// picker was closed or replaced since — is expected-normal for an
    /// async source racing the user, so it is a silent no-op, not an error:
    /// returns whether the feed was applied. `Replace` is the requery half
    /// of a live source: the previous pattern's rows stay on screen through
    /// the requery's stop/debounce/respawn gap and are only dropped once the
    /// new search has something to show in their place (or settles on
    /// nothing) — items are otherwise append-only.
    fn picker_feed(
        &mut self,
        token: u64,
        items: Vec<(String, steel::rvals::SteelVal)>,
        mode: PickerFeedMode,
    ) -> bool;

    /// `(picker-source-spawn! token cmd args #:cwd dir #:nul flag
    /// #:ok-exit-codes '(0))` — attaches a streaming external-command
    /// source to the open picker (direct argv spawn, no shell). Its stdout
    /// lines flow directly into the store, never through Steel. Replaces
    /// (killing) any source already attached to the same session — a
    /// second spawn is a re-spawn, not a second concurrent source, which is
    /// also how a live source re-runs per query change. If the outgoing
    /// source had already exited, its exit is reported exactly as it would
    /// have been had it disconnected on its own — a re-spawn must not
    /// silence a genuine failure just because a newer search superseded it
    /// before the drain got to it. `opts`: see [`PickerSourceOpts`].
    ///
    /// `Ok(false)` — same "expected-normal race, not an error" contract as
    /// `picker_feed` — means a stale token or no open picker; nothing was
    /// spawned. `Err` means the process itself failed to spawn (missing
    /// binary, bad `#:cwd`).
    fn picker_source_spawn(
        &mut self,
        token: u64,
        cmd: &str,
        args: Vec<String>,
        opts: PickerSourceOpts,
    ) -> Result<bool, String>;

    /// `(picker-source-stop! token)` — stops the open picker's attached
    /// streaming source, if any, without touching the item list.
    /// `picker-replace!` can clear stale rows but has no way to silence the
    /// search that produced them; this is that missing half, for a live
    /// requery whose new query has nothing to spawn a replacement source
    /// for (e.g. backspacing to an empty pattern). Same
    /// expected-normal-race contract as `picker_feed`: returns whether
    /// `token` matched the open session, regardless of whether a source was
    /// actually attached. Reports the outgoing source's exit if it had
    /// already exited, same as a re-spawn via `picker_source_spawn`.
    fn picker_source_stop(&mut self, token: u64) -> bool;

    /// `(picker-close! #:token [token #f])` — ends the open picker, if any,
    /// firing its `on-select` with `#f` (unlike `close-menu!`/
    /// `close-drawer!`, which drop the callback without invoking it — the
    /// picker's callback lifecycle guarantees exactly one fire per session
    /// no matter how it ends). `token` scopes the close to a specific
    /// session the same way `picker-push!`'s does: `Some(t)` is a no-op if
    /// the open picker's token doesn't match `t` (someone else's session
    /// has since taken over) — the async-callback case `picker-push!`
    /// already guards against. `#f`/omitted closes whatever picker is open,
    /// unconditionally, for the synchronous "the user hit Esc" caller that
    /// has no token to check against. Idempotent either way: closing when
    /// none is open is not an error.
    fn picker_close(&mut self, token: Option<u64>);
}

/// LSP-driven text edits, workspace edits, and go-to-location — accessed
/// through [`EditorHost::edits`].
pub trait EditHost {
    /// `(apply-text-edits! bid edits #:expect-generation gen)` — `edits` is
    /// `(start_line, start_character, end_line, end_character, new_text)`
    /// tuples in wire coordinates. Applied as one undo step.
    fn apply_text_edits(
        &mut self,
        bid: BufferId,
        edits: Vec<(usize, usize, usize, usize, String)>,
        expect_gen: Option<u64>,
    ) -> Result<(), String>;

    /// `(apply-workspace-edit! edit)` — `edit` is a decoded LSP
    /// `WorkspaceEdit` JSON blob. Returns the number of buffers modified.
    fn apply_workspace_edit(&mut self, edit: serde_json::Value) -> Result<usize, String>;

    /// `(goto-location! target)`, raw `Location`/`LocationLink` hashmap
    /// shape — `loc` decoded through `hume_lsp::location::decode_location`,
    /// the same decoder `lsp-locations->display-parts` uses for drawer rows.
    fn goto_location_value(&mut self, loc: serde_json::Value) -> Result<(), String>;

    /// `(goto-location! target)`, `(list target line char-col)` shape with a
    /// path or `file://` URI string target — already char-indexed.
    fn goto_location_path(
        &mut self,
        path_or_uri: String,
        line: usize,
        char_col: usize,
    ) -> Result<(), String>;

    /// `(goto-location! target)`, `(list target line char-col)` shape with a
    /// `bid` target — already char-indexed.
    fn goto_location_buffer(
        &mut self,
        bid: BufferId,
        line: usize,
        char_col: usize,
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
