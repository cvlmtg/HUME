//! Dot-repeat (`.`): recording a change and replaying it.
//!
//! Dot-repeat records a [`RepeatableAction`]: the repeatable command that
//! began a change, its count, the selection recipe that built the extent it
//! acted on, the [answers](Answer) its interactive inputs got, and an ordered
//! [transcript](Input) of every Insert-session input the change consumed. It
//! records a recipe rather than a raw changeset, since changesets are
//! position-dependent and can't be replayed at a different cursor.
//!
//! A change is owned, while it is in flight, by a single [`Recording`]. It
//! stays open while it has an outstanding input source (see [`Recording`])
//! and settles into the replayable slot once the last source closes, or
//! earlier, when an input it could not answer on `.` opens. Whether an
//! interactive input or a dispatched command belongs to it is decided by who
//! is running when it happens (see [`RecordingScope`]): an input is recorded
//! only when repeatable code opens it, and one any other code opens ends the
//! change ([`EditorState::waiting_input`]). A command is repeatable when its
//! `CmdMeta::repeatable` is set: at registration for a native command, by
//! `#:repeatable` for a Steel one. Every input funnel appends to its
//! transcript at the seam the input arrives, never by inferring afterward
//! what a dispatch did.

use hume_core::{Count, HostToken};
use hume_engine::pipeline::EngineView;
use hume_scripting::Rooted;
use std::collections::VecDeque;
use std::rc::{Rc, Weak};

use super::dispatch::CmdCtx;
use super::edit_session::{self, EditSessionKind, InsertSession};
use super::input_stack::char::{AfterRead, CharRead};
use super::keymap::CanonicalKey;
use super::pipe_run::RunId;
use super::registry::{Dispatchable, MappableCommand};
use super::{Editor, EditorState, Mode, commands};

pub(in crate::editor) mod capture;
mod model;
mod recipe;

pub(in crate::editor) use capture::{DotCapture, finalize_dot_capture};
pub(crate) use model::{Answer, Choice, CursorReplacement, Input, Pick, Picked, RepeatableAction};
pub(crate) use recipe::SelectionStep;

// ── Recording: the live owner of an in-flight change ──────────────────────────

/// One outstanding input source on a [`Recording`]: its command's own body
/// running, an Insert session it opened, an Insert-key binding dispatching,
/// or a pending interactive input. The source closes when the ticket drops, so a
/// holder dropped on any path (a layer a reload truncates, a queued call a
/// reload discards) closes it too. A ticket also names the recording it was
/// opened on: one resolving after a newer change superseded that recording
/// records nothing.
#[must_use]
#[derive(Debug)]
pub(in crate::editor) struct SourceTicket {
    sources: Rc<()>,
}

/// Names one [`Input::Binding`] entry of a recording, for as long as the
/// entry exists. Never reused within a recording, so a reference to an entry
/// that was removed finds nothing rather than a later entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) struct BindingId(u64);

/// Whose code a [`RecordingScope`] runs, and so where the answers of the
/// inputs it opens are recorded: the change's command, or an Insert-key
/// binding's entry.
#[derive(Debug, Clone, Copy)]
enum AnswerSlot {
    Command,
    /// A binding whose command is repeatable.
    Binding(BindingId),
    /// A binding whose command is not repeatable: `.` re-runs it, but
    /// an input it opens ends the change ([`EditorState::waiting_input`]),
    /// so it never takes an answer.
    UnrecordedBinding(BindingId),
}

/// The code of a recorded change running now, held in
/// [`EditorState::recording_scope`]: its command's body, an Insert-key
/// binding's dispatch, or the callback of one of its inputs. An interactive
/// input opened inside it records its answer in `slot`, unless the slot is
/// an [`AnswerSlot::UnrecordedBinding`]'s; an Insert session it opens feeds
/// the change, and a repeatable command it dispatches is part of the change
/// rather than a new one. Code outside every scope (a hook, a timer, an LSP
/// callback, a key) owns none of this.
#[derive(Debug)]
pub(in crate::editor) struct RecordingScope {
    source: SourceTicket,
    slot: AnswerSlot,
}

/// What runs now in [`ScopeSlot`].
#[derive(Debug)]
enum Running {
    /// A recorded change's code.
    Scope(RecordingScope),
    /// An Insert key's binding, for its whole dispatch, with its own scope
    /// when the session records a change.
    InsertKey(Option<RecordingScope>),
    /// A key's binding in the command mode prompt, for its whole dispatch.
    /// It runs no recorded change's code, and a repeatable command it
    /// dispatches starts no change: the prompt is not part of what `.`
    /// repeats.
    CommandLineKey,
    /// A picker keymap binding's call while its picker is open. Like
    /// [`Running::CommandLineKey`], it runs no recorded change's code: `.`
    /// repeats only the binding that closes the picker, which from that
    /// point runs as the picker's answer ([`ScopeSlot::picker_key_closed`]).
    PickerKey(PickerKeyRun),
}

/// A picker keymap binding's queued call: the picker whose key was pressed,
/// the key, and the item the binding was handed, from which the picker's
/// answer builds its pick if the binding closes it.
#[derive(Debug, Clone)]
pub(in crate::editor) struct PickerKeyRun {
    picker: HostToken,
    key: CanonicalKey,
    selected: Option<(usize, Rooted)>,
}

impl PickerKeyRun {
    /// The binding for `key` pressed in the picker `picker` names, with
    /// `selected` the index in the picker's items and the payload of the
    /// item it was handed, `None` when nothing matched.
    pub(in crate::editor) fn new(
        picker: HostToken,
        key: CanonicalKey,
        selected: Option<(usize, Rooted)>,
    ) -> Self {
        Self {
            picker,
            key,
            selected,
        }
    }

    pub(in crate::editor) fn picker(&self) -> HostToken {
        self.picker
    }

    /// The pick the picker's answer records when this binding closes it,
    /// `picked` naming the handed item for `.`.
    pub(in crate::editor) fn into_pick(self, picked: impl FnOnce(usize, Rooted) -> Picked) -> Pick {
        Pick {
            action: Some(self.key),
            picked: self.selected.map(|(index, payload)| picked(index, payload)),
        }
    }
}

/// The code running now, if any: a [`RecordingScope`], an Insert key's
/// binding, a command-line key's binding, or a picker keymap binding. Only
/// [`with_recording_scope`], [`dispatch_insert_key`],
/// [`dispatch_command_line_key`] and [`run_picker_key`] set it, for the
/// extent of one call, so nothing outlives the code it names. A picker
/// binding's entry turns into its picker's answer scope when the binding
/// closes the picker, for the rest of the same call.
#[derive(Debug, Default)]
pub(in crate::editor) struct ScopeSlot(Option<Running>);

