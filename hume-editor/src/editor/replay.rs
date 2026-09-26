//! Dot-repeat (`.`) and macro-replay state and execution.
//!
//! Dot-repeat records a [`RepeatableAction`] recipe (command name + count +
//! char arg + selection-building steps + insert keystrokes) rather than a
//! raw changeset, since changesets are position-dependent and can't be
//! replayed at a different cursor. Macro replay drains a queue of recorded
//! keys through the normal event path.

use std::borrow::Cow;
use termina::event::{Event as TerminalEvent, KeyEvent};

use super::dispatch::CmdCtx;
use super::edit_session::{self, EditSessionKind};
use super::registry::MappableCommand;
use super::{Editor, Mode, commands, doc_ops};

// ── Dot-repeat / insert-session state ────────────────────────────────────────

/// One unit of recorded insert-session input, replayed by `replay_dot`.
///
/// A pasted string is kept as its own variant rather than being replayed as
/// synthetic per-char `KeyEvent`s: a synthesized `Enter` would run
/// `insert_newline_indent` with auto-indent, altering text a real paste never
/// auto-indents.
///
/// `Key` means one run of Insert mode's *default* per-key behaviour
/// (`commands::insert_default_key`) for a key with no keymap binding —
/// never "replay this key through the Insert keymap". A key that *does*
/// resolve to a keymap binding is recorded as `Command` instead, naming the
/// bound command itself rather than any effect it happened to have: replay
/// re-runs that command (via [`Editor::replay_command`]) so its own
/// internal `call!`s — including ones carrying a register or `#:extend` a
/// fixed effect snapshot could never reproduce — run fresh at the new site,
/// and any motion it performs via `call!` before editing lands the edit in
/// the right place. The cost is symmetric with a live keypress: a binding
/// whose own decision logic depends on buffer content (e.g. re-opening a
/// completion popup) can decide differently on replay, exactly as it could
/// if the same key were pressed by hand at the new site.
#[derive(Debug, Clone)]
pub(crate) enum InsertInput {
    Key(KeyEvent),
    Paste(String),
    /// The command bound to an Insert key's keymap entry, resolved once at
    /// `handle_insert`'s trie-leaf match and re-run on replay via
    /// [`Editor::replay_command`] — never a native command's own recorded
    /// effect (see this enum's own doc for why).
    Command {
        name: Cow<'static, str>,
    },
}

/// State for an active insert session (entered via a repeatable command).
///
/// Tracks keystrokes for dot-repeat recording. Created by
/// `begin_insert_session` and consumed by [`Editor::end_insert_session`].
///
/// `None` on the editor when there is no active session — including during
/// replay, where the replay path pre-opens a `Replay`-kind placeholder to
/// signal `begin_insert_session` that recording should be suppressed.
pub(crate) struct InsertSession {
    pub(super) keystrokes: Vec<InsertInput>,
}

/// One selection-building step in a dot-repeat recipe.
///
/// Recorded by `step_update_recipe` as Motion/Selection commands (or
/// EditorCmds opting into `SelectionTracking::Establishes`/`Composes`) run,
/// so that `replay_dot` can replay them before the edit, rebuilding the
/// extent the edit originally acted on. See `SelectionTracking` for which
/// commands are excluded (Move-mode motions) or always recorded (`Composes`).
#[derive(Debug, Clone)]
pub(crate) struct SelectionStep {
    /// Command name (e.g. `"select-line"`, `"surround-paren"`).
    pub command: Cow<'static, str>,
    /// Count prefix originally used.
    pub count: usize,
    /// `true` if this step ran in Extend mode (grew the existing selection).
    /// A recipe's first step can be `true`: `C` (a `Composes` step) run in
    /// Extend mode against an empty recipe is itself the whole recipe — see
    /// `RepeatableAction::selection_recipe`'s doc for the `C`-from-a-bare-cursor case.
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
    /// Invariant: `[]` (edit acted on pre-existing selection, or after a
    /// Move-mode motion — `.` deletes the current selection as-is), or a
    /// sequence of `Establishes`/`Composes` steps — see `SelectionTracking`
    /// for what each records. A leading `Composes` step is legal: `C` from a
    /// bare cursor (no prior establish) duplicates that cursor onto the
    /// adjacent line and is itself the first (and only) recipe entry —
    /// replaying it reproduces exactly what the user did. Rebuilt from
    /// `EditorState::selection_recipe` each time a repeatable command is
    /// recorded.
    pub selection_recipe: Vec<SelectionStep>,
}

// ── Deferred dot-repeat ───────────────────────────────────────────────────────

