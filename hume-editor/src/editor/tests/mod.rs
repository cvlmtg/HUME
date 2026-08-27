// Shared imports and harness helpers used by all test submodules.
// Each submodule does `use super::*;` to access these.

use std::cell::Cell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::editor::buffer::Buffer;
use crate::editor::buffer::store::BufferStore;
use crate::editor::pane_state::{PaneBufferState, PaneTransient, PaneView};
use crate::editor::search::SearchPattern;
use crate::editor::{EditorState, SearchState};
use crate::settings::EditorSettings;
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use hume_engine::pane::Pane;
use hume_engine::pipeline::{BufferId, EngineView, LayoutTree, PaneId};
use hume_ops::register::{KillRing, RegisterSet};
use hume_ops::search::SearchDirection;
use hume_test_fixtures::testing::{parse_state, serialize_state};
use hume_treesitter::parse_worker::InlineParseBackend;
use slotmap::SecondaryMap;
use termina::event::{
    Event as TerminalEvent, KeyCode, KeyEvent, Modifiers, MouseButton, MouseEvent, MouseEventKind,
};

use super::{Editor, Mode, Severity};

// ── Harness ───────────────────────────────────────────────────────────────────

/// Build an Editor pre-loaded with the given state string (same DSL as other tests).
fn editor_from(input: &str) -> Editor {
    let (text, sels) = parse_state(input);
    Editor::for_testing(Buffer::new(text, sels))
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

/// Every queued `PendingWork::Call` in `pending_work`, in FIFO order,
/// ignoring any interleaved `Event` items — for tests that assert on
/// specific queued callbacks (an `lsp-request`/timer/prompt/menu/drawer/
/// picker callback).
fn pending_calls(ed: &Editor) -> Vec<(&steel::rvals::SteelVal, &Vec<steel::rvals::SteelVal>)> {
    ed.state
        .config
        .pending_work
        .iter()
        .filter_map(|w| match w {
            crate::editor::event::PendingWork::Call(proc, args) => Some((proc, args)),
            crate::editor::event::PendingWork::Event(_) => None,
        })
        .collect()
}

/// `StatusElement::Custom(name)`'s rendered text for the focused buffer —
/// the render side of `(set-statusline-text! name bid text)`. Shared by
/// `statusline_steel.rs`, `unix/git_diff_plugin.rs`, and `unix/reload_config.rs`.
fn custom_text(ed: &Editor, name: &str) -> String {
    let colors = crate::ui::theme::EditorColors::default();
    let (text, _) = crate::ui::statusline::render_element(
        &crate::ui::statusline::StatusElement::Custom(name.into()),
        ed,
        &colors,
        "",
    );
    text.into_owned()
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

/// A left-button-down mouse event at the given screen coordinates. Shared by
/// `mouse.rs` and `disk_change.rs` (click-to-focus's buffer-enter disk check).
fn mouse_left_down(x: u16, y: u16) -> TerminalEvent {
    TerminalEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: Modifiers::NONE,
    })
}

