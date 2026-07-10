use std::path::{Path, PathBuf};

use crossterm::event::KeyEvent;
use hume_engine::pipeline::{BufferId, PaneId};

use crate::types::SteelCmdDef;

/// Key-binding mode, as recognised by `bind-key!`/`unbind-key!`.
///
/// Defined here (scripting layer) so builtins do not depend on the editor's
/// internal `crate::editor::keymap::BindMode`.  The editor impl maps this to
/// its own `BindMode` in `EditorHostImpl`.
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

/// The editor interface exposed to scripting builtins during a Steel eval.
///
/// Implemented by `EditorHostImpl<'a>` in the editor crate (or `MockHost` in
/// tests).  `SteelCtx` holds `host: &'a mut dyn EditorHost`; builtins call
/// these methods rather than borrowing individual editor-domain fields directly.
///
/// All methods take/return only `'static` types (owned `String`/`PathBuf`/`Vec`,
/// `Copy` ids, scripting-owned enums) so that `SteelCtx<'static>` — the type
/// projection required by Steel's `with_mut_reference` — is valid.
///
/// # Init vs command mode
///
/// Methods that operate on buffers/panes (`open_buffer`, `close_buffer`,
/// `switch_to_buffer`, buffer reads/enumeration) are only reachable in command
/// mode: the `require_cmd_ctx!` guard in each builtin prevents them from being
/// called during init (`is_init = true`).  The init-only methods
/// (`set_global_option`, `configure_statusline`, `bind_*`/`unbind_key`) are
/// protected by the reverse guard.
///
/// # Focus snapshot
///
/// The focused buffer/pane ids are passed as explicit constructor arguments to
/// `call_steel_cmd` and `fire_hook` rather than being queried via this trait.
/// This keeps the `SteelCtx` snapshot stable: a builtin reading `ctx.focused_*`
/// always sees the pre-command snapshot, not a live value that may change
/// mid-eval (e.g. after `switch-to-buffer!`).
pub trait EditorHost {
    // ── Enumeration ─────────────────────────────────────────────────────────
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

    // ── Settings (init-only; only Global scope from scripts) ─────────────────
    fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String>;
    /// `(get-option key)` — the effective value of `key`: `bid`'s buffer
    /// override if one is set, else the global default. `Err` for an
    /// unknown key. Callable from any context (no init/plugin-load gate,
    /// unlike `set_global_option`).
    fn get_option(&self, key: &str, bid: BufferId) -> Result<OptionValue, String>;

    // ── Statusline (init-only; editor parses names → StatusElement) ──────────
    fn configure_statusline(
        &mut self,
        left: Vec<String>,
        center: Vec<String>,
        right: Vec<String>,
    ) -> Result<(), String>;

    // ── Keymap (init-only) ───────────────────────────────────────────────────
    fn bind_key(
        &mut self,
        mode: BindMode,
        keys: &[KeyEvent],
        cmd: &str,
        force_extend: bool,
    ) -> Result<(), String>;
    fn bind_wait_char(
        &mut self,
        mode: BindMode,
        keys: &[KeyEvent],
        cmd: &str,
    ) -> Result<(), String>;
    fn unbind_key(&mut self, mode: BindMode, keys: &[KeyEvent]) -> Result<(), String>;

    // ── Language / grammar (command mode) ───────────────────────────────────
    fn attach_grammar(
        &mut self,
        name: &str,
        grammar_path: &Path,
        symbol: &str,
        highlights_path: &Path,
        injections_path: Option<&Path>,
    ) -> Result<(), String>;
    fn has_grammar(&self, language: &str) -> bool;

    // ── Register validation ──────────────────────────────────────────────────
    fn is_valid_register_name(&self, ch: char) -> bool;

    // ── Budget ───────────────────────────────────────────────────────────────
    /// Steel eval budget in milliseconds for command / hook execution.
    fn steel_command_budget_ms(&self) -> u64;

    // ── Terminal safety ──────────────────────────────────────────────────────
    /// True while the command currently being dispatched is `#:inline-output`
    /// (the alt-screen TUI is suspended for the duration of its body), meaning
    /// raw stdout writes are safe. Defaults to `false` so hosts that never run
    /// under the live TUI (test stubs) need no override.
    fn is_inline_output_command(&self) -> bool {
        false
    }

