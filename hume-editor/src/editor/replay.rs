//! Dot-repeat (`.`) and macro-replay state and execution.
//!
//! Dot-repeat records a [`RepeatableAction`]: the repeatable command that
//! began a change, its count, the selection recipe that built the extent it
//! acted on, the [answers](Answer) its interactive inputs got, and an ordered
//! [transcript](Input) of every Insert-session input the change consumed. It
//! records a recipe rather than a raw changeset, since changesets are
//! position-dependent and can't be replayed at a different cursor. Macro
//! replay drains a queue of recorded keys through the normal event path.
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

use hume_engine::pipeline::EngineView;
use hume_scripting::host::CommandArg;
use std::borrow::Cow;
use std::collections::VecDeque;
use std::rc::Rc;
use steel::rvals::SteelVal;
use termina::event::{Event as TerminalEvent, KeyEvent};

use super::dispatch::CmdCtx;
use super::dot_chain::{ChainBroken, DotChain};
use super::edit_session::{self, DotCapture, EditSession, EditSessionKind, InsertState};
use super::input_stack::char::CharRead;
use super::keymap::CanonicalKey;
use super::registry::MappableCommand;
use super::{Editor, EditorState, Mode, commands, doc_ops};

// ── The recorded change ───────────────────────────────────────────────────────

/// The user's choice at one interactive input: a character read, a picker,
/// a prompt or a menu. Its place is taken when the input opens, so answers
/// stay in the order their inputs opened, which is the order a re-run opens
/// them in; it is filled when the input resolves. Handed back to the same
/// input when `.` re-runs the command or binding that opened it, which then
/// never waits for the user. What the command does with the
/// choice is decided again at the new cursor.
#[derive(Debug, Clone)]
pub(crate) enum Answer {
    /// A character: a native operand (`r`/`f`/`t`/`surround-add`) or an
    /// `on-next-char!` answer. `None` when the read ended without one.
    Char(Option<char>),
    /// A picker pick or dismissal.
    Pick(Pick),
    /// A prompt's confirmed text, `None` when it was cancelled.
    Prompt(Option<String>),
    /// A menu's chosen item, `None` when it was dismissed.
    Menu(Option<Choice>),
    /// An input that opened but never resolved: one a reload or a cascade
    /// cap dropped, or a menu `close-menu!` closed. Its callback never ran,
    /// so on replay the input opens nothing and runs nothing.
    Unanswered,
}

/// A picker's answer. `action` is the `#:actions` key that closed the
/// picker, `None` for `on-select`; `picked` is `None` for a dismissal (the
/// callback got `#f`).
#[derive(Debug, Clone)]
pub(crate) struct Pick {
    pub(in crate::editor) action: Option<CanonicalKey>,
    pub(in crate::editor) picked: Option<Picked>,
}

/// How `.` hands a picked item back to the re-run picker, decided when the
/// pick is made.
#[derive(Debug, Clone)]
pub(crate) enum Picked {
    /// By its text, when every item came from the list `picker!` opened
    /// with: the re-run builds that list again, so the payload is the one
    /// built for the new spot.
    Choice(Choice),
    /// By the payload the callback got, when items arrived after the picker
    /// opened (`#:pending`, `picker-push!`, a spawned source, a
    /// `live-picker!`): the re-run picker holds none of them yet when `.`
    /// answers it.
    Payload(SteelVal),
    /// A `Payload` a reload forgot: it belonged to the Steel engine the
    /// reload replaced. `.` refuses it.
    PayloadForgotten,
}

/// A picked menu or picker item, named by its text: the `nth` (from 0, in
/// list order) of the items shown as `label`. Replayed against the list the
/// re-run builds, so the pick follows the item's text rather than its
/// position, and of several items with the same text it is the same one.
#[derive(Debug, Clone)]
pub(crate) struct Choice {
    pub(crate) label: String,
    pub(crate) nth: usize,
}