/// A wheel-scroll event at (0, 0) — every current caller scrolls the focused
/// pane, which hit-testing never depends on, so the coordinates are fixed
/// rather than parameterized. `down` picks the direction, matching
/// `Editor::mouse_scroll`'s own `down: bool` (`editor/mouse.rs`).
fn mouse_wheel(down: bool) -> TerminalEvent {
    TerminalEvent::Mouse(MouseEvent {
        kind: if down {
            MouseEventKind::ScrollDown
        } else {
            MouseEventKind::ScrollUp
        },
        column: 0,
        row: 0,
        modifiers: Modifiers::NONE,
    })
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

/// `type_cmd`'s twin, routed through `handle_input` (see `Editor::feed_event`)
/// instead of `feed_key`/`step`. Use when the test needs the buffer-enter
/// disk check to run on the command's own dispatch, not just its keystrokes.
fn type_cmd_event(ed: &mut Editor, cmd: &str) {
    for ch in cmd.chars() {
        ed.feed_event(key(ch));
    }
    ed.feed_event(key_enter());
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
    let content: String = (0..20).map(|i| format!("line {i}\n")).collect();
    let text = BufferText::from(content.as_str());
    let pos = text.line_to_char(cursor_line);
    let sels = SelectionSet::single(hume_editing::selection::Selection::collapsed(pos));
    let doc = Buffer::new(text, sels);
    let mut ed = Editor::for_testing(doc);
    ed.state.mode = Mode::Normal;
    ed
}

/// Write `file_content` to a temp file, return an editor pointing at it.
///
/// `set_path` derives `display_path` from `path` (see `Buffer::set_path`) —
/// this fixture doesn't call a resolve-typed-path helper, so the two stay
/// paired on the raw (non-canonical) tempfile path, same as `path()` itself.
fn editor_with_file(initial_state: &str, file_content: &str) -> (Editor, tempfile::TempPath) {
    let (path, tmp_path) = temp_file(file_content);
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
        let pane = Pane::new(buffer_id);
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
                paste_stamp: None,
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
                    // directly (not `build_pane`), so it has no `ScopedHighlighter`/
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
                visual_move_target_display_cols: Vec::new(),
                macro_recording: None,
                macro_pending: None,
                replay_queue: VecDeque::new(),
                skip_macro_record: false,
                dispatching_typed_command: false,
                is_replaying: false,
                message_logged_this_input: false,
                last_entered_buffer: None,
                mouse_drag_anchor: None,
                cwd: std::env::temp_dir(),
                lsp_completion_dismiss_pending: false,
                completion_menu_view: Arc::new(RwLock::new(None)),
                minibuf_completion_view: Arc::new(RwLock::new(None)),
                diagnostic_scopes: None,
                inlay_hint_scope: None,
                virtual_text_fallback_scope: None,
                bracket_match_scope: None,
                search_match_scope: None,
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
            config_path_override: None,
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

// ── process-global test lock ─────────────────────────────────────────────────
//
// test-global-safe: definitions below are the sanctioned owners of process
// globals (cwd, HUME_RUNTIME, TMPDIR, XDG_*, HOME, PATH) — every other mutator
// in the test tree routes through them.

/// The two process globals the suite serializes access to. A `Cell<bool>` per
/// variant tracks whether *this thread* currently holds an exclusive claim on
/// it (see [`TestGlobals::claim`]).
#[derive(Clone, Copy, Debug)]
enum Global {
    /// `HUME_RUNTIME`, `TMPDIR`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `HOME`,
    /// `PATH` — every env var a guard in this tree redirects. Also claimed by
    /// a test with no guard of its own that spawns a subprocess by
    /// unqualified name (`Command::new("tree-sitter")`, `Command::new("sh")`,
    /// …): the OS resolves that name against process `PATH` at the spawn
    /// instant, so such a test is a `PATH` *reader* racing every `PATH`
    /// mutator just as much as a `set_var` call would.
    Env,
    /// The process current directory.
    Cwd,
}

struct Claims {
    env: Cell<bool>,
    cwd: Cell<bool>,
}

impl Claims {
    const fn new() -> Self {
        Claims {
            env: Cell::new(false),
            cwd: Cell::new(false),
        }
    }

    fn flag(&self, what: Global) -> &Cell<bool> {
        match what {
            Global::Env => &self.env,
            Global::Cwd => &self.cwd,
        }
    }
}

/// Exclusive claim on one [`Global`] for a guard's lifetime — released when
/// this drops. Never construct directly; go through [`TestGlobals::claim`].
struct ClaimGuard {
    what: Global,
    _lock: parking_lot::ReentrantMutexGuard<'static, Claims>,
}

impl Drop for ClaimGuard {
    fn drop(&mut self) {
        self._lock.flag(self.what).set(false);
    }
}

/// The single lock guarding every process global the suite mutates. Reentrant
/// (`parking_lot::ReentrantMutex`, not `std::sync::Mutex`): a helper that
/// re-acquires it on a thread that already holds it — e.g. `safe_tempdir()`
/// called from inside a live `HumeRuntimeGuard` — blocks only on *other*
/// threads, never on itself. A non-reentrant mutex here hung the suite twice
/// (once, and again in the `git_diff_plugin.rs` fix that prompted this
/// type) with no panic, no assertion failure — just a silent "running for
/// over 60s" from the test runner, on a process-wide lock that then starved
/// every other concurrently-running test too.
///
/// One lock, not one per `Global`: guards nest in both directions (a
/// `CwdSandbox` opened inside a live `HumeRuntimeGuard` in
/// `unix/pickers_plugin.rs`, and a `CwdSandbox`-like guard that itself claims
/// `Env` while already holding `Cwd`) — two independently-ordered locks
/// deadlock ABBA the moment both nesting directions exist. Reentrancy makes
/// that moot: nesting is fine as long as it never claims the *same* `Global`
/// twice, which [`claim`](Self::claim) enforces.
struct TestGlobals {
    inner: parking_lot::ReentrantMutex<Claims>,
}

impl TestGlobals {
    const fn new() -> Self {
        TestGlobals {
            inner: parking_lot::ReentrantMutex::new(Claims::new()),
        }
    }

    /// Exclusive claim on `what` for a guard's lifetime. Panics if this
    /// thread already claims `what`: reentrancy makes a *second* guard
    /// construct without blocking, but its `Drop` would then clear state
    /// (env vars, cwd) the outer guard still needs — silently, and strictly
    /// worse than the hang this type replaces. A guard that legitimately
    /// nests a *different* `Global` (e.g. `Cwd` inside `Env`) is fine; only
    /// same-resource nesting is the bug.
    fn claim(&'static self, what: Global) -> ClaimGuard {
        let lock = self.inner.lock();
        let flag = lock.flag(what);
        assert!(
            !flag.get(),
            "test already holds a {what:?} claim on this thread — a nested guard \
             for the same resource would clear it out from under the outer guard \
             on drop; scope the outer guard tighter instead of nesting"
        );
        flag.set(true);
        ClaimGuard { what, _lock: lock }
    }

    /// Momentary reentrant visit: blocks on another thread's claim, never on
    /// this thread's own. `safe_tempdir()`'s creation instant only needs "no
    /// other thread is mid `TMPDIR`-redirect right now", not exclusivity
    /// against a claim this same thread may already hold.
    fn enter(&'static self) -> parking_lot::ReentrantMutexGuard<'static, Claims> {
        self.inner.lock()
    }
}

static TEST_GLOBALS: TestGlobals = TestGlobals::new();

/// Creates a tempdir while holding [`TEST_GLOBALS`] — guarantees no
/// concurrent `HumeRuntimeGuard` is mid-`TMPDIR`-redirect at creation time,
/// so this directory can't land inside (and later be deleted along with)
/// that guard's tree. Only the creation instant needs the lock: once a
/// `TempDir` exists at its own stable path, a *later* guard's redirect
/// can't retroactively engulf it — `TMPDIR` only affects tempdir calls made
/// while it's set. Any test that creates its own tempdirs outside a
/// `HumeRuntimeGuard`/`RealRuntimeGuard` (which already protect everything
/// created during their lifetime) should use this instead of a bare
/// `tempfile::tempdir()`. Safe to call from inside a held guard on the same
/// thread — [`TestGlobals::enter`] is reentrant.
fn safe_tempdir() -> tempfile::TempDir {
    let _lock = TEST_GLOBALS.enter();
    tempfile::tempdir().expect("tempdir")
}

/// [`safe_tempdir`]'s twin for a single named file — for a test that keeps
/// the `NamedTempFile` itself alive (e.g. to reopen or persist it), rather
/// than [`temp_file`]'s write-content-and-hand-back-a-path shape.
fn safe_named_tempfile() -> tempfile::NamedTempFile {
    let _lock = TEST_GLOBALS.enter();
    tempfile::NamedTempFile::new().expect("named tempfile")
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

/// Runs `body` as a Steel command; the command moves the cursor iff `body`'s
/// own assertion (embedded in the Scheme source) held.
fn run_probe(
    ed: &mut Editor,
    mut host: hume_scripting::ScriptingHost,
    tmp: &std::path::Path,
    body: &str,
) -> bool {
    let source = format!(
        r#"(define-command! "probe" "" (lambda () (when (begin {body}) (call! "move-right"))))"#
    );
    eval_with_real_host(ed, &mut host, &source, tmp);
    ed.scripting = Some(host);
    let before = state(ed);
    type_cmd(ed, ":probe");
    state(ed) != before
}

/// Write `content` to a temp file and return its path (kept alive by the
/// returned `TempPath`).
///
/// Returns `TempPath` rather than keeping the `NamedTempFile` open: on
/// Windows an open handle on the destination blocks the `MoveFileEx` replace
/// behind `write_file_atomic`, so any temp file the editor might write to
/// must not keep one held.
fn temp_file(content: &str) -> (std::path::PathBuf, tempfile::TempPath) {
    let f = safe_named_tempfile();
    std::fs::write(f.path(), content).unwrap();
    let path = f.path().to_path_buf();
    (path, f.into_temp_path())
}

/// Build a fresh file-backed `Buffer` from `content`, written to a temp file.
/// `set_path` derives `display_path` from the raw tempfile path (see
/// `Buffer::set_path`) — the same default `Buffer::from_file` produces.
fn file_buffer(content: &str) -> (Buffer, tempfile::TempPath) {
    let (path, tmp_path) = temp_file(content);
    let (_, meta) = hume_platform::io::read_file(&path).unwrap();
    let mut buf = Buffer::new(BufferText::from(content), SelectionSet::default());
    buf.set_path(Some(path));
    buf.file_meta = Some(meta);
    (buf, tmp_path)
}

/// Acquire the cwd lock, save the current directory, and restore it on drop.
#[cfg(unix)]
struct CwdGuard {
    saved: PathBuf,
    _lock: ClaimGuard,
}

#[cfg(unix)]
impl CwdGuard {
    fn new() -> Self {
        let lock = TEST_GLOBALS.claim(Global::Cwd);
        let saved = std::env::current_dir().expect("current_dir");
        CwdGuard { saved, _lock: lock }
    }
}

#[cfg(unix)]
impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.saved);
    }
}

/// Saves one env var's value on construction, restores it (or removes it, if
/// it was unset before) on drop — generalizes the hand-written save/restore
/// already duplicated in `RealRuntimeGuard` (`XDG_DATA_HOME`) and
/// `NoConfigDirGuard` (`HOME`/`XDG_CONFIG_HOME`) to any single var, for sites
/// that mutate just one (e.g. `PATH` in `scripting_lsp_install.rs`) rather
/// than owning a whole guard.
///
/// Caller must already hold a `Global::Env` claim for at least this guard's
/// lifetime — this only owns the save/restore, not the exclusivity, the same
/// contract `load_plum`/`load_lsp` (`unix/injections_editor.rs`) document for
/// their own env mutation.
struct EnvVarGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let prev = std::env::var(key).ok();
        unsafe {
            std::env::set_var(key, value);
        }
        EnvVarGuard { key, prev }
    }

    /// Captures `key`'s current value without touching it — for a caller
    /// that mutates the var itself (e.g. `remove_var`, to test the "unset"
    /// case) and just wants the restore-on-drop half.
    fn capture(key: &'static str) -> Self {
        EnvVarGuard {
            key,
            prev: std::env::var(key).ok(),
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.prev {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

// ── Event-loop faithful helpers ───────────────────────────────────────────────

impl Editor {
    /// Feed one key exactly as the event loop does (lifecycle.rs:354-402):
    /// dispatch it, refresh the search cache, drain any macro-replay keys it
    /// enqueued, then refresh again. Prefer this over `handle_key` in tests
    /// whose correctness depends on the per-key ordering — e.g. smart-paste
    /// tests, where the idle replay drain runs between two keys and must not
    /// disturb the `PasteStamp` freshness check.
    fn feed_key(&mut self, key: KeyEvent) {
        self.step(key);
    }

    fn feed_keys(&mut self, keys: impl IntoIterator<Item = KeyEvent>) {
        for k in keys {
            self.feed_key(k);
        }
    }

    /// Feed one key through `handle_input`, the interactive input boundary —
    /// unlike `feed_key`/`step`, which deliberately bypass it (see
    /// `Editor::handle_input`'s doc) — then `settle()`, mirroring
    /// `Editor::run`'s loop (dispatch at the bottom of one iteration, settle
    /// at the top of the next). Needed by tests covering the buffer-enter
    /// disk check on focus change: that check is `OnBufferEnter`'s Rust
    /// reaction, observed by `settle`'s own diff, not by
    /// `handle_input` itself.
    fn feed_event(&mut self, key: KeyEvent) {
        self.handle_input(TerminalEvent::Key(key));
        self.settle();
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

mod alternate;
mod async_job_steel;
mod async_source;
mod auto_pairs;
mod bracketed_paste;
mod buffer;
mod buffer_store;
mod buffer_text_steel;
mod command_mode;
mod commands;
mod completion;
mod copy_selection;
mod diff_steel;
mod disk_change;
mod dot_repeat;
mod events;
mod file_io;
mod find;
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
mod lsp_line_backgrounds;
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
mod messages;
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
mod registers;
mod reload_config;
mod render_snapshot;
mod scripting_effects;
mod scripting_grammar;
mod scripting_host_globals;
mod search;
mod select_all;
mod settings_effects;
mod shift_punctuation;
mod statusline_steel;
mod surround;
mod sync_dispatch;
mod tabs;
mod terminator;
mod test_globals;
mod theme_loading;
mod timers;
mod undo_levels;
#[cfg(unix)]
mod unix;
mod view_scroll;
mod vim_keybind;
mod virtual_line_scroll;
mod visual_move;
mod word_motion_settings;
mod wrap;
