//! Draining the work queue to quiescence: `Editor::settle`, the observation
//! points that raise events no write site can, and running each queued
//! event and call.

use super::{CallGuard, EditorEvent, PendingWork};
use crate::editor::Editor;
use crate::editor::buffer::DiskCheckTrigger;
use crate::editor::buffer::store::EditOrigin;
use crate::editor::commands::CommandPane;
use hume_core::Severity;
use hume_scripting::PaneHandle;

/// Upper bound on total work items processed per `settle()` boundary.
///
/// Bounding *passes* instead of total work would still let an amplifying
/// cascade (a handler that enqueues more hooks than it received) blow up the
/// batch size geometrically pass over pass: few passes, but exponential
/// total evals. Counting total hooks processed bounds both that shape and the
/// constant-width ping-pong loop; unreachable in any legitimate configuration.
///
/// A backstop against a plugin gone off the rails, not a state boundary: it
/// should never fire against correct scripts, so the abort path (below) does
/// not try to preserve fidelity across it, e.g. an observation baseline like
/// `Buffer::announced_generation` that already advanced past a dropped event is
/// left advanced, not rolled back. By the time this cap trips, the editor is
/// already reporting a bug; losing one further notification on top of that is
/// not worth the bookkeeping to prevent.
const MAX_EVENT_DRAIN: usize = 1000;

impl Editor {
    /// Advance editor state to quiescence: drain completed async work (LSP
    /// responses, timer fires: `drain_async_sources`), drain
    /// `state.config.pending_work` to a fixpoint, then install finished
    /// parses and bring every visible buffer's syntax tree up to its text
    /// (`reparse_stale_buffers`). Each call ends with
    /// `apply_startup_positions`.
    ///
    /// This is the single consumer of the merged work queue: a
    /// `Call` (an `lsp-request!` callback, a timer thunk, a prompt/menu/
    /// drawer/picker callback) and an `Event` (fired to every handler
    /// registered for its name) drain in the exact order they were queued,
    /// grouped only where that's free, i.e. a contiguous run of unguarded
    /// `Call`s goes to `run_steel_calls` as one batch. A handler that
    /// queues more work is picked up within the same `settle()` call, not
    /// the next frame.
    ///
    /// Takes no arguments, so it's callable without a terminal.
    ///
    /// `drain_async_sources` runs once, *outside* [`Self::drain_pending_work`]'s
    /// fixpoint: a timer thunk that re-arms itself
    /// (`(after! 0 (lambda () (after! 0 …)))`) would otherwise never leave the
    /// loop: each firing converts straight back into a due timer the same
    /// pass would immediately redrain. Outside the fixpoint, a re-arm is
    /// picked up on the *next* `settle()` instead, one frame later, which
    /// bounds it. `:reload-config`'s `resync_config_state` call site drains
    /// only `drain_pending_work` for this reason; see its doc.
    pub(crate) fn settle(&mut self) {
        self.drain_async_sources();
        if self.drain_pending_work() {
            // The span `Editor::handle_input` opened ("this input's own
            // dispatch just logged a message") closes here, once this
            // settle() has run the buffer-enter disk check that span exists
            // to protect against; see `EditorState::message_logged_this_input`'s
            // doc.
            self.state.message_logged_this_input = false;
        }
        // After every queued call: one can change a buffer's text (a
        // drawer's `on-select` running `goto-revision!`, a timer, an LSP
        // callback), and the frame drawn next must not highlight that text
        // at the old tree's offsets.
        self.reparse_stale_buffers();
        // After the drain, so the `OnBufferOpen`/`OnLanguageSet` hooks have
        // run: centring counts a band one of them opened.
        self.apply_startup_positions();
    }

