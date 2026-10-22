//! The queue of deferred Steel work: an event to fire by name or a captured
//! callback to call, each raised here and drained in order by `pump`.

use hume_engine::pipeline::BufferId;
use hume_scripting::PaneHandle;
use steel::rvals::SteelVal;

use super::EditorEvent;
use crate::editor::buffer::store::EditOrigin;
use crate::editor::lsp::ResponseAnchor;
use crate::editor::replay::AnswerTicket;
use crate::editor::{Editor, EditorState};

/// One item of deferred Steel work, queued by a raise site and drained by
/// `Editor::settle()` in FIFO order. One merged queue closes the stranded-
/// events bug (see `tests/events.rs`'s
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
    /// a name.
    ///
    /// `anchor`: `Some` only for an `lsp-request!` callback a server
    /// answered, carrying the same `ResponseAnchor` already checked once at
    /// LSP drain time (`Editor::anchor_admits`), re-checked here, at dequeue,
    /// because arbitrary other queued work (an earlier `Call` in the same
    /// batch, a hook) can run first and change the state the drain-time
    /// check saw. `None` for every other source (a timer thunk, a picker
    /// callback, a request no server took, …).
    Call {
        proc: SteelVal,
        args: Vec<SteelVal>,
        anchor: Option<crate::editor::lsp::ResponseAnchor>,
        /// An `lsp-request!` callback's hold on its `#:tracked` position,
        /// dropped once the call has run, raised, or been dropped by its
        /// anchor or the drain cap, which releases the position unless the
        /// callback kept it.
        tracked: Option<hume_scripting::host::TrackedHold>,
        /// The ticket of the interactive input this call answers (a read,
        /// picker, prompt or menu callback), when that input belongs to a
        /// recorded change; see [`crate::editor::replay::AnswerTicket`]. Keeps the
        /// change open until the call has run. `Editor::run_pending_batch`
        /// runs such a call alone (never batched with a sibling `Call`),
        /// inside the input's scope (`Editor::run_answered`), so only this
        /// call's own work belongs to the change.
        answer: Option<crate::editor::replay::AnswerTicket>,
        /// Whose edits the call makes: the user's for a callback a key of
        /// theirs fired, background ones otherwise.
        origin: EditOrigin,
    },
    /// An editor event to fire by name at drain time. Args are built by
    /// `into_steel_args()` only if a handler is actually registered.
    Event(EditorEvent),
}

impl PendingWork {
    /// The `Call` an interactive input's callback queues: the user's, with
    /// no anchor or tracked position, answering `ticket`'s input if any.
    pub(in crate::editor) fn answer(
        proc: SteelVal,
        arg: SteelVal,
        ticket: Option<AnswerTicket>,
    ) -> Self {
        Self::Call {
            proc,
            args: vec![arg],
            anchor: None,
            tracked: None,
            answer: ticket,
            origin: EditOrigin::User,
        }
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

    /// Queue `(proc, args)`, a callback a key of the user's fired, for
    /// [`Self::queue_call`]: a drawer, live-picker query or completion-source
    /// callback. Background work queues through
    /// [`Self::queue_background_call`]; an interactive input's callback is
    /// built by `EditorState::answered_call` instead.
    pub(in crate::editor) fn queue_steel_call(
        &mut self,
        proc: steel::rvals::SteelVal,
        args: Vec<steel::rvals::SteelVal>,
    ) {
        self.queue_call(proc, args, None, None, EditOrigin::User);
    }

    /// [`Self::queue_steel_call`] for a callback background work fires: a
    /// timer thunk, an async job's or process's callback. Its edits do not
    /// count as the user's.
    pub(in crate::editor) fn queue_background_call(
        &mut self,
        proc: steel::rvals::SteelVal,
        args: Vec<steel::rvals::SteelVal>,
    ) {
        self.queue_call(proc, args, None, None, EditOrigin::Background);
    }

    /// [`Self::queue_background_call`]'s counterpart for an `lsp-request!`
    /// callback: carries the delivery's `ResponseAnchor` (`None` for a
    /// request no server took) and the request's hold on its `#:tracked`
    /// position, dropped once the call has run or been dropped. Never
    /// carries an answer ticket: an LSP response resolves no waiting input.
    pub(in crate::editor) fn queue_lsp_callback(
        &mut self,
        proc: steel::rvals::SteelVal,
        args: Vec<steel::rvals::SteelVal>,
        anchor: Option<ResponseAnchor>,
        tracked: Option<hume_scripting::host::TrackedHold>,
    ) {
        self.queue_call(proc, args, anchor, tracked, EditOrigin::Background);
    }

    /// Queue `(proc, args)` for evaluation at the next drain boundary, never
    /// called inline: LSP dispatch, timer fire and minibuffer key handling
    /// all detect their completion from inside a borrow that can't re-enter
    /// Steel. Its edits count as `origin`'s. Lives on `EditorState` (not
    /// `Editor`) so a drawer layer and `EditorHostImpl`, which only hold
    /// `&mut EditorState`, can queue too, the same reason `queue_event` lives
    /// here.
    fn queue_call(
        &mut self,
        proc: steel::rvals::SteelVal,
        args: Vec<steel::rvals::SteelVal>,
        anchor: Option<ResponseAnchor>,
        tracked: Option<hume_scripting::host::TrackedHold>,
        origin: EditOrigin,
    ) {
        self.push_work(PendingWork::Call {
            proc,
            args,
            anchor,
            tracked,
            answer: None,
            origin,
        });
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
    pub(in crate::editor) fn queue_viewport_change(
        &mut self,
        pane_id: hume_engine::pipeline::PaneId,
    ) {
        if !self.view.layout().contains_leaf(pane_id) {
            return;
        }
        let lines =
            crate::editor::lsp::introspect::viewport_range(&self.state, &self.view, pane_id);
        let bid = self.view.panes[pane_id].buffer_id();
        self.state.queue_event(EditorEvent::OnViewportChange {
            target: PaneHandle::with_pane(bid, pane_id),
            lines,
        });
    }
}
