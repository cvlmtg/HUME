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
use engine::pipeline::{BufferId, PaneId};

use crate::host::{BindMode, EditorHost};

pub(crate) struct NullHost;

impl EditorHost for NullHost {
    fn buffer_ids(&self) -> Vec<BufferId> { vec![] }
    fn pane_ids(&self) -> Vec<PaneId> { vec![] }
    fn buffer_exists(&self, _id: BufferId) -> bool { false }
    fn buffer_path(&self, _id: BufferId) -> Option<PathBuf> { None }
    fn buffer_display_name(&self, _id: BufferId) -> Option<String> { None }
    fn buffer_is_dirty(&self, _id: BufferId) -> Option<bool> { None }
    fn buffer_stored_language(&self, _id: BufferId) -> Option<String> { None }
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
    fn configure_statusline(&mut self, _l: Vec<String>, _c: Vec<String>, _r: Vec<String>) -> Result<(), String> {
        Err("NullHost: configure_statusline not available".into())
    }
    fn bind_key(&mut self, _mode: BindMode, _keys: &[KeyEvent], _cmd: &str, _fe: bool) -> Result<(), String> {
        Err("NullHost: bind_key not available".into())
    }
    fn bind_wait_char(&mut self, _mode: BindMode, _keys: &[KeyEvent], _cmd: &str) -> Result<(), String> {
        Err("NullHost: bind_wait_char not available".into())
    }
    fn unbind_key(&mut self, _mode: BindMode, _keys: &[KeyEvent]) -> Result<(), String> {
        Err("NullHost: unbind_key not available".into())
    }
    fn attach_grammar(&mut self, _name: &str, _gp: &Path, _sym: &str, _hl: &Path) -> Result<(), String> {
        Err("NullHost: attach_grammar not available".into())
    }
    fn has_grammar(&self, _language: &str) -> bool { false }
    fn is_valid_register_name(&self, _ch: char) -> bool { false }
    fn steel_command_budget_ms(&self) -> u64 { 10_000 }
    fn command_is_native(&self, _name: &str) -> Result<bool, String> {
        // No registry — treat every command as Steel/forward-raw.
        Ok(false)
    }
    fn run_command_sync(&mut self, _name: &str, _count: usize, _extend: bool) -> Result<(), String> {
        Err("stub host has no native command registry".into())
    }
    fn current_line_number(&self) -> Option<usize> { None }
    fn cursor_char_index(&self) -> Option<usize> { None }
}
