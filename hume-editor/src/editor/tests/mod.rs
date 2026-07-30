// Shared imports and harness helpers used by all test submodules.
// Each submodule does `use super::*;` to access these.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use crate::editor::buffer::Buffer;
use crate::editor::buffer::store::BufferStore;
use crate::editor::pane_state::{PaneBufferState, PaneTransient, PaneView};
use crate::editor::search::SearchPattern;
use crate::editor::{EditorState, SearchDirection, SearchState};
use crate::ops::register::{KillRing, RegisterSet};
use crate::settings::EditorSettings;
use crate::testing::{parse_state, serialize_state};
use hume_editing::selection::SelectionSet;
use hume_editing::text::Text;
use hume_engine::pane::Pane;
use hume_engine::pipeline::{BufferId, EngineView, LayoutTree, PaneId};
use hume_treesitter::parse_worker::InlineParseBackend;
use slotmap::SecondaryMap;
use termina::event::{KeyCode, KeyEvent, Modifiers};

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
    KeyEvent::new(KeyCode::Char(ch), Modifiers::NONE)
}

fn key_esc() -> KeyEvent {
    KeyEvent::new(KeyCode::Escape, Modifiers::NONE)
}

fn key_ctrl(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), Modifiers::CONTROL)
}

fn key_enter() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, Modifiers::NONE)
}

fn key_up() -> KeyEvent {
    KeyEvent::new(KeyCode::Up, Modifiers::NONE)
}

fn key_down() -> KeyEvent {
    KeyEvent::new(KeyCode::Down, Modifiers::NONE)
}

fn key_pageup() -> KeyEvent {
    KeyEvent::new(KeyCode::PageUp, Modifiers::NONE)
}

fn key_pagedown() -> KeyEvent {
    KeyEvent::new(KeyCode::PageDown, Modifiers::NONE)
}

fn key_tab() -> KeyEvent {
    KeyEvent::new(KeyCode::Tab, Modifiers::NONE)
}

fn key_backspace() -> KeyEvent {
    KeyEvent::new(KeyCode::Backspace, Modifiers::NONE)
}

fn key_left() -> KeyEvent {
    KeyEvent::new(KeyCode::Left, Modifiers::NONE)
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
    let (path, tmp_path) = temp_file(file_content);
    let (_, meta) = hume_platform::io::read_file(&path).unwrap();
    let mut ed = editor_from(initial_state);
    ed.doc_mut()
        .set_display_path(Some(hume_platform::path::display_form(
            meta.resolved_path(),
        )));
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
            lsp: Some(&mut $ed.lsp),
            timers: Some(crate::editor::timer_bridge::TimerHandle {
                wheel: &mut $ed.timer_wheel,
                payloads: &mut $ed.timer_payloads,
            }),
            terminal: $ed.terminal.as_ref(),
        }
    }};
}
// Used via `live_host!()` through submodules' `use super::*;` — the
// unused_imports lint doesn't track macro re-exports used only that way.
#[allow(unused_imports)]
pub(crate) use live_host;

// ── Test constructors ─────────────────────────────────────────────────────────

// proptest requires `Debug` on strategy values; this minimal impl satisfies it.
impl std::fmt::Debug for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Editor(buf={:?}, mode={:?})",
            self.doc().text().to_string(),
            self.state.mode()
        )
    }
}