impl Choice {
    /// The choice naming `items[index]`, each item shown as `text` gives it.
    pub(in crate::editor) fn of<T>(items: &[T], index: usize, text: impl Fn(&T) -> &str) -> Self {
        let label = text(&items[index]);
        Self {
            label: label.to_owned(),
            nth: items[..index]
                .iter()
                .filter(|item| text(item) == label)
                .count(),
        }
    }

    /// The index of the item this choice names in `items`, `None` when
    /// fewer than `nth + 1` items are shown as `label`.
    pub(in crate::editor) fn find<T>(
        &self,
        items: &[T],
        text: impl Fn(&T) -> &str,
    ) -> Option<usize> {
        items
            .iter()
            .enumerate()
            .filter(|(_, item)| text(item) == self.label)
            .nth(self.nth)
            .map(|(i, _)| i)
    }

    /// The refusal `what` raises during `.` when the re-run list has no item
    /// this choice names.
    pub(in crate::editor) fn missing(&self, what: &str) -> String {
        let label = &self.label;
        match self.nth {
            0 => format!("{what}: `.` found no item \"{label}\" to pick"),
            nth => format!(
                "{what}: `.` found no item \"{label}\" number {} among those with that text",
                nth + 1
            ),
        }
    }
}

/// One Insert-session input a change consumed, in arrival order.
///
/// `.` replays a change by re-running its command and then re-feeding each
/// transcript entry into the funnel live input goes through, never by
/// inferring, after a dispatch, what it turned out to do. The one exception
/// is a completion accept ([`Input::Result`]), recorded as its own net
/// cursor edit, since it hands no choice to a callback that could take it
/// again.
#[derive(Debug, Clone)]
pub(crate) enum Input {
    /// A key with no Insert keymap binding: one run of its *default*
    /// behaviour (`commands::insert_default_key`), never a keymap walk.
    Key(KeyEvent),
    /// A terminal paste, replayed as one bulk insert rather than synthetic
    /// per-char keys: a synthesized `Enter` would auto-indent, which a real
    /// paste never does.
    Paste(String),
    /// A key bound in the Insert keymap, re-run on replay via
    /// [`Editor::replay_command`], so it decides again at the new cursor,
    /// its own `call!`s, register prefix, and `#:extend` included. Its
    /// interactive inputs are answered from `answers`. A binding whose
    /// command is not repeatable has none: an input it opens ends the
    /// change and drops this entry.
    Binding {
        id: BindingId,
        name: Cow<'static, str>,
        answers: Vec<Answer>,
    },
    /// The net edit a completion accept produced at the cursor: a completion
    /// popup's Enter, or an Insert-key binding that called
    /// `completion-accept!` (in place of that binding's own entry). Replayed
    /// by applying the edit directly.
    Result(CursorReplacement),
}

/// The net text a completion accept wrote at the cursor: `back` chars behind
/// and `forward` ahead of the head, replaced by `text`. The primary cursor's
/// own counts, applied uniformly at every cursor on replay. Anything
/// document-absolute (a completion's `additionalTextEdits`) is excluded: it
/// has no meaning at a different cursor.
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
/// extent the edit acted on.
#[derive(Debug, Clone)]
pub(crate) struct SelectionStep {
    /// Command name (e.g. `"select-line"`, `"surround-paren"`).
    pub command: Cow<'static, str>,
    /// Count prefix originally used.
    pub count: usize,
    /// `true` if this step ran in Extend mode (grew the existing selection).
    pub extend: bool,
}

/// A recorded editing action that can be replayed by `.`.
///
/// Stores the recipe to re-execute a command rather than the raw changeset:
/// changesets are position-dependent and can't be replayed at a different
/// cursor.
#[derive(Debug, Clone)]
pub(crate) struct RepeatableAction {
    /// The command name that began this change (e.g. `"delete"`, `"change"`).
    /// `Cow::Borrowed` for built-ins (zero allocation); `Cow::Owned` for a
    /// dynamically-registered (Steel) command.
    pub command: Cow<'static, str>,
    /// The count prefix the change ran with. Overridden when `.` itself is given a
    /// count.
    pub count: usize,
    /// Selection-building recipe to replay BEFORE the command.
    ///
    /// `[]` (the command acted on a pre-existing selection), or a sequence of
    /// `Establishes`/`Composes` steps; see `SelectionTracking`. Filled from
    /// `EditorState::selection_recipe` when the change settles.
    pub selection_recipe: Vec<SelectionStep>,
    /// The answers the command's own interactive inputs got, in the order
    /// their inputs opened. See [`Answer`].
    pub answers: Vec<Answer>,
    /// Every Insert-session input the change consumed, in arrival order.
    /// See [`Input`].
    pub transcript: Vec<Input>,
}

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