/// Deferred dot-repeat job, set by `cmd_repeat` and consumed by
/// `replay_dot` at the end of the enclosing `handle_key` call.
///
/// Splitting enqueue (pure State handler) from drain (`&mut Editor` plumbing)
/// lets `cmd_repeat` keep the `FocusedCmdFn` shape (no `&mut Editor`, see
/// `registry/command.rs`) while still reaching `replay_dot` (which uses
/// `commands::run_body`/`run_steel_command`/`commands::insert_default_key`)
/// for the actual replay.
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

impl Editor {
    // ── Doc-edit wrappers ─────────────────────────────────────────────────────

    /// Close out whatever `replay_dot` left open on the focused (pane,
    /// buffer), for either its success or its failure path — the two need
    /// the identical decision, so it lives here once.
    ///
    /// A body that entered real Insert mode closes through
    /// `end_insert_session` (pops the `InsertLayer`, which
    /// `tear_down_insert` needs present to run its own bookkeeping — typed
    /// run, autoindent trim). A body that called native paste retargeted
    /// the pre-opened placeholder to `Paste` (`edit_session::
    /// open_or_retarget`, via `do_paste`) — that must stay open for a
    /// following `[`/`]`, the same as a live `p` keypress leaves it, so
    /// it's left alone here. Anything else (a plain edit-only body that
    /// never claimed the placeholder) is still `Replay`-kind, and commits
    /// directly through `doc_ops::commit_open_session` — kind-agnostic,
    /// since Insert and Paste are both already handled above.
    ///
    /// Read dynamically from the session's own kind rather than
    /// `meta.manages_own_session` — that flag only describes what the
    /// *outer* replayed command declares, and a Steel-backed outer command's
    /// meta can never set it (see `CmdMeta::manages_own_session`'s doc), so
    /// a Steel wrapper whose body dispatches native paste needs the same
    /// "leave it open" outcome without ever being able to say so statically.
    ///
    /// No-op if nothing is open at all: the body's own dispatch may already
    /// have closed the pane or buffer the pre-opened placeholder lived on —
    /// `focus::end_focus_sessions`, which every focus/buffer-switch path
    /// runs first, already commits a still-live session of any kind before
    /// that happens, so by the time control returns here there is genuinely
    /// nothing left to close.
    fn finish_replay_session(&mut self) {
        if self.state.active_session.is_none() {
            return;
        }
        if self.state.mode() == Mode::Insert {
            self.end_insert_session();
            return;
        }
        let leave_open = self
            .state
            .active_session
            .as_ref()
            .is_some_and(|s| matches!(s.kind(), EditSessionKind::Paste { .. }));
        if !leave_open {
            doc_ops::commit_open_session(
                &mut self.state.buffers,
                &self.state.panes.state,
                &mut self.state.active_session,
            );
        }
    }

    /// Run `cmd` (already resolved from the registry) as one replayed step, native
    /// or Steel/Lazy alike — the main edit body and each `InsertInput::
    /// Command` entry share this rather than each re-deriving the
    /// native-vs-Steel branch. Returns `false` on the same failure classes
    /// as `run_steel_command` (a Steel body can fail on replay even though
    /// the original run didn't — different buffer state, no match at the
    /// new cursor); a native command has no failure mode here, so this
    /// dispatches it and always returns `true`.
    fn replay_command(
        &mut self,
        cmd: MappableCommand,
        fp: commands::FocusedPane,
        ctx: &CmdCtx,
        char_arg: Option<char>,
    ) -> bool {
        match &cmd {
            MappableCommand::SteelBacked { .. } | MappableCommand::Lazy { .. } => {
                // Cloned before the body consumes `cmd`.
                let name = cmd.name().clone();
                if !self.run_steel_command(cmd, &name, ctx, char_arg) {
                    return false;
                }
                // Inner call! dispatches inside the Steel body run through
                // `commands::run` → step_update_recipe, which may append to
                // selection_recipe. Clear it so stale steps don't
                // contaminate the next command's recipe accumulation.
                self.state.selection_recipe.clear();
                true
            }
            _ => {
                // Native bodies that consume a char argument (`replace`,
                // `surround-add`) read it via `state.pending_char.take()`.
                self.state.pending_char = char_arg;
                let Ok(bound) = commands::BoundCommand::focused(cmd, fp) else {
                    unreachable!("caller resolved `cmd` from a registered command name")
                };
                commands::run_body(&mut self.state, &mut self.view, bound, ctx);
                true
            }
        }
    }

