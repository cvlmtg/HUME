//! Dot-repeat (`.`) and macro-replay state and execution.
//!
//! Dot-repeat records a [`RepeatableAction`] recipe (command name + count +
//! operand or the answers its character reads received + selection-building
//! steps + Insert-session inputs) rather than
//! a raw changeset, since changesets are position-dependent and can't be
//! replayed at a different cursor. Macro replay drains a queue of recorded
//! keys through the normal event path.

use hume_engine::pipeline::EngineView;
use hume_scripting::host::CommandArg;
use std::borrow::Cow;
use std::collections::VecDeque;
use termina::event::{Event as TerminalEvent, KeyEvent};

use super::dispatch::CmdCtx;
use super::dot_chain::{ChainBroken, DotChain};
use super::edit_session::{self, DotCapture, DotPark, DotWaiter, EditSessionKind};
use super::input_stack::char::{self as char_read, CharRead};
use super::registry::MappableCommand;
use super::{Editor, EditorState, Mode, commands, doc_ops};

// ── Dot-repeat / insert-session state ────────────────────────────────────────

/// One input an Insert session received, replayed by `replay_dot`.
///
/// `.` replays an Insert session by re-executing what arrived at the
/// Insert seam, in order, never by inferring, after a dispatch, what it
/// turned out to do. The one exception is an *interactive* input: one
/// whose outcome depends on input the user gave while it ran (accepting a
/// completion, picking a picker item). It is recorded as its own net
/// edit and replayed by applying that edit, never by re-running whatever
/// produced it: re-running would re-open the popup/picker instead of
/// reproducing the pick (see [`Result`](InsertInput::Result)'s own doc).
#[derive(Debug, Clone)]
pub(crate) enum InsertInput {
    /// A key with no Insert keymap binding: one run of its *default*
    /// behaviour (`commands::insert_default_key`), never a keymap walk.
    Key(KeyEvent),
    /// A terminal paste, replayed as one bulk insert rather than synthetic
    /// per-char keys: a synthesized `Enter` would auto-indent, which a real
    /// paste never does.
    Paste(String),
    /// A key bound in the Insert keymap that turned out non-interactive,
    /// re-run on replay via [`Editor::replay_command`], so it decides again
    /// at the new cursor, its own `call!`s, register prefix, and `#:extend`
    /// included.
    Binding { name: Cow<'static, str> },
    /// The net edit an interactive input produced at the cursor: the
    /// completion popup's own Enter, an Insert-key binding that called
    /// `completion-accept!`, or one that opened a picker whose pick (or
    /// dismissal) resolved later, via `on_select`; see [`DotCapture`]'s
    /// own doc for that last case. Replayed by applying the edit directly;
    /// the input that produced it is never re-run, since a re-run would
    /// have nothing to hand back the same pick from.
    Result(CursorReplacement),
}

/// The net text an interactive input wrote at the cursor: `back` chars
/// behind and `forward` ahead of the head, replaced by `text`. The primary
/// cursor's own counts, applied uniformly at every cursor on replay.
/// Anything document-absolute (a completion's `additionalTextEdits`, a
/// picker pick's edit landing away from the cursor) is excluded: it has no
/// meaning at a different cursor, the same reasoning that excludes
/// `additionalTextEdits` from a completion's own recorded replacement.
#[derive(Debug, Clone)]
pub(crate) struct CursorReplacement {
    pub(in crate::editor) back: usize,
    pub(in crate::editor) forward: usize,
    pub(in crate::editor) text: String,
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
    /// Extend mode against an empty recipe is itself the whole recipe; see
    /// `RepeatableAction::selection_recipe`'s doc for the `C`-from-a-bare-cursor case.
    pub extend: bool,
}

/// A recorded editing action that can be replayed by `.`.
///
/// Stores the recipe to re-execute a command rather than the raw changeset:
/// changesets are position-dependent and can't be replayed at a different cursor.
#[derive(Debug, Clone)]
pub(crate) struct RepeatableAction {
    /// The command name that initiated this action (e.g. `"delete"`, `"change"`).
    /// `Cow::Borrowed` for built-in commands (zero allocation); `Cow::Owned` for
    /// dynamically-registered commands (e.g. from the Steel scripting layer).
    pub command: Cow<'static, str>,
    /// The count prefix used originally. Overridden when `.` itself is given a count.
    pub count: usize,
    /// The operand the command ran with (the character `r`, `f`, `t`, …
    /// read). `None` for commands that take none.
    pub arg: Option<CommandArg>,
    /// Inputs the Insert session this command opened received, if any.
    ///
    /// Appended to directly while that session is live (see
    /// [`EditorState::record_insert_input`]). Empty for non-insert actions
    /// like `delete` or `paste-after`.
    pub insert_inputs: Vec<InsertInput>,
    /// Selection-building recipe to replay BEFORE the edit.
    ///
    /// Invariant: `[]` (edit acted on pre-existing selection, or after a
    /// Move-mode motion: `.` deletes the current selection as-is), or a
    /// sequence of `Establishes`/`Composes` steps; see `SelectionTracking`
    /// for what each records. A leading `Composes` step is legal: `C` from a
    /// bare cursor (no prior establish) duplicates that cursor onto the
    /// adjacent line and is itself the first (and only) recipe entry, and
    /// replaying it reproduces exactly what the user did. Rebuilt from
    /// `EditorState::selection_recipe` each time a repeatable command is
    /// recorded.
    pub selection_recipe: Vec<SelectionStep>,
    /// The answers the character reads a Steel command made received, the
    /// reads its callbacks made included. `None` for a native command,
    /// which records its character in `arg`.
    pub reads: Option<ReadLog>,
}

/// Names one Steel command dispatch, so a character read it made, directly
/// or from a callback of one of its reads, can be told apart from any
/// other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DispatchId(u64);

impl DispatchId {
    pub(in crate::editor) fn mint() -> Self {
        Self(super::host_token::next())
    }
}

/// The answers `owner`'s character reads received, in the order the reads
/// were made, `None` for one that ended without a character. Reads take
/// keys in the order they were made, so the order they were answered in is
/// the same.
#[derive(Debug, Clone)]
pub(crate) struct ReadLog {
    owner: DispatchId,
    answers: Vec<Option<char>>,
}

impl ReadLog {
    pub(in crate::editor) fn new(owner: DispatchId) -> Self {
        Self {
            owner,
            answers: Vec::new(),
        }
    }
}

/// Whose character reads the Steel code running now makes: held on
/// `EditorState::read_scope`.
pub(in crate::editor) enum ReadScope {
    /// A Steel command's body, with the answers its reads received before
    /// it returns (a read the body itself ended) and the command is
    /// recorded.
    Body(ReadLog),
    /// The callback of a read the dispatch `DispatchId` made.
    Callback(DispatchId),
}

impl ReadScope {
    pub(in crate::editor) fn owner(&self) -> DispatchId {
        match self {
            Self::Body(log) => log.owner,
            Self::Callback(owner) => *owner,
        }
    }
}

/// `replay_dot`'s state while `.` runs. A character read made during `.`
/// never waits for a key: it takes the next of `answers` and waits in
/// `answered` for replay to resolve it.
pub(in crate::editor) struct DotReplay {
    pub(in crate::editor) answers: VecDeque<Option<char>>,
    pub(in crate::editor) answered: VecDeque<(CharRead, Option<char>)>,
}

/// Proof that an input passed [`EditorState::interactive_input`], which is
/// the only way to get one.
#[must_use]
pub(in crate::editor) struct Interactive(());

/// [`Interactive`] for an input that waits for a later key, with its claim
/// on the capture of the Insert-key binding that opened it, if any. Minted
/// only by [`EditorState::waiting_input`].
#[must_use]
pub(in crate::editor) struct WaitingInput {
    waiter: Option<DotWaiter>,
}

impl WaitingInput {
    pub(in crate::editor) fn into_waiter(self) -> Option<DotWaiter> {
        self.waiter
    }
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
    /// Effective replay count, with the explicit-count override already applied.
    pub(super) count: usize,
}

// ── Macro recording state ─────────────────────────────────────────────────────

impl EditorState {
    /// Queue the keys of the macro in `reg` for replay, `count` times over.
    /// No-op when the register is empty or holds text rather than a macro.
    pub(in crate::editor) fn enqueue_macro_replay(&mut self, reg: char, count: usize) {
        if let Some(keys) = self
            .registers
            .read(reg)
            .and_then(|r| r.as_macro())
            .map(|k| k.to_vec())
        {
            for _ in 0..count {
                self.replay_queue.extend(keys.iter().copied());
            }
        }
    }