    // ── Synchronous command dispatch ─────────────────────────────────────────
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
    /// Valid only in command mode; guarded by `require_cmd_ctx!` in the caller.
    fn run_command_sync(
        &mut self,
        name: &str,
        count: Option<usize>,
        extend: bool,
        register: Option<char>,
    ) -> Result<(), String>;

    // ── Command registration (init-only) ────────────────────────────────────
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

    // ── Live cursor read ─────────────────────────────────────────────────────
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

    /// Steel-side staleness token for buffer `id` (its `text_gen`, bumped by
    /// every mutation) — `None` if `id` is unknown. Not LSP-specific (any
    /// script can compare a saved value against a live read), but the LSP
    /// bridge's own `#:allow-stale` staleness check is what motivated it.
    fn buffer_generation(&self, id: BufferId) -> Option<u64> {
        let _ = id;
        None
    }

    // ── LSP introspection (command mode; default = no LSP support) ──────────
    /// Decoded `ServerCapabilities` for `server` (a registered language name,
    /// or `None` for the focused buffer's attached server) — `None` if
    /// unresolvable or the server hasn't finished its handshake yet.
    fn lsp_capabilities(&self, server: Option<&str>) -> Option<serde_json::Value> {
        let _ = server;
        None
    }

    /// One entry per running (language, root) server.
    fn lsp_server_status(&self) -> Vec<crate::types::LspServerStatusEntry> {
        Vec::new()
    }

    /// The registered language for the server attached to buffer `id`, or
    /// `None` if `id` is unknown or has no attached server.
    fn lsp_server_for_buffer(&self, id: BufferId) -> Option<String> {
        let _ = id;
        None
    }

    /// Whether `language` currently has a `register-lsp-server!` config
    /// (registered, not necessarily attached/running) — used by the
    /// `on-language-set` missing-server hint to distinguish "not installed"
    /// from "still starting". Reports the state *as of the last completed
    /// drain*: an op queued earlier in the same eval hasn't applied yet.
    fn lsp_registered_for_language(&self, language: &str) -> bool {
        let _ = language;
        false
    }

    /// Ready-made `{"textDocument" {"uri"} "position" {"line" "character"}}`
    /// params for `id`'s primary cursor head, in its attached server's
    /// negotiated encoding — `None` if `id` has no path, no attached server,
    /// or isn't currently shown in any pane.
    fn lsp_position_params(&self, id: BufferId) -> Option<serde_json::Value> {
        let _ = id;
        None
    }

    /// Same as [`lsp_position_params`](Self::lsp_position_params) but a
    /// `{"textDocument" {"uri"} "range" {"start" "end"}}` shape from the
    /// primary selection.
    fn lsp_range_params(&self, id: BufferId) -> Option<serde_json::Value> {
        let _ = id;
        None
    }

    // ── Timers (default = no timer support) ──────────────────────────────
    /// Schedules `thunk` — opaque to this trait, a raw Steel closure — to
    /// fire after `ms` milliseconds. Returns the new timer id, or `None` if
    /// this host has no timer wheel to schedule onto (test hosts).
    fn schedule_timer(&mut self, ms: u64, thunk: steel::rvals::SteelVal) -> Option<u64> {
        let _ = (ms, thunk);
        None
    }

    /// Cancels a previously scheduled timer. A no-op if `id` already fired,
    /// was already cancelled, or this host has no timer wheel.
    fn cancel_timer(&mut self, id: u64) {
        let _ = id;
    }

    /// `(register-trigger-chars! source chars)` — registers `chars` as
    /// `OnTriggerChar`-firing chars for `source`, replacing that source's
    /// previous set (a plugin's own reload doesn't accumulate duplicates).
    /// Checked as a union across every source's set. Default no-op — test
    /// hosts have no `EditorState` to register into.
    fn register_trigger_chars(&mut self, source: String, chars: Vec<char>) {
        let _ = (source, chars);
    }

    // ── Decoration stores (default = no-op / empty) ──────────────────────
    /// `(set-inlay-hints! bid hints)` — replaces `bid`'s inlay hints
    /// wholesale. Each entry is `(wire_position, text, before)`; the wire
    /// position (raw decoded `{"line" "character"}`) is converted to a char
    /// offset using `bid`'s attached server's negotiated encoding.
    fn set_inlay_hints(&mut self, bid: BufferId, hints: Vec<(serde_json::Value, String, bool)>) {
        let _ = (bid, hints);
    }

