use std::borrow::Cow;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crossterm::event::KeyEvent;

use hume_engine::pipeline::{BufferId, EngineView, PaneId};
#[cfg(test)]
use hume_engine::pipeline::{LayoutTree, SharedBuffer};
#[cfg(test)]
use hume_engine::pane::Pane;
#[cfg(test)]
use search_state::SearchPattern;
#[cfg(test)]
use slotmap::SecondaryMap;

use self::registry::CommandRegistry;
use hume_editing::selection::SelectionSet;
use crate::editor::buffer::Buffer;
use crate::editor::buffer_store::BufferStore;
use crate::editor::pane_state::PaneView;
#[cfg(test)]
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
/// `begin_insert_session` and consumed by [`Editor::end_insert_session`].
///
/// `None` on the editor when there is no active session — including during
/// replay, where the replay path pre-opens the edit group to signal
/// `begin_insert_session` that recording should be suppressed.
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

// ── Deferred dot-repeat ───────────────────────────────────────────────────────

/// Deferred dot-repeat job, set by `cmd_repeat` and consumed by
/// `drain_pending_repeat` at the end of the enclosing `handle_key` call.
///
/// Splitting enqueue (pure State handler) from drain (`&mut Editor` plumbing)
/// lets `cmd_repeat` satisfy the D7 invariant while still reaching
/// `execute_keymap_command` and `handle_insert` for the actual replay.
#[derive(Debug, Clone, Copy)]
pub(super) struct PendingRepeat {
    /// Effective replay count — explicit-count override already applied.
    pub(super) count: usize,
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
// The editor uses `hume_engine::types::EditorMode` directly. Sticky extend is
// represented as `EditorMode::Extend`. One-shot ctrl-extend is a per-dispatch
// local variable and is NOT a mode change.
//
// `pub(crate) use EditorMode as Mode;` lets all internal modules use `Mode`
// as an unqualified alias.
pub(crate) use hume_engine::types::EditorMode as Mode;

// ── EditorState ───────────────────────────────────────────────────────────────
//
// All command-mutable editor data. Separated from `Editor` so the Steel VM
// (`scripting.steel`) and editor data are sibling borrows that never alias —
// enabling EditorCmd to dispatch synchronously from within a Steel eval.

pub(crate) struct EditorState {
    /// All open buffers. SSOT for buffer content, history, and file metadata.
    pub(crate) buffers: BufferStore,
    /// Current editing mode. `EditorMode::Extend` represents the sticky extend
    /// state. Mode is the single source of truth for whether extend is active.
    pub(crate) mode: Mode,
    /// Keys consumed so far in the current multi-key sequence (max depth 3).
    pub(super) pending_keys: Vec<KeyEvent>,
    /// Accumulated numeric prefix for the next command (e.g. `3` in `3w`).
    pub(super) count: Option<usize>,
    /// Pending wait-char state for a f/t/F/T/r binding.
    pub(super) wait_char: Option<WaitCharPending>,
    /// Character argument for the current parameterized command (find/till/replace).
    pub(super) pending_char: Option<char>,
    pub(super) registers: RegisterSet,
    /// Kill ring — bounded history of yanked / deleted text.
    pub(super) kill_ring: KillRing,
    /// Wrapper around the OS clipboard (`arboard`).
    pub(super) clipboard: clipboard::SystemClipboard,
    /// State machine for the two-keystroke `"<reg>` register-prefix sequence.
    pub(super) register_prefix: Option<RegisterPrefix>,
    /// Name of the most recently dispatched command.
    pub(super) last_command: Option<Cow<'static, str>>,
    /// Values of the most recent paste.
    pub(super) last_paste: Option<Vec<String>>,
    pub(super) should_quit: bool,
    /// Active when the user is typing a command (`:`) or a search (`/`).
    pub(crate) minibuf: Option<MiniBuffer>,
    /// Active completion session while a popup is showing.
    pub(crate) completion: Option<completion::CompletionState>,
    /// Transient one-line message shown in the statusline after an action.
    pub(crate) status_msg: Option<String>,
    /// Persistent log of warnings, errors, and trace entries.
    pub(crate) message_log: MessageLog,
    /// All editor settings — global defaults and per-buffer-overridable values.
    pub(crate) settings: EditorSettings,
    /// Registry of all mappable commands (motions, selections, edits).
    pub(super) registry: CommandRegistry,
    /// The trie-based keymap for each mode.
    pub(super) keymap: Keymap,
    /// The character and kind from the last find/till motion.
    pub(super) last_find: Option<FindChar>,
    pub(super) search: SearchState,
    /// The single pane focused in the current editing session.
    pub(crate) focused_pane_id: PaneId,
    /// Per-pane maps: (pane,buffer) selections/groups, transient mode snapshots, jump history.
    pub(super) panes: PaneView,
    /// Bounded, in-memory history for `:`, `/`, and `?` prompts.
    pub(super) history: self::minibuf_history::HistoryStore,
    /// Set by the inline-output dispatch arm to trigger a full ratatui repaint.
    pub(crate) force_full_redraw: bool,
    /// Reusable scratch buffer for format operations in visual-line movement.
    pub(super) motion_format_scratch: hume_engine::format::FormatScratch,
    /// Reusable sticky-column buffer for visual j/k movement.
    pub(super) visual_move_target_cols: Vec<u16>,
    /// The last repeatable editing action, available for replay via `.`.
    pub(super) last_repeatable_action: Option<RepeatableAction>,
    /// Deferred dot-repeat job enqueued by `cmd_repeat`; consumed by
    /// `drain_pending_repeat` at the tail of `handle_key`.
    pub(super) pending_repeat: Option<PendingRepeat>,
    /// Active insert session, present between begin/end_insert_session.
    pub(super) insert_session: Option<InsertSession>,
    /// Whether the user explicitly typed a count prefix before the current command.
    pub(super) explicit_count: bool,
    /// Active macro recording session.
    pub(super) macro_recording: Option<(char, Vec<KeyEvent>)>,
    /// Pending two-keystroke macro command.
    pub(super) macro_pending: Option<MacroPending>,
    /// Queue of keys to replay before reading the next terminal event.
    pub(super) replay_queue: VecDeque<KeyEvent>,
    /// Single-frame flag: skip recording the current key.
    pub(super) skip_macro_record: bool,
    /// `true` while draining the replay queue.
    pub(super) is_replaying: bool,
    /// Anchor char offset set on mouse-left-down when `mouse_select` is enabled.
    pub(super) mouse_drag_anchor: Option<usize>,
    /// Registry of configured language identities.
    pub(super) languages: syntax::LanguageRegistry,
    /// Current working directory. Set at startup; updated by `:cd`.
    pub(super) cwd: PathBuf,
    /// Hooks enqueued during command dispatch, drained by `Editor::drain_hooks`
    /// after each command. The unified firing path — `fire_hook_silent` pushes
    /// here; no hook fires inline during command execution.
    pub(super) pending_hooks: Vec<(hume_scripting::hooks::HookId, Vec<steel::rvals::SteelVal>)>,
}

impl EditorState {
    // ── Status messages ───────────────────────────────────────────────────────

