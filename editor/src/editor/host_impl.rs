//! [`EditorHostImpl`] — the production implementation of the scripting crate's
//! [`EditorHost`] trait.
//!
//! Holds the split borrows needed by the scripting layer, plus the editor
//! context needed by the init-only methods (settings, keymap).  Two construction
//! sites create this:
//!
//! - **Command dispatch** (`mappings/execute.rs`): all optional fields are
//!   `Some(...)`, populated from the live editor state.
//! - **Init dispatch** (`scripting_setup.rs`): buffer/pane/language fields are
//!   `None`; the init-only builtins (`set-option!`, `bind-key!`, etc.) are
//!   the only ones reachable during init, and they only touch `settings` and
//!   `keymap`.

use std::path::{Path, PathBuf};

use crossterm::event::KeyEvent;
use engine::pipeline::{BufferId, EngineView, PaneId};
use slotmap::SecondaryMap;

use super::jump_list::JumpList;
use crate::editor::buffer::Buffer;
use crate::editor::buffer_store::BufferStore;
use crate::editor::keymap::Keymap;
use crate::editor::pane_state::PaneBufferState;
use crate::editor::syntax::LanguageRegistry;
use crate::settings::{BufferOverrides, EditorSettings, SettingScope, apply_setting};
use crate::ui::statusline::{StatusElement, StatusLineConfig};
use scripting::host::{BindMode, EditorHost};

pub(crate) struct EditorHostImpl<'a> {
    pub(crate) settings: &'a mut EditorSettings,
    pub(crate) keymap: &'a mut Keymap,
    pub(crate) focused_pane_id: PaneId,
    pub(crate) buffers: Option<&'a mut BufferStore>,
    pub(crate) engine_view: Option<&'a mut EngineView>,
    pub(crate) pane_state:
        Option<&'a mut SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>>,
    pub(crate) pane_jumps: Option<&'a mut SecondaryMap<PaneId, JumpList>>,
    pub(crate) languages: Option<&'a mut LanguageRegistry>,
}

impl<'a> EditorHostImpl<'a> {
    /// Look up a buffer by id, or `None` if the buffers ref is unavailable
    /// or the id is stale/unknown.
    fn buffer(&self, id: BufferId) -> Option<&Buffer> {
        self.buffers.as_ref()?.try_get(id)
    }
}

impl<'a> EditorHost for EditorHostImpl<'a> {
    // ── Enumeration ──────────────────────────────────────────────────────────
    fn buffer_ids(&self) -> Vec<BufferId> {
        self.buffers
            .as_ref()
            .map(|b| b.iter().map(|(id, _)| id).collect())
            .unwrap_or_default()
    }
    fn pane_ids(&self) -> Vec<PaneId> {
        self.engine_view
            .as_ref()
            .map(|ev| ev.panes.iter().map(|(id, _)| id).collect())
            .unwrap_or_default()
    }

    // ── Buffer reads ─────────────────────────────────────────────────────────
    fn buffer_exists(&self, id: BufferId) -> bool {
        self.buffer(id).is_some()
    }
    fn buffer_path(&self, id: BufferId) -> Option<PathBuf> {
        self.buffer(id)?.path().map(Path::to_path_buf)
    }
    fn buffer_display_name(&self, id: BufferId) -> Option<String> {
        self.buffer(id).map(|buf| buf.display_name().to_owned())
    }
    fn buffer_is_dirty(&self, id: BufferId) -> Option<bool> {
        self.buffer(id).map(|buf| buf.is_dirty())
    }
    fn buffer_stored_language(&self, id: BufferId) -> Option<String> {
        self.buffer(id)?.language.clone()
    }