impl RecordingScope {
    /// The entry of the Insert-key binding this scope runs, `None` for a
    /// command body's.
    pub(in crate::editor) fn binding(&self) -> Option<BindingId> {
        match self.slot {
            AnswerSlot::Binding(i) | AnswerSlot::UnrecordedBinding(i) => Some(i),
            AnswerSlot::Command => None,
        }
    }
}

/// What runs now in [`ScopeSlot`].
#[derive(Debug)]
enum Running {
    /// A recorded change's code.
    Scope(RecordingScope),
    /// An Insert key's binding, for its whole dispatch, with its own scope
    /// when the session records a change.
    InsertKey(Option<RecordingScope>),
}

/// The code running now, if any: a [`RecordingScope`], or an Insert key's
/// binding. Only [`with_recording_scope`] and [`with_insert_key_scope`] set
/// it, for the extent of one call, so nothing outlives the code it names.
#[derive(Debug, Default)]
pub(in crate::editor) struct ScopeSlot(Option<Running>);

impl ScopeSlot {
    /// The [`RecordingScope`] running now, if any.
    pub(in crate::editor) fn running(&self) -> Option<&RecordingScope> {
        match &self.0 {
            Some(Running::Scope(scope) | Running::InsertKey(Some(scope))) => Some(scope),
            Some(Running::InsertKey(None)) | None => None,
        }
    }

    /// Whether an Insert key's binding is dispatching. Dispatch never yields
    /// before it returns, all the way through a Steel `call!`, so a hook
    /// queued during the same keypress drains later, in `settle`, with this
    /// `false`.
    pub(in crate::editor) fn insert_key(&self) -> bool {
        matches!(self.0, Some(Running::InsertKey(_)))
    }

    /// Whether anything runs: a recorded change's code or an Insert key's
    /// binding.
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

/// Runs `f` as an Insert key's binding, `scope` its own [`RecordingScope`]
/// (`None` when nothing records it), restoring what it replaced afterward.
/// Shared by `handle_insert`'s trie-leaf dispatch and the replay of a
/// recorded [`Input::Binding`] entry, so a binding re-run on `.` sees the
/// same context it saw live. Set for the whole call regardless of what `f`
/// does internally (mode switches included: dispatch is synchronous all the
/// way through a Steel `call!`, so this never outlives the call it wraps).
/// It keeps a repeatable command the binding dispatches from starting a
/// change of its own ([`repeat_slot_owned`]) and lets `insert-key!` run.
/// The scope holds the recording in flight open for the call, so a binding
/// that leaves Insert and re-enters it records what follows into the same
/// change.
pub(in crate::editor) fn with_insert_key_scope<H: AsMut<EditorState>, R>(
    host: &mut H,
    scope: Option<RecordingScope>,
    f: impl FnOnce(&mut H) -> R,
) -> R {
    with_running(host, Some(Running::InsertKey(scope)), f)
}

/// A fresh change a repeatable command starts, as [`record_change`] records
/// it.
pub(in crate::editor) struct NewChange {
    pub(in crate::editor) command: Cow<'static, str>,
    pub(in crate::editor) count: usize,
    /// The operand the command was dispatched with, recorded as its first
    /// answer.
    pub(in crate::editor) seed: Option<CommandArg>,
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
    let Some(NewChange {
        command,
        count,
        seed,
        recipe,
    }) = change.filter(|_| !repeat_slot_owned(host.as_mut()))
    else {
        return body(host).0;
    };
    let (prev, scope) = host.as_mut().begin_change(command, count, seed);
    let (r, succeeded) = with_recording_scope(host, Some(scope), body);
    host.as_mut().end_change(prev, succeeded, recipe);
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
///   dispatches is that session's input, not a change of its own.
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
}

impl Recording {
    fn new(action: RepeatableAction) -> Self {
        Self {
            action,
            sources: Rc::new(()),
            stopped: false,
            next_binding: 0,
        }
    }

