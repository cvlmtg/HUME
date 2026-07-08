// Shared imports and harness helpers used by all test submodules.
// Each submodule does `use super::*;` to access these.

use std::path::PathBuf;
use std::sync::Mutex;

use crate::editor::SearchDirection;
use crate::editor::buffer::Buffer;
use crate::testing::{parse_state, serialize_state};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use hume_editing::selection::SelectionSet;
use hume_editing::text::Text;

use super::{Editor, Mode, Severity};

// ── Harness ───────────────────────────────────────────────────────────────────

/// Build an Editor pre-loaded with the given state string (same DSL as other tests).
fn editor_from(input: &str) -> Editor {
    let (buf, sels) = parse_state(input);
    Editor::for_testing(Buffer::new(buf, sels))
}

/// Build a kitty-protocol-enabled editor for testing Ctrl+motion bindings.
/// Mirrors interactive kitty mode: sets the flag AND installs the kitty-only
/// default keybinds that `Keymap::default()` omits.
fn editor_from_kitty(input: &str) -> Editor {
    let mut ed = editor_from(input);
    ed.set_kitty_support(true);
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
    ed.state
        .registers
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
    let sels = SelectionSet::single(hume_editing::selection::Selection::collapsed(pos));
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
    let (_, meta) = hume_platform::io::read_file(&path).unwrap();
    let mut ed = editor_from(initial_state);
    ed.doc_mut().set_path(Some(path));
    ed.doc_mut().file_meta = Some(meta);
    (ed, tmp_path)
}

/// Build a live `EditorHostImpl` borrowing `$ed`'s state/view, for direct
/// command dispatch — bypasses the keymap entirely. Mirrors the construction
/// in `execute.rs` so the host has the same shape as in production dispatch.
macro_rules! live_host {
    ($ed:ident) => {{
        crate::editor::host_impl::EditorHostImpl {
            state: &mut $ed.state,
            view: &mut $ed.view,
            lsp: Some(&$ed.lsp),
            timers: Some(crate::editor::timer_bridge::TimerHandle {
                wheel: &mut $ed.timer_wheel,
                payloads: &mut $ed.timer_payloads,
            }),
        }
    }};
}
// Used via `live_host!()` through submodules' `use super::*;` — the
// unused_imports lint doesn't track macro re-exports used only that way.
#[allow(unused_imports)]
pub(crate) use live_host;

// ── cwd guard ─────────────────────────────────────────────────────────────────

// Process cwd is global state. Any test that calls `set_current_dir` must hold
// this mutex for its entire duration so tests do not race on cwd.
static CWD_MUTEX: Mutex<()> = Mutex::new(());

// ── HUME_RUNTIME guard ────────────────────────────────────────────────────────

// HUME_RUNTIME is a process-global env var. Any test that sets it must hold
// this mutex for its entire duration so tests do not race on the value.
static HUME_RUNTIME_MUTEX: Mutex<()> = Mutex::new(());

/// Creates a tempdir while holding `HUME_RUNTIME_MUTEX` — guarantees no
/// concurrent `HumeRuntimeGuard` is mid-`TMPDIR`-redirect at creation time,
/// so this directory can't land inside (and later be deleted along with)
/// that guard's tree. Only the creation instant needs the lock: once a
/// `TempDir` exists at its own stable path, a *later* guard's redirect
/// can't retroactively engulf it — `TMPDIR` only affects tempdir calls made
/// while it's set. Any test that creates its own tempdirs outside a
/// `HumeRuntimeGuard`/`RealRuntimeGuard` (which already protect everything
/// created during their lifetime) should use this instead of a bare
/// `tempfile::tempdir()`.
fn safe_tempdir() -> tempfile::TempDir {
    let _lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    tempfile::tempdir().expect("tempdir")
}

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
        HumeRuntimeGuard {
            runtime,
            tmp,
            _lock: lock,
        }
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

/// The real shipped `core:stdlib` plugin source, embedded so tests exercise
/// the actual file rather than a hand-rolled stand-in.
#[cfg(not(windows))]
const STDLIB_PLUGIN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../runtime/plugins/core/stdlib/plugin.scm"
));

/// Stage a real shipped core plugin's source into `guard`'s isolated
/// `HUME_RUNTIME/plugins/core/<name>/plugin.scm`, so `load-plugin` resolves it
/// as a core plugin during the test.
#[cfg(not(windows))]
fn write_core_plugin(guard: &HumeRuntimeGuard, name: &str, source: &str) {
    let plugin_dir = guard.runtime.path().join("plugins").join("core").join(name);
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(plugin_dir.join("plugin.scm"), source).unwrap();
}