impl Editor {
    /// Construct a minimal `Editor` for renderer unit tests.
    ///
    /// Only `doc` and `view` are meaningful — all other fields are set to
    /// sensible defaults (Normal mode, default colors, no file path, etc.).
    /// Use the builder methods below to override specific fields.
    pub(crate) fn for_testing(doc: Buffer) -> Self {
        // Minimal engine view for test contexts. Uses 80×24 with tab_width=4.
        let theme = crate::ui::theme::build_default_theme();
        let mut engine_view = EngineView::new(theme);
        let buffer_id = engine_view.buffers.insert(());
        let settings = EditorSettings::default();
        let jump_list_capacity = settings.jump_list_capacity;
        let history_capacity = settings.history_capacity;
        let initial_mouse_mode = (settings.mouse_enabled, settings.mouse_select);
        let pane = Pane::new(buffer_id, settings.wrap_mode);
        let pane_id = engine_view.panes.insert(pane);
        engine_view.layout = LayoutTree::Leaf(pane_id);

        let mut buffers = BufferStore::new();
        buffers.open(buffer_id, doc);

        let mut pane_buf_state: SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>> =
            SecondaryMap::new();
        pane_buf_state.insert(pane_id, SecondaryMap::new());
        super::pane_state::ensure(&mut pane_buf_state, &buffers, pane_id, buffer_id);

        Self {
            state: EditorState {
                buffers,
                config: super::ConfigState::new(false, 0),
                mode: Mode::Normal,
                pending_keys: Vec::new(),
                count: None,
                wait_char: None,
                pending_char: None,
                registers: RegisterSet::new(),
                kill_ring: KillRing::new(),
                clipboard: super::clipboard::SystemClipboard::new_unavailable(),
                register_prefix: None,
                last_command: None,
                last_paste: None,
                should_quit: false,
                terminate_exit_code: std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0)),
                minibuf: None,
                minibuf_completion: None,
                status_msg: None,
                summary_ttl: 0,
                message_log: super::message_log::MessageLog::new(),
                settings,
                last_find: None,
                force_full_redraw: false,
                inline_output: super::InlineOutputDispatch::Inactive,
                #[cfg(test)]
                inline_output_entered: false,
                last_repeatable_action: None,
                selection_recipe: Vec::new(),
                pending_repeat: None,
                insert_session: None,
                autoindent_pending: false,
                explicit_count: false,
                pending_ctrl_extend: false,
                search: SearchState::default(),
                panes: {
                    let mut jumps = SecondaryMap::new();
                    jumps.insert(pane_id, super::jump_list::JumpList::new(jump_list_capacity));
                    let mut transient = SecondaryMap::new();
                    transient.insert(pane_id, PaneTransient::default());
                    // No render entry: this pane is built via `Pane::new`
                    // directly (not `build_pane`), so it has no `SharedHighlighter`/
                    // `SignSource` providers to feed — the write sides skip panes
                    // with no entry.
                    PaneView {
                        state: pane_buf_state,
                        transient,
                        jumps,
                        render: SecondaryMap::new(),
                    }
                },
                history: super::minibuf::history::HistoryStore::new(history_capacity),
                focused_pane_id: pane_id,
                motion_format_scratch: hume_engine::format::FormatScratch::new(),
                visual_move_target_cols: Vec::new(),
                macro_recording: None,
                macro_pending: None,
                replay_queue: VecDeque::new(),
                skip_macro_record: false,
                dispatching_typed_command: false,
                is_replaying: false,
                mouse_drag_anchor: None,
                cwd: std::env::temp_dir(),
                lsp_completion_dismiss_pending: false,
                completion_menu_view: Arc::new(RwLock::new(None)),
                minibuf_completion_view: Arc::new(RwLock::new(None)),
                diagnostic_scopes: None,
                inlay_hint_scope: None,
                virtual_text_fallback_scope: None,
                runtime_scope_cache: rustc_hash::FxHashMap::default(),
                popup_view: Arc::new(RwLock::new(None)),
                popup_band_view: Arc::new(RwLock::new(None)),
                menu_view: Arc::new(RwLock::new(None)),
                drawer_view: Arc::new(RwLock::new(None)),
                picker_view: Arc::new(RwLock::new(None)),
                wake: Arc::new(|| {}),
            },
            view: engine_view,
            kitty_enabled: false,
            scripting: None,
            builtin_cmd_names: rustc_hash::FxHashSet::default(),
            parse_worker: Box::new(InlineParseBackend::new()),
            parse_worker_disconnect_logged: false,
            timer_wheel: super::timers::TimerWheel::new(),
            timer_payloads: rustc_hash::FxHashMap::default(),
            viewport_debounce: rustc_hash::FxHashMap::default(),
            last_viewport_key: rustc_hash::FxHashMap::default(),
            virtual_lines_synced: rustc_hash::FxHashMap::default(),
            lsp: super::lsp::LspState::new_inline(),
            tui_active: false,
            terminal: None,
            applied_mouse_mode: initial_mouse_mode,
        }
    }

    pub(crate) fn with_search_regex(mut self, pattern: &str) -> Self {
        if let Ok(regex) = regex_cursor::engines::meta::Regex::new(pattern) {
            let bid = self.focused_buffer_id();
            self.state.buffers.get_mut(bid).search_pattern = Some(SearchPattern {
                regex: Arc::new(regex),
                pattern_str: pattern.to_string(),
            });
        }
        self.sync_search_cache();
        self
    }

    // ── Pane choke-points (test-only) ─────────────────────────────────────────

    /// Switch focus to `target`, seeding its per-pane maps if not yet present.
    ///
    /// Precondition: editor must be in Normal mode. Focus switches are only
    /// bound in Normal mode; mode-changing commands must not switch panes.
    pub(crate) fn switch_focused_pane(&mut self, target: PaneId) {
        debug_assert!(
            self.state.mode() == Mode::Normal,
            "focus-switch must only happen in Normal mode, got {:?}",
            self.state.mode(),
        );
        self.state.focused_pane_id = target;
        if !self.state.panes.transient.contains_key(target) {
            self.state
                .panes
                .transient
                .insert(target, PaneTransient::default());
        }
        if !self.state.panes.jumps.contains_key(target) {
            self.state.panes.jumps.insert(
                target,
                super::jump_list::JumpList::new(self.state.settings.jump_list_capacity),
            );
        }
        let bid = self.focused_buffer_id();
        super::pane_state::ensure(
            &mut self.state.panes.state,
            &self.state.buffers,
            target,
            bid,
        );
    }

    /// Read-only accessor used by tests to inspect any pane's selections.
    pub(crate) fn selections_for(
        &self,
        pane: PaneId,
        buf: BufferId,
    ) -> Option<&hume_editing::selection::SelectionSet> {
        self.state
            .panes
            .state
            .get(pane)
            .and_then(|m| m.get(buf))
            .map(|s| &s.selections)
    }

    /// Execute a typed command string (e.g. `"bd"`, `"e! path"`) programmatically.
    ///
    /// Parses the trailing `!` as `force=true` and splits `cmd_with_arg` on the
    /// first space to extract the optional argument. Returns the command result.
    pub(crate) fn execute_typed(
        &mut self,
        cmd_with_arg: &str,
        extra_arg: Option<&str>,
    ) -> Result<(), crate::editor::error::CommandError> {
        let (cmd, force, inline_arg) =
            super::mappings::command_mode::parse_typed_command(cmd_with_arg);
        let arg = inline_arg.or(extra_arg);
        if let Some(tc) = self.state.config.registry.get_typed(cmd) {
            let fun = tc.fun;
            let result = fun(self, arg, force);
            if let Err(ref e) = result {
                self.report(Severity::Error, e.message().to_owned());
            }
            result
        } else {
            Err(crate::editor::error::CommandError::new(format!(
                "unknown command: {cmd}"
            )))
        }
    }
}

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

