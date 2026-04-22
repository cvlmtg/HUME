use std::borrow::Cow;
use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crossterm::event::{self, Event, KeyEvent, KeyEventKind};

use engine::builtins::line_number::{LineNumberColumn, LineNumberStyle as EngineLineNumberStyle};
use engine::pane::{Pane, ViewportState, WhitespaceConfig, WrapMode};
use engine::pipeline::{BufferId, EngineView, LayoutTree, PaneId, PaneRenderSettings, RenderContext, SharedBuffer};
use engine::types::{EditorMode, Selection as EngineSelection};

use slotmap::SecondaryMap;

use crate::core::changeset::ChangeSet;
use crate::core::text::Text;
use self::registry::CommandRegistry;
use crate::editor::buffer::{Buffer, IntoApplyResult};
use crate::editor::buffer_store::BufferStore;
use crate::editor::pane_state::{PaneBufferState, PaneTransient};
use crate::ops::motion::FindKind;
use crate::ops::register::RegisterSet;
use crate::ops::search::{find_all_matches, search_match_info};
use crate::ops::pair::find_bracket_pair;
use crate::core::selection::{Selection, SelectionSet};
use crate::settings::EditorSettings;
use crate::os::terminal::Term;
use crate::scripting::{EditorSteelRefs, SteelCmdDef, hooks::HookId};
use crate::scripting::builtins::ids::SteelBufferId;
use steel::rvals::IntoSteelVal as _;

use self::keymap::{Keymap, WaitCharPending};

pub(crate) mod buffer;
pub(crate) mod buffer_store;
pub(crate) mod completion;
pub(crate) mod ops;
pub(crate) mod pane_state;
mod registry;
mod commands;
pub(crate) mod keymap;
mod mappings;
mod message_log;
mod minibuf;
mod mouse;
pub(super) mod scroll;
mod visual_move;

pub(crate) use crate::core::search_state::{SearchDirection, SearchState};
use crate::core::search_state::{SearchPattern, SearchMatches};
use crate::core::search_state::SearchCursor;

pub(crate) use minibuf::MiniBuffer;
use minibuf::MiniBufferEvent;

pub(crate) use message_log::{Severity, ScratchView};
use message_log::MessageLog;

// ── Dot-repeat / insert-session state ────────────────────────────────────────

/// State for an active insert session (entered via a repeatable command).
///
/// Tracks keystrokes for dot-repeat recording. Created by
/// [`Editor::begin_insert_session`] and consumed by [`Editor::end_insert_session`].
///
/// `None` on the editor when there is no active session — including during
/// replay, where the replay path pre-opens the edit group to signal
/// [`begin_insert_session`] that recording should be suppressed.
pub(super) struct InsertSession {
    keystrokes: Vec<KeyEvent>,
}

/// A recorded editing action that can be replayed by `.`.
///
/// Stores the recipe to re-execute a command rather than the raw changeset —
/// changesets are position-dependent and can't be replayed at a different cursor.
#[derive(Debug, Clone)]
pub(super) struct RepeatableAction {
    /// The command name that initiated this action (e.g. `"delete"`, `"change"`).
    /// `Cow::Borrowed` for built-in commands (zero allocation); `Cow::Owned` for
    /// dynamically-registered commands (e.g. from the Steel scripting layer).
    pub command: Cow<'static, str>,
    /// The count prefix used originally. Overridden when `.` itself is given a count.
    pub count: usize,
    /// Character argument for wait-char commands (`r`, `f`, `t`, …).
    /// `None` for commands that don't consume a char.
    pub char_arg: Option<char>,
    /// Keystrokes typed during the insert session, if any.
    ///
    /// Populated by the insert-mode recording path when the command transitions
    /// to Insert mode. Empty for non-insert actions like `delete` or `paste-after`.
    pub insert_keys: Vec<KeyEvent>,
}

// ── Macro recording state ─────────────────────────────────────────────────────

/// Pending state for the two-keystroke `q<reg>` / `Q<reg>` sequences.
///
/// Set when the user presses `q` or `Q` in normal mode; cleared when the
/// next keypress is consumed as the register name (or cancelled on Esc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MacroPending {
    /// `Q` was pressed — waiting for a register name to start recording.
    Record,
    /// `q` was pressed — waiting for a register name to start replay.
    Replay,
}

// ── Find/till state ───────────────────────────────────────────────────────────

/// The character and kind stored by the last find/till motion.
///
/// Direction is NOT stored — `repeat-find-forward` and `repeat-find-backward`
/// use absolute direction, so re-searching always means "next on the right" or
/// "previous on the left" regardless of the original motion's direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FindChar {
    pub ch: char,
    pub kind: FindKind,
}

// ── Mode ──────────────────────────────────────────────────────────────────────
//
// The editor uses `engine::types::EditorMode` directly. It unifies the old
// `Mode` enum and the `extend: bool` field: sticky extend is represented as
// `EditorMode::Extend`. One-shot ctrl-extend is a per-dispatch local variable
// and is NOT a mode change.
//
// `pub(crate) use EditorMode as Mode;` lets all internal modules use `Mode`
// as an unqualified alias without a rename migration sweep.
pub(crate) use engine::types::EditorMode as Mode;

// ── Editor ────────────────────────────────────────────────────────────────────

pub(crate) struct Editor {
    /// All open buffers. SSOT for buffer content, history, and file metadata.
    pub(crate) buffers: BufferStore,
    /// Current editing mode. `EditorMode::Extend` represents the sticky extend
    /// state (previously a separate `extend: bool` field). Mode is the single
    /// source of truth — `extend: bool` has been removed.
    pub(crate) mode: Mode,
    /// Keys consumed so far in the current multi-key sequence (max depth 3).
    ///
    /// Empty when at the trie root. Re-walked from the root on each new keypress.
    /// Cleared on Esc, on a successful command dispatch, or on NoMatch.
    pub(super) pending_keys: Vec<KeyEvent>,
    /// Accumulated numeric prefix for the next command (e.g. `3` in `3w`).
    ///
    /// `None` until the user starts typing digits. Defaults to `1` at dispatch.
    pub(super) count: Option<usize>,
    /// Pending wait-char state for a f/t/F/T/r binding.
    ///
    /// When `Some`, the next character keypress is consumed as an argument,
    /// stored in `pending_char`, and the named command is dispatched.
    /// Cleared immediately after use.
    pub(super) wait_char: Option<WaitCharPending>,
    /// Character argument for the current parameterized command (find/till/replace).
    ///
    /// Set just before dispatching a wait-char command; consumed (`.take()`) by
    /// `dispatch_editor_cmd`. Always `None` between commands.
    pub(super) pending_char: Option<char>,
    pub(super) registers: RegisterSet,
    pub(super) should_quit: bool,
    /// Active when the user is typing a command (`:`) or, later, a search (`/`).
    /// `None` when the mini-buffer is not visible.
    pub(crate) minibuf: Option<MiniBuffer>,
    /// Active completion session while a popup is showing.
    /// Cleared whenever the minibuffer closes or the user edits the input with
    /// any key other than Tab / Shift-Tab.
    pub(crate) completion: Option<completion::CompletionState>,
    /// Shared completion-popup view: written by `prepare_frame`, read by the
    /// `CompletionOverlay` provider during render.
    pub(crate) completion_view: Arc<RwLock<Option<crate::ui::completion_overlay::CompletionView>>>,
    /// Transient one-line message shown in the statusline after an action
    /// (e.g. "Written 42 lines", "Error: no file name"). Cleared on the next keypress.
    pub(crate) status_msg: Option<String>,
    /// Persistent log of warnings, errors, and trace entries accumulated during
    /// the session. Reviewed via `:messages`.
    pub(crate) message_log: MessageLog,
    /// When `Some`, the editor displays this read-only overlay instead of the
    /// real document. Dismissed with `q` or Escape. Used by `:messages`.
    pub(crate) scratch_view: Option<ScratchView>,
    /// All editor settings — global defaults and per-buffer-overridable values.
    ///
    /// This is the single source of truth for every configurable setting.
    /// Per-buffer overrides live on [`Buffer::overrides`]; resolution happens
    /// at read time via [`crate::settings::BufferOverrides`] accessor methods.
    pub(crate) settings: EditorSettings,
    /// Registry of all mappable commands (motions, selections, edits).
    ///
    /// Keyed by name; looked up by `execute_keymap_command` at dispatch time.
    pub(super) registry: CommandRegistry,
    /// The trie-based keymap for each mode.
    ///
    /// Built once at startup from [`Keymap::default`]. Extended by the Steel
    /// config layer to support user overrides.
    pub(super) keymap: Keymap,
    /// The character and kind (inclusive/exclusive) from the last find/till motion.
    ///
    /// Used by `repeat-find-forward` / `repeat-find-backward`.
    /// `None` until the user performs a find/till motion.
    pub(super) last_find: Option<FindChar>,

