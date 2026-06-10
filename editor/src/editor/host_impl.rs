//! [`EditorHostImpl`] — the production implementation of the scripting crate's
//! [`EditorHost`] trait.
//!
//! Holds disjoint borrows of `EditorState` and `EngineView`, which enables the
//! Steel VM (`scripting.steel`) to take `&mut Engine` simultaneously without
//! aliasing editor data. Two construction sites create this:
//!
//! - **Command dispatch** (`mappings/execute.rs`): called with the live editor
//!   state and view from the focused pane.
//! - **Init dispatch** (`scripting_setup.rs`): called with the same fields
//!   during `init_scripting`; init-only builtins set settings/keymap.

use std::path::{Path, PathBuf};

use crossterm::event::KeyEvent;
use engine::pipeline::{BufferId, EngineView, PaneId};

use crate::editor::doc_ops;
use crate::editor::registry::MappableCommand;
use crate::ops::MotionMode;
use crate::settings::{BufferOverrides, SettingScope, apply_setting};
use crate::ui::statusline::{StatusElement, StatusLineConfig};
use scripting::host::{BindMode, EditorHost};

use super::EditorState;

pub(crate) struct EditorHostImpl<'a> {
    pub(crate) state: &'a mut EditorState,
    pub(crate) view: &'a mut EngineView,
}

impl<'a> EditorHostImpl<'a> {
    /// Look up a buffer by id.
    fn buffer(&self, id: BufferId) -> Option<&crate::editor::buffer::Buffer> {
        self.state.buffers.try_get(id)
    }

    /// Derive the focused buffer id from the live pane state.
    fn focused_buffer_id_live(&self) -> Option<BufferId> {
        Some(self.view.panes[self.state.focused_pane_id].buffer_id)
    }
}

impl<'a> EditorHost for EditorHostImpl<'a> {
    // ── Enumeration ──────────────────────────────────────────────────────────
    fn buffer_ids(&self) -> Vec<BufferId> {
        self.state.buffers.iter().map(|(id, _)| id).collect()
    }
    fn pane_ids(&self) -> Vec<PaneId> {
        self.view.panes.iter().map(|(id, _)| id).collect()
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
        let (bid, _) = crate::editor::ops::open_or_dedup(
            self.view,
            &mut self.state.buffers,
            &mut self.state.panes.state,
            self.state.focused_pane_id,
            &canonical,
        )
        .map_err(|e| format!("open-buffer!: {}: {e}", canonical.display()))?;
        Ok(bid)
    }
    fn close_buffer(&mut self, id: BufferId) -> Result<BufferId, String> {
        if self.state.buffers.try_get(id).is_none() {
            return Err(format!("close-buffer!: buffer {id:?} does not exist"));
        }
        Ok(crate::editor::ops::close_buffer(
            self.view,
            &mut self.state.buffers,
            &mut self.state.panes.state,
            &mut self.state.panes.jumps,
            self.state.focused_pane_id,
            id,
        ))
    }
    fn switch_to_buffer(&mut self, current: BufferId, target: BufferId) -> Result<(), String> {
        crate::editor::ops::switch_to_buffer_with_jump(
            self.view,
            &self.state.buffers,
            &mut self.state.panes.state,
            &mut self.state.panes.jumps,
            self.state.focused_pane_id,
            current,
            target,
        );
        Ok(())
    }