/// Write `source` as `<tmp>/init.scm`, evaluate it against the real
/// `EditorHostImpl`, and apply the effects it queued — the harness mirror of
/// `Editor::init_scripting`'s eval/apply pair.
///
/// Applying is not optional: effects an eval queues (`bind-key!`,
/// `register-lsp-server!`, …) only take hold once `apply_script_effects` runs,
/// so skipping it silently drops every one of them.
fn eval_with_real_host(
    ed: &mut Editor,
    host: &mut hume_scripting::ScriptingHost,
    source: &str,
    tmp: &std::path::Path,
) {
    let init_path = tmp.join("init.scm");
    std::fs::write(&init_path, source).unwrap();
    let effects = {
        let mut ih = crate::editor::scripting_setup::make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init");
    ed.apply_script_effects(effects);
}

/// Write `content` to a temp file and return its path (kept alive by the
/// returned `TempPath`).
///
/// Returns `TempPath` rather than keeping the `NamedTempFile` open: on
/// Windows an open handle on the destination blocks the `MoveFileEx` replace
/// behind `write_file_atomic`, so any temp file the editor might write to
/// must not keep one held.
fn temp_file(content: &str) -> (std::path::PathBuf, tempfile::TempPath) {
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), content).unwrap();
    let path = f.path().to_path_buf();
    (path, f.into_temp_path())
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
//
// Shared with hume-treesitter's test suite via the hume-test-fixtures dev
// crate — see that crate for the path helpers and require-fixtures gating.
pub(crate) use hume_test_fixtures::{
    grammar_parser_path, grammar_query_path, helix_injections_path,
};

mod alternate;
mod async_job_steel;
mod async_source;
mod auto_pairs;
mod buffer;
mod buffer_store;
mod command_mode;
mod commands;
mod completion;
mod disk_change;
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
mod lsp_diagnostics_inline;
mod lsp_drawer;
mod lsp_edits;
mod lsp_hooks;
mod lsp_inlay_hints;
mod lsp_introspect;
mod lsp_menu;
mod lsp_popup;
mod lsp_popup_markdown;
mod lsp_prompt;
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
mod paste;
mod per_pane_jumps;
mod picker;
mod picker_source_steel;
mod picker_steel;
mod plugins;
mod reload_config;
mod render_snapshot;
mod scripting_effects;
mod scripting_grammar;
mod scripting_host_globals;
mod search;
mod select_all;
mod settings_effects;
mod shift_punctuation;
mod surround;
mod sync_dispatch;
mod tabs;
mod terminator;
mod timers;
mod undo_levels;
#[cfg(unix)]
mod unix;
mod view_scroll;
mod vim_keybind;
mod virtual_line_scroll;
mod visual_move;
mod wrap;