/// Points `HUME_RUNTIME` at the *real*, on-disk `runtime/` directory (a
/// sibling of the crate root, resolved once via `CARGO_MANIFEST_DIR`) for
/// the guard's lifetime — used by multi-file core plugins (`core:lsp`,
/// mirroring `core:plum`'s layout) so tests exercise the actual shipped
/// files without hand-copying every one into a temp dir and keeping that
/// list in sync as feature files are added.
///
/// Deliberately does **not** touch `TMPDIR`, unlike [`HumeRuntimeGuard`]:
/// pointing at a persistent, never-deleted directory means there is nothing
/// for a concurrent test's cleanup to race against. `HumeRuntimeGuard`'s
/// `TMPDIR` override only protects itself from *other* `HumeRuntimeGuard`s
/// (both take the same mutex) — it does not and cannot protect unrelated
/// tests that call bare `tempfile::tempdir()`, since `TMPDIR` is a
/// process-global env var every thread's allocator reads. A slow guarded
/// test can redirect an unrelated concurrent test's `tempfile::tempdir()`
/// into its own tree and then delete that tree out from under it on drop.
/// Avoiding `TMPDIR` entirely sidesteps the hazard rather than narrowing it.
#[cfg(not(windows))]
struct RealRuntimeGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(not(windows))]
impl RealRuntimeGuard {
    fn new() -> Self {
        let lock = HUME_RUNTIME_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let real_runtime = concat!(env!("CARGO_MANIFEST_DIR"), "/../runtime");
        unsafe {
            std::env::set_var("HUME_RUNTIME", real_runtime);
        }
        RealRuntimeGuard { _lock: lock }
    }
}

#[cfg(not(windows))]
impl Drop for RealRuntimeGuard {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("HUME_RUNTIME");
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

/// Like `CwdGuard`, but also owns a tempdir the test can `cd` into.
///
/// Bundling the tempdir into the same struct as the restore-on-drop logic is
/// what fixes the historical bug, not the fields' declaration order: Rust
/// always runs a struct's custom `Drop::drop` to completion *before* dropping
/// any of its own fields, regardless of their order. So restoring cwd inside
/// `CwdSandbox::drop` is guaranteed to happen before `dir` (the `TempDir`
/// field) is deleted.
///
/// A test that instead pairs a bare `CwdGuard` with a *separately-scoped*
/// `tempfile::tempdir()` local doesn't get that guarantee — independent
/// locals in a function body drop in reverse declaration order, so the
/// tempdir (declared after the guard) drops *first*, deleting the directory
/// while the process cwd still points inside it. Any concurrently-running
/// test that calls `std::env::current_dir()` in that window — e.g. Steel's
/// `Engine::new()`, which falls back to it while compiling `ALL_MODULES` —
/// gets `ENOENT` and panics. `CwdSandbox` closes that window structurally.
#[cfg(not(windows))]
struct CwdSandbox {
    dir: tempfile::TempDir,
    saved: PathBuf,
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(not(windows))]
impl CwdSandbox {
    fn new() -> Self {
        let _lock = CWD_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let saved = std::env::current_dir().expect("current_dir");
        let dir = tempfile::tempdir().expect("tempdir");
        Self { dir, saved, _lock }
    }

    /// Raw tempdir path — build child dirs/files under this.
    fn raw(&self) -> &std::path::Path {
        self.dir.path()
    }