impl ScopeSlot {
    /// The [`RecordingScope`] running now, if any.
    pub(in crate::editor) fn running(&self) -> Option<&RecordingScope> {
        match &self.0 {
            Some(Running::Scope(scope) | Running::InsertKey(Some(scope))) => Some(scope),
            Some(Running::InsertKey(None) | Running::CommandLineKey | Running::PickerKey(_))
            | None => None,
        }
    }

    /// The binding whose pick the picker `picker` names records when the
    /// running code closes it: `Some` only inside a keymap binding of that
    /// picker that has not closed it yet.
    pub(in crate::editor) fn closing_binding(&self, picker: HostToken) -> Option<PickerKeyRun> {
        match &self.0 {
            Some(Running::PickerKey(run)) if run.picker == picker => Some(run.clone()),
            _ => None,
        }
    }

    /// The running picker binding closed its picker: the rest of the call
    /// runs as that picker's answer, in `scope` when the picker belongs to a
    /// recorded change and outside every scope otherwise.
    pub(in crate::editor) fn picker_key_closed(&mut self, scope: Option<RecordingScope>) {
        debug_assert!(
            matches!(self.0, Some(Running::PickerKey(_))),
            "only a running picker binding closes its picker as its answer"
        );
        self.0 = scope.map(Running::Scope);
    }

    /// Whether an Insert key's binding is dispatching. Dispatch never yields
    /// before it returns, all the way through a Steel `call!`, so a hook
    /// queued during the same keypress drains later, in `settle`, with this
    /// `false`.
    pub(in crate::editor) fn insert_key(&self) -> bool {
        matches!(self.0, Some(Running::InsertKey(_)))
    }

    /// Whether anything runs: a recorded change's code, an Insert key's
    /// binding, a command-line key's binding, or a picker binding that has
    /// not closed its picker.
    pub(in crate::editor) fn is_set(&self) -> bool {
        self.0.is_some()
    }
}

impl AsMut<EditorState> for EditorState {
    fn as_mut(&mut self) -> &mut EditorState {
        self
    }
}

impl AsMut<EditorState> for Editor {
    fn as_mut(&mut self) -> &mut EditorState {
        &mut self.state
    }
}

/// Runs `f` with `running` in the [`ScopeSlot`], restoring what it replaced
/// afterward. A scope in `running` drops once `f` returns, closing its
/// source on the change.
fn with_running<H: AsMut<EditorState>, R>(
    host: &mut H,
    running: Option<Running>,
    f: impl FnOnce(&mut H) -> R,
) -> R {
    let prev = std::mem::replace(&mut host.as_mut().recording_scope.0, running);
    let r = f(host);
    drop(std::mem::replace(
        &mut host.as_mut().recording_scope.0,
        prev,
    ));
    r
}

/// Runs `f` with `scope` as the running [`RecordingScope`], restoring the one
/// it replaced afterward.
pub(in crate::editor) fn with_recording_scope<H: AsMut<EditorState>, R>(
    host: &mut H,
    scope: Option<RecordingScope>,
    f: impl FnOnce(&mut H) -> R,
) -> R {
    with_running(host, scope.map(Running::Scope), f)
}

/// Dispatches `cmd` as an Insert key's binding, `scope` its own
/// [`RecordingScope`] (`None` when nothing records it), restoring what it
/// replaced afterward. Always an explicit count of 1, never extending.
/// Shared by `handle_insert`'s trie-leaf dispatch and the replay of a
/// recorded [`Input::Binding`] entry, so a binding re-run on `.` sees the
/// same context it saw live. Set for the whole dispatch regardless of what
/// the command does internally (mode switches included: dispatch is
/// synchronous all the way through a Steel `call!`, so this never outlives
/// the call it wraps).
/// It keeps a repeatable command the binding dispatches from starting a
/// change of its own ([`repeat_slot_owned`]) and lets `insert-key!` run.
/// The scope holds the recording in flight open for the call, so a binding
/// that leaves Insert and re-enters it records what follows into the same
/// change.
pub(in crate::editor) fn dispatch_insert_key(
    ed: &mut Editor,
    scope: Option<RecordingScope>,
    cmd: Dispatchable,
) -> bool {
    dispatch_key_binding(ed, Running::InsertKey(scope), cmd)
}

/// Dispatches `cmd` as a key's binding in the command mode prompt, with
/// [`Running::CommandLineKey`] set for the whole dispatch. Always an
/// explicit count of 1, never extending.
pub(in crate::editor) fn dispatch_command_line_key(ed: &mut Editor, cmd: Dispatchable) {
    dispatch_key_binding(ed, Running::CommandLineKey, cmd);
}

/// Runs `f`, a picker keymap binding's queued call, with `run` set for the
/// whole call: a repeatable command it dispatches starts no change, and a
/// `picker-close!` it makes on `run`'s picker records `run`'s pick
/// ([`ScopeSlot::closing_binding`]).
pub(in crate::editor) fn run_picker_key<R>(
    ed: &mut Editor,
    run: PickerKeyRun,
    f: impl FnOnce(&mut Editor) -> R,
) -> R {
    with_running(ed, Some(Running::PickerKey(run)), f)
}

/// Dispatches `cmd` at an explicit count of 1, never extending, with
/// `running` set for the whole dispatch.
fn dispatch_key_binding(ed: &mut Editor, running: Running, cmd: Dispatchable) -> bool {
    let ctx = CmdCtx {
        count: Count::typed(1),
        extend: false,
    };
    with_running(ed, Some(running), |ed| ed.dispatch(cmd, ctx))
}

/// A fresh change a repeatable command starts, as [`record_change`] records
/// it.
pub(in crate::editor) struct NewChange {
    pub(in crate::editor) command: Rc<str>,
    pub(in crate::editor) count: Count,
    /// The operand the command was dispatched with, recorded as its first
    /// answer.
    pub(in crate::editor) seed: Option<Answer>,
    /// The selection recipe the change takes if it is kept.
    pub(in crate::editor) recipe: Vec<SelectionStep>,
}

