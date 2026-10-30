//! The queue of deferred Steel work: an event to fire by name or a captured
//! callback to call, each raised here and drained in order by `pump`.

use hume_core::{BufferId, HostToken, PaneHandle};
use hume_scripting::SteelCall;
use hume_scripting::host::JobHandle;

use super::EditorEvent;
use crate::editor::buffer::store::EditOrigin;
use crate::editor::lsp::ResponseAnchor;
use crate::editor::replay::{AnswerTicket, PickerKeyRun};
use crate::editor::{Editor, EditorState};
use hume_core::TimerId;

/// One item of deferred Steel work, queued by a raise site and drained by
/// `Editor::settle()` in FIFO order. One merged queue closes the stranded-
/// events bug (see `tests/events/settle_queue.rs`'s
/// `event_raised_from_async_work_fires_on_settle_with_no_input`): a `Call`
/// and an `Event` queued in the same batch drain in insertion order, in one
/// fixpoint, instead of two queues drained at two different points of the
/// run loop.
///
/// Hooks always route through here rather than firing inline. This is a
/// semantic guarantee of the hook model ("when X happens, then do Y"), not a
/// consequence of the borrow architecture: a hook must run *after* the
/// command that triggers it completes, never mid-command. Even if re-entrancy
/// were fully solved mechanically, this stays queued. **Do not optimize hooks
/// to fire inline**: this decision is locked.
#[derive(Debug)]
pub(in crate::editor) enum PendingWork {
    /// A specific Steel closure already captured by the raise site: an
    /// `lsp-request!` callback, a timer thunk, a prompt/menu/drawer/picker
    /// callback. Delivered to exactly that closure, not to every handler for
    /// a name. Built only through [`PendingWork::call`].
    Call {
        call: SteelCall,
        /// What decides at the call's turn whether it still runs, and what
        /// its run or drop releases. A guarded call runs alone. `None` for
        /// a call with no such condition, which runs in one batch with the
        /// unguarded calls of the same origin queued next to it.
        guard: Option<CallGuard>,
        /// Whose edits the call makes: the user's for a callback a key of
        /// theirs fired, background ones otherwise.
        origin: EditOrigin,
    },
    /// An editor event to fire by name at drain time. Args are built by
    /// `into_steel_args()` only if a handler is actually registered.
    Event(EditorEvent),
}

/// The condition a queued [`PendingWork::Call`] runs under. Each variant is
/// checked at the call's own turn, not when it was queued, because other
/// queued work can run first and change what the check reads. A call the
/// drain cap drops unrun goes through `Editor::drop_unrun`, which releases
/// what its guard holds as running it would; a `:reload-config` drops the
/// whole queue with the state each guard names.
#[derive(Debug)]
pub(in crate::editor) enum CallGuard {
    /// An `lsp-request!` callback. `anchor` is `Some` for a request a server
    /// answered: the `ResponseAnchor` already checked once at LSP drain time
    /// (`EditorState::anchor_admits`), checked again here. `hold` is the
    /// request's hold on its `#:tracked` position, dropped once the call
    /// has run, raised, or been dropped, which releases the position unless
    /// the callback kept it. At least one of the two is `Some`.
    Reply {
        anchor: Option<ResponseAnchor>,
        hold: Option<hume_scripting::host::TrackedHold>,
    },
    /// The ticket of the interactive input this call answers (a read,
    /// picker, prompt or menu callback), when that input belongs to a
    /// recorded change; see [`crate::editor::replay::AnswerTicket`]. Keeps
    /// the change open until the call has run, inside the input's scope
    /// (`Editor::run_answered`), so only this call's own work belongs to the
    /// change.
    Answer(AnswerTicket),
    /// A picker keymap binding's call. The call is dropped when its picker
    /// closed before its turn, and runs inside `replay::run_picker_key`.
    PickerKey(PickerKeyRun),
    /// The `ConfigState.async_jobs` entry this call reports for. The call
    /// is dropped when `cancel-async!` removed the entry before its turn,
    /// and takes the entry with it once it was the entry's last report or
    /// raised.
    Job(JobHandle),
    /// A timer's Steel thunk, queued once it came due. The call is dropped
    /// when `cancel-timer!` cancelled the timer before its turn.
    Timer(TimerId),
    /// A completion source's call for request `id`. When the call raises,
    /// or is dropped unrun, the request ends: nothing will answer it
    /// further, and its source gets its `#:cancel`. Every other request is
    /// left waiting.
    Invocation(HostToken),
}

impl PendingWork {
    /// A `Call` of `call` under `guard`, its edits counted as `origin`'s.
    pub(in crate::editor) fn call(
        call: SteelCall,
        guard: Option<CallGuard>,
        origin: EditOrigin,
    ) -> Self {
        Self::Call {
            call,
            guard,
            origin,
        }
    }

    /// The `Call` an interactive input's callback queues: the user's,
    /// answering `ticket`'s input if any.
    pub(in crate::editor) fn answer(call: SteelCall, ticket: Option<AnswerTicket>) -> Self {
        Self::call(call, ticket.map(CallGuard::Answer), EditOrigin::User)
    }
}