    /// Appends `input` to the action whose command opened the live Insert
    /// session. Only the `InsertLayer`'s own handler calls this, so Insert
    /// is always open here. "The last action" is "the session's
    /// action" with no separate tracking: every entry into Insert is a
    /// repeatable native command that stamps before the first input
    /// arrives, and [`commands::repeat_slot_owned`] keeps anything
    /// dispatched mid-session from replacing it. A session with no action
    /// (a layer pushed with no command, or `replay_dot`, which holds the
    /// action for its whole extent) records nothing.
    pub(in crate::editor) fn record_insert_input(&mut self, input: InsertInput) {
        if let Some(action) = self.last_repeatable_action.as_mut() {
            action.insert_inputs.push(input);
        }
    }

    /// The check every interactive input passes before it opens: one
    /// whose outcome depends on input the user gives while it runs (a
    /// prompt, a menu, a completion trigger or accept, and through
    /// [`Self::waiting_input`] a picker or a character read). Each kind's
    /// funnel takes the [`Interactive`] this returns, so none opens without
    /// it.
    ///
    /// Refuses during `.`: an interactive input is never re-run, since its
    /// net edit was recorded as an `InsertInput::Result` and is replayed
    /// directly. Reaching one during replay means the replayed code decided
    /// differently than it did live, and an error beats opening a widget
    /// mid-repeat. Otherwise marks the capture armed for the Insert-key
    /// binding running now, if any, interactive, so the binding becomes
    /// interactive by opening the input rather than by declaring it (see
    /// `InsertInput::Result`'s own doc).
    pub(in crate::editor) fn interactive_input(
        &mut self,
        what: &str,
    ) -> Result<Interactive, String> {
        if self.dot_replay.is_some() {
            return Err(dot_refusal(what));
        }
        if let Some(cap) = self
            .active_session
            .as_mut()
            .and_then(|s| s.dot_capture_mut())
        {
            cap.interactive = true;
        }
        Ok(Interactive(()))
    }