/// Runs a command `body`, which returns its result and whether it succeeded.
///
/// With a `change` the running code does not own ([`repeat_slot_owned`]),
/// installs a new recording and runs `body` as the change's command body,
/// its running [`RecordingScope`], so an input the body opens or a
/// repeatable command it dispatches belongs to this change. The change is
/// kept when `body` succeeded, or when anything it opened (an Insert
/// session, a pending interactive input) still holds it; otherwise the
/// recording it replaced is restored, so a refused body leaves nothing
/// behind. Without one, or with one the running code owns, `body` runs in
/// whatever scope is already running.
pub(in crate::editor) fn record_change<H: AsMut<EditorState>, R>(
    host: &mut H,
    change: Option<NewChange>,
    body: impl FnOnce(&mut H) -> (R, bool),
) -> R {
    let Some(change) = change.filter(|_| !repeat_slot_owned(host.as_mut())) else {
        return body(host).0;
    };
    let (prev, scope) = host.as_mut().begin_change(change);
    let (r, succeeded) = with_recording_scope(host, Some(scope), body);
    host.as_mut().end_change(prev, succeeded);
    r
}

/// Whether a repeatable command dispatched now belongs to something already
/// running rather than starting a change of its own, in which case
/// [`record_change`] starts none. Three owners:
///
/// - The code running in the [`ScopeSlot`]: a recorded change's own code
///   ([`RecordingScope`]), its command body, an Insert-key binding, or a
///   callback of one of its inputs, where a nested `call!` is part of that
///   change; or an Insert key's binding with no scope of its own, in a
///   session no change records, where a repeatable command the binding
///   dispatches is that session's input, not a change of its own; or a
///   command-line key's binding, since the prompt is not part of what `.`
///   repeats.
/// - An Insert session that feeds a recorded change, for its whole extent:
///   a hook or timer dispatching mid-session must not replace the change
///   the session's keys record into.
/// - `replay_dot`, which holds the change for its whole extent (`dot_replay`):
///   a fresh recording in between would only be discarded.
///
/// A change only waiting on an input (a pending read, an open menu, a queued
/// callback) owns nothing: a key that dismisses its menu starts its own.
///
/// Read *before* the body: `i`/`c`/`o` open the session inside their own
/// body, so a fresh change created before the body keeps ownership of the
/// inner entry (outer name wins).
fn repeat_slot_owned(state: &mut EditorState) -> bool {
    state.recording_scope.is_set()
        || state.session_recording().is_some()
        || state.dot_replay.is_some()
}

/// An interactive input's place in its change: the [`RecordingScope`] it
/// opened in and the answer's index among that slot's answers. Carried by
/// the input until it resolves, then by its queued callback, which keeps the
/// change open until the callback has run inside that same scope
/// ([`Editor::run_answered`]).
#[must_use]
#[derive(Debug)]
pub(in crate::editor) struct AnswerTicket {
    scope: RecordingScope,
    index: usize,
}

impl AnswerTicket {
    /// A second hold on the scope the input opened in, for code that runs
    /// as the input's answer beside the call the ticket goes to.
    pub(in crate::editor) fn reopen_scope(&self) -> RecordingScope {
        RecordingScope {
            source: SourceTicket {
                sources: Rc::clone(&self.scope.source.sources),
            },
            slot: self.scope.slot,
        }
    }
}

/// The change currently being recorded, and the last one settled.
///
/// A single `Recording` owns the slot. It is *accepting input* while a
/// [`SourceTicket`] opened on it is alive, until it is stopped: an input
/// opened by code outside the change's own ends it there
/// ([`EditorState::waiting_input`]).
/// A repeatable command dispatched inside a [`RecordingScope`] or the
/// change's Insert session is *inside* that change and never replaces it;
/// one dispatched anywhere else starts a fresh change, superseding one that
/// is only waiting on an input.
/// When the last source closes the recording stops accepting and its
/// `action` is what `.` replays.
#[derive(Debug)]
pub(in crate::editor) struct Recording {
    action: RepeatableAction,
    /// Shared with every live [`SourceTicket`]: the strong count beyond this
    /// one is the number of open sources.
    sources: Rc<()>,
    /// Set by [`EditorState::stop_recording`]: the change takes no further
    /// input, though sources opened on it are still alive.
    stopped: bool,
    /// The id the next recorded binding entry gets.
    next_binding: u64,
    /// What decides whether this change is kept.
    settling: Settling,
}

/// What decides whether a fresh change is kept, from
/// [`EditorState::begin_change`] on.
#[derive(Debug)]
enum Settling {
    /// Its command body, still running.
    Body,
    /// Work its body queued, which runs after the body returns
    /// ([`EditorState::defer_change`]).
    Deferred,
    /// That work, not ended yet. Holds the change this one replaced, put
    /// back if the work is withdrawn ([`EditorState::settle_deferred`]).
    Awaiting(Option<Box<Recording>>),
    /// Nothing left to decide.
    Settled,
}

/// A fresh change whose body handed the decision to keep it to work that
/// runs after the body returns. The work ends it with
/// [`EditorState::settle_deferred`].
#[must_use]
#[derive(Debug)]
pub(in crate::editor) struct DeferredChange {
    /// The change's [`Recording::sources`], held weakly so that it names the
    /// change without counting as one of its open sources.
    sources: Weak<()>,
}

/// How the work a [`DeferredChange`] waits on ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::editor) enum DeferredOutcome {
    /// It did what the change records: keep the change.
    Landed,
    /// It changed nothing: put back the change this one replaced.
    Withdrawn,
}

impl Recording {
    fn new(action: RepeatableAction) -> Self {
        Self {
            action,
            sources: Rc::new(()),
            stopped: false,
            next_binding: 0,
            settling: Settling::Body,
        }
    }

    /// Settle `change` if it is this change or one the chain of changes this
    /// one holds to put back reaches. `Some(replacement)` when this change
    /// itself was withdrawn: what its holder puts in its place.
    fn settle(
        &mut self,
        change: &DeferredChange,
        outcome: DeferredOutcome,
    ) -> Option<Option<Box<Recording>>> {
        if std::ptr::eq(Rc::as_ptr(&self.sources), change.sources.as_ptr()) {
            let Settling::Awaiting(prev) = std::mem::replace(&mut self.settling, Settling::Settled)
            else {
                unreachable!(
                    "a deferred change awaits its work from the end of its body until settled"
                )
            };
            return (outcome == DeferredOutcome::Withdrawn).then_some(prev);
        }
        let Settling::Awaiting(Some(prev)) = &mut self.settling else {
            return None;
        };
        if let Some(replacement) = prev.settle(change, outcome) {
            self.settling = Settling::Awaiting(replacement);
        }
        None
    }

    /// Forget every pick's payload, here and in the change this one holds
    /// to put back.
    fn forget_steel_payloads(&mut self) {
        let binding_answers = self
            .action
            .transcript
            .iter_mut()
            .filter_map(|input| match input {
                Input::Binding { answers, .. } => Some(answers),
                _ => None,
            });
        for answers in std::iter::once(&mut self.action.answers).chain(binding_answers) {
            for answer in answers {
                if let Answer::Pick(Pick {
                    picked: Some(picked @ Picked::Payload(_)),
                    ..
                }) = answer
                {
                    *picked = Picked::PayloadForgotten;
                }
            }
        }
        if let Settling::Awaiting(Some(prev)) = &mut self.settling {
            prev.forget_steel_payloads();
        }
    }