impl EditorState {
    /// Enqueue `event` to fire after the current command returns. This is the
    /// single raise path every event goes through, reached as
    /// `self.state.queue_event(…)` from `Editor` methods and directly, like
    /// `settings::ops::apply_global`, from free functions that only hold
    /// `&mut EditorState`.
    pub(in crate::editor) fn queue_event(&mut self, event: EditorEvent) {
        self.push_work(PendingWork::Event(event));
    }

    /// Queue `call`, a callback a key of the user's fired: a drawer,
    /// live-picker query or completion-source callback. Background work
    /// queues through [`Self::queue_job_callback`],
    /// [`Self::queue_timer_thunk`] or [`Self::queue_lsp_callback`], whose
    /// edits do not count as the user's; an interactive input's callback is
    /// built by `EditorState::answered_call` instead.
    ///
    /// Every queued call runs at the next drain boundary, never inline: LSP
    /// dispatch, timer fire and minibuffer key handling all detect their
    /// completion from inside a borrow that can't re-enter Steel. Lives on
    /// `EditorState` (not `Editor`) so a drawer layer and `EditorHostImpl`,
    /// which only hold `&mut EditorState`, can queue too, the same reason
    /// `queue_event` lives here.
    pub(in crate::editor) fn queue_steel_call(&mut self, call: SteelCall) {
        self.push_work(PendingWork::call(call, None, EditOrigin::User));
    }

    /// Queue each `(id, call)`, a completion source's call for request
    /// `id`; see [`CallGuard::Invocation`].
    pub(in crate::editor) fn queue_source_calls(
        &mut self,
        calls: impl IntoIterator<Item = (HostToken, SteelCall)>,
    ) {
        for (id, call) in calls {
            self.push_work(PendingWork::call(
                call,
                Some(CallGuard::Invocation(id)),
                EditOrigin::User,
            ));
        }
    }

    /// Queue `call`, the callback of the `ConfigState.async_jobs` entry
    /// `job`; see [`CallGuard::Job`].
    pub(in crate::editor) fn queue_job_callback(&mut self, job: JobHandle, call: SteelCall) {
        self.push_work(PendingWork::call(
            call,
            Some(CallGuard::Job(job)),
            EditOrigin::Background,
        ));
    }

    /// Queue `call`, the thunk of timer `id`, which has come due; see
    /// [`CallGuard::Timer`].
    pub(in crate::editor) fn queue_timer_thunk(&mut self, id: TimerId, call: SteelCall) {
        self.push_work(PendingWork::call(
            call,
            Some(CallGuard::Timer(id)),
            EditOrigin::Background,
        ));
    }

    /// Queue `call`, an `lsp-request!` callback: carries the delivery's `ResponseAnchor` (`None` for a
    /// request no server took) and the request's hold on its `#:tracked`
    /// position; see [`CallGuard::Reply`]. Never carries an answer ticket:
    /// an LSP response resolves no waiting input.
    pub(in crate::editor) fn queue_lsp_callback(
        &mut self,
        call: SteelCall,
        anchor: Option<ResponseAnchor>,
        hold: Option<hume_scripting::host::TrackedHold>,
    ) {
        let guard =
            (anchor.is_some() || hold.is_some()).then_some(CallGuard::Reply { anchor, hold });
        self.push_work(PendingWork::call(call, guard, EditOrigin::Background));
    }

    /// Append `work` to the queue `Editor::settle` drains: the one place an
    /// item enters it.
    pub(in crate::editor) fn push_work(&mut self, work: PendingWork) {
        self.config.pending_work.push_back(work);
    }
}

impl Editor {
    /// Queue `OnBufferSave` for `bid`. Kept as a named seam so tests can
    /// raise the event without a real write.
    pub(in crate::editor) fn queue_buffer_save(&mut self, bid: BufferId) {
        self.state
            .queue_event(EditorEvent::OnBufferSave { buffer: bid });
    }

    /// Fire `OnViewportChange (pane first-line end-line)` for `pane_id`,
    /// called only when its debounce timer actually fires (`timer_bridge`),
    /// reading the pane's *current* bounds rather than whatever they were
    /// when the timer was armed. A no-op if the pane closed in the meantime,
    /// or if its tab went to the background before the (debounced) timer
    /// fired: the frame's scroll step stops maintaining a backgrounded
    /// pane's scroll position (only its size stays current), so firing with
    /// its frozen scroll would hand a handler a range the code itself no
    /// longer trusts. `prepare_frame` drops that pane's `last_visible_range`
    /// when its tab backgrounds, so the pane's first frame back on screen
    /// reads as a fresh change and re-arms this on its own. This guard only
    /// skips the fire for the frames spent hidden, not the one on return.
    pub(in crate::editor) fn queue_viewport_change(&mut self, pane_id: hume_core::PaneId) {
        if !self.view.layout().contains_leaf(pane_id) {
            return;
        }
        let lines = crate::editor::geometry::viewport_range(&self.state, &self.view, pane_id);
        let bid = self.view.panes[pane_id].buffer_id();
        self.state.queue_event(EditorEvent::OnViewportChange {
            target: PaneHandle::with_pane(bid, pane_id),
            lines,
        });
    }
}