    // ── Search ────────────────────────────────────────────────────────────────
    pub(super) search: SearchState,

    // ── Per-pane state ─────────────────────────────────────────────────────────
    /// Per-(pane, buffer) state: selections, search cursor, in-progress edit group.
    ///
    /// Keyed first by `PaneId`, then by `BufferId`. The inner map holds exactly
    /// one entry per buffer that this pane has ever focused. Seeded in `open()`.
    pub(super) pane_state: SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>>,
    /// Per-pane transient state: pre-search and pre-select selection snapshots.
    pub(super) pane_transient: SecondaryMap<PaneId, PaneTransient>,

    // ── Engine rendering state ────────────────────────────────────────────────
    /// The engine's rendering state: layout, panes, buffers, theme.
    pub(crate) engine_view: EngineView,
    /// The single pane created in `open()`.
    pub(crate) focused_pane_id: PaneId,
    /// Shared bracket match highlight data: `(line_idx, byte_start, byte_end)`.
    /// Written by `update_highlight_providers()` each frame; read by the provider.
    pub(crate) bracket_hl_data: Arc<RwLock<Vec<(usize, usize, usize)>>>,
    /// Shared search match highlight data: same shape as `bracket_hl_data`.
    pub(crate) search_hl_data: Arc<RwLock<Vec<(usize, usize, usize)>>>,

    // ── Jump list ────────────────────────────────────────────────────────────
    /// Per-pane navigable history of cursor positions before large movements.
    /// `jump-backward` (Ctrl+O) / `jump-forward` (Ctrl+I) traverse each pane's list.
    pub(super) pane_jumps: SecondaryMap<PaneId, crate::core::jump_list::JumpList>,
    /// Whether the kitty keyboard protocol was successfully activated at startup.
    ///
    /// When `true`, the terminal sends CSI-u sequences that disambiguate
    /// Ctrl+h from Backspace, Ctrl+j from Enter, etc. — unlocking Ctrl+motion
    /// one-shot extend shortcuts. Set by the caller after [`Editor::open`].
    pub(crate) kitty_enabled: bool,

    // ── Visual-line movement ──────────────────────────────────────────────────

    /// Reusable scratch buffer for format operations in visual-line movement.
    ///
    /// Allocated once and reused every j/k press to avoid per-keypress
    /// heap allocation.
    pub(super) motion_format_scratch: engine::format::FormatScratch,

    // ── Dot-repeat fields ─────────────────────────────────────────────────────

    /// The last repeatable editing action, available for replay via `.`.
    /// `None` until the user performs a repeatable command.
    pub(super) last_action: Option<RepeatableAction>,
    /// Active insert session, present between [`begin_insert_session`] and
    /// [`end_insert_session`]. Keystroke recording for dot-repeat lives here.
    /// `None` at all other times — including during replay, where the replay
    /// path pre-opens the edit group to suppress session creation.
    pub(super) insert_session: Option<InsertSession>,
    /// Whether the user explicitly typed a count prefix before the current command.
    ///
    /// Set in `handle_normal` when `self.count` is `Some` before being consumed.
    /// Read by `cmd_repeat` to decide whether to use the new count or reuse the
    /// original action's count. Cleared after every dispatch.
    pub(super) explicit_count: bool,

    // ── Keyboard macro fields ─────────────────────────────────────────────────

    /// Active macro recording session.
    ///
    /// `Some((register, keys))` while recording is in progress; `None` otherwise.
    /// The register name was supplied after the initial `q` keypress.
    pub(super) macro_recording: Option<(char, Vec<KeyEvent>)>,

    /// Pending two-keystroke macro command.
    ///
    /// Set when `q` or `Q` is pressed; the next keypress is consumed as the
    /// register name. Cleared (and cancelled) on Esc or invalid input.
    pub(super) macro_pending: Option<MacroPending>,

    /// Queue of keys to replay before reading the next terminal event.
    ///
    /// Populated by the `q<reg>` replay path; drained by the main event loop
    /// one key at a time at the same stack depth as normal input. This avoids
    /// recursion for long macros and allows `should_quit` to be checked between
    /// replayed keys.
    pub(super) replay_queue: VecDeque<KeyEvent>,

    /// Single-frame flag: skip recording the current key.
    ///
    /// Set by the stop-recording `Q` intercept so that the stop key itself is
    /// not appended to the macro buffer. Checked and cleared unconditionally at
    /// the end of every `handle_key` call.
    pub(super) skip_macro_record: bool,

    /// `true` while draining the replay queue; suppresses nested `Q` recording.
    pub(super) is_replaying: bool,

    // ── Mouse ─────────────────────────────────────────────────────────────────

    /// Anchor char offset set on `MouseButton::Left` down when `mouse_select`
    /// is enabled. Cleared on mouse up.
    pub(super) mouse_drag_anchor: Option<usize>,

    // ── Scripting ────────────────────────────────────────────────────────────

    /// The embedded Steel scripting host.
    ///
    /// `None` until [`Editor::init_scripting`] is called (immediately after
    /// `open()` returns, before the event loop starts). `Some` for the rest
    /// of the editor's lifetime.
    pub(super) scripting: Option<crate::scripting::ScriptingHost>,
}

// proptest requires `Debug` on strategy values; this minimal impl satisfies it.
#[cfg(test)]
impl std::fmt::Debug for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Editor(buf={:?}, mode={:?})", self.doc().text().to_string(), self.mode)
    }
}

/// Project a `SelectionSet` into an engine pane's head-sorted selection mirror.
///
/// `SelectionSet` stores selections in `start()` order; the engine asserts they
/// are sorted by `head` (see `populate_sorted_sels`).  The two orderings differ
/// whenever a selection is backward (`anchor > head`).  `primary_idx` is
/// re-located after the sort by matching the primary's unique head value.
fn write_pane_mirror(pane: &mut engine::pane::Pane, sels: &SelectionSet) {
    let primary_head = sels.primary().head;
    pane.selections.clear();
    pane.selections.extend(
        sels.iter_head_sorted()
            .map(|s| EngineSelection { anchor: s.anchor, head: s.head }),
    );
    pane.primary_idx = pane.selections.iter()
        .position(|s| s.head == primary_head)
        .unwrap_or(0);
}