    /// Fixpoint over `state.config.pending_work` only, with no async sources.
    /// The loop body of `settle`, split out so `:reload-config`'s accounting
    /// window (`typed_reload_config`) can drain only the config's own
    /// queued hooks without an unrelated LSP/parse/timer message landing
    /// inside it and being mistaken for a reload failure.
    ///
    /// Also the single observation point for `OnBufferEnter`:
    /// `detect_buffer_enter` runs at the top of every pass, not just once
    /// before the loop, so a handler-driven `switch-to-buffer!` is caught by
    /// the very next pass instead of waiting a frame, and the loop's exit
    /// condition is "queue empty **and** focus stable" rather than just
    /// "queue empty": a pass that only detects a focus change still has
    /// work to do (queuing and then draining `OnBufferEnter`) even though
    /// `pending_work` was empty when the pass began.
    ///
    /// Capped at [`MAX_EVENT_DRAIN`] total items processed (not passes), so
    /// neither a constant-width ping-pong loop (two `on-language-set`
    /// handlers flipping a value between two settings) nor an amplifying
    /// cascade (a handler that queues more work than it received, doubling
    /// the batch pass over pass) can livelock the editor. The watchdog only
    /// bounds each individual eval, not this loop.
    ///
    /// Returns `false` if the cap aborted the drain with work still
    /// unprocessed, so callers can skip whatever follow-up assumes a clean
    /// quiescent state.
    pub(in crate::editor) fn drain_pending_work(&mut self) -> bool {
        let mut total_processed = 0usize;
        loop {
            // First, so a buffer opened by any path, handlers of the previous
            // pass included, queues `OnLanguageSet` and `OnBufferOpen` ahead
            // of the `OnBufferEnter` its own focus change triggers.
            self.detect_pending_languages();
            self.detect_buffer_enter();
            self.detect_mode_change();
            // A pane switch, a closed buffer or an edit from any path leaves
            // the session stale with no write site to hang a dismiss on, so
            // it is reconciled on every pass, like `detect_buffer_enter`'s
            // focus check. A `Minibuf` session watches no buffer.
            self.state.reconcile_completion(&self.view);
            self.state.queue_completion_cancels();
            self.state.request_selected_completion_doc();
            // Inside the fixpoint, not outside it like `drain_async_sources`:
            // a *synchronous* handler that reacts to its own buffer's edit
            // (auto-format, trim-on-change) is a feedback loop, and only
            // inside the loop does `MAX_EVENT_DRAIN` (see its doc) catch it
            // in the same `settle()` call. This does not cover a debounced
            // handler: its re-triggering edit lands via a timer thunk, which
            // `drain_async_sources` converts to queued work only once per
            // `settle()` and outside this loop, so `total_processed` never
            // accumulates across the resulting one-cycle-per-interval churn
            // and the cap never trips. No rollback here on the cap path,
            // unlike `detect_buffer_enter`'s `last_entered_buffer = None`:
            // see `MAX_EVENT_DRAIN`'s doc for why.
            self.detect_text_changed();
            self.detect_history_changed();
            total_processed += self.raise_parked_writes();
            self.commit_ready_writes();
            if self.state.config.pending_work.is_empty() && self.state.pending_writes.is_settled() {
                return true;
            }
            let batch = std::mem::take(&mut self.state.config.pending_work);
            total_processed += batch.len();
            if total_processed > MAX_EVENT_DRAIN {
                // `batch` was just drained from `pending_work` above, so
                // nothing has been re-enqueued yet, so it's the entire drop.
                let dropped = batch.len();
                self.report(
                    Severity::Error,
                    format!(
                        "event/callback cascade exceeded {MAX_EVENT_DRAIN} total drained work \
                         item(s); dropping {dropped} pending item(s); handler feedback loop?"
                    ),
                );
                self.state.message_logged_this_input = false;
                // `detect_buffer_enter` already advanced `last_entered_buffer`
                // to the buffer this dropped batch's `OnBufferEnter` (if any)
                // was raised for. Undo that so the next `settle()` observes
                // the diff again and re-raises it, instead of the buffer-enter
                // disk check being lost for good because the baseline already
                // matches. Only when the batch actually held one: resetting
                // unconditionally would re-raise (and re-check) a buffer whose
                // event already fired earlier in this same drop.
                if batch
                    .iter()
                    .any(|w| matches!(w, PendingWork::Event(EditorEvent::OnBufferEnter { .. })))
                {
                    self.state.last_entered_buffer = None;
                }
                for work in batch {
                    if let PendingWork::Call {
                        guard: Some(guard), ..
                    } = work
                    {
                        self.drop_unrun(guard);
                    }
                }
                return false;
            }
            self.run_pending_batch(batch);
        }
    }

