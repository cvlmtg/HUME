// Shared imports and harness helpers used by all test submodules.
// Each submodule does `use super::*;` to access these.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crate::core::selection::SelectionSet;
use crate::core::text::Text;
use crate::editor::buffer::Buffer;
use crate::editor::SearchDirection;
use crate::testing::{parse_state, serialize_state};

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

fn key_backspace() -> KeyEvent {
    KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)
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
    let sels = SelectionSet::single(
        crate::core::selection::Selection::collapsed(pos),
    );
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
    ed.doc_mut().path = Some(Arc::new(path));
    ed.doc_mut().file_meta = Some(meta);
    (ed, tmp_path)
}


mod commands;
mod command_mode;
mod file_io;
mod auto_pairs;
mod find;
mod kitty;
mod dot_repeat;
mod search;
mod select_all;
mod jump_list;
mod surround;
mod pane_sync;
mod visual_move;
mod page_scroll;
mod macros;
mod multi_pane;
mod buffer_store;
mod per_pane_jumps;
mod hooks;
