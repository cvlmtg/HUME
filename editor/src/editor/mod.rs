use std::borrow::Cow;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crossterm::event::KeyEvent;

use engine::pane::{WhitespaceConfig, WrapMode};
use engine::pipeline::{BufferId, EngineView, PaneId};
#[cfg(test)]
use engine::pipeline::{LayoutTree, SharedBuffer};
#[cfg(test)]
use engine::pane::Pane;
#[cfg(test)]
use search_state::SearchPattern;
use engine::types::EditorMode;

use slotmap::SecondaryMap;

use self::registry::CommandRegistry;
use editing::grapheme::prev_grapheme_boundary;
use editing::selection::{Selection, SelectionSet};
use crate::editor::buffer::Buffer;
use crate::editor::buffer_store::BufferStore;
use crate::editor::pane_state::{PaneBufferState, PaneTransient};
use crate::ops::motion::FindKind;
use crate::ops::register::{KillRing, RegisterSet};
use crate::settings::EditorSettings;

use self::keymap::{Keymap, WaitCharPending};

mod buffer_ops;
pub(crate) mod error;
pub(crate) mod host_impl;
mod lifecycle;
mod parse_worker;
mod scripting_setup;

pub(crate) mod buffer;
pub(crate) mod buffer_store;
mod clipboard;
mod commands;
pub(crate) mod completion;
pub(crate) mod cursor;
pub(crate) mod doc_ops;
pub(crate) mod jump_list;
pub mod keymap;
#[cfg(test)]
mod lints;
mod mappings;
mod message_log;
pub(crate) mod minibuf_history;
mod minibuf;
mod mouse;
pub(crate) mod ops;
pub(crate) mod pane_state;
pub(crate) mod register_ops;
mod registry;
pub(crate) mod search_ops;
pub(crate) mod search_state;
pub(super) mod scroll;
pub(crate) mod syntax;
mod syntax_glue;
mod visual_move;

pub(crate) use search_state::{SearchDirection, SearchState};

// Re-export module-level helpers so sibling submodules can call `super::foo()`.
use scripting_setup::theme_search_paths;

pub(crate) use minibuf::MiniBuffer;

use message_log::MessageLog;
pub(crate) use message_log::Severity;

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
    /// Step cursor back one grapheme on exit (set for `a` / `A` entry).
    step_back_on_exit: bool,
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

/// Pending state for the two-keystroke `"<reg>` register-prefix sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RegisterPrefix {
    /// `"` pressed — waiting for the register-name character.
    Awaiting,
    /// Register name received; armed for the next yank/delete/change/paste.
    Selected(char),
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
// The editor uses `engine::types::EditorMode` directly. Sticky extend is
// represented as `EditorMode::Extend`. One-shot ctrl-extend is a per-dispatch
// local variable and is NOT a mode change.
//
// `pub(crate) use EditorMode as Mode;` lets all internal modules use `Mode`
// as an unqualified alias.
pub(crate) use engine::types::EditorMode as Mode;

// ── Editor ────────────────────────────────────────────────────────────────────

pub(crate) struct Editor {
    /// All open buffers. SSOT for buffer content, history, and file metadata.
    pub(crate) buffers: BufferStore,
    /// Current editing mode. `EditorMode::Extend` represents the sticky extend
    /// state. Mode is the single source of truth for whether extend is active.
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
    /// Kill ring — bounded history of yanked / deleted text.
    ///
    /// Bare `y`/`c`/`d` push here; `[`/`]` cycle through it; `"<digit>p` reads
    /// slot N. Depth = 10, matching named digit registers `"0`–`"9`.
    pub(super) kill_ring: KillRing,
    /// Wrapper around the OS clipboard (`arboard`).
    ///
    /// `None` handle when the clipboard server is unreachable (headless CI/SSH).
    /// Must not be placed in the Steel context — `arboard::Clipboard` is not Send.
    pub(super) clipboard: clipboard::SystemClipboard,
    /// State machine for the two-keystroke `"<reg>` register-prefix sequence.
    ///
    /// `None` = idle. `Some(Awaiting)` = `"` pressed, next char is the register name.
    /// `Some(Selected(c))` = register armed for the next yank/delete/change/paste.
    /// Consumed by `take_register_prefix()`; cleared on Esc or invalid input.
    pub(super) register_prefix: Option<RegisterPrefix>,
    /// Name of the most recently dispatched command. Updated by every command
    /// in `execute_keymap_command`, including commands run inside a macro replay.
    /// Holds the sentinel `"macro-replay"` immediately after a replay finishes
    /// so that a bare `p` typed after a macro reads the clipboard — see
    /// `drain_replay_queue`.
    ///
    /// The Smart-p heuristic reads this to decide whether bare `p` should read
    /// the kill ring head or the system clipboard.
    pub(super) last_command: Option<Cow<'static, str>>,
    /// Values of the most recent paste. A consecutive `p`/`P` (append) re-pastes
    /// these verbatim, independent of kill-ring / clipboard state. Set by every
    /// successful paste (fresh or cycle-update); read only when `last_command` is
    /// a paste-family command.
    pub(super) last_paste: Option<Vec<String>>,
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
    pub(super) pane_jumps: SecondaryMap<PaneId, self::jump_list::JumpList>,