    /// `true` while an input source is still outstanding and the change was
    /// not stopped.
    pub(in crate::editor) fn accepting(&self) -> bool {
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
            .position(|input| matches!(input, Input::Binding { id: entry, .. } if *entry == id))
    }

    /// The answers `slot` names. `None` for a binding entry that was
    /// removed (a completion accept replaced it).
    fn answers_mut(&mut self, slot: AnswerSlot) -> Option<&mut Vec<Answer>> {
        match slot {
            AnswerSlot::Command => Some(&mut self.action.answers),
            AnswerSlot::Binding(id) => {
                self.action
                    .transcript
                    .iter_mut()
                    .find_map(|input| match input {
                        Input::Binding {
                            id: entry, answers, ..
                        } if *entry == id => Some(answers),
                        _ => None,
                    })
            }
            AnswerSlot::UnrecordedBinding(_) => None,
        }
    }

    /// The count the recorded change ran with, the default `.` replays with.
    pub(in crate::editor) fn count(&self) -> usize {
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
    /// A picker, prompt or menu callback and the argument its answer gives.
    Call {
        proc: SteelVal,
        arg: SteelVal,
    },
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

    /// The next answer, if it is a character read's.
    pub(in crate::editor) fn next_char(&mut self) -> Option<Replayed<Option<char>>> {
        self.next(|a| match a {
            Answer::Char(c) => Some(*c),
            _ => None,
        })
    }

    /// The next answer, if it is a pick.
    pub(in crate::editor) fn next_pick(&mut self) -> Option<Replayed<Pick>> {
        self.next(|a| match a {
            Answer::Pick(pick) => Some(pick.clone()),
            _ => None,
        })
    }

    /// The next answer, if it is a prompt's.
    pub(in crate::editor) fn next_prompt(&mut self) -> Option<Replayed<Option<String>>> {
        self.next(|a| match a {
            Answer::Prompt(text) => Some(text.clone()),
            _ => None,
        })
    }

    /// The next answer, if it is a menu's.
    pub(in crate::editor) fn next_menu(&mut self) -> Option<Replayed<Option<Choice>>> {
        self.next(|a| match a {
            Answer::Menu(choice) => Some(choice.clone()),
            _ => None,
        })
    }

    /// Answers a re-run interactive input (a read, a picker, a prompt, a
    /// menu) from the recording instead of opening it: `next` takes its
    /// answer, and `then` turns that answer into what to run after the
    /// re-run body, or the refusal naming why it has no fit (a missing item,
    /// an unbound action, a forgotten payload). Refuses with [`dot_refusal`]
    /// when the next answer is of another kind, and queues nothing for an
    /// input that never resolved live.
    pub(in crate::editor) fn answer_call<T>(
        &mut self,
        what: &str,
        next: impl FnOnce(&mut Self) -> Option<Replayed<T>>,
        then: impl FnOnce(T) -> Result<PendingAnswer, String>,
    ) -> Result<(), String> {
        if let Replayed::Answered(answer) = next(self).ok_or_else(|| dot_refusal(what))? {
            self.pending.push_back(then(answer)?);
        }
        Ok(())
    }
}

/// Proof that a completion input passed [`EditorState::interactive_input`],
/// which is the only way to get one.
#[must_use]
pub(in crate::editor) struct Interactive(());

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
/// still reaching `replay_dot`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PendingRepeat {
    /// Effective replay count, with the explicit-count override already
    /// applied.
    pub(super) count: usize,
}

// ── Recording lifecycle (EditorState) ─────────────────────────────────────────

impl EditorState {
    /// `true` while a recording is accepting input.
    pub(in crate::editor) fn recording_accepting(&self) -> bool {
        self.recording.as_ref().is_some_and(Recording::accepting)
    }