impl Editor {
    /// Open a file from disk, or create a new empty scratch buffer.
    ///
    /// The cursor starts at position 0 in Normal mode. Terminal dimensions are
    /// placeholder values replaced on the first event-loop iteration.
    pub(crate) fn open(file_path: Option<PathBuf>) -> io::Result<Self> {
        let doc = match file_path {
            Some(ref path) => Buffer::from_file(path)?,
            None => Buffer::new(Text::empty(), SelectionSet::single(Selection::collapsed(0))),
        };

        // ── Engine view setup ─────────────────────────────────────────────────
        let theme = crate::ui::theme::build_default_theme();
        let mut engine_view = EngineView::new(theme);

        // Intern highlight scopes before registering providers.
        let bracket_scope = engine_view.registry.intern("ui.cursor.match");
        let search_scope  = engine_view.registry.intern("ui.selection.search");

        // Register the shared highlight data arcs.
        let bracket_hl_data: Arc<RwLock<Vec<(usize, usize, usize)>>> = Arc::new(RwLock::new(Vec::new()));
        let search_hl_data:  Arc<RwLock<Vec<(usize, usize, usize)>>> = Arc::new(RwLock::new(Vec::new()));
        let completion_view: Arc<RwLock<Option<crate::ui::completion_overlay::CompletionView>>> =
            Arc::new(RwLock::new(None));

        // Insert a buffer — just metadata; the rope is passed at render time.
        let buffer_id = engine_view.buffers.insert(SharedBuffer::new());

        // Build the initial pane.
        let mut providers = engine::providers::ProviderSet::new();
        providers.add_gutter_column(Box::new(
            LineNumberColumn::with_style(EngineLineNumberStyle::Hybrid)
        ));
        providers.add_highlight_source(Box::new(crate::ui::highlight_providers::SharedHighlighter {
            scope: bracket_scope,
            tier: engine::providers::HighlightTier::BracketMatch,
            data: Arc::clone(&bracket_hl_data),
        }));
        providers.add_highlight_source(Box::new(crate::ui::highlight_providers::SharedHighlighter {
            scope: search_scope,
            tier: engine::providers::HighlightTier::SearchMatch,
            data: Arc::clone(&search_hl_data),
        }));
        providers.add_overlay(Box::new(crate::ui::completion_overlay::CompletionOverlay {
            data: Arc::clone(&completion_view),
        }));

        let settings = EditorSettings::default();

        let pane = Pane { providers, ..Pane::new(buffer_id) };
        let pane_id = engine_view.panes.insert(pane);
        engine_view.layout = LayoutTree::Leaf(pane_id);

        let jump_list_capacity = settings.jump_list_capacity;

        // Seed per-pane state from the buffer's history-root selections.
        let mut per_pane_bufs: SecondaryMap<BufferId, PaneBufferState> = SecondaryMap::new();
        per_pane_bufs.insert(buffer_id, pane_state::fresh_from_buf(&doc));
        let mut pane_state: SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>> = SecondaryMap::new();
        pane_state.insert(pane_id, per_pane_bufs);
        let mut pane_transient: SecondaryMap<PaneId, PaneTransient> = SecondaryMap::new();
        pane_transient.insert(pane_id, PaneTransient::default());

        // Bake theme now that all scopes are interned.
        engine_view.theme.bake(&engine_view.registry);

        let mut buffers = BufferStore::new();
        buffers.open(buffer_id, doc);

        Ok(Self {
            buffers,
            mode: Mode::Normal,
            pending_keys: Vec::new(),
            count: None,
            wait_char: None,
            pending_char: None,
            registers: RegisterSet::new(),
            should_quit: false,
            minibuf: None,
            completion: None,
            completion_view,
            status_msg: None,
            message_log: MessageLog::new(),
            scratch_view: None,
            settings,
            registry: CommandRegistry::with_defaults(),
            keymap: Keymap::default(),
            last_find: None,
            kitty_enabled: false,
            last_action: None,
            insert_session: None,
            explicit_count: false,
            search: SearchState::default(),
            pane_jumps: {
                let mut m = SecondaryMap::new();
                m.insert(pane_id, crate::core::jump_list::JumpList::new(jump_list_capacity));
                m
            },
            pane_state,
            pane_transient,
            engine_view,
            focused_pane_id: pane_id,
            bracket_hl_data,
            search_hl_data,
            motion_format_scratch: engine::format::FormatScratch::new(),
            macro_recording: None,
            macro_pending: None,
            replay_queue: VecDeque::new(),
            skip_macro_record: false,
            is_replaying: false,
            mouse_drag_anchor: None,
            scripting: None,
        })
    }