    /// Observation point for `OnBufferEnter` and the buffer-enter disk
    /// check. `focused_buffer_id()` is a derived join of `view.focused_pane()`
    /// and `pane.buffer_id`, each written by its own chokepoint (`focus`,
    /// `switch_pane_to_buffer`), so it has no single write-site chokepoint
    /// to hang a raise on: a raise wired into just one of those two would
    /// miss a switch caused through the other. Diffed against
    /// `EditorState::last_entered_buffer` every pass of `settle`'s loop
    /// rather than once before it, so a pane-focus move and a buffer switch
    /// in the same pass coalesce into one event, and a handler that itself
    /// switches buffers is caught by the very next pass. `BufferStore.mru`
    /// needs no promotion here: the `focus` moves/`switch_pane_to_buffer`
    /// already promote it synchronously, at the same two chokepoints this
    /// diffs.
    fn detect_buffer_enter(&mut self) {
        let now = self.focused_buffer_id();
        if self.state.last_entered_buffer != Some(now) {
            self.state.last_entered_buffer = Some(now);
            self.state.queue_event(EditorEvent::OnBufferEnter {
                target: PaneHandle::with_pane(now, self.view.focused_pane()),
            });
        }
    }

    /// Observation point for `on-mode-change`. `mode()` is a derived
    /// read of `EditorState.input` (`InputStack::mode`), with no write-site
    /// chokepoint of its own (a mode-layer push/truncate or an `Extend`
    /// flag flip can each change it), so this diffs it the same shape
    /// `detect_buffer_enter` uses above. Run every pass of
    /// `drain_pending_work`'s loop, not just once before it, so a
    /// handler-driven mode change (a picker `on_select` that enters Insert)
    /// is caught by the very next pass, and a transition that nets out
    /// within one pass (a `:cmd` body that enters and leaves Insert) is
    /// invisible, same as a dot-repeat replay, which never calls
    /// `settle()` between keys.
    fn detect_mode_change(&mut self) {
        let now = self.state.mode();
        if self.state.last_observed_mode != now {
            let from = self.state.last_observed_mode;
            self.state.last_observed_mode = now;
            self.state
                .queue_event(EditorEvent::OnModeChange { from, to: now });
        }
    }

    /// Observation point for `on-text-changed` (`EditorEvent::OnTextChanged`'s
    /// doc). `Buffer::install` (the one place every text mutation funnels
    /// through) has no path to `EditorState::queue_event`, so this diffs
    /// the buffer text's version instead, the same shape `detect_buffer_enter` uses
    /// for a value with no write-site chokepoint of its own. Run every pass
    /// of `drain_pending_work`'s loop, not just once before it, so a
    /// handler-driven edit is caught by the very next pass and multiple
    /// mutations to one buffer between two passes still announce as one
    /// event (`BufferStore::take_text_changed` advances the baseline as it
    /// reports).
    fn detect_text_changed(&mut self) {
        for buffer in self.state.buffers.take_text_changed() {
            self.state
                .queue_event(EditorEvent::OnTextChanged { buffer });
            if self.state.lsp_buffer_wants_pull(buffer) {
                self.debounce_diagnostic_pull(buffer);
            }
        }
    }

    /// Observation point for `on-undo-history-changed`
    /// (`EditorEvent::OnUndoHistoryChanged`'s doc), run beside
    /// [`Self::detect_text_changed`] with the same coalescing.
    fn detect_history_changed(&mut self) {
        for buffer in self.state.buffers.take_history_changed() {
            self.state
                .queue_event(EditorEvent::OnUndoHistoryChanged { buffer });
        }
    }

