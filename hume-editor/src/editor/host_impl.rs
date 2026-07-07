//! [`EditorHostImpl`] — the production implementation of the scripting crate's
//! [`EditorHost`] trait.
//!
//! Holds disjoint borrows of `EditorState` and `EngineView`, which enables the
//! Steel VM (`scripting.steel`) to take `&mut Engine` simultaneously without
//! aliasing editor data. Two construction sites create this:
//!
//! - **Command dispatch** (`editor/mod.rs`, `run_steel_command`): called with the
//!   live editor state and view from the focused pane.
//! - **Init dispatch** (`scripting_setup.rs`): called with the same fields
//!   during `init_scripting`; init-only builtins set settings/keymap.

use std::path::{Path, PathBuf};

use crossterm::event::KeyEvent;
use hume_engine::pipeline::{BufferId, EngineView, PaneId};

use crate::editor::registry::MappableCommand;
use crate::settings::{BufferOverrides, SettingScope, apply_setting};
use crate::ui::statusline::{StatusElement, StatusLineConfig};
use hume_scripting::host::{BindMode, EditorHost};

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

    /// Seeded pane-buffer state for the focused (pane, buffer), or `None` when
    /// unseeded (stale or never-focused ids) — the shared guard behind every
    /// live cursor/selection read.
    fn focused_pane_buffer_state(
        &self,
    ) -> Option<(BufferId, &crate::editor::pane_state::PaneBufferState)> {
        let buf_id = crate::editor::commands::focused_buffer_id(self.state, self.view);
        let pbs = self
            .state
            .panes
            .state
            .get(self.state.focused_pane_id)?
            .get(buf_id)?;
        Some((buf_id, pbs))
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
        let canonical = hume_platform::fs::canonicalize(path)
            .map_err(|e| format!("open-buffer!: {}: {e}", path.display()))?;
        let (bid, _) = crate::editor::buffer::lifecycle::open_or_dedup(
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
        Ok(crate::editor::buffer::lifecycle::close_buffer(
            self.view,
            &mut self.state.buffers,
            &mut self.state.panes.state,
            &mut self.state.panes.jumps,
            self.state.focused_pane_id,
            id,
        ))
    }
    fn switch_to_buffer(&mut self, current: BufferId, target: BufferId) -> Result<(), String> {
        crate::editor::buffer::lifecycle::switch_to_buffer_with_jump(
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
        apply_setting(
            SettingScope::Global,
            key,
            value,
            &mut self.state.settings,
            &mut dummy,
        )
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
                .map(|s| {
                    s.parse::<StatusElement>()
                        .map_err(|e| format!("configure-statusline! {section}: {e}"))
                })
                .collect()
        };
        let left = parse(left, "left")?;
        let center = parse(center, "center")?;
        let right = parse(right, "right")?;
        self.state.settings.statusline = StatusLineConfig {
            left,
            center,
            right,
        };
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
        self.state
            .keymap
            .unbind_user(to_editor_bind_mode(mode), keys);
        Ok(())
    }

    // ── Language / grammar ────────────────────────────────────────────────────
    fn attach_grammar(
        &mut self,
        name: &str,
        grammar_path: &Path,
        symbol: &str,
        highlights_path: &Path,
        injections_path: Option<&Path>,
    ) -> Result<(), String> {
        self.state
            .languages
            .attach_grammar(
                name,
                grammar_path,
                symbol,
                highlights_path,
                injections_path,
                &mut self.view.registry,
            )
            .map_err(|e| format!("register-grammar! '{name}': {e}"))?;
        Ok(())
    }
    fn has_grammar(&self, language: &str) -> bool {
        self.state.languages.has_grammar(language)
    }

    // ── Command registration (init-only) ────────────────────────────────────
    fn register_command(&mut self, def: hume_scripting::SteelCmdDef) -> Result<(), String> {
        match self.state.registry.get_mappable(&def.name) {
            Some(MappableCommand::Lazy { .. }) | None => {
                self.state.registry.register(MappableCommand::SteelBacked {
                    name: def.name.into(),
                    doc: def.doc.into(),
                    arity: def.arity,
                    is_variadic: def.is_variadic,
                    inline_output: def.inline_output,
                    repeatable: def.repeatable,
                });
                Ok(())
            }
            Some(_) => Err(format!(
                "define-command!: '{}' conflicts with existing command",
                def.name
            )),
        }
    }

    fn unregister_command(&mut self, name: &str) {
        self.state.registry.unregister(name);
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

    fn run_command_sync(
        &mut self,
        name: &str,
        count: usize,
        extend: bool,
        register: Option<char>,
    ) -> Result<(), String> {
        let Some(cmd) = self.state.registry.get_mappable(name).cloned() else {
            return Err(format!("unknown command: {name}"));
        };
        if !cmd.is_native() {
            return Err(format!(
                "{name} is not a native command — use call! instead of call-native!"
            ));
        }
        // Arm the register prefix so register-aware commands (yank, delete,
        // paste-after, …) route to the right destination.
        if let Some(r) = register {
            self.state.register_prefix = Some(crate::editor::RegisterPrefix::Selected(r));
        }
        // Delegate to the shared pipeline — all bookkeeping (paste session, jump
        // list, dot-repeat, last_command) lives there so the sync path is
        // identical to the keypress path.
        crate::editor::commands::run_dispatch_pipeline(
            self.state,
            self.view,
            cmd,
            crate::editor::CmdCtx {
                count,
                extend,
                steel_args: vec![],
            },
        );
        // Clear the prefix when we armed it, so it does not bleed into the
        // next interactive command.
        if register.is_some() {
            self.state.register_prefix = None;
        }
        Ok(())
    }

    // ── Live cursor read ─────────────────────────────────────────────────────
    fn current_line_number(&self) -> Option<usize> {
        let (_, pbs) = self.focused_pane_buffer_state()?;
        self.char_index_to_line(pbs.selections.primary().head())
    }

    fn current_selections(&self) -> Option<Vec<(usize, usize, bool)>> {
        let (_, pbs) = self.focused_pane_buffer_state()?;
        let primary_index = pbs.selections.primary_index();
        Some(
            pbs.selections
                .iter_sorted()
                .enumerate()
                .map(|(i, sel)| (sel.anchor(), sel.head(), i == primary_index))
                .collect(),
        )
    }

    fn char_index_to_line(&self, idx: usize) -> Option<usize> {
        let buf_id = crate::editor::commands::focused_buffer_id(self.state, self.view);
        let text = self.buffer(buf_id)?.text();
        if idx > text.len_chars() {
            return None;
        }
        Some(text.char_to_line(idx) + 1)
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

    use hume_engine::pipeline::BufferId;
    use hume_scripting::host::EditorHost;

    use crate::editor::Editor;
    use crate::editor::scripting_setup::make_init_host;

    #[test]
    fn close_buffer_errs_when_id_unknown() {
        let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
            hume_editing::text::Text::empty(),
            hume_editing::selection::SelectionSet::default(),
        ));
        let mut host = make_init_host(&mut ed.state, &mut ed.view);
        // BufferId::default() is a zeroed key — not present in any live store.
        let err = host.close_buffer(BufferId::default()).unwrap_err();
        assert!(!err.is_empty(), "expected an error message");
    }

    #[test]
    fn switch_to_buffer_noop_when_same() {
        let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
            hume_editing::text::Text::empty(),
            hume_editing::selection::SelectionSet::default(),
        ));
        let bid = ed.focused_buffer_id();
        let mut host = make_init_host(&mut ed.state, &mut ed.view);
        // Switching to the same buffer should not error.
        host.switch_to_buffer(bid, bid).expect("same-buffer switch");
    }

    #[test]
    fn attach_grammar_errs_for_bad_path() {
        let mut ed = Editor::for_testing(crate::editor::buffer::Buffer::new(
            hume_editing::text::Text::empty(),
            hume_editing::selection::SelectionSet::default(),
        ));
        let mut host = make_init_host(&mut ed.state, &mut ed.view);
        let err = host
            .attach_grammar(
                "rust",
                Path::new("/no/such/lib.dylib"),
                "rust_language",
                Path::new("/no/such/highlights.scm"),
                None,
            )
            .unwrap_err();
        assert!(
            err.contains("register-grammar!"),
            "unexpected message: {err}"
        );
    }
}