    /// Record a status message / warning / error on this state.
    ///
    /// Called by EditorCmd handlers that only have `&mut EditorState` access.
    /// The `Editor::report` method delegates here.
    pub(super) fn report(&mut self, severity: Severity, text: String) {
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

    // ── Insert session ────────────────────────────────────────────────────────

    /// Mark the active insert session as append-style so the cursor steps back
    /// one grapheme on exit.
    pub(super) fn mark_insert_step_back(&mut self) {
        if let Some(s) = self.insert_session.as_mut() {
            s.step_back_on_exit = true;
        }
    }
}

// ── Editor ────────────────────────────────────────────────────────────────────

pub(crate) struct Editor {
    /// All command-mutable editor data. Disjoint from `scripting` so Steel evals
    /// can borrow `state` and `scripting.steel` simultaneously without aliasing.
    pub(crate) state: EditorState,
    /// Engine rendering state: layout, panes, buffers, theme.
    pub(crate) view: EngineView,
    /// Shared bracket match highlight data: `(line_idx, byte_start, byte_end)`.
    pub(crate) bracket_hl_data: Arc<RwLock<Vec<(usize, usize, usize)>>>,
    /// Shared search match highlight data: same shape as `bracket_hl_data`.
    pub(crate) search_hl_data: Arc<RwLock<Vec<(usize, usize, usize)>>>,
    /// Shared completion-popup view: written by `prepare_frame`, read by provider.
    pub(crate) completion_view: Arc<RwLock<Option<crate::ui::completion_overlay::CompletionView>>>,
    /// Whether the kitty keyboard protocol was successfully activated at startup.
    pub(crate) kitty_enabled: bool,
    /// The embedded Steel scripting host.
    pub(super) scripting: Option<hume_scripting::ScriptingHost>,
    /// Snapshot of Rust-builtin command names taken at end of `init_scripting`.
    pub(super) builtin_cmd_names: std::collections::HashSet<String>,
    /// Parse backend: threaded in production, synchronous-inline in tests.
    parse_worker: Box<dyn parse_worker::ParseBackend>,
    /// Whether the one-shot "parse worker disconnected" message has been logged.
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
            self.state.mode
        )
    }
}