    // ── Settings ─────────────────────────────────────────────────────────────
    fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String> {
        let mut dummy = BufferOverrides::default();
        apply_setting(SettingScope::Global, key, value, &mut self.state.settings, &mut dummy)
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
        self.state.settings.statusline = StatusLineConfig { left, center, right };
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
        self.state.keymap.bind_user_with_extend(
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
        self.state.keymap.bind_wait_char_user(
            to_editor_bind_mode(mode),
            keys,
            std::borrow::Cow::Owned(cmd.to_owned()),
        );
        Ok(())
    }
    fn unbind_key(&mut self, mode: BindMode, keys: &[KeyEvent]) -> Result<(), String> {
        self.state.keymap.unbind_user(to_editor_bind_mode(mode), keys);
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
        self.state.languages
            .attach_grammar(name, grammar_path, symbol, highlights_path, &mut self.view.registry)
            .map_err(|e| format!("register-grammar! '{name}': {e}"))?;
        self.view.theme.bake(&self.view.registry);
        Ok(())
    }
    fn has_grammar(&self, language: &str) -> bool {
        self.state.languages.has_grammar(language)
    }

    // ── Register validation ───────────────────────────────────────────────────
    fn is_valid_register_name(&self, ch: char) -> bool {
        crate::ops::register::is_valid_register_name(ch)
    }

    // ── Budget ────────────────────────────────────────────────────────────────
    fn steel_command_budget_ms(&self) -> u64 {
        self.state.settings.steel_command_budget_ms as u64
    }

    // ── Synchronous command dispatch ─────────────────────────────────────────
    fn command_is_native(&self, name: &str) -> Result<bool, String> {
        self.state
            .registry
            .get_mappable(name)
            .map(MappableCommand::is_native)
            .ok_or_else(|| format!("unknown command: {name}"))
    }

    fn run_command_sync(&mut self, name: &str, count: usize, extend: bool) -> Result<(), String> {
        let Some(cmd) = self.state.registry.get_mappable(name).cloned() else {
            return Err(format!("unknown command: {name}"));
        };
        // Read focused buffer id from the live pane; the shared borrow ends at
        // the semicolon (NLL), before any mutable access to other fields.
        let buf_id = self.view.panes[self.state.focused_pane_id].buffer_id;
        let motion_mode = if extend { MotionMode::Extend } else { MotionMode::Move };
        match cmd {
            MappableCommand::Motion { fun, .. } => {
                doc_ops::apply_doc_motion(
                    &self.state.buffers,
                    &mut self.state.panes.state,
                    self.state.focused_pane_id,
                    buf_id,
                    |b, s| fun(b, s, count, motion_mode),
                );
                Ok(())
            }
            MappableCommand::Selection { fun, .. } => {
                doc_ops::apply_doc_motion(
                    &self.state.buffers,
                    &mut self.state.panes.state,
                    self.state.focused_pane_id,
                    buf_id,
                    |b, s| fun(b, s, motion_mode),
                );
                Ok(())
            }
            MappableCommand::Edit { fun, .. } => {
                doc_ops::apply_doc_edit(
                    &mut self.state.buffers,
                    &mut self.state.panes.state,
                    self.state.focused_pane_id,
                    buf_id,
                    fun,
                );
                Ok(())
            }
            MappableCommand::EditorCmd { fun, .. } => {
                if let Err(e) = fun(&mut self.state, &mut self.view, count, motion_mode) {
                    self.state.report(crate::editor::Severity::Error, e.message().to_owned());
                }
                Ok(())
            }
            // Non-native commands must be queued for Steel dispatch, never run here;
            // the %call! gate (command_is_native) guarantees we never reach this.
            MappableCommand::SteelBacked { .. } | MappableCommand::Lazy { .. } => {
                unreachable!("run_command_sync called on non-native command '{name}'; classify with command_is_native first")
            }
        }
    }

    // ── Live cursor/selection reads ──────────────────────────────────────────
    fn current_line_number(&self) -> Option<usize> {
        let buf_id = self.focused_buffer_id_live()?;
        let pbs = self.state.panes.state
            .get(self.state.focused_pane_id)?
            .get(buf_id)?;
        let head = pbs.selections.primary().head();
        Some(self.state.buffers.get(buf_id).text().rope().char_to_line(head) + 1)
    }

    fn cursor_char_index(&self) -> Option<usize> {
        let buf_id = self.focused_buffer_id_live()?;
        let pbs = self.state.panes.state
            .get(self.state.focused_pane_id)?
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

    use crate::editor::scripting_setup::make_init_host;
    use crate::editor::Editor;

    #[test]
    fn close_buffer_errs_when_id_unknown() {
        let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
            editing::text::Text::empty(),
            editing::selection::SelectionSet::default(),
        ));
        let mut host = make_init_host(&mut ed.state, &mut ed.view);
        // BufferId::default() is a zeroed key — not present in any live store.
        let err = host.close_buffer(BufferId::default()).unwrap_err();
        assert!(!err.is_empty(), "expected an error message");
    }

    #[test]
    fn switch_to_buffer_noop_when_same() {
        let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
            editing::text::Text::empty(),
            editing::selection::SelectionSet::default(),
        ));
        let bid = ed.focused_buffer_id();
        let mut host = make_init_host(&mut ed.state, &mut ed.view);
        // Switching to the same buffer should not error.
        host.switch_to_buffer(bid, bid).expect("same-buffer switch");
    }

    #[test]
    fn attach_grammar_errs_for_bad_path() {
        let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
            editing::text::Text::empty(),
            editing::selection::SelectionSet::default(),
        ));
        let mut host = make_init_host(&mut ed.state, &mut ed.view);
        let err = host
            .attach_grammar("rust", Path::new("/no/such/lib.dylib"), "rust_language", Path::new("/no/such/highlights.scm"))
            .unwrap_err();
        assert!(err.contains("register-grammar!"), "unexpected message: {err}");
    }
}