    /// `(set-signs! source bid signs)` — replaces `source`'s signs for `bid`
    /// wholesale. Each entry is `(line, text, scope, priority)`.
    fn set_signs(
        &mut self,
        source: String,
        bid: BufferId,
        signs: Vec<(usize, String, String, i64)>,
    ) {
        let _ = (source, bid, signs);
    }

    /// `(set-virtual-lines! source bid lines)` — replaces `source`'s virtual
    /// lines for `bid` wholesale. Each entry is `(line, text)` or `(line
    /// text scope)` — `scope` styles the whole line (`ui.virtual` fallback
    /// when absent).
    fn set_virtual_lines(
        &mut self,
        source: String,
        bid: BufferId,
        lines: Vec<(usize, String, Option<String>)>,
    ) {
        let _ = (source, bid, lines);
    }

    /// `(set-extra-highlights! source bid spans)` — replaces `source`'s
    /// extra highlights for `bid` wholesale. Each entry is `(start, end,
    /// scope)`, char offsets.
    fn set_extra_highlights(
        &mut self,
        source: String,
        bid: BufferId,
        spans: Vec<(usize, usize, String)>,
    ) {
        let _ = (source, bid, spans);
    }

    /// `(diagnostics-for-buffer bid #:severity floor #:range (start end))` —
    /// decoded `{"start" "end" "line" "col" "severity" "message" "code"
    /// "source"}` hashmaps, filtered then capped at 1000. `severity_floor`
    /// is `None` for "no floor" (everything); `range` is `None` for the
    /// whole buffer.
    fn diagnostics_for_buffer(
        &self,
        bid: BufferId,
        severity_floor: Option<&str>,
        range: Option<(usize, usize)>,
    ) -> Vec<serde_json::Value> {
        let _ = (bid, severity_floor, range);
        Vec::new()
    }

    /// `(diagnostic-counts bid)` → `(errors . warnings)`.
    fn diagnostic_counts(&self, bid: BufferId) -> (usize, usize) {
        let _ = bid;
        (0, 0)
    }

    // ── Edit + navigation primitives (default = "not supported") ────────
    /// `(apply-text-edits! bid edits #:expect-generation gen)` — `edits` is
    /// `(start_line, start_char, end_line, end_char, new_text)` tuples in
    /// wire coordinates. Applied as one undo step.
    fn apply_text_edits(
        &mut self,
        bid: BufferId,
        edits: Vec<(usize, usize, usize, usize, String)>,
        expect_gen: Option<u64>,
    ) -> Result<(), String> {
        let _ = (bid, edits, expect_gen);
        Err("apply-text-edits!: not supported by this host".to_string())
    }

    /// `(apply-workspace-edit! edit)` — `edit` is a decoded LSP
    /// `WorkspaceEdit` JSON blob. Returns the number of buffers modified.
    fn apply_workspace_edit(&mut self, edit: serde_json::Value) -> Result<usize, String> {
        let _ = edit;
        Err("apply-workspace-edit!: not supported by this host".to_string())
    }

    /// `(goto-location! target)`, raw `Location`/`LocationLink` shape —
    /// `uri` a wire URI string, `line`/`character` wire coordinates.
    fn goto_location_wire(
        &mut self,
        uri: String,
        line: usize,
        character: usize,
    ) -> Result<(), String> {
        let _ = (uri, line, character);
        Err("goto-location!: not supported by this host".to_string())
    }

    /// `(goto-location! target)`, `(list target line col)` shape with a
    /// path or `file://` URI string target — already char-indexed.
    fn goto_location_path(
        &mut self,
        path_or_uri: String,
        line: usize,
        col: usize,
    ) -> Result<(), String> {
        let _ = (path_or_uri, line, col);
        Err("goto-location!: not supported by this host".to_string())
    }

    /// `(goto-location! target)`, `(list target line col)` shape with a
    /// `bid` target — already char-indexed.
    fn goto_location_buffer(
        &mut self,
        bid: BufferId,
        line: usize,
        col: usize,
    ) -> Result<(), String> {
        let _ = (bid, line, col);
        Err("goto-location!: not supported by this host".to_string())
    }

    /// `(selection-spans-full-line? bid)`.
    fn selection_spans_full_line(&self, bid: BufferId) -> bool {
        let _ = bid;
        false
    }