    /// Run the editor event loop until the user quits.
    ///
    /// Each iteration:
    /// 1. Prepare the frame: sync all editor state to the engine pane.
    /// 2. Render.
    /// 3. Block until the next terminal event.
    /// 4. Dispatch the event.
    pub(crate) fn run(&mut self, term: &mut Term) -> io::Result<()> {
        // Render context lives here — allocated once, reused every frame.
        // It must be outside `self` so `HumeStatusline { editor: self }` can
        // borrow `self` immutably while ctx is borrowed mutably.
        let mut ctx = RenderContext::new();
        let mut last_cursor_color_mode: Option<EditorMode> = None;
        loop {
            // ── 1. Prepare frame (single sync point) ─────────────────────────
            let size = term.size()?;
            self.prepare_frame(size.width, size.height, &mut ctx);

            // ── 2. Render ─────────────────────────────────────────────────────
            // Compute terminal cursor position before the draw closure to avoid
            // split-borrow conflicts: pane borrows and rope borrows must end
            // before `&mut self.engine_view` is captured by the closure.
            let cursor_screen = if let Some(mb) = &self.minibuf {
                // Minibuf active (Command / Search): place the terminal cursor
                // in the statusline at the minibuf edit position.
                let statusline_row = size.height.saturating_sub(1);
                Some((mb.statusline_cursor_col(), statusline_row))
            } else if self.mode.cursor_is_bar() {
                // Insert / Select: place the terminal cursor at the document head.
                let cursor_char = self.pane_state[self.focused_pane_id][self.focused_buffer_id()].selections.primary().head;
                let (vp, gutter_w) = {
                    let pane = &self.engine_view.panes[self.focused_pane_id];
                    let gw = crate::cursor::gutter_width(pane.providers.gutter_columns(), self.doc().text().len_lines());
                    (pane.viewport.clone(), gw)
                };
                let wrap_mode = self.doc().overrides.wrap_mode(&self.settings);
                let tab_width = self.doc().overrides.tab_width(&self.settings);
                let whitespace = self.doc().overrides.whitespace(&self.settings);
                crate::cursor::screen_pos(
                    &vp, self.doc().text().rope(), cursor_char,
                    &wrap_mode, tab_width, &whitespace,
                    &mut ctx,
                ).map(|(col, row)| (col + gutter_w, row))
            } else {
                None
            };

            // The statusline provider borrows `self` — create it before the
            // draw closure so the lifetime is tied to this stack frame.
            let statusline = crate::ui::statusline::HumeStatusline { editor: self };

            // Split borrows: `engine_view`, `doc`, and `scratch_view` are
            // disjoint fields of `self`. Extract the rope and pane settings
            // to render before moving `engine_view` into the draw closure.
            let rope: &ropey::Rope = if let Some(ref sv) = self.scratch_view {
                sv.buf.rope()
            } else {
                self.doc().text().rope()
            };
            let buffer_id = self.focused_buffer_id();
            let pane_id   = self.focused_pane_id;
            // Resolve mode and display settings once — passed to the engine via
            // closure so the engine never stores editor-domain state on Pane.
            let pane_settings = {
                let mode = if self.scratch_view.is_some() { EditorMode::Normal } else { self.mode };
                let wrap_mode  = self.doc().overrides.wrap_mode(&self.settings);
                let tab_width  = self.doc().overrides.tab_width(&self.settings);
                let whitespace = self.doc().overrides.whitespace(&self.settings);
                PaneRenderSettings { mode, wrap_mode, tab_width, whitespace }
            };
            let engine_view = &self.engine_view;
            term.draw(|frame| {
                engine_view.render(
                    frame.area(), frame.buffer_mut(),
                    |bid| if bid == buffer_id { Some(rope) } else { None },
                    |pid| if pid == pane_id { pane_settings.clone() } else { PaneRenderSettings::default() },
                    Some(&statusline),
                    &mut ctx,
                );
                if let Some((col, row)) = cursor_screen {
                    frame.set_cursor_position((col, row));
                }
            })?;

            // ── 2b. Cursor shape ──────────────────────────────────────────────
            // Emitted *after* draw so it's the last escape sequence the terminal
            // sees before we block — ratatui's ShowCursor flush can otherwise
            // reset the shape on some terminals.
            let _ = crate::os::terminal::set_cursor_shape(self.mode);
            if last_cursor_color_mode != Some(self.mode) {
                let _ = crate::os::terminal::set_cursor_color_for_mode(self.mode);
                last_cursor_color_mode = Some(self.mode);
            }

            // ── 3. Event ──────────────────────────────────────────────────────
            match event::read()? {
                // Release events arrive only with kitty keyboard protocol
                // (REPORT_EVENT_TYPES flag). Ignore them — we act on Press and
                // Repeat (held key). Without kitty all events are Press anyway.
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    self.handle_key(key);
                    self.sync_search_cache();
                }
                Event::Key(_) => {}
                Event::Mouse(mouse) => {
                    self.handle_mouse(mouse);
                }
                Event::Resize(_, _) => {} // dimensions re-read at loop top
                _ => {}
            }

            if self.should_quit {
                break;
            }

            // ── 4. Drain macro replay queue ───────────────────────────────────
            // Drain after handling the terminal event so that a key that
            // populates the queue (e.g. the register name after `Q`) causes
            // replay to run immediately — the results are visible on the very
            // next frame rather than requiring an additional keypress.
            // `last_action` is saved/restored so replay does not corrupt dot-repeat.
            self.drain_replay_queue();
            // One cache update covers the entire replay batch — the search
            // cache only changes when the buffer revision changes, so calling
            // it per-key would redundantly clone the regex on every iteration.
            self.sync_search_cache();
            if self.should_quit { break; }
        }
        // Restore the user's default cursor shape and colour before returning to the shell.
        crate::os::terminal::reset_cursor_shape()?;
        let _ = crate::os::terminal::set_cursor_color_for_mode(EditorMode::Normal); // emits reset sequence
        Ok(())
    }

    /// Prepare the engine pane for rendering by syncing all editor-authoritative
    /// state in one place, once per frame.
    ///
    /// `sync_all_pane_mirrors` is the **single sync point** for `pane.selections`
    /// and `pane.primary_idx` — it covers every pane in one pass.  No other code
    /// path writes those fields.  Highlight and statusline shared buffers are also
    /// written here, immediately before every `render()` call.  Mode and display
    /// settings are resolved lazily via the `get_pane_settings` closure passed to
    /// `render()`.
    fn prepare_frame(&mut self, terminal_width: u16, terminal_height: u16, ctx: &mut RenderContext) {
        // 1. Sync viewport dimensions.
        // Engine reserves 1 row for the statusline; the pane gets the rest.
        {
            let vp = self.viewport_mut();
            vp.width  = terminal_width;
            vp.height = terminal_height.saturating_sub(1);
        }

        // 2. Sync selection mirrors for every pane (scratch-view override is
        //    handled inside sync_all_pane_mirrors for the focused pane).
        self.sync_all_pane_mirrors();

        if let Some(ref sv) = self.scratch_view {
            // ── Scratch view path ─────────────────────────────────────────────
            // Use the scratch rope for scroll calculations.
            // The real document and all highlight providers are untouched.
            let cursor_char = sv.sels.primary().head;
            let rope = sv.buf.rope();
            let v_margin = self.settings.scroll_margin;
            let h_margin = self.settings.scroll_margin_h;
            let wrap_mode = self.settings.wrap_mode.clone();
            let tab_width = self.settings.tab_width;
            let whitespace = self.settings.whitespace.clone();
            let pane = &mut self.engine_view.panes[self.focused_pane_id];
            scroll_into_view(pane, rope, cursor_char, &mut ctx.cursor_format, &wrap_mode, tab_width, &whitespace, v_margin, h_margin);
            // No highlight updates for scratch view — no search or bracket matches.
        } else {
            // ── Normal document path ──────────────────────────────────────────

            // 3. Sync line-number style provider (depends on buffer overrides).
            {
                let ln_style = self.doc().overrides.line_number_style(&self.settings);
                self.engine_view.panes[self.focused_pane_id].providers.sync_line_number_style(ln_style);
            }

            // 4. Scroll so the primary cursor stays visible.
            let cursor_char = self.pane_state[self.focused_pane_id][self.focused_buffer_id()].selections.primary().head;
            let v_margin = self.settings.scroll_margin;
            let h_margin = self.settings.scroll_margin_h;
            let wrap_mode = self.doc().overrides.wrap_mode(&self.settings);
            let tab_width = self.doc().overrides.tab_width(&self.settings);
            let whitespace = self.doc().overrides.whitespace(&self.settings);
            {
                let buf_id = self.focused_buffer_id();
                let rope = self.buffers.get(buf_id).text().rope();
                let pane = &mut self.engine_view.panes[self.focused_pane_id];
                scroll_into_view(pane, rope, cursor_char, &mut ctx.cursor_format, &wrap_mode, tab_width, &whitespace, v_margin, h_margin);
            }

            // 5. Sync highlight data (search matches, bracket matches) to shared
            //    Arc buffers read by the highlight providers during rendering.
            self.update_highlight_providers();

            // 6. Sync completion-popup view to the shared Arc for `CompletionOverlay`.
            self.sync_completion_view();
        }
    }

    // ── Message reporting ─────────────────────────────────────────────────────

    /// Report a message, routing it based on severity:
    ///
    /// - `Info`    → set `status_msg` only (ephemeral, not logged)
    /// - `Warning` → push to `message_log` AND set `status_msg`
    /// - `Error`   → push to `message_log` AND set `status_msg`
    /// - `Trace`   → push to `message_log` only (not shown in statusline)
    pub(crate) fn report(&mut self, severity: Severity, text: String) {
        match severity {
            Severity::Info => {
                self.status_msg = Some(text);
            }
            Severity::Warning | Severity::Error => {
                self.message_log.push(severity, text.clone());
                self.status_msg = Some(text);
            }
            Severity::Trace => {
                self.message_log.push(severity, text);
            }
        }
    }

    /// Drain any pending `(log! …)` messages from the scripting host and
    /// report each one.  Collected into a temporary vec first to satisfy the
    /// borrow checker (both `self.scripting` and `self` are `&mut`).
    pub(crate) fn flush_script_messages(&mut self) {
        let msgs = self.scripting
            .as_mut()
            .map(|h| h.pending_messages.drain(..).collect::<Vec<_>>())
            .unwrap_or_default();
        for (sev, text) in msgs {
            self.report(sev, text);
        }
    }

    // ── Hook firing ──────────────────────────────────────────────────────────

    /// Fire `OnBufferSave` hooks for `bid`. Both `:w` write paths in
    /// `commands.rs` share this rather than duplicating the arg construction.
    pub(super) fn fire_hook_buffer_save(&mut self, bid: BufferId) {
        let val = SteelBufferId(bid).into_steel_val();
        self.fire_hook_silent(HookId::OnBufferSave, &[val]);
    }

    /// Fire all Steel handlers for `hook_id`, passing `args` to each.
    ///
    /// No-ops immediately if no scripting host is present or if no handlers
    /// are registered for the hook.  Commands queued by `(call! …)` inside
    /// hook bodies are dispatched after all handlers return.  Errors from
    /// handlers are reported as `Severity::Error`.
    pub(super) fn fire_hook_silent(&mut self, hook_id: HookId, args: &[steel::rvals::SteelVal]) {
        if self.scripting.as_ref().is_none_or(|h| h.hooks.is_empty_for(hook_id)) {
            return;
        }
        let pid = self.focused_pane_id;
        let bid = self.focused_buffer_id();
        let result = {
            let host = self.scripting.as_mut().expect("checked above");
            host.fire_hook(
                hook_id, args,
                EditorSteelRefs {
                    settings:          &mut self.settings,
                    keymap:            &mut self.keymap,
                    focused_pane_id:   pid,
                    focused_buffer_id: bid,
                    buffers:           Some(&mut self.buffers),
                    engine_view:       Some(&mut self.engine_view),
                    pane_state:        Some(&mut self.pane_state),
                    pane_jumps:        Some(&mut self.pane_jumps),
                },
            )
        };
        self.flush_script_messages();
        match result {
            Ok(queue) => {
                for cmd in queue {
                    self.execute_keymap_command(cmd.into(), 1, false, None);
                }
            }
            Err(e) => self.report(Severity::Error, format!("hook error: {e}")),
        }
    }

    // ── Scripting ─────────────────────────────────────────────────────────────

    /// Initialise the Steel scripting host and evaluate `init.scm`.
    ///
    /// Must be called once, after `Editor::open` returns and before
    /// `Editor::run` starts. Any error from `init.scm` is reported as
    /// `Severity::Error` and shown in the statusline.
    pub(crate) fn init_scripting(&mut self) {
        // Resolve the config path up front. `None` means neither XDG_CONFIG_HOME
        // nor HOME (APPDATA on Windows) is set — there is no meaningful place
        // to look for init.scm, so we skip scripting entirely and log a warning.
        let Some(config_dir) = crate::os::dirs::config_dir() else {
            self.report(Severity::Warning, "scripting: no config directory — HOME/APPDATA unset; init.scm skipped".into());
            return;
        };
        let init_path = config_dir.join("init.scm");
        let mut host = crate::scripting::ScriptingHost::new();
        // Trace the resolved directories so they're visible in `:messages`.
        // A missing runtime dir is a warning because `core:*` plugins need it.
        match &host.runtime_dir {
            Some(rt) => self.report(Severity::Trace, format!("scripting: runtime dir = {}", rt.display())),
            None => self.report(Severity::Warning, "scripting: no runtime directory found — core:* plugins unavailable; set HUME_RUNTIME to fix".into()),
        }
        match &host.data_dir {
            Some(d) => self.report(Severity::Trace, format!("scripting: data dir = {}", d.display())),
            None => self.report(Severity::Warning, "scripting: no data directory — HOME/APPDATA unset; user plugins unavailable".into()),
        }
        let builtin_names: std::collections::HashSet<String> =
            self.registry.names().map(String::from).collect();
        match host.eval_init(&init_path, &mut self.settings, &mut self.keymap, builtin_names) {
            Ok(cmds) => self.register_steel_cmds(cmds),
            Err(msg) => self.report(Severity::Error, format!("init.scm: {msg}")),
        }
        // Flush any `(log! …)` messages produced during init.scm evaluation.
        for (sev, text) in host.pending_messages.drain(..) {
            self.report(sev, text);
        }
        self.scripting = Some(host);
    }

    // ── Engine accessors ──────────────────────────────────────────────────────

    pub(crate) fn viewport(&self) -> &ViewportState {
        &self.engine_view.panes[self.focused_pane_id].viewport
    }

    pub(crate) fn viewport_mut(&mut self) -> &mut ViewportState {
        &mut self.engine_view.panes[self.focused_pane_id].viewport
    }

    /// Sync every engine pane's selection mirror from the authoritative `pane_state`.
    ///
    /// The engine requires `pane.selections` sorted by `head` (not by `start()` as
    /// `SelectionSet` stores internally); `primary_idx` is re-located by matching
    /// the primary's head value after the sort.  This is the **single sync point** —
    /// no other code path writes `pane.selections` or `pane.primary_idx`.
    ///
    /// Called once per frame from `prepare_frame`, before `render()`.
    pub(crate) fn sync_all_pane_mirrors(&mut self) {
        let Self { pane_state, engine_view, scratch_view, focused_pane_id, .. } = &mut *self;
        for (pid, pane) in engine_view.panes.iter_mut() {
            if pid == *focused_pane_id {
                if let Some(sv) = scratch_view.as_ref() {
                    write_pane_mirror(pane, &sv.sels);
                    continue;
                }
            }
            if let Some(pbs) = pane_state.get(pid).and_then(|m| m.get(pane.buffer_id)) {
                write_pane_mirror(pane, &pbs.selections);
            }
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Accessor for the focused buffer's active search pattern.
    pub(crate) fn search_pattern(&self) -> Option<&SearchPattern> {
        self.buffers.get(self.focused_buffer_id()).search_pattern.as_ref()
    }

    /// Accessor for the focused buffer's match cache.
    #[cfg(test)]
    pub(crate) fn search_matches(&self) -> &SearchMatches {
        &self.buffers.get(self.focused_buffer_id()).search_matches
    }

    /// Accessor for the focused pane's search cursor (match count, wrapped flag).
    pub(crate) fn current_search_cursor(&self) -> &SearchCursor {
        &self.pane_state[self.focused_pane_id][self.focused_buffer_id()].search_cursor
    }

    /// Mutable accessor for the focused pane's search cursor.
    pub(crate) fn current_search_cursor_mut(&mut self) -> &mut SearchCursor {
        let bid = self.focused_buffer_id();
        &mut self.pane_state[self.focused_pane_id][bid].search_cursor
    }

    /// Clear the active search state for buffer `bid`: drop the pattern,
    /// reset the match cache, and reset every pane's search cursor.
    pub(crate) fn clear_buffer_search(&mut self, bid: BufferId) {
        let buf = self.buffers.get_mut(bid);
        buf.search_pattern = None;
        buf.search_matches = SearchMatches::default();
        for buf_map in self.pane_state.values_mut() {
            if let Some(state) = buf_map.get_mut(bid) {
                state.search_cursor = SearchCursor::default();
            }
        }
    }

    /// Recompute the match list for `bid` if the pattern or revision changed.
    ///
    /// No-op when no search is active. Designed so calling it per-key is cheap —
    /// the cache check short-circuits before any regex work when nothing changed.
    pub(super) fn update_buffer_matches(&mut self, bid: BufferId) {
        {
            let buf = self.buffers.get(bid);
            let Some(sp) = buf.search_pattern.as_ref() else { return; };
            let sm = &buf.search_matches;
            if sm.cache == Some((buf.revision_id(), sp.pattern_str.clone())) {
                return;
            }
        }
        let (pattern_str, regex, revision) = {
            let buf = self.buffers.get(bid);
            let sp = buf.search_pattern.as_ref().expect("checked above");
            (sp.pattern_str.clone(), Arc::clone(&sp.regex), buf.revision_id())
        };

        let matches = find_all_matches(self.buffers.get(bid).text(), &regex);
        let sm = &mut self.buffers.get_mut(bid).search_matches;
        sm.matches = matches;
        sm.cache = Some((revision, pattern_str));
    }

    /// Recompute `pane_state[pid][bid].search_cursor.match_count` if stale.
    ///
    /// Short-circuits when head position, match-list revision, and pattern
    /// all match the cached values — zero regex work on cache hit.
    pub(super) fn update_pane_cursor(&mut self, pid: PaneId, bid: BufferId) {
        let head = self.pane_state[pid][bid].selections.primary().head;
        {
            let sm = &self.buffers.get(bid).search_matches;
            let cur = &self.pane_state[pid][bid].search_cursor;
            if cur.cache_head == Some(head) && cur.cache_matches == sm.cache {
                return;
            }
        }

        let (match_count, cache_matches) = {
            let sm = &self.buffers.get(bid).search_matches;
            if sm.cache.is_none() {
                // Buffer has no active search — cursor should be default.
                return;
            }
            let count = search_match_info(&sm.matches, head);
            (Some(count), sm.cache.clone())
        };

        let cursor = &mut self.pane_state[pid][bid].search_cursor;
        cursor.match_count = match_count;
        cursor.cache_head = Some(head);
        cursor.cache_matches = cache_matches;
    }

    /// Convenience: run `update_buffer_matches` + `update_pane_cursor` for the
    /// focused pane/buffer. Replaces the old `update_search_cache`.
    pub(super) fn sync_search_cache(&mut self) {
        let bid = self.focused_buffer_id();
        let pid = self.focused_pane_id;
        self.update_buffer_matches(bid);
        self.update_pane_cursor(pid, bid);
    }

    /// Write per-frame highlight data to the shared `Arc<RwLock<...>>` buffers
    /// read by `BracketMatchHighlighter` and `SearchMatchHighlighter`.
    ///
    /// Called once per frame, after scroll is resolved and before `term.draw`.
    /// Bracket matching is suppressed in Insert mode.
    pub(super) fn update_highlight_providers(&mut self) {
        let buf = self.doc().text();

        // Visible line range — skip matches outside the viewport (search matches
        // are sorted by document order, so we can break early past the bottom).
        let top_line = self.viewport().top_line;
        let bot_line = top_line + self.viewport().height as usize;

        // ── Search match highlights ───────────────────────────────────────────
        {
            let mut data = self.search_hl_data.write().expect("RwLock not poisoned");
            data.clear();
            // Hidden in Insert mode — matches aren't actionable while typing and
            // clutter the view. Same pattern as bracket match highlights below.
            if self.mode != EditorMode::Insert {
                // Matches are sorted by document order. Binary-search to the first
                // match that starts at or after `top_line` to skip pre-viewport entries.
                let top_char = buf.line_to_char(top_line.min(buf.len_lines().saturating_sub(1)));
                let matches = &self.buffers.get(self.focused_buffer_id()).search_matches.matches;
                let first = matches.partition_point(|&(start, _)| start < top_char);
                for &(start, end_incl) in &matches[first..] {
                    let (line, byte_start) = char_to_line_byte(buf, start);
                    if line > bot_line { break; }
                    // end_incl is inclusive char offset; +1 makes it exclusive in chars,
                    // then convert to byte.
                    let end_char = (end_incl + 1).min(buf.len_chars());
                    let (_, byte_end) = char_to_line_byte(buf, end_char);
                    data.push((line, byte_start, byte_end));
                }
            }
        }

        // ── Bracket match highlight ───────────────────────────────────────────
        {
            let mut data = self.bracket_hl_data.write().expect("RwLock not poisoned");
            data.clear();
            if self.mode != EditorMode::Insert {
                let head = self.pane_state[self.focused_pane_id][self.focused_buffer_id()].selections.primary().head;
                if let Some(ch) = buf.char_at(head) {
                    let pair = match ch {
                        '(' | ')' => Some(('(', ')')),
                        '[' | ']' => Some(('[', ']')),
                        '{' | '}' => Some(('{', '}')),
                        '<' | '>' => Some(('<', '>')),
                        _ => None,
                    };
                    if let Some((open, close)) = pair
                        && let Some((op, cp)) = find_bracket_pair(buf, head, open, close)
                    {
                        let match_pos = if head == op { cp } else { op };
                        let (line, byte) = char_to_line_byte(buf, match_pos);
                        // Single-char match: byte_end = byte + utf8 length of the char.
                        let ch_len = buf.char_at(match_pos).map(|c| c.len_utf8()).unwrap_or(1);
                        data.push((line, byte, byte + ch_len));
                    }
                }
            }
        }
    }

    /// Write the current completion state into the shared `CompletionView` Arc
    /// so `CompletionOverlay` can render it during this frame.
    ///
    /// Called from `prepare_frame` after highlight data is synced.
    fn sync_completion_view(&self) {
        use unicode_width::UnicodeWidthChar as _;
        use unicode_width::UnicodeWidthStr as _;
        let view = self.completion.as_ref().map(|state| {
            let anchor_col = self.minibuf.as_ref().map(|mb| {
                let pad: u16 = 1;
                let prompt_w = mb.prompt.width().unwrap_or(1) as u16;
                let safe_end = state.span_start.min(mb.input.len());
                let token_col = mb.input[..safe_end].width() as u16;
                pad + prompt_w + token_col
            }).unwrap_or(0);
            crate::ui::completion_overlay::CompletionView {
                rows: state.candidates.iter().map(|c| c.display.clone()).collect(),
                selected: state.selected,
                anchor_col,
            }
        });
        *self.completion_view.write().expect("RwLock not poisoned") = view;
    }

    /// Set the editing mode. The cursor shape reflecting the new mode will be
    /// emitted after the current frame's draw call.
    ///
    /// For Insert mode entry and exit use [`begin_insert_session`] and
    /// [`end_insert_session`] instead — they manage the undo group and
    /// dot-repeat recording alongside the mode change.
    pub(super) fn set_mode(&mut self, mode: EditorMode) {
        let old = self.mode;
        self.mode = mode;
        if old != mode && self.scripting.as_ref().is_some_and(|h| !h.hooks.is_empty_for(HookId::OnModeChange)) {
            let old_val = mode_name(old).into_steelval().expect("mode str into_steelval");
            let new_val = mode_name(mode).into_steelval().expect("mode str into_steelval");
            self.fire_hook_silent(HookId::OnModeChange, &[old_val, new_val]);
        }
    }

    // ── Buffer accessors ──────────────────────────────────────────────────────

    /// The `BufferId` the focused pane is currently viewing.
    pub(crate) fn focused_buffer_id(&self) -> BufferId {
        self.engine_view.panes[self.focused_pane_id].buffer_id
    }

    /// Shared reference to the focused buffer.
    pub(crate) fn doc(&self) -> &Buffer {
        self.buffers.get(self.focused_buffer_id())
    }

    /// The most-recently-focused buffer other than the current one, or `None`
    /// when only one buffer is open. Derives from `BufferStore.mru` (SSOT).
    pub(crate) fn alternate_buffer(&self) -> Option<BufferId> {
        self.buffers.mru_excluding(self.focused_buffer_id())
    }

    /// Mutable reference to the focused buffer.
    ///
    /// Uses a split borrow — `buffers` and other fields on `Editor` are
    /// disjoint, so you can hold this reference while reading e.g. `self.settings`.
    /// Do NOT keep this reference live across a call that also borrows `self`.
    pub(crate) fn doc_mut(&mut self) -> &mut Buffer {
        let bid = self.focused_buffer_id();
        self.buffers.get_mut(bid)
    }

    // ── Pane-state accessors ──────────────────────────────────────────────────

    /// The focused pane's selections for the current buffer.
    pub(super) fn current_selections(&self) -> &SelectionSet {
        &self.pane_state[self.focused_pane_id][self.focused_buffer_id()].selections
    }

    /// Replace the focused pane's selections for the current buffer.
    pub(super) fn set_current_selections(&mut self, sels: SelectionSet) {
        let bid = self.focused_buffer_id();
        self.pane_state[self.focused_pane_id][bid].selections = sels;
    }

    // ── Doc-edit wrappers ─────────────────────────────────────────────────────

    /// Propagate a committed ChangeSet to all non-acting panes viewing `buf_id`.
    ///
    /// `rope_pre` is the buffer text **before** the edit — required by
    /// `translate_in_place` to identify which line each head was on before
    /// mapping, so it can decide whether to reset `Selection.horiz`.
    ///
    /// The engine pane mirrors are **not** updated here; `sync_all_pane_mirrors`
    /// in the next `prepare_frame` handles that.  Only the authoritative
    /// `SelectionSet` in `pane_state` must be kept rope-valid between edits,
    /// because other mid-event code (e.g. `update_pane_cursor`) reads it.
    fn propagate_cs_to_panes(&mut self, buf_id: BufferId, cs: &ChangeSet, rope_pre: &ropey::Rope) {
        let focused = self.focused_pane_id;

        // Collect affected pane IDs before mutating to satisfy the borrow checker.
        let affected: Vec<PaneId> = self.pane_state.iter()
            .filter_map(|(pid, buf_map)| {
                (pid != focused && buf_map.contains_key(buf_id)).then_some(pid)
            })
            .collect();

        for pid in affected {
            self.pane_state[pid][buf_id].selections.translate_in_place(cs, rope_pre);
        }
    }

    /// Apply an ungrouped edit: read selections from pane_state, call
    /// `doc.apply_edit`, write new selections back, propagate CS to other panes.
    /// Returns `(displaced, cs)`.
    pub(super) fn doc_edit<R: IntoApplyResult>(
        &mut self,
        cmd: impl FnOnce(Text, SelectionSet) -> R,
    ) -> (Option<Vec<String>>, ChangeSet) {
        let pane_id = self.focused_pane_id;
        let buf_id = self.focused_buffer_id();
        // Snapshot pre-edit rope (O(1) — ropey uses structural sharing).
        let rope_pre = self.buffers.get(buf_id).text().rope().clone();
        let sels = self.pane_state[pane_id][buf_id].selections.clone();
        let (new_sels, displaced, cs) = self.buffers.get_mut(buf_id).apply_edit(sels, cmd);
        self.pane_state[pane_id][buf_id].selections = new_sels;
        self.propagate_cs_to_panes(buf_id, &cs, &rope_pre);
        (displaced, cs)
    }

    /// Apply a grouped edit (inside an insert session). Reads and writes
    /// selections via pane_state, propagates CS to other panes.
    /// Returns `(displaced, cs)`.
    ///
    /// The split borrow (`&mut self.buffers` ∥ `&mut self.pane_state`)
    /// is safe because both are disjoint fields of `Editor`.
    pub(super) fn doc_edit_grouped<R: IntoApplyResult>(
        &mut self,
        cmd: impl FnOnce(Text, SelectionSet) -> R,
    ) -> (Option<Vec<String>>, ChangeSet) {
        let pane_id = self.focused_pane_id;
        let buf_id = self.focused_buffer_id();
        let rope_pre = self.buffers.get(buf_id).text().rope().clone();
        let sels = self.pane_state[pane_id][buf_id].selections.clone();
        // Split borrow: self.buffers and self.pane_state are disjoint fields.
        let doc = self.buffers.get_mut(buf_id);
        let pbs = &mut self.pane_state[pane_id][buf_id];
        let (new_sels, displaced, cs) = doc.apply_edit_grouped(sels, &mut pbs.edit_group, cmd);
        pbs.selections = new_sels;
        // propagate_cs_to_panes needs &mut self; the split borrows above have ended.
        self.propagate_cs_to_panes(buf_id, &cs, &rope_pre);
        (displaced, cs)
    }

    /// Apply undo to the focused buffer and propagate the inverse CS to other panes.
    pub(super) fn doc_undo(&mut self) {
        let pane_id = self.focused_pane_id;
        let buf_id = self.focused_buffer_id();
        // rope_pre for undo is the *current* (post-edit) text: undo's CS maps
        // post-edit positions back to pre-edit, so non-acting panes' heads (which
        // live in post-edit space) must be translated through that CS.
        let rope_pre = self.buffers.get(buf_id).text().rope().clone();
        if let Some((new_sels, cs)) = self.buffers.get_mut(buf_id).undo() {
            self.pane_state[pane_id][buf_id].selections = new_sels;
            self.propagate_cs_to_panes(buf_id, &cs, &rope_pre);
        }
    }

    /// Apply redo to the focused buffer and propagate the forward CS to other panes.
    pub(super) fn doc_redo(&mut self) {
        let pane_id = self.focused_pane_id;
        let buf_id = self.focused_buffer_id();
        let rope_pre = self.buffers.get(buf_id).text().rope().clone();
        if let Some((new_sels, cs)) = self.buffers.get_mut(buf_id).redo() {
            self.pane_state[pane_id][buf_id].selections = new_sels;
            self.propagate_cs_to_panes(buf_id, &cs, &rope_pre);
        }
    }

    fn is_group_open_current(&self) -> bool {
        self.pane_state[self.focused_pane_id][self.focused_buffer_id()].edit_group.is_some()
    }

    fn begin_edit_group_current(&mut self) {
        let pane_id = self.focused_pane_id;
        let buf_id = self.focused_buffer_id();
        let sels = self.pane_state[pane_id][buf_id].selections.clone();
        let doc = self.buffers.get_mut(buf_id);
        let pbs = &mut self.pane_state[pane_id][buf_id];
        doc.begin_edit_group(&mut pbs.edit_group, sels);
    }

    fn commit_edit_group_current(&mut self) {
        let pane_id = self.focused_pane_id;
        let buf_id = self.focused_buffer_id();
        let sels = self.pane_state[pane_id][buf_id].selections.clone();
        let doc = self.buffers.get_mut(buf_id);
        let pbs = &mut self.pane_state[pane_id][buf_id];
        doc.commit_edit_group(&mut pbs.edit_group, sels);
    }

    // ── Mode transitions ──────────────────────────────────────────────────────

    /// Enter Insert mode as a repeatable insert action.
    ///
    /// Opens a new undo edit group and starts keystroke recording for
    /// dot-repeat, then sets the mode to Insert.
    ///
    /// **Replay signal**: if an edit group is already open when this is called,
    /// recording is suppressed but the mode change still happens. The replay
    /// path in [`cmd_repeat`] pre-opens the group before re-executing the
    /// original command, so that the re-executed command's call here becomes a
    /// no-op for undo/repeat purposes — only the cursor motion takes effect.
    pub(super) fn begin_insert_session(&mut self) {
        if !self.is_group_open_current() {
            self.begin_edit_group_current();
            self.insert_session = Some(InsertSession { keystrokes: Vec::new() });
        }
        self.mode = Mode::Insert;
    }

    /// Exit Insert mode and finalise the undo/repeat state.
    ///
    /// Commits the open edit group (creating one undo step for the whole
    /// insert session) and moves the recorded keystrokes into `last_action`
    /// for dot-repeat, then sets the mode to Normal.
    pub(super) fn end_insert_session(&mut self) {
        self.commit_edit_group_current();
        if let (Some(session), Some(action)) =
            (self.insert_session.take(), self.last_action.as_mut())
        {
            action.insert_keys = session.keystrokes;
        }
        // Engine pane is synced by `prepare_frame` each frame.
        self.mode = EditorMode::Normal;
    }

    /// Apply a motion command and store the resulting selection.
    pub(super) fn apply_motion(&mut self, f: impl FnOnce(&Text, SelectionSet) -> SelectionSet) {
        let pane_id = self.focused_pane_id;
        let buf_id = self.focused_buffer_id();
        let new_sels = {
            let buf = self.doc().text();
            let sels = self.pane_state[pane_id][buf_id].selections.clone();
            f(buf, sels)
        };
        self.pane_state[pane_id][buf_id].selections = new_sels;
    }

    /// Drain the macro replay queue, executing each key in order.
    ///
    /// Sets `is_replaying` for the duration so that `Q`/`q` intercepts inside
    /// replayed keys cannot start nested recording or replay sessions — including
    /// when the last key in the macro is `Q` (where `replay_queue.is_empty()`
    /// would already be `true` and would fail to suppress it).
    ///
    /// Saves and restores `last_action` so replay does not corrupt dot-repeat.
    pub(crate) fn drain_replay_queue(&mut self) {
        let saved_action = self.last_action.take();
        self.is_replaying = true;
        while let Some(key) = self.replay_queue.pop_front() {
            self.handle_key(key);
            if self.should_quit { break; }
        }
        self.is_replaying = false;
        self.last_action = saved_action;
    }

    // ── Scripting helpers ─────────────────────────────────────────────────────

    /// Register each `SteelCmdDef` in the command registry, reporting
    /// conflicts as errors.  Used after both init and plugin-reload evals.
    pub(super) fn register_steel_cmds(&mut self, defs: impl IntoIterator<Item = SteelCmdDef>) {
        for def in defs {
            if self.registry.get_mappable(&def.name).is_some() {
                self.report(Severity::Error, format!(
                    "define-command!: '{}' conflicts with existing command", def.name));
            } else {
                self.registry.register(registry::MappableCommand::SteelBacked {
                    name: def.name.into(),
                    doc: def.doc.into(),
                    steel_proc: def.steel_proc,
                });
            }
        }
    }

    // ── Buffer choke-points ───────────────────────────────────────────────────

    /// Dedup-open a canonicalized path: returns `(id, false)` if already open,
    /// `(id, true)` if newly opened (including `OnBufferOpen` hook fire).
    pub(super) fn open_or_dedup(&mut self, canonical: &std::path::Path) -> std::io::Result<(BufferId, bool)> {
        if let Some(existing) = self.buffers.find_by_path(canonical) {
            return Ok((existing, false));
        }
        Ok((self.open_buffer(Buffer::from_file(canonical)?), true))
    }

    /// Allocate a new buffer slot (engine + BufferStore), seed the focused pane's
    /// `pane_state`, and return the allocated `BufferId`.
    pub(crate) fn open_buffer(&mut self, doc: Buffer) -> BufferId {
        let bid = ops::open_buffer(
            &mut self.engine_view, &mut self.buffers, &mut self.pane_state,
            self.focused_pane_id, doc,
        );
        let val = SteelBufferId(bid).into_steel_val();
        self.fire_hook_silent(HookId::OnBufferOpen, &[val]);
        bid
    }

    /// Remove buffer `id`, handling two cases:
    ///
    /// - At least one other buffer: redirect every pane viewing `id` to the
    ///   MRU replacement, then free the slot.
    /// - Only buffer: replace in-place with a fresh scratch buffer.
    pub(crate) fn close_buffer(&mut self, id: BufferId) {
        ops::close_buffer(
            &mut self.engine_view, &mut self.buffers, &mut self.pane_state,
            &mut self.pane_jumps, self.focused_pane_id, id,
        );
        // Fire with the ID that was closed, not the new current buffer.
        let val = SteelBufferId(id).into_steel_val();
        self.fire_hook_silent(HookId::OnBufferClose, &[val]);
    }

    /// Replace buffer `id` with `new_doc` in-place, reseeding all pane state.
    ///
    /// Used by `:e!` reload. Caller contract: `new_doc.search_pattern` must be `None`
    /// (enforced by debug_assert — `Buffer::from_file` satisfies this by construction).
    pub(crate) fn replace_buffer_in_place(&mut self, id: BufferId, new_doc: Buffer) {
        ops::replace_buffer_in_place(
            &mut self.engine_view, &mut self.buffers, &mut self.pane_state,
            &mut self.pane_jumps, id, new_doc,
        );
    }

    /// Redirect the focused pane to `target` without recording a jump.
    pub(crate) fn switch_to_buffer_without_jump(&mut self, target: BufferId) {
        let pid = self.focused_pane_id;
        ops::switch_pane_to_buffer(&mut self.engine_view, &self.buffers, &mut self.pane_state, pid, target);
    }

    /// Redirect the focused pane to `target`, recording the outgoing position
    /// in `pane_jumps[focused_pane]`.
    ///
    /// Caller contract: all fallible steps (path resolution, file read, etc.)
    /// must succeed before calling this — `push()` truncates forward history.
    pub(crate) fn switch_to_buffer_with_jump(&mut self, target: BufferId) {
        let current = self.focused_buffer_id();
        ops::switch_to_buffer_with_jump(
            &mut self.engine_view, &self.buffers, &mut self.pane_state,
            &mut self.pane_jumps, self.focused_pane_id, current, target,
        );
    }

    /// Snapshot the focused pane's current cursor as a `JumpEntry`.
    pub(crate) fn current_jump_entry(&self) -> crate::core::jump_list::JumpEntry {
        use crate::core::jump_list::JumpEntry;
        let pid = self.focused_pane_id;
        let bid = self.focused_buffer_id();
        let sels = self.pane_state[pid][bid].selections.clone();
        JumpEntry::new(sels, self.buffers.get(bid).text(), bid)
    }
}

// ── Test constructors ─────────────────────────────────────────────────────────

#[cfg(test)]
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
        let buffer_id = engine_view.buffers.insert(SharedBuffer::new());
        let settings = EditorSettings::default();
        let jump_list_capacity = settings.jump_list_capacity;
        let pane = Pane::new(buffer_id);
        let pane_id = engine_view.panes.insert(pane);
        engine_view.layout = LayoutTree::Leaf(pane_id);
        engine_view.theme.bake(&engine_view.registry);

        let mut buffers = BufferStore::new();
        buffers.open(buffer_id, doc);

        let mut pane_state: SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>> = SecondaryMap::new();
        pane_state.insert(pane_id, SecondaryMap::new());
        pane_state::ensure(&mut pane_state, &buffers, pane_id, buffer_id);
        let mut pane_transient: SecondaryMap<PaneId, PaneTransient> = SecondaryMap::new();
        pane_transient.insert(pane_id, PaneTransient::default());

        Self {
            buffers,
            mode: Mode::Normal,
            pending_keys: Vec::new(),
            count: None,
            wait_char: None,
            pending_char: None,
            registers: RegisterSet::new(),
            should_quit: false,
            minibuf: None,
            completion: None,
            completion_view: Arc::new(RwLock::new(None)),
            status_msg: None,
            message_log: MessageLog::new(),
            scratch_view: None,
            settings,
            registry: registry::CommandRegistry::with_defaults(),
            keymap: keymap::Keymap::default(),
            last_find: None,
            kitty_enabled: false,
            last_action: None,
            insert_session: None,
            explicit_count: false,
            search: SearchState::default(),
            pane_jumps: {
                let mut m = SecondaryMap::new();
                m.insert(pane_id, crate::core::jump_list::JumpList::new(jump_list_capacity));
                m
            },
            pane_state,
            pane_transient,
            engine_view,
            focused_pane_id: pane_id,
            bracket_hl_data: Arc::new(RwLock::new(Vec::new())),
            search_hl_data: Arc::new(RwLock::new(Vec::new())),
            motion_format_scratch: engine::format::FormatScratch::new(),
            macro_recording: None,
            macro_pending: None,
            replay_queue: VecDeque::new(),
            skip_macro_record: false,
            is_replaying: false,
            mouse_drag_anchor: None,
            scripting: None,
        }
    }

    pub(crate) fn with_search_regex(mut self, pattern: &str) -> Self {
        if let Ok(regex) = regex_cursor::engines::meta::Regex::new(pattern) {
            let bid = self.focused_buffer_id();
            self.buffers.get_mut(bid).search_pattern = Some(SearchPattern {
                regex: Arc::new(regex),
                pattern_str: pattern.to_string(),
            });
        }
        self.sync_search_cache();
        self
    }

    // ── Pane choke-points ─────────────────────────────────────────────────────

    /// Create a new pane viewing `buffer_id`, seed all per-pane maps, return its id.
    pub(crate) fn open_pane(&mut self, buffer_id: BufferId) -> PaneId {
        let pid = self.engine_view.panes.insert(Pane::new(buffer_id));
        self.pane_state.insert(pid, SecondaryMap::new());
        pane_state::ensure(&mut self.pane_state, &self.buffers, pid, buffer_id);
        self.pane_transient.insert(pid, PaneTransient::default());
        self.pane_jumps.insert(pid, crate::core::jump_list::JumpList::new(
            self.settings.jump_list_capacity,
        ));
        pid
    }

    /// Switch focus to `target`, seeding its per-pane maps if not yet present.
    ///
    /// Precondition: editor must be in Normal mode. Focus switches are only
    /// bound in Normal mode; mode-changing commands must not switch panes.
    pub(crate) fn switch_focused_pane(&mut self, target: PaneId) {
        debug_assert!(
            self.mode == Mode::Normal,
            "focus-switch must only happen in Normal mode, got {:?}",
            self.mode,
        );
        self.focused_pane_id = target;
        if !self.pane_transient.contains_key(target) {
            self.pane_transient.insert(target, PaneTransient::default());
        }
        if !self.pane_jumps.contains_key(target) {
            self.pane_jumps.insert(
                target,
                crate::core::jump_list::JumpList::new(self.settings.jump_list_capacity),
            );
        }
        let bid = self.focused_buffer_id();
        pane_state::ensure(&mut self.pane_state, &self.buffers, target, bid);
    }

    /// Remove pane `target` and all its per-pane state.
    ///
    /// Precondition: at least one other pane exists. Callers must switch focus
    /// away before calling this if `target` is the focused pane.
    #[allow(dead_code)] // wired in M9+ :split/:close
    pub(crate) fn close_pane(&mut self, target: PaneId) {
        self.engine_view.panes.remove(target);
        self.pane_state.remove(target);
        self.pane_transient.remove(target);
        self.pane_jumps.remove(target);
    }

    /// Read-only accessor used by tests to inspect any pane's selections.
    pub(crate) fn selections_for(
        &self,
        pane: PaneId,
        buf: BufferId,
    ) -> Option<&crate::core::selection::SelectionSet> {
        self.pane_state.get(pane).and_then(|m| m.get(buf)).map(|s| &s.selections)
    }

    /// Execute a typed command string (e.g. `"bd"`, `"e! path"`) programmatically.
    ///
    /// Parses the trailing `!` as `force=true` and splits `cmd_with_arg` on the
    /// first space to extract the optional argument. Returns the command result.
    pub(crate) fn execute_typed(
        &mut self,
        cmd_with_arg: &str,
        extra_arg: Option<&str>,
    ) -> Result<(), crate::core::error::CommandError> {
        use crate::editor::Severity;
        let (cmd_raw, inline_arg) = match cmd_with_arg.split_once(' ') {
            Some((c, a)) => (c, Some(a.trim())),
            None => (cmd_with_arg, None),
        };
        let (cmd, force) = match cmd_raw.strip_suffix('!') {
            Some(base) => (base, true),
            None => (cmd_raw, false),
        };
        let arg = inline_arg.or(extra_arg);
        if let Some(tc) = self.registry.get_typed(cmd) {
            let fun = tc.fun;
            let result = fun(self, arg, force);
            if let Err(ref e) = result {
                self.report(Severity::Error, e.0.clone());
            }
            result
        } else {
            Err(crate::core::error::CommandError(format!("unknown command: {cmd}")))
        }
    }
}