    /// `true` while an input source is still outstanding and the change was
    /// not stopped.
    fn accepting(&self) -> bool {
        !self.stopped && Rc::strong_count(&self.sources) > 1
    }

    fn open_ticket(&self) -> SourceTicket {
        SourceTicket {
            sources: Rc::clone(&self.sources),
        }
    }

    fn opened(&self, ticket: &SourceTicket) -> bool {
        Rc::ptr_eq(&self.sources, &ticket.sources)
    }

    /// Whether `ticket` was opened on this recording and it still takes
    /// input.
    fn feeds(&self, ticket: &SourceTicket) -> bool {
        self.opened(ticket) && self.accepting()
    }

    /// The transcript index of binding entry `id`, `None` once it was
    /// removed.
    fn binding_index(&self, id: BindingId) -> Option<usize> {
        self.action
            .transcript
            .iter()
            .rposition(|input| matches!(input, Input::Binding { id: entry, .. } if *entry == id))
    }

    /// The answers `slot` names. `None` for a binding entry that was
    /// removed (a completion accept replaced it).
    fn answers_mut(&mut self, slot: AnswerSlot) -> Option<&mut Vec<Answer>> {
        match slot {
            AnswerSlot::Command => Some(&mut self.action.answers),
            AnswerSlot::Binding(id) => {
                let i = self.binding_index(id)?;
                match &mut self.action.transcript[i] {
                    Input::Binding { answers, .. } => Some(answers),
                    _ => None,
                }
            }
            AnswerSlot::UnrecordedBinding(_) => None,
        }
    }

    /// The count the recorded change ran with, the default `.` replays with.
    pub(in crate::editor) fn count(&self) -> Count {
        self.action.count
    }

    /// The settled change, for test inspection.
    #[cfg(test)]
    pub(in crate::editor) fn action(&self) -> &RepeatableAction {
        &self.action
    }
}

/// `replay_dot`'s state while `.` runs: the answers for the command or
/// binding being re-run, and the callbacks its interactive inputs queued
/// (run after its body, as live). An interactive input reached during `.`
/// never waits for the user: it takes the next answer, or refuses when that
/// is not one of its kind.
pub(in crate::editor) struct DotReplay {
    answers: VecDeque<Answer>,
    pending: VecDeque<PendingAnswer>,
    /// Every pipe run the replay starts has an id at least this.
    runs_from: RunId,
    /// `Ctrl-c` stopped a pipe run the replay started: nothing after it runs.
    stopped: bool,
}

impl EditorState {
    /// Stop every replay in progress after a pipe run it waited for was
    /// cancelled: the macro's queued keys are dropped, and a `.` runs
    /// nothing after the input that started the run.
    pub(in crate::editor) fn stop_replays(&mut self) {
        self.macros.abort_replay();
        if let Some(replay) = self.dot_replay.as_mut() {
            replay.stopped = true;
        }
    }

    /// Whether [`Self::stop_replays`] stopped the `.` in progress.
    fn dot_replay_stopped(&self) -> bool {
        self.dot_replay
            .as_ref()
            .is_some_and(|replay| replay.stopped)
    }
}

/// What a re-run interactive input finds as its answer.
pub(in crate::editor) enum Replayed<T> {
    Answered(T),
    /// The input never resolved live ([`Answer::Unanswered`]): it opens
    /// nothing and runs nothing.
    Unanswered,
}

/// A replayed answer waiting to run after the re-run body, as live.
pub(in crate::editor) enum PendingAnswer {
    Read(CharRead, Option<char>),
    /// A command's text prompt: the command and the text it was confirmed
    /// with, `None` when it was cancelled.
    Text(commands::PendingDispatch, Option<String>),
    /// What a picker, prompt or menu answer runs, in order: its callback
    /// called with what the answer gives; for a picker, the binding that
    /// closed it and then its `#:on-close`.
    Calls(Vec<hume_scripting::SteelCall>),
}

impl DotReplay {
    /// Take the next answer if `kind` accepts it, or if it is
    /// [`Answer::Unanswered`]. `None` when the next answer is of another kind
    /// or none is left: the re-run took a different branch, and the input
    /// refuses.
    fn next<T>(&mut self, kind: impl FnOnce(&Answer) -> Option<T>) -> Option<Replayed<T>> {
        let taken = match self.answers.front()? {
            Answer::Unanswered => Replayed::Unanswered,
            answer => Replayed::Answered(kind(answer)?),
        };
        self.answers.pop_front();
        Some(taken)
    }

    /// Answers a re-run interactive input (a read, a picker, a prompt, a
    /// menu) from the recording instead of opening it: `kind` takes its
    /// answer out of the next recorded one, and `then` turns it into what to
    /// run after the re-run body, or the refusal naming why it has no fit (a
    /// missing item, an unbound action, a forgotten payload). Refuses with
    /// [`ReplayRefusal::Diverged`] when the next answer is of another kind,
    /// and queues nothing for an input that never resolved live.
    pub(in crate::editor) fn answer_call<T>(
        &mut self,
        kind: impl FnOnce(&Answer) -> Option<T>,
        then: impl FnOnce(T) -> Result<PendingAnswer, ReplayRefusal>,
    ) -> Result<(), ReplayRefusal> {
        if let Replayed::Answered(answer) = self.next(kind).ok_or(ReplayRefusal::Diverged)? {
            self.pending.push_back(then(answer)?);
        }
        Ok(())
    }
}

/// Proof that a completion input passed [`EditorState::interactive_input`],
/// which is the only way to get one.
#[must_use]
pub(in crate::editor) struct Interactive(());

/// Proof that an input resolving on a later key passed
/// [`EditorState::waiting_input`], which is the only way to get one. Every
/// such input's layer holds it while open, so none can open without the
/// check. Carries the [`AnswerTicket`] the input's answer records under, if
/// it has one.
#[must_use]
pub(in crate::editor) struct WaitingInput(Option<AnswerTicket>);

impl WaitingInput {
    /// Takes the ticket the input's answer records under, if it has one.
    pub(in crate::editor) fn take_answer(&mut self) -> Option<AnswerTicket> {
        self.0.take()
    }

    /// An input opened directly, for a test of the layer itself.
    #[cfg(test)]
    pub(in crate::editor) fn for_test() -> Self {
        Self(None)
    }
}

