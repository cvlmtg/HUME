//! [`NullHost`] — a minimal no-op [`EditorHost`] for scripting crate unit tests.
//!
//! Does not depend on any editor types; returns empty / default / error values
//! for all methods.  Sufficient for tests that only verify scripting guards
//! (e.g. `is_init` checks) without needing real editor state.

use std::path::{Path, PathBuf};

use crossterm::event::KeyEvent;
use engine::pipeline::{BufferId, PaneId};

use crate::host::{BindMode, EditorHost};

pub(crate) struct NullHost;

impl EditorHost for NullHost {
    fn focused_buffer_id(&self) -> BufferId { BufferId::default() }
    fn focused_pane_id(&self) -> PaneId { PaneId::default() }
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
    fn close_buffer(&mut self, _id: BufferId) -> BufferId { BufferId::default() }
    fn switch_to_buffer(&mut self, _current: BufferId, _target: BufferId) {}
    fn set_global_option(&mut self, _key: &str, _value: &str) -> Result<(), String> { Ok(()) }
    fn configure_statusline(&mut self, _l: Vec<String>, _c: Vec<String>, _r: Vec<String>) -> Result<(), String> { Ok(()) }
    fn bind_key(&mut self, _mode: BindMode, _keys: &[KeyEvent], _cmd: &str, _fe: bool) -> Result<(), String> { Ok(()) }
    fn bind_wait_char(&mut self, _mode: BindMode, _keys: &[KeyEvent], _cmd: &str) -> Result<(), String> { Ok(()) }
    fn unbind_key(&mut self, _mode: BindMode, _keys: &[KeyEvent]) -> Result<(), String> { Ok(()) }
    fn attach_grammar(&mut self, _name: &str, _gp: &Path, _sym: &str, _hl: &Path) -> Result<(), String> { Ok(()) }
    fn has_grammar(&self, _language: &str) -> bool { false }
    fn is_valid_register_name(&self, _ch: char) -> bool { false }
    fn steel_init_budget_ms(&self) -> u64 { 10_000 }
    fn steel_command_budget_ms(&self) -> u64 { 10_000 }
}