// ---------------------------------------------------------------------------
// Module-level helpers
// ---------------------------------------------------------------------------

/// Scroll the pane viewport so `cursor_char` stays within the visible area.
///
/// Calls both the vertical and horizontal `ensure_cursor_visible` helpers in
/// one shot. Used by `prepare_frame` for both the scratch-view path and the
/// normal document path.
#[allow(clippy::too_many_arguments)]
fn scroll_into_view(
    pane: &mut Pane,
    rope: &ropey::Rope,
    cursor_char: usize,
    scratch: &mut engine::format::FormatScratch,
    wrap_mode: &WrapMode,
    tab_width: u8,
    whitespace: &WhitespaceConfig,
    v_margin: usize,
    h_margin: usize,
) {
    scroll::ensure_cursor_visible(&mut pane.viewport, rope, cursor_char, wrap_mode, tab_width, whitespace, scratch, v_margin);
    scroll::ensure_cursor_visible_horizontal(&mut pane.viewport, rope, cursor_char, wrap_mode, tab_width, whitespace, scratch, h_margin);
}

/// Map an `EditorMode` to the Steel-facing string name used in hook arguments.
fn mode_name(m: EditorMode) -> &'static str {
    match m {
        EditorMode::Normal  => "normal",
        EditorMode::Insert  => "insert",
        EditorMode::Extend  => "extend",
        EditorMode::Command => "command",
        EditorMode::Search  => "search",
        EditorMode::Select  => "select",
    }
}

/// Convert a char-offset position to a line-relative byte offset.
///
/// Returns `(line_idx, byte_in_line)` where `byte_in_line` is the byte offset
/// from the start of the line — suitable for building highlight spans that the
/// engine expects in line-relative byte coordinates.
fn char_to_line_byte(buf: &Text, char_pos: usize) -> (usize, usize) {
    let line = buf.char_to_line(char_pos);
    let line_start_byte = buf.char_to_byte(buf.line_to_char(line));
    let byte = buf.char_to_byte(char_pos).saturating_sub(line_start_byte);
    (line, byte)
}

#[cfg(test)]
mod tests;