    /// Replay a dot-repeat action directly, bypassing dispatch bookkeeping.
    ///
    /// Runs the selection recipe motions with [`commands::run_body`]
    /// (avoiding pipeline re-entry), the edit body and each recorded insert
    /// `Command` entry with [`Self::replay_command`], and a recorded `Key`/
    /// `Paste` entry via `commands::insert_default_key`/
    /// `Self::apply_insert_mode_paste` directly (never the Insert keymap;
    /// see [`InsertInput`]'s own doc). Preserves `last_repeatable_action` so
    /// `.` chains.
    pub(in crate::editor) fn replay_dot(&mut self, count: usize) {
        let Some(action) = self.state.last_repeatable_action.take() else {
            return;
        };
        // Read once, at entry: recipe steps are selection-tracking motions
        // and never move focus, so one mint covers the whole replay.
        let fp = commands::FocusedPane::current(&self.state);

        // Resolve the edit body before opening the edit group: a missing command
        // must return while there is still no cleanup obligation, so this path
        // cannot leak an open group.
        let Some(edit_cmd) = self
            .state
            .config
            .registry
            .get_mappable(action.command.as_ref())
            .cloned()
        else {
            self.state.last_repeatable_action = Some(action);
            return;
        };

        let meta = edit_cmd.meta();

        // Commit (or defer) any paste session left open by the keypress that
        // set up `.` itself, using the REPLAYED command's own meta rather
        // than `repeat-last-action`'s (which always defers — see its
        // registration). A session can only still be open here if the
        // command being replayed is itself a ring-cycle command: every other
        // dispatch commits unconditionally before running, so by the time
        // `.` was pressed any unrelated session was already closed. This
        // reproduces `p [ .` as one more ring-cycle step instead of losing
        // the session to `repeat-last-action`'s own dispatch.
        commands::step_paste_commit(&mut self.state, meta.defers_paste_commit);

        // Pre-open a Replay-kind placeholder — the "replay signal" used by
        // begin_insert_session to suppress keystroke recording, and the
        // wrapper that folds a recipe replay + the main edit into one undo
        // revision. Its own kind (rather than reusing Insert directly) keeps
        // it distinct from a real, already-open Insert session: only a
        // Replay-kind placeholder is eligible for `open_or_retarget`'s
        // retarget branch (see `EditSessionKind::Replay`'s own doc) — a real
        // empty Insert session must still refuse a conflicting open, not
        // silently hand its group to whatever this pre-open is used for.
        // Skipped for a `manages_own_session` command (the paste family): it
        // opens or continues `active_session` itself (`do_paste`/
        // `do_paste_cycle`), which would collide with a placeholder
        // pre-opened here — see `CmdMeta::manages_own_session`'s own doc. A
        // Steel body that itself calls a native paste command still
        // succeeds despite this pre-open: `do_paste`'s own opener retargets
        // this still-open placeholder to `Paste` in place rather than
        // colliding with it (see `edit_session::open_or_retarget`).
        //
        // `Err` here means a real session (of any kind) is already open —
        // not expected given `step_paste_commit` just ran above, but if it
        // ever happens, defer rather than stealing or corrupting that
        // session.
        if !meta.manages_own_session {
            let pid = fp.pid();
            let bid = fp.bid(&self.view);
            let sels = self.state.panes.state[pid][bid].selections().clone();
            let opened = edit_session::open_or_retarget(
                &mut self.state.active_session,
                pid,
                bid,
                EditSessionKind::Replay,
                || self.state.buffers.get(bid).begin_edit_group(sels),
            );
            if opened.is_err() {
                self.state.last_repeatable_action = Some(action);
                return;
            }
        }

        // Rebuild the selection extent the edit originally acted on. No
        // recipe-step command reads `pending_char` — every `wait_char!`-bound
        // command is `Untracked` and so can never reach the recipe — so
        // unlike the edit body below, no step here needs it set.
        for step in &action.selection_recipe {
            // A recipe step is only ever pushed for a `selection_tracking !=
            // Untracked` command, and `SteelBacked`/`Lazy` are always
            // `Untracked` (`registry/command.rs`), so every step names a
            // native command. Native entries are registered once at startup
            // and can never be unregistered or shadowed
            // (`Registry::unregister`/`register_command` both refuse
            // anything but a `Lazy`/absent slot), so the lookup is
            // guaranteed to resolve.
            let cmd = self
                .state
                .config
                .registry
                .get_mappable(step.command.as_ref())
                .cloned()
                .expect("a dot-repeat selection-recipe step always names a native command");
            let Ok(bound) = commands::BoundCommand::focused(cmd, fp) else {
                unreachable!("a dot-repeat selection-recipe step always names a native command")
            };
            commands::run_body(
                &mut self.state,
                &mut self.view,
                bound,
                &CmdCtx {
                    count: Some(step.count),
                    extend: step.extend,
                },
            );
        }

        let ctx = CmdCtx {
            count: Some(count),
            extend: false,
        };
        if !self.replay_command(edit_cmd, fp, &ctx, action.char_arg) {
            // Close whatever the group opened above became so it can't
            // leak — same decision the success path below makes (see
            // `finish_replay_session`'s own doc). commit drops an empty
            // group (clean noop) and records a partial one (a failure
            // mid-edit stays undoable).
            self.finish_replay_session();
            self.state.last_repeatable_action = Some(action);
            return;
        }

        // Feed recorded insert input back through the same paths the original
        // session used — a paste replays as one bulk insert, not synthesized
        // per-char keys (which would wrongly re-trigger auto-indent on an
        // embedded newline).
        for input in &action.insert_keys {
            match input {
                // Never the Insert keymap walk: this `Key` variant means
                // a keymap-*unbound* key's default behaviour only (see
                // `InsertInput`'s own doc) — a bound key is `Command` below.
                InsertInput::Key(key) => {
                    commands::insert_default_key(&mut self.state, &self.view, fp, *key);
                }
                InsertInput::Paste(text) => self.apply_insert_mode_paste(text),
                InsertInput::Command { name } => {
                    // A command name loses its keymap binding only if the
                    // binding itself is removed between the original
                    // keypress and this replay (`unbind-key!`, a plugin
                    // reload) — unlike the selection-recipe/edit-body
                    // lookups above, this name was never guaranteed to
                    // stay registered, so a miss reports rather than
                    // panicking.
                    let Some(cmd) = self
                        .state
                        .config
                        .registry
                        .get_mappable(name.as_ref())
                        .cloned()
                    else {
                        self.report_unknown_command(
                            name.as_ref(),
                            format!("unknown command: {name}"),
                        );
                        break;
                    };
                    // This entry exists only because the original keypress
                    // dispatched it with `in_insert_key_dispatch` set (see
                    // that field's own doc) — replay recreates the same
                    // context, so a body that calls `insert-key!` still
                    // finds it in flight, same as the live keypress did.
                    let ok = self.with_insert_key_dispatch(|ed| {
                        ed.replay_command(
                            cmd,
                            fp,
                            &super::input_stack::insert::INSERT_KEY_CTX,
                            None,
                        )
                    });
                    if !ok {
                        break;
                    }
                }
            }
        }

        self.finish_replay_session();

        // Restore the action so `.` can be pressed again.
        self.state.last_repeatable_action = Some(action);
    }