impl Editor {
    // ── Buffer accessors ──────────────────────────────────────────────────────

    /// The `BufferId` the focused pane is currently viewing.
    pub(crate) fn focused_buffer_id(&self) -> BufferId {
        self.view.panes[self.state.focused_pane_id].buffer_id
    }

    /// Shared reference to the focused buffer.
    pub(crate) fn doc(&self) -> &Buffer {
        self.state.buffers.get(self.focused_buffer_id())
    }

    /// The most-recently-focused buffer other than the current one, or `None`
    /// when only one buffer is open. Derives from `BufferStore.mru` (SSOT).
    pub(crate) fn alternate_buffer(&self) -> Option<BufferId> {
        self.state.buffers.mru_excluding(self.focused_buffer_id())
    }

    /// Mutable reference to the focused buffer.
    ///
    /// Uses a split borrow — `buffers` and other fields on `Editor` are
    /// disjoint, so you can hold this reference while reading e.g. `self.state.settings`.
    /// Do NOT keep this reference live across a call that also borrows `self`.
    pub(crate) fn doc_mut(&mut self) -> &mut Buffer {
        let bid = self.focused_buffer_id();
        self.state.buffers.get_mut(bid)
    }

    /// `true` when the focused buffer rejects user edits.
    pub(crate) fn focused_buffer_read_only(&self) -> bool {
        self.doc().is_read_only()
    }

    // ── Pane-state accessors ──────────────────────────────────────────────────

    /// The focused pane's selections for the current buffer.
    pub(super) fn current_selections(&self) -> &SelectionSet {
        &self.state.panes.state[self.state.focused_pane_id][self.focused_buffer_id()].selections
    }

    /// Replace the focused pane's selections for the current buffer.
    pub(super) fn set_current_selections(&mut self, sels: SelectionSet) {
        commands::set_current_selections(&mut self.state, &self.view, sels);
    }

    // ── Doc-edit wrappers ─────────────────────────────────────────────────────

    /// Open a new edit group on the focused (pane, buffer) pair.
    fn begin_edit_group_current(&mut self) {
        let pane_id = self.state.focused_pane_id;
        let buf_id = self.focused_buffer_id();
        doc_ops::begin_edit_group(&self.state.buffers, &mut self.state.panes.state, pane_id, buf_id);
    }

