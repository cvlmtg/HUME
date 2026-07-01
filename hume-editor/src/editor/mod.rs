use std::borrow::Cow;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crossterm::event::{Event, KeyEvent};

#[cfg(test)]
use hume_engine::pane::Pane;
use hume_engine::pipeline::{BufferId, EngineView, PaneId};
#[cfg(test)]
use hume_engine::pipeline::{LayoutTree, SharedBuffer};
#[cfg(test)]
use search_state::SearchPattern;
#[cfg(test)]
use slotmap::SecondaryMap;

use self::registry::{CommandRegistry, MappableCommand};
use crate::editor::buffer::Buffer;
use crate::editor::buffer_store::BufferStore;
#[cfg(test)]
use crate::editor::pane_state::PaneBufferState;
#[cfg(test)]
use crate::editor::pane_state::PaneTransient;
use crate::editor::pane_state::PaneView;
use crate::ops::motion::FindKind;
use crate::ops::register::{KillRing, RegisterSet};
use crate::settings::EditorSettings;
use hume_editing::selection::SelectionSet;

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
mod minibuf;
pub(crate) mod minibuf_history;
mod mouse;
pub(crate) mod ops;
pub(crate) mod pane_state;
pub(crate) mod register_ops;
mod registry;
pub(super) mod scroll;
pub(crate) mod search_ops;
pub(crate) mod search_state;
pub(crate) mod syntax;
mod syntax_glue;
mod visual_move;

pub(crate) use search_state::{SearchDirection, SearchState};

// Re-export module-level helpers so sibling submodules can call `super::foo()`.
use scripting_setup::theme_search_paths;

pub(crate) use minibuf::MiniBuffer;

use message_log::MessageLog;
pub(crate) use message_log::Severity;

// ── Command dispatch context ──────────────────────────────────────────────────

/// Per-dispatch context assembled by the key handler and passed through
/// [`Editor::dispatch`].
#[derive(Debug, Clone)]
pub(super) struct CmdCtx {
    /// Numeric count prefix (default 1).
    pub count: usize,
    /// Whether this command runs in Extend mode.
    pub extend: bool,
    /// Pre-computed Steel lambda arguments (supplied by keymap trie leaf).
    /// Empty for native commands and keymap-navigated Steel commands.
    pub steel_args: Vec<steel::rvals::SteelVal>,
}

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
    /// Step cursor back one grapheme on exit (set for `a` / `A` / `o` / `O` entry).
    step_back_on_exit: bool,
}

/// One selection-building step in a dot-repeat recipe.
///
/// Recorded by `step_update_recipe` as Motion/Selection commands run, so that
/// `replay_dot` can replay them before the edit, rebuilding the
/// extent the edit originally acted on.
///
/// Only in-place selections (e.g. `select-line`) appear as establish steps;
/// reaching selections (`select-next-word` / `-prev-word` / uppercase-word variants) are
/// not recorded in Move mode — replaying one would advance past the cursor and
/// act on the wrong region. Extend steps of any selection are always recorded.
#[derive(Debug, Clone)]
pub(super) struct SelectionStep {
    /// Command name (e.g. `"select-line"`, `"find-char"`).
    pub command: Cow<'static, str>,
    /// Count prefix originally used.
    pub count: usize,
    /// Char argument for wait-char selection commands (`f`/`t`); else `None`.
    pub char_arg: Option<char>,
    /// `true` if this step ran in Extend mode (grew the existing selection).
    /// The first step in a recipe is always `false` (a fresh Move-mode establish).
    pub extend: bool,
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
    /// Selection-building recipe to replay BEFORE the edit.
    ///
    /// Invariant: `[]` (edit acted on pre-existing selection or after a reaching
    /// selection — `.` deletes the current selection as-is) or `[one in-place
    /// Move-mode establish, then zero+ Extend appends]`. Reaching selections
    /// (`select-next-word` / `-prev-word` / uppercase-word variants) are excluded from
    /// establish steps because replaying them advances past the cursor. Rebuilt
    /// from `EditorState::selection_recipe` each time a repeatable command is
    /// recorded.
    pub selection_recipe: Vec<SelectionStep>,
}