    /// Drain the macro replay queue, executing each key in order and
    /// settling after each one.
    ///
    /// Sets `is_replaying` for the duration so that `Q`/`q` intercepts inside
    /// replayed keys cannot start nested recording or replay sessions — including
    /// when the last key in the macro is `Q` (where `replay_queue.is_empty()`
    /// would already be `true` and would fail to suppress it).
    ///
    /// Saves and restores `last_repeatable_action` so replay does not corrupt dot-repeat.
    ///
    /// Settling once per replayed key, not once after the whole queue, keeps
    /// a macro's own hooks in the loop: a plugin that reconfigures the buffer
    /// on `on-buffer-open` (indent width, a language keymap) must see that
    /// reaction run before the macro's remaining keys type into the buffer,
    /// the same as it would if those keys were typed by hand. `is_replaying`
    /// stays `true` across every one of these settles, so `can_open_confirm`'s
    /// `!is_replaying` guard still blocks a confirm the macro would have no
    /// queued key left to answer — see that guard's "Macro replay" doc
    /// paragraph. The deferred prompt still arrives on the next real
    /// buffer-enter.
    ///
    /// `message_logged_this_input` is OR'd back in every iteration, not left
    /// to whatever the just-replayed key's own `handle_input` set it to: the
    /// triggering dispatch (e.g. the register char after `@`, which
    /// populated `replay_queue` in the first place) may itself have logged a
    /// message moments before this function was even called, and each
    /// settle() call clears the flag at its own end — losing
    /// `report_disk_state`'s shadowing guard for that earlier message if a
    /// stale-buffer warning fires from a later key's settle.
    pub(in crate::editor) fn drain_replay_queue(&mut self) {
        if self.state.replay_queue.is_empty() {
            return;
        }
        let saved_action = self.state.last_repeatable_action.take();
        let message_already_logged = self.state.message_logged_this_input;
        self.state.is_replaying = true;
        while let Some(key) = self.state.replay_queue.pop_front() {
            self.handle_input(TerminalEvent::Key(key));
            self.state.message_logged_this_input |= message_already_logged;
            self.settle();
            if self.state.should_quit {
                break;
            }
        }
        self.state.is_replaying = false;
        self.state.last_repeatable_action = saved_action;
    }
}
