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
use crate::editor::doc_ops;
use crate::editor::keymap::Keymap;
use crate::editor::pane_state::PaneBufferState;
use crate::editor::registry::{CommandRegistry, MappableCommand};
use crate::editor::syntax::LanguageRegistry;
use crate::ops::MotionMode;
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
    /// Read-only command registry for synchronous dispatch via `run_command_sync`.
    /// `None` during init evals where sync dispatch is unreachable via `require_cmd_ctx!`.
    pub(crate) registry: Option<&'a CommandRegistry>,
}

impl<'a> EditorHostImpl<'a> {
    /// Look up a buffer by id, or `None` if the buffers ref is unavailable
    /// or the id is stale/unknown.
    fn buffer(&self, id: BufferId) -> Option<&Buffer> {
        self.buffers.as_ref()?.try_get(id)
    }

    /// Derive the focused buffer id from the live pane state.
    fn focused_buffer_id_live(&self) -> Option<BufferId> {
        Some(self.engine_view.as_ref()?.panes[self.focused_pane_id].buffer_id)
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

    // ── Synchronous command dispatch ─────────────────────────────────────────
    fn run_command_sync(&mut self, name: &str, count: usize, extend: bool) -> Result<bool, String> {
        let registry = self.registry.ok_or_else(|| {
            "run-command-sync!: command registry unavailable (init mode)".to_owned()
        })?;
        let Some(cmd) = registry.get_mappable(name).cloned() else {
            return Err(format!("unknown command: {name}"));
        };
        // Read focused buffer id from the live pane; borrow ends before any
        // mutable access to other fields (NLL splits the borrows).
        let buf_id = {
            if let Some(ev) = &self.engine_view {
                ev.panes[self.focused_pane_id].buffer_id
            } else {
                return Err("run-command-sync!: editor refs unavailable".to_owned());
            }
        };
        let motion_mode = if extend { MotionMode::Extend } else { MotionMode::Move };
        match cmd {
            MappableCommand::Motion { fun, .. } => {
                let bufs = self.buffers.as_deref()
                    .ok_or_else(|| "run-command-sync!: editor refs unavailable".to_owned())?;
                let ps = self.pane_state.as_deref_mut()
                    .ok_or_else(|| "run-command-sync!: editor refs unavailable".to_owned())?;
                doc_ops::apply_doc_motion(bufs, ps, self.focused_pane_id, buf_id,
                    |b, s| fun(b, s, count, motion_mode));
                Ok(true)
            }
            MappableCommand::Selection { fun, .. } => {
                let bufs = self.buffers.as_deref()
                    .ok_or_else(|| "run-command-sync!: editor refs unavailable".to_owned())?;
                let ps = self.pane_state.as_deref_mut()
                    .ok_or_else(|| "run-command-sync!: editor refs unavailable".to_owned())?;
                doc_ops::apply_doc_motion(bufs, ps, self.focused_pane_id, buf_id,
                    |b, s| fun(b, s, motion_mode));
                Ok(true)
            }
            MappableCommand::Edit { fun, .. } => {
                let bufs = self.buffers.as_deref_mut()
                    .ok_or_else(|| "run-command-sync!: editor refs unavailable".to_owned())?;
                let ps = self.pane_state.as_deref_mut()
                    .ok_or_else(|| "run-command-sync!: editor refs unavailable".to_owned())?;
                doc_ops::apply_doc_edit(bufs, ps, self.focused_pane_id, buf_id, fun);
                Ok(true)
            }
            // EditorCmd / SteelBacked / Lazy — caller must queue for post-eval dispatch.
            _ => Ok(false),
        }
    }

    // ── Live cursor/selection reads ──────────────────────────────────────────
    fn current_line_number(&self) -> Option<usize> {
        let buf_id = self.focused_buffer_id_live()?;
        let pbs = self.pane_state.as_ref()?
            .get(self.focused_pane_id)?
            .get(buf_id)?;
        let head = pbs.selections.primary().head();
        // char_to_line is 0-indexed; add 1 for user-facing 1-indexed result.
        Some(self.buffers.as_ref()?.get(buf_id).text().rope().char_to_line(head) + 1)
    }

    fn cursor_char_index(&self) -> Option<usize> {
        let buf_id = self.focused_buffer_id_live()?;
        let pbs = self.pane_state.as_ref()?
            .get(self.focused_pane_id)?
            .get(buf_id)?;
        Some(pbs.selections.primary().head())
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::Path;

    use engine::pipeline::BufferId;
    use scripting::host::EditorHost;

    use super::*;
    use crate::editor::keymap::Keymap;
    use crate::editor::scripting_setup::make_init_host;
    use crate::settings::EditorSettings;

    // Build a host with all optional refs set to None — the same shape as the
    // init-mode host (`make_init_host`). In production, `require_cmd_ctx!` in
    // the Steel builtins prevents buffer-lifecycle methods from being called
    // this way. These tests exercise the defensive `Err` branches directly.
    fn none_host<'a>(settings: &'a mut EditorSettings, keymap: &'a mut Keymap) -> EditorHostImpl<'a> {
        make_init_host(settings, keymap)
    }

    #[test]
    fn close_buffer_errs_when_refs_unavailable() {
        let mut settings = EditorSettings::default();
        let mut keymap = Keymap::default();
        let mut host = none_host(&mut settings, &mut keymap);
        let err = host.close_buffer(BufferId::default()).unwrap_err();
        assert!(err.contains("editor refs unavailable"), "unexpected message: {err}");
    }

    #[test]
    fn switch_to_buffer_errs_when_refs_unavailable() {
        let mut settings = EditorSettings::default();
        let mut keymap = Keymap::default();
        let mut host = none_host(&mut settings, &mut keymap);
        let err = host
            .switch_to_buffer(BufferId::default(), BufferId::default())
            .unwrap_err();
        assert!(err.contains("editor refs unavailable"), "unexpected message: {err}");
    }

    #[test]
    fn attach_grammar_errs_when_langs_unavailable() {
        let mut settings = EditorSettings::default();
        let mut keymap = Keymap::default();
        let mut host = none_host(&mut settings, &mut keymap);
        let err = host
            .attach_grammar("rust", Path::new("/no"), "rust_language", Path::new("/no"))
            .unwrap_err();
        // language registry is None → "unavailable" error fires before path resolution
        assert!(err.contains("unavailable"), "unexpected message: {err}");
    }
}