/// What opens an interactive input, for [`EditorState::waiting_input`] to
/// tell whether it belongs to a recorded change.
#[derive(Clone, Copy)]
pub(in crate::editor) enum InputOpener {
    /// The code running now: the running [`RecordingScope`] decides.
    Code,
    /// A repeatable native command's operand read. Outside every scope this
    /// is a key collecting the operand before its command runs, which seeds
    /// that command's own change; inside one, the scope decides, as for
    /// `Code`.
    RepeatableOperand,
}

// ── Deferred dot-repeat ───────────────────────────────────────────────────────

/// Deferred dot-repeat job, set by `cmd_repeat` and consumed by `replay_dot`
/// at the end of the enclosing `handle_key` call.
///
/// Splitting enqueue (pure State handler) from drain (`&mut Editor` plumbing)
/// lets `cmd_repeat` keep the `FocusedCmdFn` shape (no `&mut Editor`) while
/// still reaching `replay_dot`. Only a key dispatches `cmd_repeat`
/// (`CmdMeta::key_only`), and no other command runs between it and the drain,
/// so the replay runs where the key put it, once per key.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PendingRepeat {
    /// Effective replay count, with the explicit-count override already
    /// applied.
    pub(super) count: Count,
}

// ── Recording lifecycle (EditorState) ─────────────────────────────────────────

impl EditorState {
    /// Forget every pick's payload: payloads belong to the Steel engine a
    /// reload is replacing. A pick replayed by its choice is unaffected; one
    /// replayed by its payload becomes [`Picked::PayloadForgotten`].
    pub(in crate::editor) fn forget_steel_payloads(&mut self) {
        if let Some(rec) = self.recording.as_mut() {
            rec.forget_steel_payloads();
        }
    }

    /// The settled change `.` would replay, for test inspection.
    #[cfg(test)]
    pub(in crate::editor) fn last_action(&self) -> Option<&RepeatableAction> {
        self.recording.as_ref().map(Recording::action)
    }

    /// Start recording a fresh change for [`record_change`]: install a new
    /// recording, seeding the operand the command was dispatched with as its
    /// first answer. Returns the recording it replaced, which
    /// [`Self::end_change`] restores if the change is discarded, and the
    /// scope its command body runs in.
    fn begin_change(&mut self, change: NewChange) -> (Option<Recording>, RecordingScope) {
        let action = RepeatableAction {
            command: change.command,
            count: change.count,
            selection_recipe: change.recipe,
            answers: change.seed.into_iter().collect(),
            transcript: Vec::new(),
        };
        let rec = Recording::new(action);
        let body = RecordingScope {
            source: rec.open_ticket(),
            slot: AnswerSlot::Command,
        };
        (self.recording.replace(rec), body)
    }

    /// Settle the change [`Self::begin_change`] installed, once its body
    /// scope has closed. The change is kept when its body `succeeded`, or
    /// when anything it opened (an Insert session, a pending interactive
    /// input) still holds it: then it is a real change regardless of how the
    /// body ended. A discarded one restores `prev`, so a refused body leaves
    /// nothing behind. A kept change whose body deferred the decision holds
    /// `prev` until its work settles it.
    fn end_change(&mut self, prev: Option<Recording>, succeeded: bool) {
        let Some(rec) = self
            .recording
            .as_mut()
            .filter(|rec| succeeded || rec.accepting())
        else {
            self.recording = prev;
            return;
        };
        rec.settling = match std::mem::replace(&mut rec.settling, Settling::Settled) {
            Settling::Deferred => Settling::Awaiting(prev.map(Box::new)),
            _ => Settling::Settled,
        };
    }

    /// Hand the decision to keep the change whose command body runs now to
    /// work the body queues, which runs after the body returns: until that
    /// work calls [`Self::settle_deferred`], the change is kept and holds the
    /// one it replaced. `None` when no fresh change's body runs now (during
    /// `.`, or for a command an Insert session's change owns): the work then
    /// has nothing to settle.
    pub(in crate::editor) fn defer_change(&mut self) -> Option<DeferredChange> {
        let scope = self.recording_scope.running()?;
        if !matches!(scope.slot, AnswerSlot::Command) {
            return None;
        }
        let rec = self
            .recording
            .as_mut()
            .filter(|rec| rec.opened(&scope.source) && matches!(rec.settling, Settling::Body))?;
        rec.settling = Settling::Deferred;
        Some(DeferredChange {
            sources: Rc::downgrade(&rec.sources),
        })
    }

    /// End `change` as its work's `outcome` says: kept when it landed, and
    /// replaced by the change it superseded when it was withdrawn, whether
    /// it is the latest change or one a newer deferred change holds to put
    /// back. Does nothing once a settled change has replaced it.
    pub(in crate::editor) fn settle_deferred(
        &mut self,
        change: DeferredChange,
        outcome: DeferredOutcome,
    ) {
        if let Some(rec) = self.recording.as_mut()
            && let Some(replacement) = rec.settle(&change, outcome)
        {
            self.recording = replacement.map(|prev| *prev);
        }
    }

    /// The recording the open Insert session holds a source on, while it
    /// accepts input: the one change every Insert-session input records
    /// into, whatever else is accepting.
    fn session_recording(&mut self) -> Option<&mut Recording> {
        session_feed(&mut self.recording, self.input.insert_session()?)
    }

    /// Append `input` to the open Insert session's recording, if any. Called
    /// at the seam each input arrives (a typed Insert key, a completion
    /// accept; a paste goes through [`Self::record_paste`]).
    pub(in crate::editor) fn record_input(&mut self, input: Input) {
        if let Some(rec) = self.session_recording() {
            rec.action.transcript.push(input);
        }
    }

    /// Append a paste of `values` to the open Insert session's recording, if
    /// any, unless the code running now belongs to that same change: `.`
    /// re-runs that code (a binding, a command, an answered callback), which
    /// pastes again, so recording the paste too would replay it twice.
    pub(in crate::editor) fn record_paste(&mut self, values: Vec<String>) {
        let Some(session) = self.input.insert_session() else {
            return;
        };
        let Some(rec) = session_feed(&mut self.recording, session) else {
            return;
        };
        if self
            .recording_scope
            .running()
            .is_some_and(|scope| rec.opened(&scope.source))
        {
            return;
        }
        rec.action.transcript.push(Input::Paste(values));
    }