    // ── Minibuffer prompt (default = "not supported") ────────────────────
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
    ) -> Result<(), String> {
        let _ = (label, prefill, callback);
        Err("prompt!: not supported by this host".to_string())
    }

    /// `(symbol-under-cursor bid)` — the word at the primary cursor head,
    /// `""` on whitespace/punctuation.
    fn symbol_under_cursor(&self, bid: BufferId) -> String {
        let _ = bid;
        String::new()
    }

    // ── Cursor-anchored popup (default = "not supported") ────────────────
    /// `(show-popup! text #:anchor 'cursor)` — shows `text` in a floating
    /// panel anchored near the focused pane's cursor. Geometry (wrap width,
    /// flip/clamp position) is resolved fresh every frame by the host, not
    /// here — this just stores the raw content. Replaces any popup already
    /// showing (no stacking).
    fn show_popup(&mut self, text: String) -> Result<(), String> {
        let _ = text;
        Err("show-popup!: not supported by this host".to_string())
    }

    /// `(close-popup!)` — dismisses the popup. Idempotent: closing when none
    /// is showing is not an error (only an unsupported *host* errors).
    fn close_popup(&mut self) -> Result<(), String> {
        Err("close-popup!: not supported by this host".to_string())
    }

    // ── Selection menu (default = "not supported") ───────────────────────
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
    ) -> Result<(), String> {
        let _ = (items, callback);
        Err("show-menu!: not supported by this host".to_string())
    }

    /// `(close-menu!)` — dismisses the menu *without* invoking its callback
    /// (caller-initiated close, distinct from the key-driven dismissal paths
    /// which do call back with `#f`).
    fn close_menu(&mut self) -> Result<(), String> {
        Err("close-menu!: not supported by this host".to_string())
    }

    // ── Bottom drawer (default = "not supported") ────────────────────────
    /// `(show-drawer-list! items on-select)` — opens a scrolling list in the
    /// bottom chrome band. `items` are pre-formatted display strings; the
    /// drawer never interprets their content — the jump (if any) is the
    /// caller's job, typically `(goto-location! ...)` inside `on-select`.
    /// `on-select` receives the chosen index and, unlike the popup/menu's
    /// one-shot callback, may fire more than once: the drawer stays open
    /// across `Enter` (Helix-style browse) until `Esc` or `close-drawer!`.
    /// Replaces any drawer already open (no stacking).
    fn show_drawer_list(
        &mut self,
        items: Vec<String>,
        callback: steel::rvals::SteelVal,
    ) -> Result<(), String> {
        let _ = (items, callback);
        Err("show-drawer-list!: not supported by this host".to_string())
    }

    /// `(close-drawer!)` — dismisses the drawer *without* invoking its
    /// callback (caller-initiated close, distinct from `Esc`, which does
    /// call back with `#f`).
    fn close_drawer(&mut self) -> Result<(), String> {
        Err("close-drawer!: not supported by this host".to_string())
    }

    // ── Completion orchestration (default = "not supported") ─────────────
    /// `(completion-begin! bid items #:incomplete f)` — `items` is a list of
    /// decoded `CompletionItem` hashmaps (JSON already converted by the
    /// caller). Starting a session replaces any session already open.
    fn completion_begin(
        &mut self,
        bid: BufferId,
        items: Vec<serde_json::Value>,
        incomplete: bool,
    ) -> Result<(), String> {
        let _ = (bid, items, incomplete);
        Err("completion-begin!: not supported by this host".to_string())
    }

    /// `(completion-update-filter! text)` — re-ranks the open session
    /// against `text`; Rust-side work only, safe to call every keystroke.
    fn completion_update_filter(&mut self, text: String) -> Result<(), String> {
        let _ = text;
        Err("completion-update-filter!: not supported by this host".to_string())
    }

    /// `(completion-top n)` — up to `n` ranked items as hashmaps, `[]` with
    /// no open session.
    fn completion_top(&self, n: usize) -> Vec<serde_json::Value> {
        let _ = n;
        Vec::new()
    }

    /// `(completion-accept! idx)` — applies `idx`'s item (an index into the
    /// ranked/filtered list, not the raw response order) and ends the
    /// session, success or failure.
    fn completion_accept(&mut self, idx: usize) -> Result<(), String> {
        let _ = idx;
        Err("completion-accept!: not supported by this host".to_string())
    }

    /// `(completion-dismiss!)` — clears any open session; no-op if none.
    fn completion_dismiss(&mut self) {}
}