    /// Canonicalized tempdir path (macOS /var → /private/var) for cwd asserts.
    fn path(&self) -> PathBuf {
        std::fs::canonicalize(self.dir.path()).expect("canonicalize")
    }
}

#[cfg(not(windows))]
impl Drop for CwdSandbox {
    fn drop(&mut self) {
        // Restore first; `dir` is only deleted afterwards, when the field drops.
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
        self.step(key);
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
/// Scope: the five effects that `run_dispatch_pipeline` is exclusively responsible
/// for. Register routing (caller-armed) and handle_key-tail concerns
/// (replay_dot, hooks, search-cache) are intentionally excluded — the former is
/// seeding-dependent, the latter has dedicated tests.
///
/// Deliberate exclusion — `selection_recipe`: the Steel dispatch branch clears it
/// unconditionally (inner `call!` dispatches overwrite it, outer Steel AFTER always
/// resets), so it legitimately diverges across paths and cannot be a parity field.
#[derive(Debug, PartialEq)]
pub(super) struct BookkeepingSnapshot {
    /// `ed.state.last_command` — name stamped by `step_stamp_last_command` for smart-p.
    pub last_command: Option<String>,
    /// `ed.state.last_repeatable_action` — (command, count, char_arg) if set.
    /// `insert_keys` is excluded: it is always empty at dispatch time and only
    /// filled later by `end_insert_session` (a handle_key-tail concern).
    pub last_repeatable: Option<(String, usize, Option<char>)>,
    /// Total jump entries in the focused pane (not filtered by buffer) after dispatch.
    pub jump_len: usize,
    /// Whether any (pane, buffer) pair has an open paste session (`paste_group.is_some()`).
    pub paste_session_open: bool,
    /// `ed.state.mode` — set by `step_clear_extend` for selection-consuming edits.
    pub mode: Mode,
}

/// Capture the current bookkeeping state of an editor.
///
/// Call once before dispatch and once after; `assert_eq!` the two snapshots on
/// a path-parity test or diff them for targeted assertions.
pub(super) fn snapshot_bookkeeping(ed: &Editor) -> BookkeepingSnapshot {
    let pane_id = ed.state.focused_pane_id;
    BookkeepingSnapshot {
        last_command: ed.state.last_command.as_deref().map(str::to_owned),
        last_repeatable: ed
            .state
            .last_repeatable_action
            .as_ref()
            .map(|a| (a.command.to_string(), a.count, a.char_arg)),
        // JumpList::len() is cfg(test)-only; safe to call here.
        jump_len: ed.state.panes.jumps[pane_id].len(),
        paste_session_open: ed
            .state
            .panes
            .state
            .iter()
            .flat_map(|(_, inner)| inner.iter())
            .any(|(_, pbs)| pbs.paste_group.is_some()),
        mode: ed.state.mode,
    }
}

// ── Grammar fixture paths ─────────────────────────────────────────────────────

/// Absolute path to the pre-built grammar shared library for `name`.
///
/// Callers that require the file to exist should check or load it immediately
/// after calling this — the helper does not verify presence.
///
/// Fixtures are installed by `scripts/fetch-test-grammars.sh`.
pub(crate) fn grammar_parser_path(name: &str) -> PathBuf {
    let suffix = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(windows) {
        "dll"
    } else {
        "so"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/grammars")
        .join(name)
        .join(format!("parser.{suffix}"))
}

/// Subpath within the cloned grammar repo holding its `queries/` and `src/`
/// (`None` for single-grammar repos; `Some` for monorepos like
/// tree-sitter-markdown, which holds `tree-sitter-markdown` and
/// `tree-sitter-markdown-inline` as subdirectories of one clone). Mirrors
/// the lookup in `scripts/fetch-test-grammars.sh`.
fn grammar_subpath(name: &str) -> Option<String> {
    let catalog = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("runtime/scheme/grammar-sources.scm"),
    )
    .ok()?;
    let needle = format!("(\"{name}\" ");
    let line = catalog
        .lines()
        .find(|l| l.trim_start().starts_with(&needle))?;
    let subpath = line.split('"').nth(9)?;
    (!subpath.is_empty()).then(|| subpath.to_owned())
}

fn grammar_fixture_root(name: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/grammars")
        .join(name);
    match grammar_subpath(name) {
        Some(sub) => base.join(sub),
        None => base,
    }
}

/// Absolute path to the highlights query file for `name`.
pub(crate) fn grammar_query_path(name: &str) -> PathBuf {
    grammar_fixture_root(name).join("queries/highlights.scm")
}

/// Absolute path to the *Helix-maintained* injections query for `name`,
/// fetched by `scripts/fetch-test-grammars.sh` from the pinned Helix commit —
/// distinct from (and can differ from!) the grammar's own bundled
/// `queries/injections.scm`. PLUM installs the Helix version, so tests
/// validating what PLUM actually ships should use this. `None` if the fetch
/// script found no Helix injections query for `name` (most grammars have none).
pub(crate) fn helix_injections_path(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/grammars")
        .join(name)
        .join("helix-injections.scm");
    path.exists().then_some(path)
}

mod alternate;
mod async_source;
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
mod incremental_parse;
mod injections_editor;
mod jump_list;
mod kitty;
mod language;
mod list_buffers;
mod lsp;
mod lsp_bridge;
mod lsp_completion;
mod lsp_completion_menu;
mod lsp_decorations;
mod lsp_diagnostics;
mod lsp_drawer;
mod lsp_edits;
mod lsp_goto;
mod lsp_hooks;
mod lsp_hover;
mod lsp_inlay_hints;
mod lsp_introspect;
mod lsp_menu;
mod lsp_popup;
mod lsp_prompt;
mod lsp_references;
mod lsp_render;
mod lsp_signs;
mod lsp_status;
mod lsp_statusline;
mod lsp_sync;
mod lsp_virtual_lines;
mod macros;
mod mouse;
mod multi_pane;
mod page_scroll;
mod pane_focus;
mod pane_sync;
mod per_pane_jumps;
mod plugins;
mod render_snapshot;
mod scripting_grammar;
mod search;
mod select_all;
mod shift_punctuation;
mod surround;
mod sync_dispatch;
mod tabs;
mod timers;
mod tutor;
mod view_scroll;
mod vim_keybind;
mod visual_move;
mod wrap;