    /// Commit and close the open edit group on the focused (pane, buffer) pair.
    fn commit_edit_group_current(&mut self) {
        let pane_id = self.state.focused_pane_id;
        let buf_id = self.focused_buffer_id();
        doc_ops::commit_edit_group(&mut self.state.buffers, &mut self.state.panes.state, pane_id, buf_id);
    }

    // ── Mode transitions ──────────────────────────────────────────────────────

    pub(super) fn end_insert_session(&mut self) {
        commands::end_insert_session(&mut self.state, &self.view);
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
        if self.state.replay_queue.is_empty() {
            return;
        }
        let saved_action = self.state.last_repeatable_action.take();
        self.state.is_replaying = true;
        while let Some(key) = self.state.replay_queue.pop_front() {
            self.handle_key(key);
            if self.state.should_quit {
                break;
            }
        }
        self.state.is_replaying = false;
        // After replay, reset Smart-p to clipboard mode so a bare `p` typed
        // immediately after a macro reads the clipboard rather than whatever
        // delete/change happened to be the last command inside the macro.
        self.state.last_command = Some(Cow::Borrowed("macro-replay"));
        self.state.last_repeatable_action = saved_action;
    }

    /// Replay the pending dot-repeat action, if any.
    ///
    /// Called at the tail of every `handle_key` so the replay runs with
    /// `&mut Editor` — outside any Steel eval and able to re-enter the VM if
    /// the recorded command is (or becomes) SteelBacked.
    ///
    /// The heavy work (edit-group bracketing, re-dispatch, insert-key replay)
    /// lives here rather than in `cmd_repeat` so that handler can be a pure
    /// `fn(&mut EditorState, &mut EngineView)` satisfying the D7 invariant.
    fn drain_pending_repeat(&mut self) {
        let Some(PendingRepeat { count }) = self.state.pending_repeat.take() else {
            return;
        };
        let Some(action) = self.state.last_repeatable_action.take() else {
            return;
        };

        // Restore the char arg so wait-char commands (replace, find/till) work.
        self.state.pending_char = action.char_arg;

        // Pre-open the edit group — the "replay signal" used by begin_insert_session
        // to suppress both the redundant begin_edit_group call and keystroke recording
        // (insert_session is only created when no group is open).
        self.begin_edit_group_current();

        // Re-dispatch through the full keymap dispatcher: any command kind —
        // including future SteelBacked repeatable commands — fires correctly here.
        self.execute_keymap_command(action.command.clone(), count, false, vec![]);

        // Feed recorded insert keystrokes through the insert handler.
        for key in &action.insert_keys {
            self.handle_insert(*key);
        }

        // Close the edit group: insert commands → end_insert_session commits it;
        // non-insert commands → the group is empty and commit is a no-op.
        if self.state.mode == Mode::Insert {
            self.end_insert_session();
        } else {
            self.commit_edit_group_current();
        }

        // Restore the action so `.` can be pressed again. execute_keymap_command
        // may have overwritten last_repeatable_action during replay; this
        // assignment ensures the stored action is always the one the user performed.
        self.state.last_repeatable_action = Some(action);
        // Re-stamp last_command: the outer dispatcher saw "repeat-last-action",
        // which would wrongly suppress clipboard paste for smart-p (p after c/d).
        self.state.last_command = Some(Cow::Borrowed("repeat-last-action"));
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

        let mut pane_buf_state: SecondaryMap<PaneId, SecondaryMap<BufferId, PaneBufferState>> =
            SecondaryMap::new();
        pane_buf_state.insert(pane_id, SecondaryMap::new());
        pane_state::ensure(&mut pane_buf_state, &buffers, pane_id, buffer_id);

        Self {
            state: EditorState {
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
                status_msg: None,
                message_log: MessageLog::new(),
                settings,
                registry: registry::CommandRegistry::with_defaults(),
                keymap: keymap::Keymap::default(),
                last_find: None,
                force_full_redraw: false,
                last_repeatable_action: None,
                pending_repeat: None,
                insert_session: None,
                explicit_count: false,
                search: SearchState::default(),
                panes: {
                    let mut jumps = SecondaryMap::new();
                    jumps.insert(pane_id, self::jump_list::JumpList::new(jump_list_capacity));
                    let mut transient = SecondaryMap::new();
                    transient.insert(pane_id, pane_state::PaneTransient::default());
                    PaneView { state: pane_buf_state, transient, jumps }
                },
                history: self::minibuf_history::HistoryStore::new(history_capacity),
                focused_pane_id: pane_id,
                motion_format_scratch: hume_engine::format::FormatScratch::new(),
                visual_move_target_cols: Vec::new(),
                macro_recording: None,
                macro_pending: None,
                replay_queue: VecDeque::new(),
                skip_macro_record: false,
                is_replaying: false,
                mouse_drag_anchor: None,
                languages: syntax::LanguageRegistry::new(),
                cwd: std::env::temp_dir(),
                pending_hooks: Vec::new(),
            },
            view: engine_view,
            bracket_hl_data: Arc::new(RwLock::new(Vec::new())),
            search_hl_data: Arc::new(RwLock::new(Vec::new())),
            completion_view: Arc::new(RwLock::new(None)),
            kitty_enabled: false,
            scripting: None,
            builtin_cmd_names: std::collections::HashSet::new(),
            parse_worker: Box::new(parse_worker::InlineParseBackend::new()),
            parse_worker_disconnect_logged: false,
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

    // ── Pane choke-points ─────────────────────────────────────────────────────

    /// Create a new pane viewing `buffer_id`, seed all per-pane maps, return its id.
    pub(crate) fn open_pane(&mut self, buffer_id: BufferId) -> PaneId {
        let pid = self.view.panes.insert(Pane::new(buffer_id));
        self.state.panes.state.insert(pid, SecondaryMap::new());
        pane_state::ensure(&mut self.state.panes.state, &self.state.buffers, pid, buffer_id);
        self.state.panes.transient.insert(pid, PaneTransient::default());
        self.state.panes.jumps.insert(
            pid,
            self::jump_list::JumpList::new(self.state.settings.jump_list_capacity),
        );
        pid
    }

    /// Switch focus to `target`, seeding its per-pane maps if not yet present.
    ///
    /// Precondition: editor must be in Normal mode. Focus switches are only
    /// bound in Normal mode; mode-changing commands must not switch panes.
    pub(crate) fn switch_focused_pane(&mut self, target: PaneId) {
        debug_assert!(
            self.state.mode == Mode::Normal,
            "focus-switch must only happen in Normal mode, got {:?}",
            self.state.mode,
        );
        self.state.focused_pane_id = target;
        if !self.state.panes.transient.contains_key(target) {
            self.state.panes.transient.insert(target, PaneTransient::default());
        }
        if !self.state.panes.jumps.contains_key(target) {
            self.state.panes.jumps.insert(
                target,
                self::jump_list::JumpList::new(self.state.settings.jump_list_capacity),
            );
        }
        let bid = self.focused_buffer_id();
        pane_state::ensure(&mut self.state.panes.state, &self.state.buffers, target, bid);
    }

    /// Remove pane `target` and all its per-pane state.
    ///
    /// Precondition: at least one other pane exists. Callers must switch focus
    /// away before calling this if `target` is the focused pane.
    #[allow(dead_code)] // wired in M9+ :split/:close
    pub(crate) fn close_pane(&mut self, target: PaneId) {
        self.view.panes.remove(target);
        self.state.panes.state.remove(target);
        self.state.panes.transient.remove(target);
        self.state.panes.jumps.remove(target);
    }

    /// Read-only accessor used by tests to inspect any pane's selections.
    pub(crate) fn selections_for(
        &self,
        pane: PaneId,
        buf: BufferId,
    ) -> Option<&hume_editing::selection::SelectionSet> {
        self.state.panes.state
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
        if let Some(tc) = self.state.registry.get_typed(cmd) {
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