    /// [`Self::interactive_input`] for an input that resolves on a later
    /// key (a picker, a character read), which also takes a claim on the
    /// armed capture: it joins the [`DotPark`] every input of that binding
    /// shares. The capture stays armed until the run ends, so the rest of
    /// the binding's body is recorded too; `Editor::run_dot_captured` then
    /// parks it. No claim when no capture is armed: one is armed only for
    /// the extent of a captured run.
    pub(in crate::editor) fn waiting_input(&mut self, what: &str) -> Result<WaitingInput, String> {
        let Interactive(()) = self.interactive_input(what)?;
        let armed = self
            .active_session
            .as_mut()
            .and_then(|s| s.dot_capture_mut())
            .is_some();
        let waiter = armed.then(|| self.dot_park.get_or_insert_with(DotPark::new).join());
        Ok(WaitingInput { waiter })
    }

    /// Records `answer` for a read `owner` made: into the log of the body
    /// running now when it is `owner`'s, otherwise into the recorded action
    /// when `owner` is its dispatch. Dropped for a dispatch that was not
    /// recorded.
    pub(in crate::editor) fn record_read_answer(
        &mut self,
        owner: DispatchId,
        answer: Option<char>,
    ) {
        let log = match &mut self.read_scope {
            Some(ReadScope::Body(log)) if log.owner == owner => Some(log),
            _ => self
                .last_repeatable_action
                .as_mut()
                .and_then(|a| a.reads.as_mut())
                .filter(|log| log.owner == owner),
        };
        if let Some(log) = log {
            log.answers.push(answer);
        }
    }