    // ── Minibuffer history ───────────────────────────────────────────────────
    /// Bounded, in-memory history for `:`, `/`, and `?` prompts.
    /// Recalled via Up/Down while the minibuffer is open.
    pub(super) history: self::minibuf_history::HistoryStore,
    /// Whether the kitty keyboard protocol was successfully activated at startup.
    ///
    /// When `true`, the terminal sends CSI-u sequences that disambiguate
    /// Ctrl+h from Backspace, Ctrl+j from Enter, etc. — unlocking Ctrl+motion
    /// one-shot extend shortcuts. Set by the caller after [`Editor::open`].
    pub(crate) kitty_enabled: bool,
    /// Set by the inline-output dispatch arm after `leave_inline_output` to
    /// trigger a full ratatui repaint, clearing its diff cache after the
    /// alt-screen toggle invalidated the terminal's previous contents.
    pub(crate) force_full_redraw: bool,

    // ── Visual-line movement ──────────────────────────────────────────────────
    /// Reusable scratch buffer for format operations in visual-line movement.
    ///
    /// Allocated once and reused every j/k press to avoid per-keypress
    /// heap allocation.
    pub(super) motion_format_scratch: engine::format::FormatScratch,
    /// Reusable sticky-column buffer for visual j/k movement.
    /// One `u16` entry per active selection; cleared and refilled each press.
    pub(super) visual_move_target_cols: Vec<u16>,

    // ── Dot-repeat fields ─────────────────────────────────────────────────────
    /// The last repeatable editing action, available for replay via `.`.
    /// `None` until the user performs a repeatable command.
    pub(super) last_repeatable_action: Option<RepeatableAction>,
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
    pub(super) scripting: Option<scripting::ScriptingHost>,
    /// Snapshot of Rust-builtin command names taken at the end of
    /// `init_scripting`.  Stable across reloads (built-ins never change at
    /// runtime).  Stored as a field so dispatch-time activation can borrow it
    /// disjointly from `&mut self.scripting` / `settings` / `keymap`.
    pub(super) builtin_cmd_names: std::collections::HashSet<String>,
    /// Registry of configured language identities.
    /// Reset at the start of each `init_scripting` call so `:reload-config`
    /// gets a fresh set of registrations from `languages.scm`.
    pub(super) languages: syntax::LanguageRegistry,

    // ── Working directory ─────────────────────────────────────────────────────
    /// Current working directory.
    ///
    /// Set at startup; updated by `:cd`. Avoids a `getcwd` syscall every frame.
    pub(super) cwd: PathBuf,

    // ── Tree-sitter parse worker ──────────────────────────────────────────────
    /// Parse backend: threaded in production, synchronous-inline in tests.
    ///
    /// `reparse_stale_buffers` drains completed results and posts new requests
    /// each frame.  Tests use `InlineParseBackend` (via `for_testing`) which
    /// completes parses inside `post` — no blocking helpers needed.
    parse_worker: Box<dyn parse_worker::ParseBackend>,
    /// Whether the one-shot "parse worker disconnected" message has already been
    /// pushed to the message log.  Lives here, not on the backend trait, because
    /// it is UI dedup state, not execution-backend state.
    parse_worker_disconnect_logged: bool,
}