    /// Forget every pick's payload: payloads belong to the Steel engine a
    /// reload is replacing. A pick replayed by its choice is unaffected; one
    /// replayed by its payload becomes [`Picked::PayloadForgotten`].
    pub(in crate::editor) fn forget_steel_payloads(&mut self) {
        let Some(rec) = self.recording.as_mut() else {
            return;
        };
        let binding_answers = rec
            .action
            .transcript
            .iter_mut()
            .filter_map(|input| match input {
                Input::Binding { answers, .. } => Some(answers),
                _ => None,
            });
        for answers in std::iter::once(&mut rec.action.answers).chain(binding_answers) {
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
    fn begin_change(
        &mut self,
        command: Cow<'static, str>,
        count: usize,
        seed: Option<CommandArg>,
    ) -> (Option<Recording>, RecordingScope) {
        let mut action = RepeatableAction {
            command,
            count,
            selection_recipe: Vec::new(),
            answers: Vec::new(),
            transcript: Vec::new(),
        };
        if let Some(CommandArg::Char(ch)) = seed {
            action.answers.push(Answer::Char(Some(ch)));
        }
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
    /// body ended. A kept change takes `recipe` as its selection recipe; a
    /// discarded one restores `prev`, so a refused body leaves nothing
    /// behind.
    fn end_change(&mut self, prev: Option<Recording>, succeeded: bool, recipe: Vec<SelectionStep>) {
        let rec = self
            .recording
            .as_mut()
            .expect("begin_change installed the recording this closes");
        if succeeded || rec.accepting() {
            rec.action.selection_recipe = recipe;
        } else {
            self.recording = prev;
        }
    }

    /// The recording the open Insert session holds a source on, while it
    /// accepts input: the one change every Insert-session input records
    /// into, whatever else is accepting.
    fn session_recording(&mut self) -> Option<&mut Recording> {
        let ticket = self.active_session.as_ref()?.insert()?.recording()?;
        self.recording.as_mut().filter(|rec| rec.feeds(ticket))
    }

    /// Append `input` to the open Insert session's recording, if any. Called
    /// at the seam each input arrives (a typed Insert key, a paste, a
    /// completion accept).
    pub(in crate::editor) fn record_input(&mut self, input: Input) {
        if let Some(rec) = self.session_recording() {
            rec.action.transcript.push(input);
        }
    }

    /// Record the Insert-key binding `name` about to dispatch, before its
    /// body runs, so the answers its interactive inputs get have an entry to
    /// go to. Returns the [`RecordingScope`] the binding runs in, `None` when
    /// the open Insert session records no change.
    pub(in crate::editor) fn record_binding(
        &mut self,
        name: Cow<'static, str>,
        repeatable: bool,
    ) -> Option<RecordingScope> {
        let rec = self.session_recording()?;
        let id = BindingId(rec.next_binding);
        rec.next_binding += 1;
        rec.action.transcript.push(Input::Binding {
            id,
            name,
            answers: Vec::new(),
        });
        Some(RecordingScope {
            source: rec.open_ticket(),
            slot: if repeatable {
                AnswerSlot::Binding(id)
            } else {
                AnswerSlot::UnrecordedBinding(id)
            },
        })
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
        proc: SteelVal,
        arg: SteelVal,
    ) -> super::event::PendingWork {
        if let Some(ticket) = &ticket {
            self.record_answer(ticket, answer);
        }
        super::event::PendingWork::Call {
            proc,
            args: vec![arg],
            anchor: None,
            answer: ticket,
        }
    }

    /// [`Self::answered_call`], queued: how a picker, prompt or menu fires
    /// its callback once it closes.
    pub(in crate::editor) fn queue_answered_call(
        &mut self,
        ticket: Option<AnswerTicket>,
        answer: Answer,
        proc: SteelVal,
        arg: SteelVal,
    ) {
        let call = self.answered_call(ticket, answer, proc, arg);
        self.config.pending_work.push_back(call);
    }

    /// The check a completion trigger or accept passes before it opens.
    ///
    /// Refuses during `.`: a completion accept is replayed from the net edit
    /// recorded for it ([`Input::Result`]), so reaching one during replay
    /// means the replayed code decided differently than it did live, and an
    /// error beats opening a widget mid-repeat. Otherwise marks the capture
    /// armed for the Insert-key binding running now, if any, interactive, so
    /// its net edit replaces its entry.
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
            .and_then(EditSession::insert_mut)
            .and_then(InsertState::dot_capture_mut)
        {
            cap.interactive = true;
        }
        Ok(Interactive(()))
    }

    /// The check an interactive input that resolves on a later key (a read,
    /// a picker, a prompt, a menu) passes before it opens. Its caller answers
    /// it from the recorded change during `.` instead, so reaching this
    /// during replay refuses.
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
        what: &str,
        opener: InputOpener,
    ) -> Result<Option<AnswerTicket>, String> {
        if self.dot_replay.is_some() {
            return Err(dot_refusal(what));
        }
        let Some(scope) = self.recording_scope.running() else {
            if !matches!(opener, InputOpener::RepeatableOperand) {
                self.stop_recording(None);
            }
            return Ok(None);
        };
        let slot = scope.slot;
        if let AnswerSlot::UnrecordedBinding(binding) = slot {
            self.stop_recording(Some(binding));
            return Ok(None);
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
        Ok(ticket)
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

    /// Finalize a capture into the recording. Records nothing unless a
    /// completion trigger or accept made it interactive; then the entry of
    /// the binding it wrapped is replaced, so `.` does not re-run a binding
    /// whose completion it cannot replay: by its net cursor edit, an
    /// [`Input::Result`], when it changed text, otherwise by nothing. A bare
    /// completion-popup Enter, which has no entry, appends its net edit.
    pub(in crate::editor) fn finalize_dot_capture(&mut self, cap: DotCapture) {
        if !cap.interactive {
            return;
        }
        let replacement = match cap.chain.finish() {
            Ok(replacement) => replacement,
            Err(ChainBroken) => {
                self.report(
                    super::Severity::Warning,
                    "`.` won't repeat this edit: the buffer changed while it ran".to_string(),
                );
                None
            }
        };
        let Some(binding) = cap.binding else {
            if let Some(r) = replacement {
                self.record_input(Input::Result(r));
            }
            return;
        };
        let Some(rec) = self.session_recording() else {
            return;
        };
        if let Some(i) = rec.binding_index(binding) {
            match replacement {
                Some(r) => rec.action.transcript[i] = Input::Result(r),
                None => {
                    rec.action.transcript.remove(i);
                }
            }
        }
    }

    /// End an Insert session's part in the recording: finalize the capture
    /// still armed on it, if any, then close the session's source. In that
    /// order: a finalized capture records into the recording only while it
    /// still accepts input.
    ///
    /// The capture is still armed when its binding tears the session down
    /// from inside its own dispatch (`completion-accept!` then `exit-insert`
    /// in one body), before `with_dot_capture` takes it back.
    pub(in crate::editor) fn end_insert_recording(&mut self) {
        if let Some(cap) = self
            .active_session
            .as_mut()
            .and_then(EditSession::insert_mut)
            .and_then(InsertState::take_dot_capture)
        {
            self.finalize_dot_capture(cap);
        }
        drop(
            self.active_session
                .as_mut()
                .and_then(EditSession::insert_mut)
                .and_then(InsertState::release_recording),
        );
    }
}

// ── Macro recording state (EditorState) ───────────────────────────────────────

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
    /// commits directly through `doc_ops::commit_open_session`.
    ///
    /// No-op if nothing is open at all: the body's own dispatch may already
    /// have closed the pane or buffer; `focus::end_focus_sessions` already
    /// committed a still-live session before that happened.
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

    /// Run `cmd` (already resolved from the registry) as one replayed step,
    /// native or Steel/Lazy alike. The main command and each `Input::Binding`
    /// share this. A native command's operand is its first answer, as live
    /// (dispatched with one, or reading it); a Steel command's interactive
    /// inputs take their answers during the body. Returns `false` on the same
    /// failure classes as `run_steel_command` (a Steel body can fail on
    /// replay even though the original run didn't), and when a native
    /// command's operand is missing or doesn't fit, which is reported. A
    /// native command whose operand read was cancelled or never resolved did
    /// nothing live, and does nothing here.
    fn replay_command(
        &mut self,
        cmd: MappableCommand,
        fp: commands::FocusedPane,
        ctx: &CmdCtx,
    ) -> bool {
        match &cmd {
            MappableCommand::SteelBacked { .. } | MappableCommand::Lazy { .. } => {
                if !self.run_steel_command(cmd.name(), ctx) {
                    return false;
                }
                // Inner call! dispatches inside the Steel body run through
                // `commands::run` → step_update_recipe. Clear it so stale steps
                // don't contaminate the next command's recipe accumulation.
                self.state.selection_recipe.clear();
                true
            }
            _ => {
                let arg = match self
                    .state
                    .dot_replay
                    .as_mut()
                    .and_then(DotReplay::next_char)
                {
                    Some(Replayed::Answered(Some(ch))) => Some(CommandArg::Char(ch)),
                    Some(Replayed::Answered(None) | Replayed::Unanswered) => return true,
                    None => None,
                };
                match commands::BoundCommand::focused(cmd, fp, arg) {
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
                }
            }
        }
    }

    /// Replay a dot-repeat change directly, bypassing dispatch bookkeeping.
    ///
    /// Takes the recording out for the whole replay and restores it afterward
    /// so `.` chains, which is also what keeps the replay from recording into
    /// it. Sets [`EditorState::dot_replay`] for the same extent.
    pub(in crate::editor) fn replay_dot(&mut self, count: usize) {
        let Some(recording) = self.state.recording.take() else {
            return;
        };
        let replay = DotReplay {
            answers: recording.action.answers.iter().cloned().collect(),
            pending: VecDeque::new(),
        };
        let prev = self.state.dot_replay.replace(replay);
        self.replay_action(&recording.action, count);
        self.state.dot_replay = prev;
        self.state.recording = Some(recording);
    }

    /// `replay_dot`'s body: replays the selection recipe, re-runs the command
    /// (its interactive inputs answered from its recorded answers), then walks
    /// the transcript, feeding each input into the path it names (see
    /// [`Input`]).
    fn replay_action(&mut self, action: &RepeatableAction, count: usize) {
        // Read once, at entry: recipe steps never move focus.
        let fp = commands::FocusedPane::current(&self.state);

        // Resolve the command before opening the edit group: a missing command
        // must return while there is still no cleanup obligation.
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

        // Rebuild the selection extent the command acted on. Every
        // recipe step names a native command taking no operand (a `Composes`/
        // `Establishes` selection command): `SteelBacked`/`Lazy` are always
        // `Untracked`, native entries can never be unregistered, so the lookup
        // and bind always succeed.
        for step in &action.selection_recipe {
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
        let ran = self.replay_command(edit_cmd, fp, &ctx);
        self.finish_replayed_answers(ran);
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
        // it belongs to a session that's gone. `break`, not `return`, so
        // `finish_replay_session` below still runs.
        for input in &action.transcript {
            match input {
                Input::Key(key) => {
                    commands::insert_default_key(&mut self.state, &self.view, fp, *key);
                }
                Input::Paste(text) => self.apply_insert_mode_paste(text),
                Input::Result(r) => {
                    if let Err(msg) = self.state.apply_cursor_replacement(&self.view, fp, r) {
                        self.report(super::Severity::Error, msg);
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
                    let ran = with_insert_key_scope(self, None, |ed| {
                        ed.replay_command(cmd, fp, &super::input_stack::insert::INSERT_KEY_CTX)
                    });
                    self.finish_replayed_answers(ran);
                    if self.state.mode() != Mode::Insert {
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
    /// shorter branch this time) so the next body starts from its own.
    fn finish_replayed_answers(&mut self, ran: bool) {
        while ran
            && let Some(answer) = self
                .state
                .dot_replay
                .as_mut()
                .and_then(|replay| replay.pending.pop_front())
        {
            let call = match answer {
                PendingAnswer::Read(read, answer) => {
                    let resolved = super::input_stack::char::resolve(&mut self.state, read, answer);
                    super::input_stack::char::run_resolved(self, resolved)
                }
                PendingAnswer::Call { proc, arg } => Some(super::event::PendingWork::Call {
                    proc,
                    args: vec![arg],
                    anchor: None,
                    answer: None,
                }),
            };
            if let Some(call) = call {
                self.run_pending_batch(VecDeque::from([call]));
            }
        }
        if let Some(replay) = self.state.dot_replay.as_mut() {
            replay.answers.clear();
            replay.pending.clear();
        }
    }

    // ── Dot capture (completion accepts) ──────────────────────────────────────

    /// Runs `f` with a fresh [`DotCapture`] armed on the focused Insert
    /// session, wrapping an operation that may accept a completion: an
    /// Insert-key binding's dispatch (`binding` is its entry), or a bare
    /// completion-popup Enter (`binding` is `None`). Takes the capture back
    /// and finalizes it once `f` returns. With no Insert session there, `f`
    /// runs with nothing armed.
    pub(in crate::editor) fn with_dot_capture(
        &mut self,
        binding: Option<BindingId>,
        f: impl FnOnce(&mut Self),
    ) {
        let fp = commands::FocusedPane::current(&self.state);
        let (pane, buffer) = (fp.pid(), fp.bid(&self.view));
        let armed = self
            .state
            .active_session
            .as_ref()
            .is_some_and(|s| s.is_insert_at(pane, buffer));
        if armed {
            let head = commands::pane_view(&self.state, &self.view, fp.pane())
                .primary()
                .head();
            let chain = DotChain::new(head, self.state.buffers.get(buffer).text());
            self.state
                .active_session
                .as_mut()
                .and_then(EditSession::insert_mut)
                .expect("an Insert session was just confirmed open here")
                .arm_dot_capture(DotCapture {
                    chain,
                    interactive: false,
                    binding,
                });
        }
        f(self);
        if !armed {
            return;
        }
        if let Some(cap) = self
            .state
            .active_session
            .as_mut()
            .filter(|s| s.is_insert_at(pane, buffer))
            .and_then(EditSession::insert_mut)
            .and_then(InsertState::take_dot_capture)
        {
            self.state.finalize_dot_capture(cap);
        }
    }

    /// Drain the macro replay queue, executing each key in order and settling
    /// after each one.
    ///
    /// Sets `is_replaying` for the duration so that `Q`/`q` intercepts inside
    /// replayed keys cannot start nested recording or replay sessions,
    /// including when the macro's last key is `Q`, where
    /// `replay_queue.is_empty()` would already be `true` and would fail to
    /// suppress it.
    ///
    /// Saves and restores the recording so replay does not corrupt dot-repeat,
    /// unless the macro left its own change accepting input (an Insert
    /// session, a pending read or pick): keeping it, rather than restoring
    /// the saved one, makes a following `.` repeat what the macro started.
    ///
    /// Settling once per replayed key, not once after the whole queue, keeps a
    /// macro's own hooks in the loop: a plugin that reconfigures the buffer on
    /// `on-buffer-open` (indent width, a language keymap) has that reaction run
    /// before the macro's remaining keys type into the buffer, as it would for
    /// keys typed by hand. `is_replaying` stays `true` across these settles,
    /// so `confirm_permit` still holds back a confirm no queued key could
    /// answer; it waits for the next real buffer-enter.
    ///
    /// `message_logged_this_input` is OR'd back in every iteration so a
    /// stale-buffer warning from a later key's settle doesn't lose an earlier
    /// message's shadowing guard.
    pub(in crate::editor) fn drain_replay_queue(&mut self) {
        if self.state.replay_queue.is_empty() {
            return;
        }
        let saved = self.state.recording.take();
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
        if !self.state.recording_accepting() {
            self.state.recording = saved;
        }
    }
}

/// The error an interactive input reached during `.` raises.
fn dot_refusal(what: &str) -> String {
    format!("{what}: reached during `.`, but the recorded input took a different branch this time")
}