    /// Run one snapshot of `pending_work` in queued order: event handlers
    /// fire one event at a time, and a contiguous run of unguarded `Call`s
    /// of one origin goes to `run_steel_calls` as one batch before the next
    /// `Event` (or end of batch). Each call still runs in its own Steel
    /// session; the batch shares one host borrow and one application of
    /// the effects. A guarded `Call` runs alone through
    /// [`Self::run_guarded`]. `.` also runs a read's or pick's callback
    /// through here, at once (`Editor::finish_replayed_answers`).
    pub(in crate::editor) fn run_pending_batch(
        &mut self,
        mut items: std::collections::VecDeque<PendingWork>,
    ) {
        while let Some(item) = items.pop_front() {
            match item {
                PendingWork::Event(event) => {
                    // Any buffer-scoped event is queued from a point earlier
                    // than this drain (a live-buffer sweep, an LSP dispatch,
                    // a completion accept), but fires behind whatever `Call`
                    // items (timer thunks, async callbacks) or earlier
                    // `Event`s were already queued ahead of it in the same
                    // batch, and one of those can close the buffer first.
                    // Checked here, ahead of both `react_to_event` and
                    // `fire_one_event`, so neither ever sees a dead id.
                    // `OnBufferClose` is exempt: it is raised
                    // for an id that's already gone by design (see
                    // `buffer/lifecycle.rs`'s pairing check); checking it here
                    // would drop every `OnBufferClose` outright.
                    //
                    // A pane-carrying event (`OnBufferEnter`,
                    // `OnViewportChange`, `OnTriggerChar`,
                    // `OnCompletionAccept`) goes stale the same way: a
                    // handler earlier in this same batch (a timer's
                    // `pane-close`, a buffer switch) can close that pane or
                    // repoint it at another buffer before this event's own
                    // turn, and the builtin its handler goes on to call
                    // would otherwise raise on a pane it never asked for.
                    // A buffer-only event just needs its buffer still live;
                    // a pane-carrying one needs that pane to still show it:
                    // the same dual check `EditorEvent::handle`'s own doc
                    // describes.
                    let live = |handle: hume_scripting::PaneHandle| {
                        if handle.pane().is_some() {
                            CommandPane::resolve(&self.state, &self.view, handle).is_ok()
                        } else {
                            self.state.buffers.try_get(handle.buffer()).is_some()
                        }
                    };
                    if !matches!(event, EditorEvent::OnBufferClose { .. })
                        && let Some(handle) = event.handle()
                        && !live(handle)
                    {
                        continue;
                    }
                    self.react_to_event(&event);
                    self.with_edit_origin(EditOrigin::Background, |ed| ed.fire_one_event(event));
                }
                PendingWork::Call {
                    call,
                    guard: Some(guard),
                    origin,
                } => self.run_guarded(guard, origin, call),
                PendingWork::Call {
                    call,
                    guard: None,
                    origin,
                } => {
                    let mut calls = vec![call];
                    while matches!(
                        items.front(),
                        Some(PendingWork::Call {
                            guard: None,
                            origin: next,
                            ..
                        }) if *next == origin
                    ) {
                        let Some(PendingWork::Call { call, .. }) = items.pop_front() else {
                            unreachable!("front() just confirmed an unguarded Call")
                        };
                        calls.push(call);
                    }
                    self.with_edit_origin(origin, |ed| ed.run_call_batch(calls));
                }
            }
        }
    }

    /// Editor-internal reactions to an event: the Rust counterpart to Steel
    /// handlers, and the only `match` over `EditorEvent` that drives editor
    /// behaviour. Runs unconditionally, before `fire_one_event`
    /// and its `has_hook_handlers` early-exit: unlike a Steel handler, a
    /// Rust reaction has no registration to short-circuit on, and editor
    /// behaviour must not depend on whether a plugin happens to be
    /// installed.
    ///
    /// No `_` catch-all: this is the one match where forgetting
    /// a variant changes editor behaviour, not just a hook payload, so a new
    /// `EditorEvent` with no reaction wired must fail to compile here rather
    /// than silently doing nothing.
    fn react_to_event(&mut self, event: &EditorEvent) {
        match event {
            EditorEvent::OnBufferEnter { target } => self.enter_buffer_disk_check(target.buffer()),
            EditorEvent::OnFocusGained => self.check_all_disk_state(DiskCheckTrigger::Ambient),
            EditorEvent::OnBufferOpen { .. }
            | EditorEvent::OnBufferClose { .. }
            | EditorEvent::OnBufferSave { .. }
            | EditorEvent::OnModeChange { .. }
            | EditorEvent::OnLanguageSet { .. }
            | EditorEvent::OnLspAttach { .. }
            | EditorEvent::OnLspDetach { .. }
            | EditorEvent::OnDiagnosticsChanged { .. }
            | EditorEvent::OnViewportChange { .. }
            | EditorEvent::OnTriggerChar { .. }
            | EditorEvent::OnCompletionAccept { .. }
            | EditorEvent::OnOptionChange { .. }
            | EditorEvent::OnLspNotification { .. }
            | EditorEvent::OnTextChanged { .. }
            | EditorEvent::OnUndoHistoryChanged { .. } => {}
        }
    }

