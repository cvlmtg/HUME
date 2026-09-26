//! Dot-repeat (`.`) and macro-replay state and execution.
//!
//! Dot-repeat records a [`RepeatableAction`] recipe (command name + count +
//! char arg + selection-building steps + Insert-session inputs) rather than
//! a raw changeset, since changesets are position-dependent and can't be
//! replayed at a different cursor. Macro replay drains a queue of recorded
//! keys through the normal event path.

use hume_editing::changeset::{ChangeSet, Operation};
use hume_engine::pipeline::EngineView;
use hume_rope::offset::CharOffset;
use std::borrow::Cow;
use termina::event::{Event as TerminalEvent, KeyEvent};

use super::dispatch::CmdCtx;
use super::edit_session::{self, DotCapture, EditSessionKind};
use super::registry::MappableCommand;
use super::{Editor, EditorState, Mode, commands, doc_ops};

// ── Dot-repeat / insert-session state ────────────────────────────────────────

/// One input an Insert session received, replayed by `replay_dot`.
///
/// `.` replays an Insert session by re-executing what arrived at the
/// Insert seam, in order — never by inferring, after a dispatch, what it
/// turned out to do. The one exception is an *interactive* input — one
/// whose outcome depends on input the user gave while it ran (accepting a
/// completion, picking a picker item) — which is recorded as its own net
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
    /// at the new cursor — its own `call!`s, register prefix, and `#:extend`
    /// included.
    Binding { name: Cow<'static, str> },
    /// The net edit an interactive input produced at the cursor: the
    /// completion popup's own Enter, an Insert-key binding that called
    /// `completion-accept!`, or one that opened a picker whose pick (or
    /// dismissal) resolved later, via `on_select` — see [`DotCapture`]'s
    /// own doc for that last case. Replayed by applying the edit directly;
    /// the input that produced it is never re-run, since a re-run would
    /// have nothing to hand back the same pick from.
    Result(CursorReplacement),
}

/// The net text an interactive input wrote at the cursor: `back` chars
/// behind and `forward` ahead of the head, replaced by `text`. The primary
/// cursor's own counts, applied uniformly at every cursor on replay.
/// Anything document-absolute (a completion's `additionalTextEdits`, a
/// picker pick's edit landing away from the cursor) is excluded — it has no
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
    /// Inputs the Insert session this command opened received, if any.
    ///
    /// Appended to directly while that session is live (see
    /// [`EditorState::record_insert_input`]). Empty for non-insert actions
    /// like `delete` or `paste-after`.
    pub insert_inputs: Vec<InsertInput>,
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

impl EditorState {
    /// Appends `input` to the action whose command opened the live Insert
    /// session. Only the `InsertLayer`'s own handler calls this, so Insert
    /// is open by construction. "The last action" is "the session's
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

    /// Marks the Insert-key dispatch or completion-accept currently in
    /// progress as interactive — called by `completion-accept!`/`picker!`/
    /// `live-picker!`/`completion-trigger` themselves, whichever fires
    /// first, so a binding becomes interactive by calling one of them
    /// rather than by declaring it (see `InsertInput::Result`'s own doc).
    /// No-op if no capture is armed — a stray call from outside an armed
    /// dispatch (a hook, a timer) is harmless, since nothing else reads it.
    pub(in crate::editor) fn mark_dot_interactive(&mut self) {
        if let Some(cap) = self
            .active_session
            .as_mut()
            .and_then(|s| s.dot_capture_mut())
        {
            cap.interactive = true;
        }
    }