// ── Deferred dot-repeat ───────────────────────────────────────────────────────

/// Deferred dot-repeat job, set by `cmd_repeat` and consumed by
/// `replay_dot` at the end of the enclosing `handle_key` call.
///
/// Splitting enqueue (pure State handler) from drain (`&mut Editor` plumbing)
/// lets `cmd_repeat` satisfy the D7 invariant while still reaching
/// `replay_dot` (which uses `run_native_body`/`run_steel_command` and
/// `handle_insert`) for the actual replay.
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
    /// Private: all transitions go through [`EditorState::set_mode`].
    mode: Mode,
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
    /// Keystrokes the message-log summary stays visible before auto-dismissing.
    /// Armed when `status_msg` clears with unseen entries; ticked down in `handle_key`.
    pub(crate) summary_ttl: u8,
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
    /// Accumulating selection-recipe buffer for the *next* edit's dot-repeat.
    ///
    /// Tracks how the current selection was built: Motion/Selection commands
    /// append or reset this buffer; repeatable edits snapshot it into
    /// `RepeatableAction::selection_recipe` (via `mem::take`) and clear it.
    /// Non-selection commands clear it. Invariant: `[]` or
    /// `[Move-establish, Extend*]`.
    pub(super) selection_recipe: Vec<SelectionStep>,
    /// Deferred dot-repeat job enqueued by `cmd_repeat`; consumed by
    /// `replay_dot` at the tail of `handle_key`.
    pub(super) pending_repeat: Option<PendingRepeat>,
    /// Active insert session, present between begin/end_insert_session.
    pub(super) insert_session: Option<InsertSession>,
    /// Whether the user explicitly typed a count prefix before the current command.
    pub(super) explicit_count: bool,
    /// `true` when the current multi-key sequence began with a kitty one-shot
    /// Ctrl+key that resolved to a prefix (Interior) node. Cleared on sequence
    /// completion or abort. At Leaf resolution, only applied if the command is
    /// extendable.
    pub(super) pending_ctrl_extend: bool,
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
    // ── Mode ──────────────────────────────────────────────────────────────────

    pub(crate) fn mode(&self) -> Mode {
        self.mode
    }

    /// Single write path for all mode transitions.
    ///
    /// Captures the old mode, writes the new one, and enqueues `OnModeChange`
    /// for firing by `Editor::drain_hooks` after the command returns. The
    /// no-op guard prevents spurious hook fires when mode is already correct.
    ///
    /// The `mode` field is private so the compiler enforces that every
    /// transition goes through here.
    pub(crate) fn set_mode(&mut self, new: Mode) {
        use hume_scripting::hooks::HookId;
        use steel::rvals::IntoSteelVal;
        let old = self.mode;
        if old == new {
            return;
        }
        self.mode = new;
        let old_val = mode_name(old)
            .into_steelval()
            .expect("mode str into_steelval");
        let new_val = mode_name(new)
            .into_steelval()
            .expect("mode str into_steelval");
        self.pending_hooks
            .push((HookId::OnModeChange, vec![old_val, new_val]));
    }

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

fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Normal => "normal",
        Mode::Insert => "insert",
        Mode::Extend => "extend",
        Mode::Command => "command",
        Mode::Search => "search",
        Mode::Select => "select",
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
            self.state.mode()
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
        doc_ops::begin_edit_group(
            &self.state.buffers,
            &mut self.state.panes.state,
            pane_id,
            buf_id,
        );
    }

    /// Commit and close the open edit group on the focused (pane, buffer) pair.
    fn commit_edit_group_current(&mut self) {
        let pane_id = self.state.focused_pane_id;
        let buf_id = self.focused_buffer_id();
        doc_ops::commit_edit_group(
            &mut self.state.buffers,
            &mut self.state.panes.state,
            pane_id,
            buf_id,
        );
    }

    // ── Mode transitions ──────────────────────────────────────────────────────

    pub(super) fn end_insert_session(&mut self) {
        commands::end_insert_session(&mut self.state, &self.view);
    }

    // ── Unified command dispatch pipeline ──────────────────────────────────────

    /// Execute a `MappableCommand` through the unified dispatch pipeline.
    ///
    /// Native commands delegate to [`commands::run_dispatch_pipeline`].  Steel-backed
    /// commands run the pipeline's BEFORE/AFTER stages inline, with the body
    /// executed via [`Editor::run_steel_command`] (which needs `&mut Editor` for
    /// `self.scripting`).
    ///
    /// Dot-repeat replay bypasses this entirely — it calls
    /// [`commands::run_native_body`] directly.
    pub(crate) fn dispatch(&mut self, cmd: MappableCommand, ctx: CmdCtx) {
        let is_steel = matches!(
            &cmd,
            MappableCommand::SteelBacked { .. } | MappableCommand::Lazy { .. }
        );
        if !is_steel {
            // Native path — delegate to the standalone pipeline.
            commands::run_dispatch_pipeline(&mut self.state, &mut self.view, cmd, ctx);
            return;
        }

        // Steel path — composed from shared step functions.
        let meta = cmd.meta();
        // Clone the name once, before the body consumes `cmd`.
        let name = cmd.name().clone();

        // BEFORE
        commands::step_paste_commit(&mut self.state, meta.defers_paste_commit);
        // Pre-stamp last_command — inner dispatches via `call!` override it.
        commands::step_stamp_last_command(&mut self.state, name.clone(), meta.stamps_last_command);
        let char_arg = self.state.pending_char;
        // Always snapshot the recipe before the body — inner dispatches via `call!`
        // overwrite selection_recipe during the body, so the snapshot must be taken
        // before they run (the native path uses step_snapshot_recipe, which gates on
        // repeatable; here we snapshot unconditionally and decide after the body).
        let pre_recipe = std::mem::take(&mut self.state.selection_recipe);

        // BODY — consumes `cmd`.
        if !self.run_steel_command(cmd, name.as_ref(), &ctx, char_arg) {
            self.state.selection_recipe.clear();
            return;
        }

        // AFTER — re-query to get the resolved command's repeatable flag.
        // A Lazy stub becomes SteelBacked after activation; re-query reflects that.
        if self
            .state
            .registry
            .get_mappable(name.as_ref())
            .is_some_and(|c| c.meta().repeatable)
        {
            // Outer-name-wins: stamp the outer command so `.` replays it, not
            // any inner native command the body dispatched via `call!`.
            commands::step_stamp_repeatable(
                &mut self.state,
                &name,
                ctx.count,
                char_arg,
                Some(pre_recipe),
            );
        }
        // Non-repeatable outer: leave inner dispatch's repeatable action intact.
        self.state.selection_recipe.clear();
        // Outer Steel commands skip step_record_jump and step_clear_extend: their meta
        // hardcodes is_jump = clears_extend = false. An inner native (call! …) still
        // fires both — it routes through run_dispatch_pipeline with its own meta.
    }

    /// Run the body of a Steel-backed or Lazy command.
    ///
    /// Returns `false` if the command aborted (lazy activation failure, scripting
    /// error, or `scripting` is `None`). On error, the caller skips AFTER stages.
    fn run_steel_command(
        &mut self,
        cmd: MappableCommand,
        name: &str,
        ctx: &CmdCtx,
        char_arg: Option<char>,
    ) -> bool {
        let count = ctx.count.max(1);
        let extend = ctx.extend;

        // For a Lazy stub, activate the owning plugin now so we can read
        // `inline_output` from the resolved SteelBacked entry before dispatch.
        if let MappableCommand::Lazy { plugin, .. } = &cmd {
            let plugin = plugin.clone();
            if !self.activate_lazy_plugin(&plugin, name) {
                self.report(Severity::Warning, format!("unknown command: {name}"));
                return false;
            }
        }

        let focused_pane_id = self.state.focused_pane_id;
        let focused_buffer_id = self.focused_buffer_id();

        let scripting = match self.scripting.as_mut() {
            Some(s) => s,
            None => return false,
        };

        // Re-query: a Lazy stub is now SteelBacked after activation above;
        // a SteelBacked entry is unchanged.
        let (inline_output, cmd_arity, cmd_is_variadic) =
            match self.state.registry.get_mappable(name) {
                Some(MappableCommand::SteelBacked {
                    inline_output,
                    arity,
                    is_variadic,
                    ..
                }) => (*inline_output, *arity, *is_variadic),
                _ => {
                    self.report(
                        Severity::Error,
                        format!("{name}: internal error — command lost after activation"),
                    );
                    return false;
                }
            };

        // Inject count and extend as leading lambda args based on declared arity.
        let steel_args = &ctx.steel_args;
        if steel_args.is_empty() && cmd_arity > 2 {
            self.report(
                Severity::Error,
                format!(
                    "{name}: lambda declares {cmd_arity} required params; \
                     keymap injection supplies at most 2 (count, extend)"
                ),
            );
            return false;
        }
        let effective_args = if steel_args.is_empty() {
            match (cmd_arity, cmd_is_variadic) {
                (0, false) => vec![],
                (1, false) => vec![steel::rvals::SteelVal::IntV(count as isize)],
                _ => vec![
                    steel::rvals::SteelVal::IntV(count as isize),
                    steel::rvals::SteelVal::BoolV(extend),
                ],
            }
        } else {
            steel_args.clone()
        };

        // Alt-screen bracketing for inline-output commands.
        if inline_output {
            let kitty = self.kitty_enabled;
            let mouse = self.state.settings.mouse_enabled;
            if let Err(e) = hume_platform::terminal::enter_inline_output(kitty, mouse) {
                self.report(Severity::Error, format!("inline-output enter failed: {e}"));
                return false;
            }
            hume_platform::terminal::print_running_banner(name);
        }

        let result = {
            let mut impl_host = crate::editor::host_impl::EditorHostImpl {
                state: &mut self.state,
                view: &mut self.view,
            };
            scripting.call_steel_cmd(
                name,
                char_arg,
                effective_args,
                focused_pane_id,
                focused_buffer_id,
                &mut impl_host,
            )
        };

        if inline_output {
            hume_platform::terminal::print_return_prompt();
            hume_platform::terminal::wait_for_keypress();
            let kitty = self.kitty_enabled;
            let mouse = self.state.settings.mouse_enabled;
            let mouse_select = self.state.settings.mouse_select;
            let _ = hume_platform::terminal::leave_inline_output(kitty, mouse, mouse_select);
            self.state.force_full_redraw = true;
        }

        let (wait_char_cmd, lang_sets, grammar_sweeps) = match result {
            Ok(r) => (
                r.wait_char_request,
                r.pending_language_sets,
                r.grammar_sweeps,
            ),
            Err(e) => {
                self.report(Severity::Error, e);
                return false;
            }
        };

        self.flush_script_messages();
        for (bid, lang) in lang_sets {
            self.set_buffer_language(bid, lang);
        }
        if !grammar_sweeps.is_empty() {
            self.sweep_buffers_for_grammars(grammar_sweeps);
        }
        if let Some(wc) = wait_char_cmd {
            self.state.wait_char = Some(crate::editor::keymap::WaitCharPending {
                cmd_name: wc.into(),
                ctrl_extend: false,
            });
        }

        true
    }

    /// Replay a dot-repeat action directly, bypassing dispatch bookkeeping.
    ///
    /// Runs the selection recipe motions and edit body with [`commands::run_native_body`]
    /// (avoiding pipeline re-entry), then feeds insert keys through `handle_insert`.
    ///
    /// After replay, neutralizes `last_command` so bare `p` reads the clipboard,
    /// but preserves `last_repeatable_action` so `.` chains.
    pub(crate) fn replay_dot(&mut self, count: usize) {
        let Some(action) = self.state.last_repeatable_action.take() else {
            return;
        };

        // Resolve the edit body before opening the edit group: a missing command
        // must return while there is still no cleanup obligation, so this path
        // cannot leak an open group.
        let Some(edit_cmd) = self
            .state
            .registry
            .get_mappable(action.command.as_ref())
            .cloned()
        else {
            self.state.last_repeatable_action = Some(action);
            return;
        };

        // Pre-open the edit group — the "replay signal" used by
        // begin_insert_session to suppress keystroke recording.
        self.begin_edit_group_current();

        // Rebuild the selection extent the edit originally acted on.
        for step in &action.selection_recipe {
            self.state.pending_char = step.char_arg;
            let Some(cmd) = self
                .state
                .registry
                .get_mappable(step.command.as_ref())
                .cloned()
            else {
                continue;
            };
            commands::run_native_body(
                &mut self.state,
                &mut self.view,
                cmd,
                step.count,
                step.extend,
            );
        }

        // Restore the edit's own char arg.
        self.state.pending_char = action.char_arg;

        // Run the edit body.
        match &edit_cmd {
            MappableCommand::SteelBacked { .. } | MappableCommand::Lazy { .. } => {
                let ctx = CmdCtx {
                    count,
                    extend: false,
                    steel_args: vec![],
                };
                let cmd_name = action.command.clone();
                // A Steel command can succeed when first run yet fail on dot-repeat:
                // the buffer state differs (no match at the new cursor, a guard that
                // now throws), so replay must handle failure even though the original
                // run didn't.
                if !self.run_steel_command(edit_cmd, cmd_name.as_ref(), &ctx, action.char_arg) {
                    // Close the group opened above so it can't leak. commit drops
                    // an empty group (clean noop) and records a partial one (a
                    // failure mid-edit stays undoable).
                    self.commit_edit_group_current();
                    self.state.last_repeatable_action = Some(action);
                    return;
                }
                // Inner call! dispatches inside the Steel body run through
                // run_dispatch_pipeline → step_update_recipe, which may append to
                // selection_recipe. Clear it so stale steps don't contaminate the
                // next command's recipe accumulation.
                self.state.selection_recipe.clear();
            }
            _ => {
                commands::run_native_body(&mut self.state, &mut self.view, edit_cmd, count, false);
            }
        }

        // Feed recorded insert keystrokes through the insert handler.
        for key in &action.insert_keys {
            self.handle_insert(*key);
        }

        // Close the edit group.
        if self.state.mode() == Mode::Insert {
            self.end_insert_session();
        } else {
            self.commit_edit_group_current();
        }

        // Restore the action so `.` can be pressed again.
        self.state.last_repeatable_action = Some(action);
        // Neutralize last_command after replay so a bare `p` reads the clipboard.
        self.state.last_command = None;
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
            self.handle_event(Event::Key(key));
            if self.state.should_quit {
                break;
            }
        }
        self.state.is_replaying = false;
        // Neutralize last_command after replay so a bare `p` reads the clipboard
        // rather than whatever kill command ran last inside the macro.
        self.state.last_command = None;
        self.state.last_repeatable_action = saved_action;
    }
}