    /// Whether a handler is registered for `name` and `key`. Lazy event
    /// plugins activate first so their `register-hook!` calls land before
    /// the check.
    pub(in crate::editor) fn has_event_handlers(&mut self, name: &str, key: Option<&str>) -> bool {
        self.activate_lazy_event_plugins(name);
        self.scripting
            .as_ref()
            .is_some_and(|h| h.has_hook_handlers(name, key))
    }

    /// Fire every handler registered for one `EditorEvent`, if any are:
    /// the per-item body of `settle`'s `Event` arm.
    fn fire_one_event(&mut self, event: EditorEvent) {
        let name = event.name();
        let key = event.hook_key();
        if !self.has_event_handlers(name, key) {
            if let Some(message) = event.unhandled_trace() {
                self.report(Severity::Trace, message);
            }
            return;
        }
        // Built only once a handler is confirmed registered, so an event
        // nobody subscribes to never allocates a `SteelVal`.
        let key = key.map(str::to_owned);
        let args = event.into_steel_args();
        let result = {
            let (host_scr, mut impl_host) = self.scripting_with_host().expect("checked above");
            host_scr.fire_hook(name, key.as_deref(), &args, &mut impl_host)
        };
        // A hook body's own `call!` to an `#:inline-output` command is
        // closed here too.
        self.apply_script_result(result, "hook error: ");
    }

    /// Run one contiguous run of queued `Call` items through
    /// `run_steel_calls`: the per-batch body of `settle`'s `Call` arm.
    /// `false` when a call failed.
    fn run_call_batch(&mut self, calls: Vec<hume_scripting::SteelCall>) -> bool {
        let Some((host_scr, mut impl_host)) = self.scripting_with_host() else {
            return true;
        };
        let result = host_scr.run_steel_calls(calls, &mut impl_host);
        let ran = result.is_ok();
        self.apply_script_result(result, "steel call error: ");
        ran
    }

    /// Runs `call` alone, under `guard`. A guarded call is never grouped
    /// with a sibling: its check must run immediately before this call, not
    /// before an earlier sibling in the same batch has had the chance to
    /// change the state the check reads (a `switch-to-buffer!`, an edit),
    /// and what the guard holds is released right after this call alone.
    fn run_guarded(
        &mut self,
        guard: CallGuard,
        origin: EditOrigin,
        call: hume_scripting::SteelCall,
    ) {
        match guard {
            CallGuard::Reply { anchor, hold } => {
                if anchor.is_none_or(|anchor| self.state.anchor_admits(&self.view, &anchor)) {
                    self.with_edit_origin(origin, |ed| ed.run_call_batch(vec![call]));
                }
                drop(hold);
            }
            CallGuard::Answer(ticket) => {
                self.with_edit_origin(origin, |ed| {
                    ed.run_answered(Some(ticket), |ed| {
                        ed.run_call_batch(vec![call]);
                    });
                });
            }
            CallGuard::Job(id) => {
                if !self.state.config.async_jobs.contains_key(&id) {
                    return;
                }
                // A callback that fails once would fail on every later
                // report, so a raise ends the job.
                if self.with_edit_origin(origin, |ed| ed.run_call_batch(vec![call])) {
                    self.forget_job_callback(id);
                } else {
                    self.state.config.async_jobs.remove(&id);
                }
            }
            CallGuard::Timer(id) => {
                if self.timers.take_queued(id) {
                    self.with_edit_origin(origin, |ed| ed.run_call_batch(vec![call]));
                }
            }
            CallGuard::Invocation(id) => {
                if !self.with_edit_origin(origin, |ed| ed.run_call_batch(vec![call])) {
                    self.state.end_completion_request(&self.view, id);
                }
            }
        }
    }

    /// Releases what `guard` holds for a call dropped unrun, as running the
    /// call would have.
    fn drop_unrun(&mut self, guard: CallGuard) {
        match guard {
            CallGuard::Reply { .. } | CallGuard::Answer(_) => {}
            CallGuard::Job(id) => self.forget_job_callback(id),
            CallGuard::Timer(id) => {
                self.timers.take_queued(id);
            }
            CallGuard::Invocation(id) => self.state.end_completion_request(&self.view, id),
        }
    }

    /// Run `f` with the edits it makes counted as `origin`'s, then put back
    /// the origin before it.
    pub(in crate::editor) fn with_edit_origin<R>(
        &mut self,
        origin: EditOrigin,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let outer = self.state.buffers.set_edit_origin(origin);
        let result = f(self);
        self.state.buffers.set_edit_origin(outer);
        result
    }
}