    /// Record the Insert-key binding `name` about to dispatch, before its
    /// body runs, so the answers its interactive inputs get have an entry to
    /// go to. Returns the entry's id and the [`RecordingScope`] the binding
    /// runs in, `None` when the open Insert session records no change.
    pub(in crate::editor) fn record_binding(
        &mut self,
        name: Rc<str>,
        repeatable: bool,
    ) -> Option<(BindingId, RecordingScope)> {
        let rec = self.session_recording()?;
        let id = BindingId(rec.next_binding);
        rec.next_binding += 1;
        rec.action.transcript.push(Input::Binding {
            id,
            name,
            answers: Vec::new(),
        });
        let scope = RecordingScope {
            source: rec.open_ticket(),
            slot: if repeatable {
                AnswerSlot::Binding(id)
            } else {
                AnswerSlot::UnrecordedBinding(id)
            },
        };
        Some((id, scope))
    }

    /// Open a source on the recording of the running [`RecordingScope`], for
    /// an Insert session that scope opens, so the change keeps accepting the
    /// session's keys until the returned ticket drops. `None` outside every
    /// scope, or when a newer change superseded the scope's.
    pub(in crate::editor) fn open_scope_source(&mut self) -> Option<SourceTicket> {
        let scope = self.recording_scope.running()?;
        self.recording
            .as_ref()
            .filter(|rec| rec.feeds(&scope.source))
            .map(Recording::open_ticket)
    }

    /// The recording `ticket` was opened on, if it is still current.
    fn current_recording(&mut self, ticket: &SourceTicket) -> Option<&mut Recording> {
        self.recording.as_mut().filter(|rec| rec.opened(ticket))
    }

    /// Record `answer` in the place `ticket` took, if that recording is
    /// still current. A binding entry a completion accept replaced takes no
    /// answer.
    pub(in crate::editor) fn record_answer(&mut self, ticket: &AnswerTicket, answer: Answer) {
        if let Some(place) = self
            .current_recording(&ticket.scope.source)
            .and_then(|rec| rec.answers_mut(ticket.scope.slot))
            .and_then(|answers| answers.get_mut(ticket.index))
        {
            *place = answer;
        }
    }

    /// The callback call an interactive input queues once it resolves:
    /// records `answer` in the place `ticket` took, if any, and hands the
    /// ticket to the call, which then runs in the input's scope. Every
    /// picker, prompt, menu and `on-next-char!` callback is built here.
    pub(in crate::editor) fn answered_call(
        &mut self,
        ticket: Option<AnswerTicket>,
        answer: Answer,
        call: hume_scripting::SteelCall,
    ) -> super::event::PendingWork {
        if let Some(ticket) = &ticket {
            self.record_answer(ticket, answer);
        }
        super::event::PendingWork::answer(call, ticket)
    }

    /// [`Self::answered_call`], queued: how a picker, prompt or menu fires
    /// its callback once it closes.
    pub(in crate::editor) fn queue_answered_call(
        &mut self,
        ticket: Option<AnswerTicket>,
        answer: Answer,
        call: hume_scripting::SteelCall,
    ) {
        let work = self.answered_call(ticket, answer, call);
        self.push_work(work);
    }

    /// The check a completion trigger or accept passes before it opens.
    ///
    /// Refuses during `.`: a completion accept is replayed from the net edit
    /// recorded for it ([`Input::Result`]), so reaching one during replay
    /// means the replayed code decided differently than it did live, and an
    /// error beats opening a widget mid-repeat. Otherwise marks the capture
    /// armed for the Insert-key binding running now, if any, interactive, so
    /// its net edit replaces its entry.
    pub(in crate::editor) fn interactive_input(&mut self) -> Result<Interactive, ReplayRefusal> {
        if self.dot_replay.is_some() {
            return Err(ReplayRefusal::Diverged);
        }
        if let Some(cap) = self
            .input
            .insert_session_mut()
            .and_then(InsertSession::dot_capture_mut)
        {
            cap.interactive = true;
        }
        Ok(Interactive(()))
    }

    /// The check an interactive input that resolves on a later key (a read,
    /// a picker, a prompt, a menu, a command's text prompt, the `:` line)
    /// passes before it opens, minting the [`WaitingInput`] its layer holds.
    /// During `.` a read, picker, prompt, menu or text prompt takes its
    /// recorded answer instead, and the `:` line is never answered, so
    /// reaching this during replay refuses.
    ///
    /// Opens the [`AnswerTicket`] its answer records under, in the running
    /// [`RecordingScope`]'s slot, when that scope is a repeatable command's
    /// code. An input any other code opens (a binding whose command is not
    /// repeatable, a hook, a timer, a key) records no
    /// answer, and ends the change in flight, if any
    /// ([`Self::stop_recording`]): `.` could not answer it. The exception
    /// is a key collecting a repeatable native command's operand
    /// ([`InputOpener::RepeatableOperand`]), which seeds that command's own
    /// change.
    pub(in crate::editor) fn waiting_input(
        &mut self,
        opener: InputOpener,
    ) -> Result<WaitingInput, ReplayRefusal> {
        if self.dot_replay.is_some() {
            return Err(ReplayRefusal::Diverged);
        }
        let Some(scope) = self.recording_scope.running() else {
            if !matches!(opener, InputOpener::RepeatableOperand) {
                self.stop_recording(None);
            }
            return Ok(WaitingInput(None));
        };
        let slot = scope.slot;
        if let AnswerSlot::UnrecordedBinding(binding) = slot {
            self.stop_recording(Some(binding));
            return Ok(WaitingInput(None));
        }
        let ticket = self
            .recording
            .as_mut()
            .filter(|rec| rec.feeds(&scope.source))
            .and_then(|rec| {
                let source = rec.open_ticket();
                let answers = rec.answers_mut(slot)?;
                answers.push(Answer::Unanswered);
                Some(AnswerTicket {
                    scope: RecordingScope { source, slot },
                    index: answers.len() - 1,
                })
            });
        Ok(WaitingInput(ticket))
    }

    /// End the change in flight, if any, at an input it cannot answer on
    /// `.`: it keeps what it recorded before `binding`, the non-repeatable
    /// Insert-key binding that opened the input, if any, and takes no
    /// further input. The binding's entry and anything after it are dropped,
    /// so `.` does not re-run it.
    fn stop_recording(&mut self, binding: Option<BindingId>) {
        let Some(rec) = self.recording.as_mut().filter(|rec| rec.accepting()) else {
            return;
        };
        if let Some(i) = binding.and_then(|id| rec.binding_index(id)) {
            rec.action.transcript.truncate(i);
        }
        rec.stopped = true;
    }