// proptest requires `Debug` on strategy values; this minimal impl satisfies it.
#[cfg(test)]
impl std::fmt::Debug for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Editor(buf={:?}, mode={:?})",
            self.doc().text().to_string(),
            self.mode
        )
    }
}

impl Editor {
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

    /// `true` when the focused buffer rejects user edits.
    pub(crate) fn focused_buffer_read_only(&self) -> bool {
        self.doc().is_read_only()
    }

    /// Resolved formatting context for the focused doc and pane:
    /// `(wrap_mode, tab_width, whitespace)`. The wrap_mode has its `width: 0`
    /// sentinel substituted via `pane.content_width(...)` — safe to hand to
    /// engine code. `wrap_mode.is_wrapping()` matches the unresolved value.
    pub(super) fn focused_format_context(&self) -> (WrapMode, u8, WhitespaceConfig) {
        let raw_wrap = self.doc().overrides.wrap_mode(&self.settings);
        let tab_width = self.doc().overrides.tab_width(&self.settings);
        let whitespace = self.doc().overrides.whitespace(&self.settings);
        let pane = &self.engine_view.panes[self.focused_pane_id];
        let wrap_mode = raw_wrap.resolve(pane.content_width(self.doc().text().len_lines()));
        (wrap_mode, tab_width, whitespace)
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

    fn is_group_open_current(&self) -> bool {
        self.pane_state[self.focused_pane_id][self.focused_buffer_id()]
            .edit_group
            .is_some()
    }

    /// Open a new edit group on the focused (pane, buffer) pair.
    fn begin_edit_group_current(&mut self) {
        let pane_id = self.focused_pane_id;
        let buf_id = self.focused_buffer_id();
        doc_ops::begin_edit_group(&self.buffers, &mut self.pane_state, pane_id, buf_id);
    }

    /// Commit and close the open edit group on the focused (pane, buffer) pair.
    fn commit_edit_group_current(&mut self) {
        let pane_id = self.focused_pane_id;
        let buf_id = self.focused_buffer_id();
        doc_ops::commit_edit_group(&mut self.buffers, &mut self.pane_state, pane_id, buf_id);
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
        if self.focused_buffer_read_only() {
            self.report(Severity::Info, "Buffer is read-only".to_string());
            return;
        }
        if !self.is_group_open_current() {
            self.begin_edit_group_current();
            self.insert_session = Some(InsertSession {
                keystrokes: Vec::new(),
                step_back_on_exit: false,
            });
        }
        self.mode = Mode::Insert;
    }

    /// Mark the active insert session as append-style so the cursor steps back
    /// one grapheme on exit (see [`end_insert_session`]).
    pub(super) fn mark_insert_step_back(&mut self) {
        if let Some(s) = self.insert_session.as_mut() {
            s.step_back_on_exit = true;
        }
    }

    /// Exit Insert mode and finalise the undo/repeat state.
    ///
    /// Commits the open edit group (creating one undo step for the whole
    /// insert session) and moves the recorded keystrokes into `last_repeatable_action`
    /// for dot-repeat, then sets the mode to Normal.
    ///
    /// When the session was started with `mark_insert_step_back` (i.e. entered via
    /// `a` or `A`), each selection head steps back one grapheme so that pressing
    /// `a` again re-enters Insert at the same position rather than advancing forward.
    /// The step is clamped to the current line start so it never crosses a `\n`.
    pub(super) fn end_insert_session(&mut self) {
        let step_back = self.insert_session.as_ref().is_some_and(|s| s.step_back_on_exit);
        self.commit_edit_group_current();
        if let (Some(session), Some(action)) =
            (self.insert_session.take(), self.last_repeatable_action.as_mut())
        {
            action.insert_keys = session.keystrokes;
        }
        if step_back {
            let focused = self.focused_pane_id;
            let buf = self.focused_buffer_id();
            doc_ops::apply_doc_motion(&self.buffers, &mut self.pane_state, focused, buf, |b, sels| {
                sels.map(|sel| {
                    let head = sel.head();
                    let line_start = b.line_to_char(b.char_to_line(head));
                    let new_head = if head > line_start {
                        prev_grapheme_boundary(b, head)
                    } else {
                        head
                    };
                    Selection::collapsed(new_head)
                })
            });
        }
        // Engine pane is synced by `prepare_frame` each frame.
        self.mode = EditorMode::Normal;
    }

    /// Drain the macro replay queue, executing each key in order.
    ///
    /// Sets `is_replaying` for the duration so that `Q`/`q` intercepts inside
    /// replayed keys cannot start nested recording or replay sessions — including
    /// when the last key in the macro is `Q` (where `replay_queue.is_empty()`
    /// would already be `true` and would fail to suppress it).
    ///
    /// Saves and restores `last_repeatable_action` so replay does not corrupt dot-repeat.
    pub(crate) fn drain_replay_queue(&mut self) {
        if self.replay_queue.is_empty() {
            return;
        }
        let saved_action = self.last_repeatable_action.take();
        self.is_replaying = true;
        while let Some(key) = self.replay_queue.pop_front() {
            self.handle_key(key);
            if self.should_quit {
                break;
            }
        }
        self.is_replaying = false;
        // After replay, reset Smart-p to clipboard mode so a bare `p` typed
        // immediately after a macro reads the clipboard rather than whatever
        // delete/change happened to be the last command inside the macro.
        self.last_command = Some(Cow::Borrowed("macro-replay"));
        self.last_repeatable_action = saved_action;
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
        let history_capacity = settings.history_capacity;
        let pane = Pane::new(buffer_id);
        let pane_id = engine_view.panes.insert(pane);
        engine_view.layout = LayoutTree::Leaf(pane_id);
        engine_view.theme.bake(&engine_view.registry);

        let mut buffers = BufferStore::new();
        buffers.open(buffer_id, doc);

        let mut pane_state: SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>> =
            SecondaryMap::new();
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
            kill_ring: KillRing::new(),
            clipboard: clipboard::SystemClipboard::new_unavailable(),
            register_prefix: None,
            last_command: None,
            last_paste: None,
            should_quit: false,
            minibuf: None,
            completion: None,
            completion_view: Arc::new(RwLock::new(None)),
            status_msg: None,
            message_log: MessageLog::new(),
            settings,
            registry: registry::CommandRegistry::with_defaults(),
            keymap: keymap::Keymap::default(),
            last_find: None,
            kitty_enabled: false,
            force_full_redraw: false,
            last_repeatable_action: None,
            insert_session: None,
            explicit_count: false,
            search: SearchState::default(),
            pane_jumps: {
                let mut m = SecondaryMap::new();
                m.insert(
                    pane_id,
                    self::jump_list::JumpList::new(jump_list_capacity),
                );
                m
            },
            history: self::minibuf_history::HistoryStore::new(history_capacity),
            pane_state,
            pane_transient,
            engine_view,
            focused_pane_id: pane_id,
            bracket_hl_data: Arc::new(RwLock::new(Vec::new())),
            search_hl_data: Arc::new(RwLock::new(Vec::new())),
            motion_format_scratch: engine::format::FormatScratch::new(),
            visual_move_target_cols: Vec::new(),
            macro_recording: None,
            macro_pending: None,
            replay_queue: VecDeque::new(),
            skip_macro_record: false,
            is_replaying: false,
            mouse_drag_anchor: None,
            scripting: None,
            builtin_cmd_names: std::collections::HashSet::new(),
            languages: syntax::LanguageRegistry::new(),
            cwd: std::env::temp_dir(),
            parse_worker: Box::new(parse_worker::InlineParseBackend::new()),
            parse_worker_disconnect_logged: false,
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
        self.pane_jumps.insert(
            pid,
            self::jump_list::JumpList::new(self.settings.jump_list_capacity),
        );
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
                self::jump_list::JumpList::new(self.settings.jump_list_capacity),
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
    ) -> Option<&editing::selection::SelectionSet> {
        self.pane_state
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
        use crate::editor::Severity;
        let (cmd, force, inline_arg) = mappings::command_mode::parse_typed_command(cmd_with_arg);
        let arg = inline_arg.or(extra_arg);
        if let Some(tc) = self.registry.get_typed(cmd) {
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

#[cfg(test)]
mod tests;