    /// Applies a recorded interactive result `r` at `fp`'s cursors, as one
    /// edit composed into the open Insert group — the replay-side
    /// counterpart of `BufferSession::accept`'s own cursor edit. Dismisses
    /// any completion session afterwards: one a replayed key opened is
    /// stale once the replacement lands, the same as a live accept
    /// consuming its session.
    ///
    /// `Err` without an open Insert session on `(pid, bid)`, or with a real
    /// (non-collapsed) selection there — the same guard the live accept
    /// enforces (`completion-accept!`'s own doc), which replay had skipped
    /// until now: `replace_around_cursors` force-collapses any selection it
    /// touches, silently discarding it, exactly what the live guard exists
    /// to refuse instead.
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
        let Some(pbs) = self.panes.buffer_state(pid, bid) else {
            return Err("no Insert session to apply the recorded result to".to_string());
        };
        if !pbs.selections().iter_sorted().all(|s| s.is_collapsed()) {
            return Err("cannot replay a recorded result onto a selection".to_string());
        }
        doc_ops::apply_doc_edit_grouped(
            &mut self.buffers,
            &self.config.decorations,
            &mut self.panes.state,
            &mut self.panes.jumps,
            &mut self.active_session,
            pid,
            bid,
            |b, s| hume_ops::edit::replace_around_cursors(b, s, r.back, r.forward, &r.text),
        );
        self.dismiss_completion(view);
        Ok(())
    }

    /// `Editor::resolve_dot_capture`'s finalize step, and `tear_down_insert`'s
    /// own backstop for a capture whose session ends before that checkpoint
    /// runs (an Insert-key binding that accepts a completion and then calls
    /// `exit-insert` in the same dispatch). Composes `cap`'s recorded edits
    /// into one net transform via [`ChangeSet::compose_all`], extracts the
    /// region under `cap.head_before` via [`cursor_replacement_at`], and
    /// either replaces the `Binding` placeholder entry with it
    /// (`cap.has_placeholder`) or pushes it as a new entry
    /// (`accept_completion_selection`'s own Enter, which pushed no
    /// placeholder). Drops or skips it instead when there is nothing to
    /// replay: nothing was edited at all (an Esc-dismissed picker whose
    /// `on_select` received `#f` and did nothing), or the edit didn't touch
    /// the cursor.
    pub(in crate::editor) fn finalize_dot_capture(&mut self, cap: DotCapture) {
        let Some(action) = self.last_repeatable_action.as_mut() else {
            return;
        };
        if cap.has_placeholder
            && !matches!(
                action.insert_inputs.last(),
                Some(InsertInput::Binding { .. })
            )
        {
            // Nothing else can append while a modal picker owns input (see
            // `DotCapture`'s own doc) — reaching this means the Insert
            // session this capture belonged to has since ended some other
            // way, and there is nothing left to replace.
            return;
        }
        let outcome = ChangeSet::compose_all(cap.edits)
            .and_then(|delta| cursor_replacement_at(&delta, cap.head_before));
        match (outcome, cap.has_placeholder) {
            (Some(r), true) => {
                let entry = action
                    .insert_inputs
                    .last_mut()
                    .expect("checked present above; nothing else touches it meanwhile");
                *entry = InsertInput::Result(r);
            }
            (Some(r), false) => action.insert_inputs.push(InsertInput::Result(r)),
            (None, true) => {
                action.insert_inputs.pop();
            }
            (None, false) => {} // no placeholder was pushed; nothing to undo
        }
    }
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
    /// Binding` entry share this rather than each re-deriving the
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
                if !self.run_steel_command(cmd.name(), ctx, char_arg) {
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
    /// Holds `last_repeatable_action` for the whole replay and restores it
    /// afterwards so `.` chains — which is also what keeps the replay from
    /// recording into it (see [`EditorState::record_insert_input`]) — and
    /// sets [`EditorState::dot_replay`] for the same extent, restoring its
    /// previous value on every exit path.
    pub(in crate::editor) fn replay_dot(&mut self, count: usize) {
        let Some(action) = self.state.last_repeatable_action.take() else {
            return;
        };
        let prev = std::mem::replace(&mut self.state.dot_replay, true);
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
        // than `repeat-last-action`'s (which always defers — see its
        // registration). A session can only still be open here if the
        // command being replayed is itself a ring-cycle command: every other
        // dispatch commits unconditionally before running, so by the time
        // `.` was pressed any unrelated session was already closed. This
        // reproduces `p [ .` as one more ring-cycle step instead of losing
        // the session to `repeat-last-action`'s own dispatch.
        commands::step_paste_commit(&mut self.state, meta.defers_paste_commit);

        // Pre-open a Replay-kind placeholder — the wrapper that folds a
        // recipe replay + the main edit into one undo revision. Its own kind (rather than reusing Insert directly) keeps
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
            return;
        }

        self.replay_insert_inputs(&action.insert_inputs, fp);
        self.finish_replay_session();
    }

    /// Re-executes a recorded Insert session's inputs in order (see
    /// [`InsertInput`]). A failed or unregistered `Binding`, or a `Result`
    /// that no longer applies, is reported and skipped rather than stopping
    /// the loop: the live session kept going past it too, so replay does the
    /// same. Any `Binding` that leaves Insert genuinely stops it, whether or
    /// not it also succeeded — a body that calls `exit-insert` and then
    /// errors has still left, and nothing recorded after it can apply
    /// outside the session it was typed into.
    fn replay_insert_inputs(&mut self, inputs: &[InsertInput], fp: commands::FocusedPane) {
        for input in inputs {
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
                        continue;
                    };
                    // The live keypress dispatched with
                    // `in_insert_key_dispatch` set (see that field's own
                    // doc); replay recreates the same context, so a body
                    // that calls `insert-key!` still finds it in flight. An
                    // interactive builtin the body calls this time (it
                    // decided differently than it did live) refuses loudly
                    // instead — see `EditorState::dot_replay`'s own doc.
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
                    // that call already ran) still left Insert — nothing
                    // recorded after it belongs to a session that's gone,
                    // whether the binding that closed it also succeeded or
                    // not.
                    if self.state.mode() != Mode::Insert {
                        return;
                    }
                }
            }
        }
    }

    /// Arms [`edit_session::DotCapture`] around an operation that might go
    /// interactive — `handle_insert`'s Leaf branch, or
    /// `accept_completion_selection`'s own accept. Called before the
    /// operation runs, the only chance to see the primary head at that
    /// point. No-op if no Insert session is open here, which never happens
    /// in practice: both callers only ever run from inside one.
    pub(in crate::editor) fn arm_dot_capture(&mut self, has_placeholder: bool) {
        let fp = commands::FocusedPane::current(&self.state);
        let (pid, bid) = (fp.pid(), fp.bid(&self.view));
        let head_before = self.state.panes.state[pid][bid]
            .selections()
            .primary()
            .head();
        if let Some(session) = self
            .state
            .active_session
            .as_mut()
            .filter(|s| s.is_insert_at(pid, bid))
        {
            session.arm_dot_capture(head_before, has_placeholder);
        }
    }

    /// The checkpoint run right after an Insert-key dispatch or a completion
    /// accept returns, and again after each batch `drain_pending_work`
    /// drains — the same "checked every pass, no single write-site
    /// chokepoint" shape `detect_mode_change`'s own doc describes, for the
    /// same reason: a chained picker's own `on_select` can reopen another
    /// picker or resolve outright, and only checking *after* a batch has
    /// actually run tells them apart.
    ///
    /// Drops the armed capture if nothing interactive happened; finalizes
    /// it on the spot if something did and no picker is left open (a direct
    /// `completion-accept!`, or a picker opened and already resolved within
    /// the same dispatch); otherwise leaves it armed for a later call —
    /// from `drain_pending_work` — to finalize once the still-open picker
    /// (or chain) resolves. No-op if no session is open, or none is armed:
    /// `tear_down_insert`'s own backstop already resolved a capture whose
    /// session ended before this ran.
    pub(in crate::editor) fn resolve_dot_capture(&mut self) {
        let Some(session) = self.state.active_session.as_mut() else {
            return;
        };
        let Some(interactive) = session.dot_capture().map(|cap| cap.interactive) else {
            return;
        };
        if !interactive {
            session.take_dot_capture();
            return;
        }
        if self.state.input.picker().is_some() {
            return;
        }
        let cap = session
            .take_dot_capture()
            .expect("dot_capture() returned Some above");
        self.state.finalize_dot_capture(cap);
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
        // If the macro left Insert open, its own entry command's action is
        // still recording (a `record_insert_input` call for every key typed
        // since) — restoring `saved_action` over it would both discard that
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

/// Extracts the edited region of `delta` that contains `head` (a position
/// in `delta`'s *old* document), as a cursor-relative replacement — `None`
/// if `delta` is identity, or every edited region lands away from `head`.
/// `delta` can hold more than one region — a completion's own
/// `additionalTextEdits`, or a second cursor's own edit under a
/// multi-cursor accept — and any region other than the one at `head` is
/// simply skipped: it's document-absolute, or belongs to a different
/// cursor, the same reasoning [`CursorReplacement`]'s own doc gives for
/// excluding `additionalTextEdits` from the replacement it records.
fn cursor_replacement_at(delta: &ChangeSet, head: CharOffset) -> Option<CursorReplacement> {
    let head = head.index();
    let mut pos = 0usize; // old-doc position of the op cursor
    let mut ops = delta.ops().iter().peekable();
    while let Some(op) = ops.next() {
        match op {
            Operation::Retain(n) => pos += n,
            Operation::Insert(s) => {
                if head == pos {
                    return Some(CursorReplacement {
                        back: 0,
                        forward: 0,
                        text: s.clone(),
                    });
                }
            }
            Operation::Delete(n) => {
                let region_start = pos;
                pos += n;
                // A Delete this codebase ever writes as part of a
                // replacement is immediately followed by the Insert of the
                // replacement text — consumed together as one region.
                let text = match ops.peek() {
                    Some(Operation::Insert(s)) => {
                        let s = s.clone();
                        ops.next();
                        s
                    }
                    _ => String::new(),
                };
                if head >= region_start && head <= pos {
                    return Some(CursorReplacement {
                        back: head - region_start,
                        forward: pos - head,
                        text,
                    });
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod cursor_replacement_tests {
    use super::cursor_replacement_at;
    use hume_editing::changeset::ChangeSetBuilder;
    use hume_rope::offset::CharOffset;

    fn co(n: usize) -> CharOffset {
        CharOffset::new(n)
    }

    #[test]
    fn identity_has_no_replacement() {
        let mut b = ChangeSetBuilder::new(co(6));
        b.retain_rest();
        assert!(cursor_replacement_at(&b.finish(), co(3)).is_none());
    }

    #[test]
    fn insert_only_region_at_head() {
        // "ab|cd" → "ab|Xcd": inserting with nothing deleted, head right at
        // the insertion point.
        let mut b = ChangeSetBuilder::new(co(4));
        b.retain(2);
        b.insert("X");
        b.retain_rest();
        let r = cursor_replacement_at(&b.finish(), co(2)).expect("insert at head must match");
        assert_eq!((r.back, r.forward, r.text.as_str()), (0, 0, "X"));
    }

    #[test]
    fn delete_and_insert_region_containing_head() {
        // "abcd" → "aXd": delete "bc" (old positions 1..3), insert "X". A
        // head inside the deleted span (2) reports the split around it.
        let mut b = ChangeSetBuilder::new(co(4));
        b.retain(1);
        b.delete(2);
        b.insert("X");
        b.retain_rest();
        let r = cursor_replacement_at(&b.finish(), co(2))
            .expect("head inside the deleted span must match");
        assert_eq!((r.back, r.forward, r.text.as_str()), (1, 1, "X"));
    }

    #[test]
    fn region_away_from_head_is_ignored() {
        // The edit lands at the start; head sits at the untouched end.
        let mut b = ChangeSetBuilder::new(co(4));
        b.delete(1);
        b.insert("X");
        b.retain_rest();
        assert!(cursor_replacement_at(&b.finish(), co(4)).is_none());
    }

    /// The shape a completion's `additionalTextEdits` produces alongside its
    /// own cursor edit: two edited regions in one delta. Only the one at
    /// `head` is extracted; the other — document-absolute, or a different
    /// cursor's own edit under a multi-cursor accept — is skipped.
    #[test]
    fn picks_the_region_at_head_and_skips_the_other() {
        let mut b = ChangeSetBuilder::new(co(6));
        b.insert("// "); // an import, landing away from the cursor
        b.retain(2);
        b.delete(2);
        b.insert("XYZ"); // the accept's own edit, at the cursor
        b.retain_rest();
        let r = cursor_replacement_at(&b.finish(), co(4)).expect("the region at head must match");
        assert_eq!((r.back, r.forward, r.text.as_str()), (2, 0, "XYZ"));
    }
}
