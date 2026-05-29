use std::path::{Path, PathBuf};

use crossterm::event::KeyEvent;
use engine::pipeline::{BufferId, PaneId};

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
/// called during init (`is_init = true`), where the corresponding refs would be
/// `None`.  The init-only methods (`set_global_option`, `configure_statusline`,
/// `bind_*`/`unbind_key`) are protected by the reverse guard.
pub trait EditorHost {
    // ── Focus snapshot ──────────────────────────────────────────────────────
    fn focused_buffer_id(&self) -> BufferId;
    fn focused_pane_id(&self) -> PaneId;

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
    /// Close `id`.  Returns the new live focused buffer id.
    fn close_buffer(&mut self, id: BufferId) -> BufferId;
    fn switch_to_buffer(&mut self, current: BufferId, target: BufferId);

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
    /// Steel eval budget in milliseconds for init.scm / plugin loads.
    fn steel_init_budget_ms(&self) -> u64;
    /// Steel eval budget in milliseconds for command / hook execution.
    fn steel_command_budget_ms(&self) -> u64;
}