// ── Test constructors ─────────────────────────────────────────────────────────

#[cfg(test)]
impl Editor {
    /// Create a new pane viewing `buffer_id`, seed all per-pane maps, return its id.
    /// Delegates to `commands::open_pane` — the single source of truth, also
    /// used directly by production `EditorCmd` handlers (`pane-split`,
    /// `pane-vsplit`) that only have `state`+`view` access. Test-only:
    /// production code builds panes exclusively through `split_pane_onto`.
    pub(crate) fn open_pane(&mut self, buffer_id: BufferId) -> PaneId {
        commands::open_pane(&mut self.state, &mut self.view, buffer_id)
    }

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
                summary_ttl: 0,
                message_log: MessageLog::new(),
                settings,
                registry: registry::CommandRegistry::with_defaults(),
                keymap: keymap::Keymap::default(),
                last_find: None,
                force_full_redraw: false,
                last_repeatable_action: None,
                selection_recipe: Vec::new(),
                pending_repeat: None,
                insert_session: None,
                explicit_count: false,
                pending_ctrl_extend: false,
                search: SearchState::default(),
                panes: {
                    let mut jumps = SecondaryMap::new();
                    jumps.insert(pane_id, self::jump_list::JumpList::new(jump_list_capacity));
                    let mut transient = SecondaryMap::new();
                    transient.insert(pane_id, pane_state::PaneTransient::default());
                    PaneView {
                        state: pane_buf_state,
                        transient,
                        jumps,
                    }
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
                self::jump_list::JumpList::new(self.state.settings.jump_list_capacity),
            );
        }
        let bid = self.focused_buffer_id();
        pane_state::ensure(
            &mut self.state.panes.state,
            &self.state.buffers,
            target,
            bid,
        );
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
