//! [`NullHost`] — a minimal [`EditorHost`] for scripting crate unit tests.
//!
//! Does not depend on any editor types.  Read methods return empty / default
//! values; **all mutators return `Err`** — so any test that accidentally drives
//! a mutating builtin through NullHost fails loudly instead of silently succeeding.
//!
//! Suitable only for guard tests (init-guard, activation-state, budget, register
//! validation) where the host mutators are never reached.  Tests that need working
//! mutations (bind-key!, set-option!, attach-grammar!, …) must use `MockHost`
//! in the editor crate.
//!
//! [`FailingRegisterHost`], [`InlineOutputHost`], and [`RecordingInlineOutputHost`]
//! each embed a real `NullHost` and delegate every capability accessor to it,
//! overriding only the one or two accessors/methods that make them distinct.

use std::path::{Path, PathBuf};

use crossterm::event::KeyEvent;
use hume_engine::pipeline::{BufferId, PaneId};

use crate::host::{
    BindMode, BufferHost, CommandHost, CursorHost, EditorHost, KeymapHost, LanguageHost,
    OptionValue, OutputHost, SettingsHost,
};
use crate::types::SteelCmdDef;

#[derive(Default)]
pub(crate) struct NullHost;

impl EditorHost for NullHost {
    fn cursor(&mut self) -> &mut dyn CursorHost {
        self
    }
    fn commands(&mut self) -> &mut dyn CommandHost {
        self
    }
    fn language(&mut self) -> &mut dyn LanguageHost {
        self
    }
    fn keymap(&mut self) -> &mut dyn KeymapHost {
        self
    }
    fn settings(&mut self) -> &mut dyn SettingsHost {
        self
    }
    fn buffers(&mut self) -> &mut dyn BufferHost {
        self
    }
}

