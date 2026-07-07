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

    // ── Timers (B4; default = no timer support) ──────────────────────────────
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

    // ── Decoration stores (B5; default = no-op / empty) ──────────────────────
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
    /// lines for `bid` wholesale. Each entry is `(line, text)`.
    fn set_virtual_lines(&mut self, source: String, bid: BufferId, lines: Vec<(usize, String)>) {
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

    // ── Edit + navigation primitives (B6; default = "not supported") ────────
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

    // ── Minibuffer prompt (B9; default = "not supported") ────────────────────
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
}
