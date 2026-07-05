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

use std::path::{Path, PathBuf};

use crossterm::event::KeyEvent;
use hume_engine::pipeline::{BufferId, PaneId};

use crate::host::{BindMode, EditorHost};
use crate::types::SteelCmdDef;

pub(crate) struct NullHost;

impl EditorHost for NullHost {
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
    fn set_global_option(&mut self, _key: &str, _value: &str) -> Result<(), String> {
        Err("NullHost: set_global_option not available".into())
    }
    fn configure_statusline(
        &mut self,
        _l: Vec<String>,
        _c: Vec<String>,
        _r: Vec<String>,
    ) -> Result<(), String> {
        Err("NullHost: configure_statusline not available".into())
    }
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
    fn is_valid_register_name(&self, _ch: char) -> bool {
        false
    }
    fn steel_command_budget_ms(&self) -> u64 {
        10_000
    }
    fn command_is_native(&self, _name: &str) -> Result<bool, String> {
        // No registry — treat every command as Steel/forward-raw.
        Ok(false)
    }
    fn run_command_sync(
        &mut self,
        _name: &str,
        _count: usize,
        _extend: bool,
        _register: Option<char>,
    ) -> Result<(), String> {
        Err("stub host has no native command registry".into())
    }
    fn register_command(&mut self, _def: SteelCmdDef) -> Result<(), String> {
        Ok(())
    }
    fn unregister_command(&mut self, _name: &str) {}
    fn current_line_number(&self) -> Option<usize> {
        None
    }
    fn cursor_char_index(&self) -> Option<usize> {
        None
    }
}

/// Like [`NullHost`] but `register_command` fails.
///
/// Exercises the `define-command!` path where the editor-side registry rejects
/// the name (e.g. it shadows a native command): the builtin must propagate the
/// error *without* recording the command in `command_table`/`cmd_owners`.
pub(crate) struct FailingRegisterHost;

impl EditorHost for FailingRegisterHost {
    fn buffer_ids(&self) -> Vec<BufferId> {
        NullHost.buffer_ids()
    }
    fn pane_ids(&self) -> Vec<PaneId> {
        NullHost.pane_ids()
    }
    fn buffer_exists(&self, id: BufferId) -> bool {
        NullHost.buffer_exists(id)
    }
    fn buffer_path(&self, id: BufferId) -> Option<PathBuf> {
        NullHost.buffer_path(id)
    }
    fn buffer_display_name(&self, id: BufferId) -> Option<String> {
        NullHost.buffer_display_name(id)
    }
    fn buffer_is_dirty(&self, id: BufferId) -> Option<bool> {
        NullHost.buffer_is_dirty(id)
    }
    fn buffer_stored_language(&self, id: BufferId) -> Option<String> {
        NullHost.buffer_stored_language(id)
    }
    fn open_buffer(&mut self, path: &Path) -> Result<BufferId, String> {
        NullHost.open_buffer(path)
    }
    fn close_buffer(&mut self, id: BufferId) -> Result<BufferId, String> {
        NullHost.close_buffer(id)
    }
    fn switch_to_buffer(&mut self, current: BufferId, target: BufferId) -> Result<(), String> {
        NullHost.switch_to_buffer(current, target)
    }
    fn set_global_option(&mut self, key: &str, value: &str) -> Result<(), String> {
        NullHost.set_global_option(key, value)
    }
    fn configure_statusline(
        &mut self,
        l: Vec<String>,
        c: Vec<String>,
        r: Vec<String>,
    ) -> Result<(), String> {
        NullHost.configure_statusline(l, c, r)
    }
    fn bind_key(
        &mut self,
        mode: BindMode,
        keys: &[KeyEvent],
        cmd: &str,
        fe: bool,
    ) -> Result<(), String> {
        NullHost.bind_key(mode, keys, cmd, fe)
    }
    fn bind_wait_char(
        &mut self,
        mode: BindMode,
        keys: &[KeyEvent],
        cmd: &str,
    ) -> Result<(), String> {
        NullHost.bind_wait_char(mode, keys, cmd)
    }
    fn unbind_key(&mut self, mode: BindMode, keys: &[KeyEvent]) -> Result<(), String> {
        NullHost.unbind_key(mode, keys)
    }
    fn attach_grammar(
        &mut self,
        name: &str,
        gp: &Path,
        sym: &str,
        hl: &Path,
        inj: Option<&Path>,
    ) -> Result<(), String> {
        NullHost.attach_grammar(name, gp, sym, hl, inj)
    }
    fn has_grammar(&self, language: &str) -> bool {
        NullHost.has_grammar(language)
    }
    fn is_valid_register_name(&self, ch: char) -> bool {
        NullHost.is_valid_register_name(ch)
    }
    fn steel_command_budget_ms(&self) -> u64 {
        NullHost.steel_command_budget_ms()
    }
    fn command_is_native(&self, name: &str) -> Result<bool, String> {
        NullHost.command_is_native(name)
    }
    fn run_command_sync(
        &mut self,
        name: &str,
        count: usize,
        extend: bool,
        register: Option<char>,
    ) -> Result<(), String> {
        NullHost.run_command_sync(name, count, extend, register)
    }
    fn register_command(&mut self, def: SteelCmdDef) -> Result<(), String> {
        Err(format!(
            "FailingRegisterHost: '{}' rejected by the command registry",
            def.name
        ))
    }
    fn unregister_command(&mut self, name: &str) {
        NullHost.unregister_command(name)
    }
    fn current_line_number(&self) -> Option<usize> {
        NullHost.current_line_number()
    }
    fn cursor_char_index(&self) -> Option<usize> {
        NullHost.cursor_char_index()
    }
}