    /// Applies a recorded completion accept `r` at `fp`'s cursors, as one
    /// edit composed into the open Insert group: the replay-side counterpart
    /// of `BufferSession::accept`'s own cursor edit. Dismisses any completion
    /// session afterwards.
    ///
    /// `Err` without an open Insert session on `(pid, bid)`, or with a real
    /// (non-collapsed) selection there, the same guard the live accept
    /// enforces: `replace_around_cursors` force-collapses any selection it
    /// touches, discarding it, which this guard refuses.
    pub(in crate::editor) fn apply_cursor_replacement(
        &mut self,
        view: &EngineView,
        fp: commands::FocusedPane,
        r: &CursorReplacement,
    ) -> Result<(), String> {
        let (pid, bid) = (fp.pid(), fp.bid(view));
        if !self.input.inserting_into(pid, bid) {
            return Err("no Insert session to apply the recorded result to".to_string());
        }
        if !self
            .panes
            .entries
            .shown(&view.panes, pid)
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

    /// End the closing Insert `session`'s part in the recording: finalize the
    /// capture still armed on it, if any, then close the session's source.
    /// In that order: a finalized capture records into the recording only
    /// while it still accepts input.
    ///
    /// The capture is still armed when its binding tears the session down
    /// from inside its own dispatch (`completion-accept!` then `exit-insert`
    /// in one body), before `with_dot_capture` takes it back.
    pub(in crate::editor) fn end_insert_recording(&mut self, session: &mut InsertSession) {
        if let Some(cap) = session.take_dot_capture()
            && let Err(warning) = finalize_dot_capture(&mut self.recording, session, cap)
        {
            self.report(hume_core::Severity::Warning, warning);
        }
        drop(session.release_recording());
    }
}

// ── Replay (Editor) ────────────────────────────────────────────────────────────

impl Editor {
    /// Runs `f`, what an interactive input does once it has its answer (its
    /// callback, or the native command it read an operand for), inside the
    /// scope the input opened in, so what `f` does belongs to the same
    /// change. Outside every scope with no `ticket`.
    pub(in crate::editor) fn run_answered<R>(
        &mut self,
        ticket: Option<AnswerTicket>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        with_recording_scope(self, ticket.map(|t| t.scope), f)
    }

    /// Close out whatever `replay_dot` left open on the focused (pane,
    /// buffer), for either its success or its failure path.
    ///
    /// A body that entered real Insert mode closes through
    /// `end_insert_session` (pops the `InsertLayer`, which `tear_down_insert`
    /// needs present). A body that called native paste retargeted the
    /// pre-opened placeholder to `Paste`, which must stay open for a following
    /// `[`/`]`, so it's left alone. Anything else is still `Replay`-kind and
    /// commits directly through `edit_session::commit_open_session`.
    ///
    /// No-op if nothing is open at all: the body's own dispatch may already
    /// have closed the pane or buffer; `focus::end_focus_sessions` already
    /// committed a still-live session before that happened.
    fn finish_replay_session(&mut self) {
        if self.state.open_session().is_none() {
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
            super::edit_session::commit_open_session(
                &mut self.state.buffers,
                &self.state.panes.entries,
                &mut self.state.active_session,
            );
        }
    }

    /// Replay a dot-repeat change.
    ///
    /// Takes the recording out for the whole replay and restores it afterward
    /// so `.` chains, which is also what keeps the replay from recording into
    /// it. Sets [`EditorState::dot_replay`] for the same extent.
    pub(in crate::editor) fn replay_dot(&mut self, count: Count) {
        let Some(recording) = self.state.recording.take() else {
            return;
        };
        let replay = DotReplay {
            answers: recording.action.answers.iter().cloned().collect(),
            pending: VecDeque::new(),
            runs_from: self.state.pipe_runs.mark(),
            stopped: false,
        };
        let prev = self.state.dot_replay.replace(replay);
        self.replay_action(&recording.action, count);
        self.state.dot_replay = prev;
        self.state.recording = Some(recording);
    }