    /// Whether the code running now is a callback of a read the recorded
    /// action's dispatch made, so its edits belong to that action.
    pub(in crate::editor) fn in_recorded_callback(&self) -> bool {
        let Some(ReadScope::Callback(owner)) = &self.read_scope else {
            return false;
        };
        self.last_repeatable_action
            .as_ref()
            .and_then(|a| a.reads.as_ref())
            .is_some_and(|log| log.owner == *owner)
    }

    /// Ends `waiter`'s claim for an input that ended without running
    /// anything. The last claim to end finalizes the park's capture, unless
    /// a resolution has it armed, which finalizes it itself.
    pub(in crate::editor) fn release_dot_waiter(&mut self, waiter: DotWaiter) {
        let Some(park) = self.dot_park.as_mut() else {
            return;
        };
        if !park.leave(waiter) || park.is_waited_on() {
            return;
        }
        if let Some(cap) = park.take_cap() {
            self.dot_park = None;
            self.finalize_dot_capture(cap);
        }
    }

    /// Applies a recorded interactive result `r` at `fp`'s cursors, as one
    /// edit composed into the open Insert group: the replay-side
    /// counterpart of `BufferSession::accept`'s own cursor edit. Dismisses
    /// any completion session afterwards: one a replayed key opened is
    /// stale once the replacement lands, the same as a live accept
    /// consuming its session.
    ///
    /// `Err` without an open Insert session on `(pid, bid)`, or with a real
    /// (non-collapsed) selection there, the same guard the live accept
    /// enforces (`completion-accept!`'s own doc): `replace_around_cursors`
    /// force-collapses any selection it touches, silently discarding it,
    /// exactly what this guard exists to refuse instead.
    pub(in crate::editor) fn apply_cursor_replacement(
        &mut self,
        view: &EngineView,
        fp: commands::FocusedPane,
        r: &CursorReplacement,
    ) -> Result<(), String> {
        let (pid, bid) = (fp.pid(), fp.bid(view));
        if !self
            .active_session
            .as_ref()
            .is_some_and(|s| s.is_insert_at(pid, bid))
        {
            return Err("no Insert session to apply the recorded result to".to_string());
        }
        if !self.panes.state[pid][bid]
            .view(self.buffers.get(bid).text())
            .all_cursors()
        {
            return Err("cannot replay a recorded result onto a selection".to_string());
        }
        commands::apply_focused_edit_grouped(self, view, fp, |s| {
            hume_ops::edit::replace_around_cursors(s, r.back, r.forward, &r.text)
        });
        self.dismiss_completion(view);
        Ok(())
    }

    /// `Editor::run_dot_captured`'s finalize step, and `tear_down_insert`'s
    /// own backstop for a capture whose session ends before that checkpoint
    /// runs (an Insert-key binding that accepts a completion and then calls
    /// `exit-insert` in the same dispatch). Interactive: records the
    /// chain's replacement at the cursor as a `Result`, or nothing at all
    /// if the composed edit didn't touch the cursor (an Esc-dismissed
    /// picker whose `on_select` received `#f` and did nothing) or an edit
    /// the chain never saw broke it. Not interactive: records
    /// `cap.fallback` instead: the `Binding` entry an ordinary Insert-key
    /// dispatch stands for, or nothing for a bare completion-popup Enter,
    /// which has no binding of its own.
    pub(in crate::editor) fn finalize_dot_capture(&mut self, cap: DotCapture) {
        let input = if cap.interactive {
            match cap.chain.finish() {
                Ok(replacement) => replacement.map(InsertInput::Result),
                Err(ChainBroken) => {
                    self.report(
                        super::Severity::Warning,
                        "`.` won't repeat this edit: the buffer changed while it ran".to_string(),
                    );
                    None
                }
            }
        } else {
            cap.fallback
        };
        if let Some(input) = input {
            self.record_insert_input(input);
        }
    }
}

impl Editor {
    // ── Doc-edit wrappers ─────────────────────────────────────────────────────