impl BufferHost for NullHost {
    fn buffer_ids(&self) -> Vec<BufferId> {
        vec![]
    }
    fn pane_ids(&self) -> Vec<PaneId> {
        vec![]
    }
    fn buffer_exists(&self, _id: BufferId) -> bool {
        false
    }
    fn buffer_path(&self, _id: BufferId) -> Option<PathBuf> {
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
    fn open_buffer(&mut self, _path: &Path) -> Result<BufferId, String> {
        Err("NullHost: open_buffer not available".into())
    }
    fn close_buffer(&mut self, _id: BufferId) -> Result<BufferId, String> {
        Err("NullHost: close_buffer not available".into())
    }
    fn switch_to_buffer(&mut self, _current: BufferId, _target: BufferId) -> Result<(), String> {
        Err("NullHost: switch_to_buffer not available".into())
    }
    fn buffer_generation(&self, _id: BufferId) -> Option<u64> {
        None
    }
    fn viewport_range(&self, _id: BufferId) -> Option<(usize, usize)> {
        None
    }
}

impl SettingsHost for NullHost {
    fn set_global_option(&mut self, _key: &str, _value: &str) -> Result<(), String> {
        Err("NullHost: set_global_option not available".into())
    }
    fn get_option(&self, _key: &str, _bid: BufferId) -> Result<OptionValue, String> {
        Err("NullHost: get_option not available".into())
    }
    fn configure_statusline(
        &mut self,
        _l: Vec<String>,
        _c: Vec<String>,
        _r: Vec<String>,
    ) -> Result<(), String> {
        Err("NullHost: configure_statusline not available".into())
    }
    fn steel_command_budget_ms(&self) -> u64 {
        10_000
    }
}

impl KeymapHost for NullHost {
    fn bind_key(
        &mut self,
        _mode: BindMode,
        _keys: &[KeyEvent],
        _cmd: &str,
        _fe: bool,
    ) -> Result<(), String> {
        Err("NullHost: bind_key not available".into())
    }
    fn bind_wait_char(
        &mut self,
        _mode: BindMode,
        _keys: &[KeyEvent],
        _cmd: &str,
    ) -> Result<(), String> {
        Err("NullHost: bind_wait_char not available".into())
    }
    fn unbind_key(&mut self, _mode: BindMode, _keys: &[KeyEvent]) -> Result<(), String> {
        Err("NullHost: unbind_key not available".into())
    }
}

impl LanguageHost for NullHost {
    fn attach_grammar(
        &mut self,
        _name: &str,
        _gp: &Path,
        _sym: &str,
        _hl: &Path,
        _inj: Option<&Path>,
    ) -> Result<(), String> {
        Err("NullHost: attach_grammar not available".into())
    }
    fn has_grammar(&self, _language: &str) -> bool {
        false
    }
    fn register_trigger_chars(&mut self, _source: String, _language: String, _chars: Vec<char>) {}
}

impl CommandHost for NullHost {
    fn is_valid_register_name(&self, _ch: char) -> bool {
        false
    }
    fn command_is_native(&self, _name: &str) -> Result<bool, String> {
        // No registry — treat every command as Steel/forward-raw.
        Ok(false)
    }
    fn run_command_sync(
        &mut self,
        _name: &str,
        _count: Option<usize>,
        _extend: bool,
        _register: Option<char>,
    ) -> Result<(), String> {
        Err("stub host has no native command registry".into())
    }
    fn register_command(&mut self, _def: SteelCmdDef) -> Result<(), String> {
        Ok(())
    }
    fn unregister_command(&mut self, _name: &str) {}
}

impl CursorHost for NullHost {
    fn current_line_number(&self) -> Option<usize> {
        None
    }
    fn current_selections(&self) -> Option<Vec<(usize, usize, bool)>> {
        None
    }
    fn char_index_to_line(&self, _idx: usize) -> Option<usize> {
        None
    }
    fn symbol_under_cursor(&self, _bid: BufferId) -> String {
        String::new()
    }
    fn selection_spans_full_line(&self, _bid: BufferId) -> bool {
        false
    }
}

/// Like [`NullHost`] but `register_command` fails.
///
/// Exercises the `define-command!` path where the editor-side registry rejects
/// the name (e.g. it shadows a native command): the builtin must propagate the
/// error *without* recording the command in `command_table`/`cmd_owners`.
#[derive(Default)]
pub(crate) struct FailingRegisterHost {
    inner: NullHost,
}

impl EditorHost for FailingRegisterHost {
    fn cursor(&mut self) -> &mut dyn CursorHost {
        &mut self.inner
    }
    fn commands(&mut self) -> &mut dyn CommandHost {
        self
    }
    fn language(&mut self) -> &mut dyn LanguageHost {
        &mut self.inner
    }
    fn keymap(&mut self) -> &mut dyn KeymapHost {
        &mut self.inner
    }
    fn settings(&mut self) -> &mut dyn SettingsHost {
        &mut self.inner
    }
    fn buffers(&mut self) -> &mut dyn BufferHost {
        &mut self.inner
    }
}

impl CommandHost for FailingRegisterHost {
    fn is_valid_register_name(&self, ch: char) -> bool {
        self.inner.is_valid_register_name(ch)
    }
    fn command_is_native(&self, name: &str) -> Result<bool, String> {
        self.inner.command_is_native(name)
    }
    fn run_command_sync(
        &mut self,
        name: &str,
        count: Option<usize>,
        extend: bool,
        register: Option<char>,
    ) -> Result<(), String> {
        self.inner.run_command_sync(name, count, extend, register)
    }
    fn register_command(&mut self, def: SteelCmdDef) -> Result<(), String> {
        Err(format!(
            "FailingRegisterHost: '{}' rejected by the command registry",
            def.name
        ))
    }
    fn unregister_command(&mut self, name: &str) {
        self.inner.unregister_command(name)
    }
}

/// Like [`NullHost`] but reports `is_inline_output_command() == true`.
///
/// Exercises the `SteelCtx::new_command` wiring that reads the flag off the
/// host (see `context.rs` tests) without pulling in the editor crate's real
/// `EditorHostImpl`.
#[derive(Default)]
pub(crate) struct InlineOutputHost {
    inner: NullHost,
}

impl EditorHost for InlineOutputHost {
    fn cursor(&mut self) -> &mut dyn CursorHost {
        &mut self.inner
    }
    fn commands(&mut self) -> &mut dyn CommandHost {
        &mut self.inner
    }
    fn language(&mut self) -> &mut dyn LanguageHost {
        &mut self.inner
    }
    fn keymap(&mut self) -> &mut dyn KeymapHost {
        &mut self.inner
    }
    fn settings(&mut self) -> &mut dyn SettingsHost {
        &mut self.inner
    }
    fn buffers(&mut self) -> &mut dyn BufferHost {
        &mut self.inner
    }
    fn output(&mut self) -> Option<&mut dyn OutputHost> {
        Some(self)
    }
}

impl OutputHost for InlineOutputHost {
    fn is_inline_output_command(&self) -> bool {
        true
    }
    fn ensure_inline_output_screen(&mut self) -> Result<(), String> {
        Ok(())
    }
}

/// Like [`InlineOutputHost`] but also counts calls to
/// `ensure_inline_output_screen` — lets a test assert a builtin opens the
/// inline-output bracket exactly when (and only when) it has real terminal
/// output to produce, without a real terminal.
#[derive(Default)]
pub(crate) struct RecordingInlineOutputHost {
    inner: NullHost,
    pub(crate) ensure_calls: usize,
}

impl EditorHost for RecordingInlineOutputHost {
    fn cursor(&mut self) -> &mut dyn CursorHost {
        &mut self.inner
    }
    fn commands(&mut self) -> &mut dyn CommandHost {
        &mut self.inner
    }
    fn language(&mut self) -> &mut dyn LanguageHost {
        &mut self.inner
    }
    fn keymap(&mut self) -> &mut dyn KeymapHost {
        &mut self.inner
    }
    fn settings(&mut self) -> &mut dyn SettingsHost {
        &mut self.inner
    }
    fn buffers(&mut self) -> &mut dyn BufferHost {
        &mut self.inner
    }
    fn output(&mut self) -> Option<&mut dyn OutputHost> {
        Some(self)
    }
}

impl OutputHost for RecordingInlineOutputHost {
    fn is_inline_output_command(&self) -> bool {
        true
    }
    fn ensure_inline_output_screen(&mut self) -> Result<(), String> {
        self.ensure_calls += 1;
        Ok(())
    }
}
