//! Dot-repeat (`.`) and macro-replay state and execution.
//!
//! Dot-repeat records a [`RepeatableAction`] recipe (command name + count +
//! char arg + selection-building steps + insert keystrokes) rather than a
//! raw changeset, since changesets are position-dependent and can't be
//! replayed at a different cursor. Macro replay drains a queue of recorded
//! keys through the normal event path.

use std::borrow::Cow;
use termina::event::{Event, KeyEvent};

use super::dispatch::{ArgSource, CmdCtx};
use super::registry::MappableCommand;
use super::{Editor, EditorState, Mode, commands, doc_ops};

// ── Dot-repeat / insert-session state ────────────────────────────────────────

/// One unit of recorded insert-session input, replayed by `replay_dot`.
///
/// A pasted string is kept as its own variant rather than being replayed as
/// synthetic per-char `KeyEvent`s: a synthesized `Enter` would run
/// `insert_newline_indent` with auto-indent, altering text a real paste never
/// auto-indents.
#[derive(Debug, Clone)]
pub(crate) enum InsertInput {
    Key(KeyEvent),
    Paste(String),
}

/// State for an active insert session (entered via a repeatable command).
///
/// Tracks keystrokes for dot-repeat recording. Created by
/// `begin_insert_session` and consumed by [`Editor::end_insert_session`].
///
/// `None` on the editor when there is no active session — including during
/// replay, where the replay path pre-opens the edit group to signal
/// `begin_insert_session` that recording should be suppressed.
pub(crate) struct InsertSession {
    pub(super) keystrokes: Vec<InsertInput>,
    /// Step cursor back one grapheme on exit (set for `a` / `A` / `o` / `O` entry).
    pub(super) step_back_on_exit: bool,
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
pub(crate) struct SelectionStep {
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
pub(crate) struct RepeatableAction {
    /// The command name that initiated this action (e.g. `"delete"`, `"change"`).
    /// `Cow::Borrowed` for built-in commands (zero allocation); `Cow::Owned` for
    /// dynamically-registered commands (e.g. from the Steel scripting layer).
    pub command: Cow<'static, str>,
    /// The count prefix used originally. Overridden when `.` itself is given a count.
    pub count: usize,
    /// Character argument for wait-char commands (`r`, `f`, `t`, …).
    /// `None` for commands that don't consume a char.
    pub char_arg: Option<char>,
    /// Keystrokes (and pasted text) recorded during the insert session, if any.
    ///
    /// Populated by the insert-mode recording path when the command transitions
    /// to Insert mode. Empty for non-insert actions like `delete` or `paste-after`.
    pub insert_keys: Vec<InsertInput>,
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
pub(crate) struct PendingRepeat {
    /// Effective replay count — explicit-count override already applied.
    pub(super) count: usize,
}

// ── Macro recording state ─────────────────────────────────────────────────────

/// Pending state for the two-keystroke `q<reg>` / `Q<reg>` sequences.
///
/// Set when the user presses `q` or `Q` in normal mode; cleared when the
/// next keypress is consumed as the register name (or cancelled on Esc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MacroPending {
    /// `Q` was pressed — waiting for a register name to start recording.
    Record,
    /// `q` was pressed — waiting for a register name to start replay.
    Replay,
}

impl EditorState {
    // ── Insert session ────────────────────────────────────────────────────────

    /// Mark the active insert session as append-style so the cursor steps back
    /// one grapheme on exit.
    pub(super) fn mark_insert_step_back(&mut self) {
        if let Some(s) = self.insert_session.as_mut() {
            s.step_back_on_exit = true;
        }
    }
}

impl Editor {
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
                Some(step.count),
                step.extend,
            );
        }

        // Restore the edit's own char arg.
        self.state.pending_char = action.char_arg;

        // Run the edit body.
        match &edit_cmd {
            MappableCommand::SteelBacked { .. } | MappableCommand::Lazy { .. } => {
                let ctx = CmdCtx {
                    count: Some(count),
                    extend: false,
                    arg_source: ArgSource::Keymap,
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
                commands::run_native_body(
                    &mut self.state,
                    &mut self.view,
                    edit_cmd,
                    Some(count),
                    false,
                );
            }
        }

        // Feed recorded insert input back through the same paths the original
        // session used — a paste replays as one bulk insert, not synthesized
        // per-char keys (which would wrongly re-trigger auto-indent on an
        // embedded newline).
        for input in &action.insert_keys {
            match input {
                InsertInput::Key(key) => self.handle_insert(*key),
                InsertInput::Paste(text) => self.apply_insert_mode_paste(text),
            }
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
