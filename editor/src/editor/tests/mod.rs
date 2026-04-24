// Shared imports and harness helpers used by all test submodules.
// Each submodule does `use super::*;` to access these.

use std::path::PathBuf;
use std::sync::Mutex;

use crate::core::selection::SelectionSet;
use crate::core::text::Text;
use crate::editor::SearchDirection;
use crate::editor::buffer::Buffer;
use crate::testing::{parse_state, serialize_state};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{Editor, Mode};

// ── Harness ───────────────────────────────────────────────────────────────────

/// Build an Editor pre-loaded with the given state string (same DSL as other tests).
fn editor_from(input: &str) -> Editor {
    let (buf, sels) = parse_state(input);
    Editor::for_testing(Buffer::new(buf, sels))
}

/// Build a kitty-protocol-enabled editor for testing Ctrl+motion bindings.
fn editor_from_kitty(input: &str) -> Editor {
    let mut ed = editor_from(input);
    ed.kitty_enabled = true;
    ed
}

/// Serialize the editor's current buffer + selection state.
fn state(ed: &Editor) -> String {
    serialize_state(ed.doc().text(), ed.current_selections())
}

/// A normal (no modifier) character key event.
fn key(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
}

fn key_esc() -> KeyEvent {
    KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
}

fn key_ctrl(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
}

fn key_enter() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
}

fn key_up() -> KeyEvent {
    KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)
}

fn key_down() -> KeyEvent {
    KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
}

fn key_tab() -> KeyEvent {
    KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
}

fn key_backspace() -> KeyEvent {
    KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)
}

/// Type a colon command into the editor via `handle_key`, going through the
/// mini-buffer path (and thus `%`/`#` expansion). Useful when testing typed
/// commands that must be verified end-to-end through the keymap dispatcher.
fn type_cmd(ed: &mut Editor, cmd: &str) {
    for ch in cmd.chars() {
        ed.handle_key(key(ch));
    }
    ed.handle_key(key_enter());
}

fn reg(ed: &Editor, name: char) -> Vec<String> {
    ed.registers
        .read(name)
        .and_then(|r| r.as_text())
        .unwrap_or_default()
        .to_vec()
}

/// Build a 20-line buffer with the cursor on a given line for jump list tests.
fn jump_editor(cursor_line: usize) -> Editor {
    let text: String = (0..20).map(|i| format!("line {i}\n")).collect();
    let buf = Text::from(text.as_str());
    let pos = buf.line_to_char(cursor_line);
    let sels = SelectionSet::single(crate::core::selection::Selection::collapsed(pos));
    let doc = Buffer::new(buf, sels);
    let mut ed = Editor::for_testing(doc);
    ed.mode = Mode::Normal;
    ed
}

/// Write `file_content` to a temp file, return an editor pointing at it.
fn editor_with_file(initial_state: &str, file_content: &str) -> (Editor, tempfile::TempPath) {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), file_content).unwrap();
    let path = tmp.path().to_path_buf();
    let tmp_path = tmp.into_temp_path();
    let (_, meta) = crate::os::io::read_file(&path).unwrap();
    let mut ed = editor_from(initial_state);
    ed.doc_mut().set_path(Some(path));
    ed.doc_mut().file_meta = Some(meta);
    (ed, tmp_path)
}

// ── cwd guard ─────────────────────────────────────────────────────────────────

// Process cwd is global state. Any test that calls `set_current_dir` must hold
// this mutex for its entire duration so tests do not race on cwd.
static CWD_MUTEX: Mutex<()> = Mutex::new(());

/// Acquire the cwd lock, save the current directory, and restore it on drop.
struct CwdGuard {
    saved: PathBuf,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl CwdGuard {
    fn new() -> Self {
        let lock = CWD_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let saved = std::env::current_dir().expect("current_dir");
        CwdGuard { saved, _lock: lock }
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.saved);
    }
}

mod alternate;
mod auto_pairs;
mod buffer;
mod buffer_store;
mod cd;
mod command_mode;
mod commands;
mod completion;
mod dot_repeat;
mod file_io;
mod find;
mod hooks;
mod jump_list;
mod kitty;
mod list_buffers;
mod macros;
mod multi_pane;
mod page_scroll;
mod pane_sync;
mod per_pane_jumps;
mod search;
mod select_all;
mod surround;
mod visual_move;
