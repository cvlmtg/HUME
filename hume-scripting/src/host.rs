use std::path::{Path, PathBuf};

use crossterm::event::KeyEvent;
use hume_engine::pipeline::{BufferId, PaneId};

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
    ) -> Result<(), String>;
    fn has_grammar(&self, language: &str) -> bool;

    // ── Register validation ──────────────────────────────────────────────────
    fn is_valid_register_name(&self, ch: char) -> bool;

    // ── Budget ───────────────────────────────────────────────────────────────
    /// Steel eval budget in milliseconds for command / hook execution.
    fn steel_command_budget_ms(&self) -> u64;

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
    /// state. Only call this after `command_is_native` returned `Ok(true)` for the
    /// same name — the caller's classification gate is what prevents non-native
    /// names from reaching here. An unknown name returns `Err(msg)`.
    ///
    /// `register` arms `state.register_prefix` before dispatch so register-aware
    /// commands (`yank`, `delete`, `paste-after`, etc.) route to the right
    /// destination. Pass `None` when no explicit register was set.
    ///
    /// Returns `Ok(())` on success (includes `EditorCmd` errors, which are reported
    /// to the user and treated as success for the Steel caller).
    /// Returns `Err(msg)` when the name is not found in the registry.
    ///
    /// Valid only in command mode; guarded by `require_cmd_ctx!` in the caller.
    fn run_command_sync(
        &mut self,
        name: &str,
        count: usize,
        extend: bool,
        register: Option<char>,
    ) -> Result<(), String>;

    // ── Live cursor/selection reads ──────────────────────────────────────────
    /// Line number (1-indexed) of the primary cursor in the focused buffer.
    ///
    /// Returns `None` when the focused (pane, buffer) has no seeded pane state
    /// (stale or never-focused ids).
    fn current_line_number(&self) -> Option<usize>;

    /// Char-index of the primary cursor head in the focused buffer.
    ///
    /// Returns `None` under the same conditions as [`Self::current_line_number`].
    fn cursor_char_index(&self) -> Option<usize>;
}
