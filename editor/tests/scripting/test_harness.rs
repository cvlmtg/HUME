//! [`MockHost`] — a minimal in-memory [`scripting::EditorHost`] implementation
//! for scripting integration tests in the editor crate.
//!
//! Holds real `EditorSettings` and `Keymap` so tests can assert on
//! `set-option!` / `bind-key!` side effects directly, without needing a full
//! editor session.

use crossterm::event::KeyEvent;
use engine::pipeline::{BufferId, PaneId};

use scripting::host::{BindMode, EditorHost};

/// Minimal [`EditorHost`] that uses real `EditorSettings` and `Keymap`.
///
/// Buffer/pane lifecycle methods (`open_buffer`, `close_buffer`, etc.) are not
/// exercised in init-mode or pure-scripting tests; they return `Err` if called.
pub(crate) struct MockHost {
    pub(crate) settings: hume::settings::EditorSettings,
    pub(crate) keymap: hume::Keymap,
    /// Grammar names attached via `(register-grammar! …)` in command mode.
    pub(crate) grammars: std::collections::HashSet<String>,
    pub(crate) focused_buffer_id: BufferId,
    pub(crate) focused_pane_id: PaneId,
}

impl MockHost {
    pub(crate) fn new() -> Self {
        Self {
            settings: hume::settings::EditorSettings::default(),
            keymap: hume::Keymap::default(),
            grammars: std::collections::HashSet::new(),
            focused_buffer_id: BufferId::default(),
            focused_pane_id: PaneId::default(),
        }
    }
}

impl Default for MockHost {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorHost for MockHost {
    fn focused_buffer_id(&self) -> BufferId {
        self.focused_buffer_id
    }
    fn focused_pane_id(&self) -> PaneId {
        self.focused_pane_id
    }
    fn buffer_ids(&self) -> Vec<BufferId> {
        Vec::new()
    }
    fn pane_ids(&self) -> Vec<PaneId> {
        Vec::new()
    }
    fn buffer_exists(&self, _id: BufferId) -> bool {
        false
    }
    fn buffer_path(&self, _id: BufferId) -> Option<std::path::PathBuf> {
        None
    }
    fn buffer_display_name(&self, _id: BufferId) -> Option<String> {
        None
    }
    fn buffer_is_dirty(&self, _id: BufferId) -> Option<bool> {
        None
    }
    fn buffer_stored_language(&self, _id: BufferId) -> Option<String> {
        None
    }
    fn open_buffer(&mut self, _path: &std::path::Path) -> Result<BufferId, String> {
        Err("MockHost: open_buffer not implemented".into())
    }
    fn close_buffer(&mut self, _id: BufferId) -> BufferId {
        self.focused_buffer_id
    }
    fn switch_to_buffer(&mut self, _current: BufferId, target: BufferId) {
        self.focused_buffer_id = target;
    }
    fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String> {
        use hume::settings::{BufferOverrides, SettingScope, apply_setting};
        let mut dummy = BufferOverrides::default();
        apply_setting(SettingScope::Global, key, value, &mut self.settings, &mut dummy)
    }
    fn configure_statusline(
        &mut self,
        left: Vec<String>,
        center: Vec<String>,
        right: Vec<String>,
    ) -> Result<(), String> {
        use hume::ui::statusline::{StatusElement, StatusLineConfig};
        let parse = |list: Vec<String>, section: &str| -> Result<Vec<StatusElement>, String> {
            list.iter()
                .map(|s| s.parse::<StatusElement>().map_err(|e| format!("{section}: {e}")))
                .collect()
        };
        let left = parse(left, "left")?;
        let center = parse(center, "center")?;
        let right = parse(right, "right")?;
        self.settings.statusline = StatusLineConfig { left, center, right };
        Ok(())
    }
    fn bind_key(
        &mut self,
        mode: BindMode,
        keys: &[KeyEvent],
        cmd: &str,
        force_extend: bool,
    ) -> Result<(), String> {
        self.keymap.bind_user_with_extend(
            to_editor_bind_mode(mode),
            keys,
            std::borrow::Cow::Owned(cmd.to_owned()),
            force_extend,
        );
        Ok(())
    }
    fn bind_wait_char(
        &mut self,
        mode: BindMode,
        keys: &[KeyEvent],
        cmd: &str,
    ) -> Result<(), String> {
        self.keymap.bind_wait_char_user(
            to_editor_bind_mode(mode),
            keys,
            std::borrow::Cow::Owned(cmd.to_owned()),
        );
        Ok(())
    }
    fn unbind_key(&mut self, mode: BindMode, keys: &[KeyEvent]) -> Result<(), String> {
        self.keymap.unbind_user(to_editor_bind_mode(mode), keys);
        Ok(())
    }
    fn attach_grammar(
        &mut self,
        name: &str,
        _grammar_path: &std::path::Path,
        _symbol: &str,
        _highlights_path: &std::path::Path,
    ) -> Result<(), String> {
        self.grammars.insert(name.to_owned());
        Ok(())
    }
    fn has_grammar(&self, language: &str) -> bool {
        self.grammars.contains(language)
    }
    fn is_valid_register_name(&self, ch: char) -> bool {
        hume::ops::register::is_valid_register_name(ch)
    }
    fn steel_init_budget_ms(&self) -> u64 {
        self.settings.steel_init_budget_ms as u64
    }
    fn steel_command_budget_ms(&self) -> u64 {
        self.settings.steel_command_budget_ms as u64
    }
}

/// Map scripting `BindMode` → editor `hume::KeymapBindMode`.
fn to_editor_bind_mode(mode: BindMode) -> hume::KeymapBindMode {
    match mode {
        BindMode::Normal => hume::KeymapBindMode::Normal,
        BindMode::Extend => hume::KeymapBindMode::Extend,
        BindMode::Insert => hume::KeymapBindMode::Insert,
    }
}