    // ── Buffer lifecycle ─────────────────────────────────────────────────────
    fn open_buffer(&mut self, path: &Path) -> Result<BufferId, String> {
        let canonical = platform::fs::canonicalize(path)
            .map_err(|e| format!("open-buffer!: {}: {e}", path.display()))?;
        let ev = self
            .engine_view
            .as_mut()
            .ok_or_else(|| "open-buffer!: editor refs unavailable".to_owned())?;
        let bufs = self
            .buffers
            .as_mut()
            .ok_or_else(|| "open-buffer!: editor refs unavailable".to_owned())?;
        let ps = self
            .pane_state
            .as_mut()
            .ok_or_else(|| "open-buffer!: editor refs unavailable".to_owned())?;
        let (bid, _) = crate::editor::ops::open_or_dedup(ev, bufs, ps, self.focused_pane_id, &canonical)
            .map_err(|e| format!("open-buffer!: {}: {e}", canonical.display()))?;
        Ok(bid)
    }
    fn close_buffer(&mut self, id: BufferId) -> Result<BufferId, String> {
        let ev = self.engine_view.as_mut()
            .ok_or_else(|| "close-buffer!: editor refs unavailable".to_owned())?;
        let bufs = self.buffers.as_mut()
            .ok_or_else(|| "close-buffer!: editor refs unavailable".to_owned())?;
        let ps = self.pane_state.as_mut()
            .ok_or_else(|| "close-buffer!: editor refs unavailable".to_owned())?;
        let jumps = self.pane_jumps.as_mut()
            .ok_or_else(|| "close-buffer!: editor refs unavailable".to_owned())?;
        Ok(crate::editor::ops::close_buffer(ev, bufs, ps, jumps, self.focused_pane_id, id))
    }
    fn switch_to_buffer(&mut self, current: BufferId, target: BufferId) -> Result<(), String> {
        let ev = self.engine_view.as_mut()
            .ok_or_else(|| "switch-to-buffer!: editor refs unavailable".to_owned())?;
        let bufs = self.buffers.as_mut()
            .ok_or_else(|| "switch-to-buffer!: editor refs unavailable".to_owned())?;
        let ps = self.pane_state.as_mut()
            .ok_or_else(|| "switch-to-buffer!: editor refs unavailable".to_owned())?;
        let jumps = self.pane_jumps.as_mut()
            .ok_or_else(|| "switch-to-buffer!: editor refs unavailable".to_owned())?;
        crate::editor::ops::switch_to_buffer_with_jump(
            ev,
            bufs,
            ps,
            jumps,
            self.focused_pane_id,
            current,
            target,
        );
        Ok(())
    }

    // ── Settings ─────────────────────────────────────────────────────────────
    fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String> {
        let mut dummy = BufferOverrides::default();
        apply_setting(SettingScope::Global, key, value, self.settings, &mut dummy)
    }

    // ── Statusline ────────────────────────────────────────────────────────────
    fn configure_statusline(
        &mut self,
        left: Vec<String>,
        center: Vec<String>,
        right: Vec<String>,
    ) -> Result<(), String> {
        let parse = |list: Vec<String>, section: &str| -> Result<Vec<StatusElement>, String> {
            list.iter()
                .map(|s| s.parse::<StatusElement>().map_err(|e| format!("configure-statusline! {section}: {e}")))
                .collect()
        };
        let left = parse(left, "left")?;
        let center = parse(center, "center")?;
        let right = parse(right, "right")?;
        self.settings.statusline = StatusLineConfig { left, center, right };
        Ok(())
    }

    // ── Keymap ────────────────────────────────────────────────────────────────
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

    // ── Language / grammar ────────────────────────────────────────────────────
    fn attach_grammar(
        &mut self,
        name: &str,
        grammar_path: &Path,
        symbol: &str,
        highlights_path: &Path,
    ) -> Result<(), String> {
        let langs = self
            .languages
            .as_mut()
            .ok_or_else(|| "register-grammar!: language registry unavailable".to_owned())?;
        let ev = self
            .engine_view
            .as_mut()
            .ok_or_else(|| "register-grammar!: engine view unavailable".to_owned())?;
        langs
            .attach_grammar(name, grammar_path, symbol, highlights_path, &mut ev.registry)
            .map_err(|e| format!("register-grammar! '{name}': {e}"))?;
        ev.theme.bake(&ev.registry);
        Ok(())
    }
    fn has_grammar(&self, language: &str) -> bool {
        self.languages
            .as_ref()
            .is_some_and(|l| l.has_grammar(language))
    }

    // ── Register validation ───────────────────────────────────────────────────
    fn is_valid_register_name(&self, ch: char) -> bool {
        crate::ops::register::is_valid_register_name(ch)
    }

    // ── Budget ────────────────────────────────────────────────────────────────
    fn steel_command_budget_ms(&self) -> u64 {
        self.settings.steel_command_budget_ms as u64
    }
}

/// Map scripting `BindMode` → editor `keymap::BindMode`.
pub(crate) fn to_editor_bind_mode(mode: BindMode) -> crate::editor::keymap::BindMode {
    match mode {
        BindMode::Normal => crate::editor::keymap::BindMode::Normal,
        BindMode::Extend => crate::editor::keymap::BindMode::Extend,
        BindMode::Insert => crate::editor::keymap::BindMode::Insert,
    }
}