    /// Close out whatever `replay_dot` left open on the focused (pane,
    /// buffer), for either its success or its failure path. The two need
    /// the identical decision, so it lives here once.
    ///
    /// A body that entered real Insert mode closes through
    /// `end_insert_session` (pops the `InsertLayer`, which
    /// `tear_down_insert` needs present to run its own bookkeeping: typed
    /// run, autoindent trim). A body that called native paste retargeted
    /// the pre-opened placeholder to `Paste` (`edit_session::
    /// open_or_retarget`, via `do_paste`), which must stay open for a
    /// following `[`/`]`, the same as a live `p` keypress leaves it, so
    /// it's left alone here. Anything else (a plain edit-only body that
    /// never claimed the placeholder) is still `Replay`-kind, and commits
    /// directly through `doc_ops::commit_open_session`, kind-agnostic,
    /// since Insert and Paste are both already handled above.
    ///
    /// Read dynamically from the session's own kind rather than
    /// `meta.manages_own_session`: that flag only describes what the
    /// *outer* replayed command declares, and a Steel-backed outer command's
    /// meta can never set it (see `CmdMeta::manages_own_session`'s doc), so
    /// a Steel wrapper whose body dispatches native paste needs the same
    /// "leave it open" outcome without ever being able to say so statically.
    ///
    /// No-op if nothing is open at all: the body's own dispatch may already
    /// have closed the pane or buffer the pre-opened placeholder lived on:
    /// `focus::end_focus_sessions`, which every focus/buffer-switch path
    /// runs first, already commits a still-live session of any kind before
    /// that happens, so by the time control returns here there is
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
    /// or Steel/Lazy alike. The main edit body and each `InsertInput::
    /// Binding` entry share this rather than each re-deriving the
    /// native-vs-Steel branch. Returns `false` on the same failure classes
    /// as `run_steel_command` (a Steel body can fail on replay even though
    /// the original run didn't: different buffer state, no match at the
    /// new cursor), and when `arg` doesn't fit `cmd`, which is reported. A
    /// native command that binds runs and returns `true`. A Steel command
    /// takes no operand.
    fn replay_command(
        &mut self,
        cmd: MappableCommand,
        fp: commands::FocusedPane,
        ctx: &CmdCtx,
        arg: Option<CommandArg>,
    ) -> bool {
        match &cmd {
            MappableCommand::SteelBacked { .. } | MappableCommand::Lazy { .. } => {
                if arg.is_some() {
                    commands::report_arg_error(
                        &mut self.state,
                        cmd.name(),
                        &commands::ArgError::Unexpected,
                    );
                    return false;
                }
                if !self.run_steel_command(cmd.name(), ctx) {
                    return false;
                }
                // Inner call! dispatches inside the Steel body run through
                // `commands::run` → step_update_recipe, which may append to
                // selection_recipe. Clear it so stale steps don't
                // contaminate the next command's recipe accumulation.
                self.state.selection_recipe.clear();
                true
            }
            _ => match commands::BoundCommand::focused(cmd, fp, arg) {
                Ok(bound) => {
                    commands::run_body(&mut self.state, &mut self.view, bound, ctx);
                    true
                }
                Err(commands::FocusedBindError::Arg { name, error }) => {
                    commands::report_arg_error(&mut self.state, &name, &error);
                    false
                }
                Err(commands::FocusedBindError::NeedsOperand(missing)) => {
                    commands::report_arg_error(&mut self.state, &missing.name, &missing);
                    false
                }
                Err(commands::FocusedBindError::NotNative(_)) => {
                    unreachable!("this arm already excluded SteelBacked/Lazy above")
                }
            },
        }
    }

    /// Replay a dot-repeat action directly, bypassing dispatch bookkeeping.
    ///
    /// Holds `last_repeatable_action` for the whole replay and restores it
    /// afterwards so `.` chains, which is also what keeps the replay from
    /// recording into it (see [`EditorState::record_insert_input`]). It also
    /// sets [`EditorState::dot_replay`] for the same extent, restoring its
    /// previous value on every exit path.
    pub(in crate::editor) fn replay_dot(&mut self, count: usize) {
        let Some(action) = self.state.last_repeatable_action.take() else {
            return;
        };
        let answers = action
            .reads
            .as_ref()
            .map(|log| log.answers.iter().copied().collect())
            .unwrap_or_default();
        let replay = DotReplay {
            answers,
            answered: VecDeque::new(),
        };
        let prev = self.state.dot_replay.replace(replay);
        self.replay_action(&action, count);
        self.state.dot_replay = prev;
        self.state.last_repeatable_action = Some(action);
    }