    /// `replay_dot`'s body: replays the selection recipe, re-runs the command
    /// (its interactive inputs answered from its recorded answers), then walks
    /// the transcript, feeding each input into the path it names (see
    /// [`Input`]). The command and each [`Input::Binding`] run through
    /// [`Editor::dispatch`], the funnel their live keypress went through.
    fn replay_action(&mut self, action: &RepeatableAction, count: Count) {
        // Read once, at entry: recipe steps never move focus.
        let fp = commands::FocusedPane::current(&self.view);
        let pid = fp.pid();
        let bid = fp.bid(&self.view);

        // Resolve the command before opening the edit group: a missing command
        // must return while there is still no cleanup obligation.
        let Some(edit_cmd) = self.resolve_mappable(action.command.as_ref()) else {
            return;
        };

        let meta = edit_cmd.meta();

        // Commit (or defer) any paste session left open by the keypress that
        // set up `.` itself, using the REPLAYED command's own meta rather than
        // `repeat-last-action`'s (which always defers). A session can only
        // still be open here if the replayed command is itself a ring-cycle
        // command. This reproduces `p [ .` as one more ring-cycle step.
        commands::step_paste_commit(&mut self.state, meta.defers_paste_commit);

        // Pre-open a Replay-kind placeholder, folding a recipe replay + the
        // command into one undo revision. Skipped for a `manages_own_session`
        // command (the paste family), which opens its own session; a Steel
        // body that itself calls native paste retargets this placeholder to
        // `Paste` in place.
        //
        // `Err` here means a real session is already open, not expected given
        // `step_paste_commit` just ran; defer rather than steal it.
        if !meta.manages_own_session {
            let sels = self
                .state
                .panes
                .entries
                .shown(&self.view.panes, pid)
                .selections()
                .clone();
            let buffers = &self.state.buffers;
            let opened = edit_session::open_or_retarget(
                self.state.input.insert_session(),
                &mut self.state.active_session,
                pid,
                bid,
                EditSessionKind::Replay,
                || buffers.get(bid).begin_edit_group(sels.clone(), sels),
            );
            if opened.is_err() {
                return;
            }
        }

        // Rebuild the selection extent the command acted on. Every
        // recipe step names a native command taking no operand (a `Composes`/
        // `Establishes` selection command): `SteelBacked`/`Lazy` are always
        // `Untracked`, native entries can never be unregistered, so the lookup
        // and bind always succeed. A refused step cancels the replay before
        // the edit runs, with the selections put back as they were.
        let pre_recipe = self
            .state
            .panes
            .entries
            .shown(&self.view.panes, pid)
            .selections()
            .clone();
        for step in &action.selection_recipe {
            let Some(MappableCommand::Native(cmd)) = self
                .state
                .config
                .registry
                .get_mappable(step.command.as_ref())
                .cloned()
            else {
                unreachable!("a dot-repeat selection-recipe step always names a native command");
            };
            let Ok(bound) = commands::BoundCommand::focused(cmd, fp, None) else {
                unreachable!(
                    "a dot-repeat selection-recipe step always names a native command taking no character"
                )
            };
            let ran = commands::run_body(
                &mut self.state,
                &mut self.view,
                bound,
                &CmdCtx {
                    count: step.count,
                    extend: step.extend,
                },
            );
            if !ran {
                self.state
                    .panes
                    .entries
                    .seeded_mut(pid, bid)
                    .set_selections(pre_recipe, self.state.buffers.get(bid).text());
                self.finish_replay_session();
                return;
            }
        }

        let dispatched = self.dispatch(
            edit_cmd,
            CmdCtx {
                count,
                extend: false,
            },
        );
        let ran = self.finish_replayed_answers(dispatched);
        if !ran {
            // Close whatever the group opened above became so it can't leak,
            // the same decision the success path makes. commit drops an empty
            // group (clean noop) and records a partial one.
            self.finish_replay_session();
            return;
        }

        // Walk the transcript. A failed or unregistered `Binding`, or a
        // `Result` the new text rejects, is reported and skipped rather than
        // stopping the loop: the live session kept going past it too. A
        // `Binding` that leaves Insert stops the walk: nothing recorded after
        // it belongs to a session that's gone. So does one whose pipe run was
        // cancelled. `break`, not `return`, so `finish_replay_session` below
        // still runs.
        for input in &action.transcript {
            match input {
                Input::Key(key) => {
                    let entry = self
                        .state
                        .input
                        .insert_entry()
                        .expect("a replayed Insert key runs in the replayed Insert session");
                    commands::insert_default_key(&mut self.state, &mut self.view, &entry, fp, *key);
                }
                Input::Paste(values) => {
                    if let Err(e) = self.state.apply_insert_mode_paste(&self.view, values) {
                        self.state.report_error(&e);
                    }
                }
                Input::Result(r) => {
                    if let Err(msg) = self.state.apply_cursor_replacement(&self.view, fp, r) {
                        self.report(hume_core::Severity::Error, msg);
                    }
                }
                Input::Binding { name, answers, .. } => {
                    // A command name loses its binding only if it was removed
                    // between the original keypress and this replay
                    // (`unbind-key!`, a plugin reload), so a miss reports
                    // rather than panicking.
                    let Some(cmd) = self.resolve_mappable(name.as_ref()) else {
                        continue;
                    };
                    if let Some(replay) = self.state.dot_replay.as_mut() {
                        replay.answers = answers.iter().cloned().collect();
                    }
                    // The live keypress dispatched as an Insert-key binding;
                    // replay recreates the same context. An interactive input
                    // the body reaches with no answer for it (it decided
                    // differently than it did live) refuses loudly instead.
                    let ran = dispatch_insert_key(self, None, cmd);
                    self.finish_replayed_answers(ran);
                    if self.state.mode() != Mode::Insert || self.state.dot_replay_stopped() {
                        break;
                    }
                }
            }
        }
        self.finish_replay_session();
    }

    /// After a re-run command or binding body: when it `ran`, resolve each
    /// interactive input it reached with the answer it took, in order, running
    /// each callback at once (an input a callback reaches is resolved by this
    /// same loop). Either way, drop the answers it did not reach (it took a
    /// shorter branch this time) so the next body starts from its own, and
    /// wait for a pipe run it started so the rest of the replay sees its
    /// output.
    ///
    /// Returns whether the body ran: `ran`, and the native command a read
    /// collected the operand for, if any, ran too, and no pipe run it
    /// started was cancelled. A failed one drops the answers after it.
    fn finish_replayed_answers(&mut self, mut ran: bool) -> bool {
        while ran
            && let Some(answer) = self
                .state
                .dot_replay
                .as_mut()
                .and_then(|replay| replay.pending.pop_front())
        {
            let calls = match answer {
                PendingAnswer::Read(read, answer) => {
                    let resolved = super::input_stack::char::resolve(&mut self.state, read, answer);
                    match super::input_stack::char::run_resolved(self, resolved) {
                        AfterRead::Done(done) => {
                            ran = done;
                            Vec::new()
                        }
                        AfterRead::Call(call) => vec![call],
                    }
                }
                PendingAnswer::Text(then, text) => {
                    ran = commands::finish_text_operand(self, then, None, text);
                    Vec::new()
                }
                PendingAnswer::Calls(calls) => calls
                    .into_iter()
                    .map(|call| super::event::PendingWork::answer(call, None))
                    .collect(),
            };
            // One batch each, as live: a raise in a picker's binding still
            // runs its `#:on-close`.
            for call in calls {
                self.run_pending_batch(VecDeque::from([call]));
            }
        }
        let replay = self
            .state
            .dot_replay
            .as_mut()
            .expect("a re-run body runs inside replay_dot");
        replay.answers.clear();
        replay.pending.clear();
        let runs_from = replay.runs_from;
        self.wait_pipe_runs_from(runs_from);
        ran && !self.state.dot_replay_stopped()
    }
}

/// The recording `session` holds a source on, while it accepts input.
fn session_feed<'a>(
    recording: &'a mut Option<Recording>,
    session: &InsertSession,
) -> Option<&'a mut Recording> {
    let ticket = session.recording()?;
    recording.as_mut().filter(|rec| rec.feeds(ticket))
}

/// Why `.` could not answer an input a re-run change opened. It names no
/// input: whoever opened one adds that, and a builtin's error gets its name
/// from the builtin.
#[derive(Debug)]
pub(in crate::editor) enum ReplayRefusal {
    /// The re-run code opened an input the recording has no answer for.
    Diverged,
    /// The re-run list has no item the recorded choice names.
    MissingItem(Choice),
    /// The recorded pick's key is not bound in the re-run picker's keymap.
    UnboundKey,
    /// A config reload forgot the value the recorded pick carried.
    ForgottenPayload,
}

impl std::fmt::Display for ReplayRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Diverged => f.write_str(
                "reached during `.`, but the recorded input took a different branch this time",
            ),
            Self::MissingItem(Choice { label, nth: 0 }) => {
                write!(f, "`.` found no item \"{label}\" to pick")
            }
            Self::MissingItem(Choice { label, nth }) => write!(
                f,
                "`.` found no item \"{label}\" number {} among those with that text",
                nth + 1
            ),
            Self::UnboundKey => f.write_str(
                "`.` can't repeat this pick: its key is not bound in this picker's keymap",
            ),
            Self::ForgottenPayload => {
                f.write_str("`.` can't repeat this pick: a config reload forgot the picked value")
            }
        }
    }
}
