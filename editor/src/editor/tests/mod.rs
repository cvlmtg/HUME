// Shared imports and harness helpers used by all test submodules.
// Each submodule does `use super::*;` to access these.

use std::path::PathBuf;
use std::sync::Mutex;

use editing::selection::SelectionSet;
use editing::text::Text;
use crate::editor::SearchDirection;
use crate::editor::buffer::Buffer;
use crate::testing::{parse_state, serialize_state};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{Editor, Mode, Severity};

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

fn key_left() -> KeyEvent {
    KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)
}

/// Type a colon command into the editor via `handle_key`, going through the
/// mini-buffer path (and thus `%`/`#` expansion). Useful when testing typed
/// commands that must be verified end-to-end through the keymap dispatcher.
fn type_cmd(ed: &mut Editor, cmd: &str) {
    for ch in cmd.chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_enter());
}

fn reg(ed: &Editor, name: char) -> Vec<String> {
    ed.state.registers
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
    let sels = SelectionSet::single(editing::selection::Selection::collapsed(pos));
    let doc = Buffer::new(buf, sels);
    let mut ed = Editor::for_testing(doc);
    ed.state.mode = Mode::Normal;
    ed
}

/// Write `file_content` to a temp file, return an editor pointing at it.
fn editor_with_file(initial_state: &str, file_content: &str) -> (Editor, tempfile::TempPath) {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), file_content).unwrap();
    let path = tmp.path().to_path_buf();
    let tmp_path = tmp.into_temp_path();
    let (_, meta) = platform::io::read_file(&path).unwrap();
    let mut ed = editor_from(initial_state);
    ed.doc_mut().set_path(Some(path));
    ed.doc_mut().file_meta = Some(meta);
    (ed, tmp_path)
}

// ── cwd guard ─────────────────────────────────────────────────────────────────

// Process cwd is global state. Any test that calls `set_current_dir` must hold
// this mutex for its entire duration so tests do not race on cwd.
static CWD_MUTEX: Mutex<()> = Mutex::new(());

// ── HUME_RUNTIME guard ────────────────────────────────────────────────────────

// HUME_RUNTIME is a process-global env var. Any test that sets it must hold
// this mutex for its entire duration so tests do not race on the value.
static HUME_RUNTIME_MUTEX: Mutex<()> = Mutex::new(());

/// Lock `HUME_RUNTIME_MUTEX`, create isolated `runtime` and `tmp` tempdirs,
/// set `HUME_RUNTIME` and `TMPDIR`, and restore both on drop.
///
/// The mutex is acquired BEFORE the tempdirs are created so that a concurrent
/// guarded test's TMPDIR does not cause our tempdirs to be nested inside it —
/// which would make them disappear when that test's guard drops and deletes its
/// tree.
#[cfg(not(windows))]
struct HumeRuntimeGuard {
    runtime: tempfile::TempDir,
    tmp: tempfile::TempDir,
    // Last field — released after runtime/tmp dirs are deleted.
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(not(windows))]
impl HumeRuntimeGuard {
    fn new() -> Self {
        let lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let runtime = tempfile::tempdir().expect("tempdir");
        let tmp = tempfile::tempdir().expect("tempdir");
        unsafe {
            std::env::set_var("HUME_RUNTIME", runtime.path());
            std::env::set_var("TMPDIR", tmp.path());
        }
        HumeRuntimeGuard { runtime, tmp, _lock: lock }
    }
}

#[cfg(not(windows))]
impl Drop for HumeRuntimeGuard {
    fn drop(&mut self) {
        // Clear env vars before the TempDir fields delete their directories and
        // before _lock releases the mutex, so the next waiter sees a clean env.
        unsafe {
            std::env::remove_var("HUME_RUNTIME");
            std::env::remove_var("TMPDIR");
        }
    }
}

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

// ── Event-loop faithful helpers ───────────────────────────────────────────────

impl Editor {
    /// Feed one key exactly as the event loop does (lifecycle.rs:354-402):
    /// dispatch it, refresh the search cache, drain any macro-replay keys it
    /// enqueued, then refresh again. Prefer this over `handle_key` in tests
    /// whose correctness depends on the per-key ordering — e.g. Smart-p logic
    /// that reads `last_command`, which an idle drain must not clobber (432c24f).
    fn feed_key(&mut self, key: KeyEvent) {
        self.handle_key(key);
        self.sync_search_cache();
        self.drain_replay_queue();
        self.sync_search_cache();
    }

    fn feed_keys(&mut self, keys: impl IntoIterator<Item = KeyEvent>) {
        for k in keys {
            self.feed_key(k);
        }
    }
}

// ── Bookkeeping snapshot ──────────────────────────────────────────────────────

/// Captures the entire funnel-owned side-effect cluster in one shot so a test
/// can assert all bookkeeping in one `assert_eq!` without missing a field.
///
/// Scope: the four effects that `dispatch_native` is exclusively responsible for
/// (commands/mod.rs:147–221).  Register routing (caller-armed) and handle_key-tail
/// concerns (drain_pending_repeat, hooks, search-cache) are intentionally excluded —
/// the former is seeding-dependent, the latter has dedicated tests.
#[derive(Debug, PartialEq)]
pub(super) struct BookkeepingSnapshot {
    /// `ed.state.last_command` — name stamped by `dispatch_native` for smart-p.
    pub last_command: Option<String>,
    /// `ed.state.last_repeatable_action` — (command, count, char_arg) if set.
    /// `insert_keys` is excluded: it is always empty at dispatch time and only
    /// filled later by `end_insert_session` (a handle_key-tail concern).
    pub last_repeatable: Option<(String, usize, Option<char>)>,
    /// Total jump entries in the focused pane (not filtered by buffer) after dispatch.
    pub jump_len: usize,
    /// Whether any (pane, buffer) pair has an open paste session (`paste_group.is_some()`).
    pub paste_session_open: bool,
}

/// Capture the current bookkeeping state of an editor.
///
/// Call once before dispatch and once after; `assert_eq!` the two snapshots on
/// a path-parity test or diff them for targeted assertions.
pub(super) fn snapshot_bookkeeping(ed: &Editor) -> BookkeepingSnapshot {
    let pane_id = ed.state.focused_pane_id;
    BookkeepingSnapshot {
        last_command: ed.state.last_command.as_deref().map(str::to_owned),
        last_repeatable: ed.state.last_repeatable_action.as_ref().map(|a| {
            (a.command.to_string(), a.count, a.char_arg)
        }),
        // JumpList::len() is cfg(test)-only; safe to call here.
        jump_len: ed.state.panes.jumps[pane_id].len(),
        paste_session_open: ed.state.panes.state
            .iter()
            .flat_map(|(_, inner)| inner.iter())
            .any(|(_, pbs)| pbs.paste_group.is_some()),
    }
}

mod alternate;
mod auto_pairs;
mod buffer;
mod buffer_store;
mod cd;
mod plugins;
mod command_mode;
mod commands;
mod completion;
mod dot_repeat;
mod file_io;
mod find;
mod hooks;
mod incremental_parse;
mod language;
mod jump_list;
mod kitty;
mod list_buffers;
mod macros;
mod multi_pane;
mod page_scroll;
mod pane_sync;
mod per_pane_jumps;
mod scripting_grammar;
mod search;
mod select_all;
mod surround;
mod sync_dispatch;
mod tutor;
mod view_scroll;
mod visual_move;