    /// `replay_dot`'s body: runs the selection recipe motions with
    /// [`commands::run_body`] (avoiding pipeline re-entry), the edit body and
    /// each recorded `Binding` with [`Self::replay_command`], and each other
    /// recorded input through the path it names (see [`InsertInput`]).
    fn replay_action(&mut self, action: &RepeatableAction, count: usize) {
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
            return;
        };

        let meta = edit_cmd.meta();

        // Commit (or defer) any paste session left open by the keypress that
        // set up `.` itself, using the REPLAYED command's own meta rather
        // than `repeat-last-action`'s (which always defers; see its
        // registration). A session can only still be open here if the
        // command being replayed is itself a ring-cycle command: every other
        // dispatch commits unconditionally before running, so by the time
        // `.` was pressed any unrelated session was already closed. This
        // reproduces `p [ .` as one more ring-cycle step instead of losing
        // the session to `repeat-last-action`'s own dispatch.
        commands::step_paste_commit(&mut self.state, meta.defers_paste_commit);

        // Pre-open a Replay-kind placeholder, the wrapper that folds a
        // recipe replay + the main edit into one undo revision. Its own kind
        // (rather than reusing Insert directly) keeps it distinct from a
        // real, already-open Insert session: only a
        // Replay-kind placeholder is eligible for `open_or_retarget`'s
        // retarget branch (see `EditSessionKind::Replay`'s own doc); a real
        // empty Insert session must still refuse a conflicting open, not
        // silently hand its group to whatever this pre-open is used for.
        // Skipped for a `manages_own_session` command (the paste family): it
        // opens or continues `active_session` itself (`do_paste`/
        // `do_paste_cycle`), which would collide with a placeholder
        // pre-opened here; see `CmdMeta::manages_own_session`'s own doc. A
        // Steel body that itself calls a native paste command still
        // succeeds despite this pre-open: `do_paste`'s own opener retargets
        // this still-open placeholder to `Paste` in place rather than
        // colliding with it (see `edit_session::open_or_retarget`).
        //
        // `Err` here means a real session (of any kind) is already open,
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
                || {
                    self.state
                        .buffers
                        .get(bid)
                        .begin_edit_group(sels.clone(), sels)
                },
            );
            if opened.is_err() {
                return;
            }
        }

        // Rebuild the selection extent the edit originally acted on. No
        // recipe-step command takes an operand: every character command is
        // `Untracked` and so can never reach the recipe. So unlike the edit
        // body below, every step binds with `None`.
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
            let Ok(bound) = commands::BoundCommand::focused(cmd, fp, None) else {
                unreachable!(
                    "a dot-repeat selection-recipe step always names a native command taking no character"
                )
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
        let ran = self.replay_command(edit_cmd, fp, &ctx, action.arg.clone());
        // The answered reads resolve after the body, as live; the recorded
        // Insert inputs after them. Those inputs replay with no answers: a
        // binding that read a character live is recorded as its edit.
        if ran {
            self.resolve_replayed_reads();
        }
        if let Some(replay) = self.state.dot_replay.as_mut() {
            replay.answers.clear();
            replay.answered.clear();
        }
        if !ran {
            // Close whatever the group opened above became so it can't
            // leak, the same decision the success path below makes (see
            // `finish_replay_session`'s own doc). commit drops an empty
            // group (clean noop) and records a partial one (a failure
            // mid-edit stays undoable).
            self.finish_replay_session();
            return;
        }

        // Re-executes the recorded Insert session's inputs in order (see
        // `InsertInput`). A failed or unregistered `Binding`, or a `Result`
        // that no longer applies, is reported and skipped rather than
        // stopping the loop: the live session kept going past it too, so
        // replay does the same. Any `Binding` that leaves Insert
        // stops it, whether or not it also succeeded: a body that calls
        // `exit-insert` and then errors has still left, and nothing recorded
        // after it can apply outside the session it was typed into; `break`,
        // not `return`, so `finish_replay_session` below still runs.
        for input in &action.insert_inputs {
            match input {
                InsertInput::Key(key) => {
                    commands::insert_default_key(&mut self.state, &self.view, fp, *key);
                }
                InsertInput::Paste(text) => self.apply_insert_mode_paste(text),
                InsertInput::Result(r) => {
                    if let Err(msg) = self.state.apply_cursor_replacement(&self.view, fp, r) {
                        self.report(super::Severity::Error, msg);
                    }
                }
                InsertInput::Binding { name } => {
                    // A command name loses its keymap binding only if the
                    // binding itself is removed between the original
                    // keypress and this replay (`unbind-key!`, a plugin
                    // reload). Unlike the selection-recipe/edit-body
                    // lookups above, this name was never guaranteed to
                    // stay registered, so a miss reports rather than
                    // panicking.
                    let Some(cmd) = self.resolve_mappable(name.as_ref()) else {
                        continue;
                    };
                    // The live keypress dispatched with
                    // `in_insert_key_dispatch` set (see that field's own
                    // doc); replay recreates the same context, so a body
                    // that calls `insert-key!` still finds it in flight. An
                    // interactive builtin the body calls this time (it
                    // decided differently than it did live) refuses loudly
                    // instead; see `EditorState::dot_replay`'s own doc.
                    self.with_insert_key_dispatch(|ed| {
                        ed.replay_command(
                            cmd,
                            fp,
                            &super::input_stack::insert::INSERT_KEY_CTX,
                            None,
                        )
                    });
                    // Checked regardless of success: a body that calls
                    // `exit-insert` and then fails (an error raised after
                    // that call already ran) still left Insert, so nothing
                    // recorded after it belongs to a session that's gone,
                    // whether the binding that closed it also succeeded or
                    // not.
                    if self.state.mode() != Mode::Insert {
                        break;
                    }
                }
            }
        }
        self.finish_replay_session();
    }

    /// Resolves each read made during `.` with the answer it took, in the
    /// order they were made, running a callback at once: a read a callback
    /// makes is resolved by this same loop.
    fn resolve_replayed_reads(&mut self) {
        while let Some((read, answer)) = self
            .state
            .dot_replay
            .as_mut()
            .and_then(|replay| replay.answered.pop_front())
        {
            let resolved = char_read::resolve(&mut self.state, read, answer);
            if let Some(call) = char_read::run_resolved(self, resolved) {
                self.run_pending_batch(VecDeque::from([call]));
            }
        }
    }

    /// Builds a fresh [`edit_session::DotCapture`] at the focused cursor and
    /// runs `f` with it armed, wrapping only the one operation that might go
    /// interactive. `fallback` is what to record
    /// if `f` turns out *not* interactive (see [`DotCapture::fallback`]'s
    /// own doc). Delegates to [`Self::run_dot_captured`] for the arm/
    /// take/finalize mechanics.
    pub(in crate::editor) fn with_dot_capture(
        &mut self,
        fallback: Option<InsertInput>,
        f: impl FnOnce(&mut Self),
    ) {
        let fp = commands::FocusedPane::current(&self.state);
        let (pane, buffer) = (fp.pid(), fp.bid(&self.view));
        let text = self.state.buffers.get(buffer).text();
        let head = self.state.panes.state[pane][buffer]
            .view(text)
            .primary()
            .head();
        let cap = DotCapture {
            pane,
            buffer,
            chain: DotChain::new(head, text),
            interactive: false,
            fallback,
        };
        self.run_dot_captured(cap, f);
    }

    /// Runs `f` as the resolution of a waiting input that holds `waiter`,
    /// with the park's capture armed so `f`'s edits are recorded into it.
    /// With no waiter, the park gone (its session ended) or no capture in
    /// it, `f` runs unrecorded.
    pub(in crate::editor) fn resolve_dot_waiter(
        &mut self,
        waiter: Option<DotWaiter>,
        f: impl FnOnce(&mut Self),
    ) {
        let Some(waiter) = waiter else {
            return f(self);
        };
        let Some(park) = self.state.dot_park.as_mut() else {
            return f(self);
        };
        if !park.leave(waiter) {
            return f(self);
        }
        match park.take_cap() {
            Some(cap) => self.run_dot_captured(cap, f),
            None => {
                if !park.is_waited_on() {
                    self.state.dot_park = None;
                }
                f(self);
            }
        }
    }

    /// Arms `cap` on its own `(pane, buffer)`'s Insert session, runs `f`,
    /// then takes the capture back: into the park while inputs still wait
    /// on it, including inputs `f` opened ([`EditorState::waiting_input`]),
    /// otherwise finalized. No-op arming if the target session is already gone
    /// by the time this runs, which a resolution queued as a callback can
    /// reach: `f` still runs, with nothing recording it for `.`.
    pub(in crate::editor) fn run_dot_captured(
        &mut self,
        mut cap: DotCapture,
        f: impl FnOnce(&mut Self),
    ) {
        let (pane, buffer) = (cap.pane, cap.buffer);
        if let Some(session) = self
            .state
            .active_session
            .as_mut()
            .filter(|s| s.is_insert_at(pane, buffer))
        {
            let text = self.state.buffers.get(buffer).text();
            let head = self.state.panes.state[pane][buffer]
                .view(text)
                .primary()
                .head();
            match cap.chain.rearm(head, text) {
                Some(chain) => {
                    cap.chain = chain;
                    session.arm_dot_capture(cap);
                }
                None => {
                    // An edit landed on this buffer while `cap` sat detached
                    // (armed on a picker instead of this session) after it
                    // had recorded one of its own, so nothing it holds
                    // composes with the next. Report and drop the whole
                    // capture: nothing here is safe to record, interactive
                    // or not.
                    self.report(
                        super::Severity::Warning,
                        "`.` won't repeat this key: the buffer changed while it waited for input"
                            .to_string(),
                    );
                }
            }
        }
        f(self);
        let Some(cap) = self
            .state
            .active_session
            .as_mut()
            .filter(|s| s.is_insert_at(pane, buffer))
            .and_then(|s| s.take_dot_capture())
        else {
            return;
        };
        match self.state.dot_park.as_mut() {
            Some(park) if park.is_waited_on() => park.put_cap(cap),
            _ => {
                self.state.dot_park = None;
                self.state.finalize_dot_capture(cap);
            }
        }
    }

    /// Drain the macro replay queue, executing each key in order and
    /// settling after each one.
    ///
    /// Sets `is_replaying` for the duration so that `Q`/`q` intercepts inside
    /// replayed keys cannot start nested recording or replay sessions, including
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
    /// stays `true` across every one of these settles, so `confirm_permit`'s
    /// `!is_replaying` guard still blocks a confirm the macro would have no
    /// queued key left to answer; see that guard's "Macro replay" doc
    /// paragraph. The deferred prompt still arrives on the next real
    /// buffer-enter.
    ///
    /// `message_logged_this_input` is OR'd back in every iteration, not left
    /// to whatever the just-replayed key's own `handle_input` set it to: the
    /// triggering dispatch (e.g. the register char after `@`, which
    /// populated `replay_queue` in the first place) may itself have logged a
    /// message moments before this function was even called, and each
    /// settle() call clears the flag at its own end, losing
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
        // If the macro left Insert open, its own entry command's action is
        // still recording (a `record_insert_input` call for every key typed
        // since), so restoring `saved_action` over it would both discard that
        // action, mid-session, and hand the still-live Insert session's
        // input to whatever the macro replaced instead (see
        // `record_insert_input`'s own doc: "the session's action" is
        // whichever one occupies the slot, not tracked separately). The
        // macro's own action is the correct one to keep repeatable here:
        // finishing that Insert session by hand and pressing `.` should
        // repeat the whole thing the macro started, not `saved_action`.
        if self.state.mode() != Mode::Insert {
            self.state.last_repeatable_action = saved_action;
        }
    }
}

/// The error an interactive input reached during `.` raises.
pub(in crate::editor) fn dot_refusal(what: &str) -> String {
    format!("{what}: reached during `.`, but the recorded input took a different branch this time")
}
