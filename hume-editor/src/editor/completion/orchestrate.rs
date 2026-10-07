//! The one driver of a completion session, for both targets: picks the
//! sources a trigger applies to, mints each an [`Invocation`] against the
//! live document, runs it (a native source inline, a Steel one via the
//! deferred-call queue), lands answers, reacts to edits, and decides when a
//! session has nothing left to show. Everything here is `impl EditorState`
//! (not `Editor`), since every input it needs (the input stack, the source
//! registry, the buffers, the Steel call queue) lives there, so it is
//! reachable from `EditorHostImpl` (`completion-emit!`) and from the key
//! handlers alike, and never from inside a Steel eval's own borrow.
//!
//! Steel sources are only ever *queued* (`queue_steel_call`), never called
//! inline, same as the picker's live source and every other Rust→Steel
//! callback. A trigger is user-intent frequency, so the per-keystroke work
//! (`rank`) stays here in Rust and only a source flagged `isIncomplete` is
//! called again as the user types.

use hume_engine::pipeline::{BufferId, EngineView};
use hume_rope::offset::CharOffset;
use steel::rvals::SteelVal;

use super::registry::{BufferSourceId, MinibufBody, MinibufSourceId, SourceRegistry};
use super::session::{BufferSession, Invocation, LiveDoc, MinibufSession, Reconciled};
use super::{CompletionCtx, CompletionItem, arg_span};
use crate::editor::buffer::store::BufferStore;
use crate::editor::input_stack::{
    BufferCompletionLayer, InsertLayer, LayerRef, MinibufCompletionLayer,
};
use crate::editor::registry::CommandRegistry;
use crate::editor::settings::{CommandCompletion, EditorSettings};
use crate::editor::{EditorState, Severity};

/// What set an Insert-mode trigger in motion, and so which sources it
/// invokes.
pub(in crate::editor) enum Trigger {
    /// Ctrl-Space / the `completion-trigger` command: every `Buffer` source.
    Explicit,
    /// A char landed in Insert mode: these `Buffer` sources
    /// `(set-completion-triggers! …)` registered it for
    /// (`EditorState::triggered_by`). Separate from `set-hook-triggers!`'s
    /// sets, which only feed the generic `on-trigger-char` hook.
    Sources(Vec<BufferSourceId>),
}

/// A Steel proc plus its args, minted while a session is borrowed and
/// queued once it isn't: `queue_steel_call` needs the whole `EditorState`.
type SteelCall = (SteelVal, Vec<SteelVal>);

impl EditorState {
    // ── Buffer target ────────────────────────────────────────────────────────

    /// Opens (or re-invokes into) the Insert-mode completion session on the
    /// focused buffer. A session already open on this buffer survives:
    /// its sources are called again, superseding their earlier answers
    /// once the new ones land, so the menu never blinks empty. It is
    /// re-pushed as a *fresh* layer: `completion_input_buffer` dismisses
    /// the layer it dispatched a bound command through once that command
    /// returns, and a trigger is exactly such a command.
    pub(in crate::editor) fn trigger_buffer_completion(
        &mut self,
        view: &EngineView,
        trigger: Trigger,
    ) {
        if !self.input.is::<InsertLayer>(self.input.mode_layer()) {
            self.report(
                Severity::Info,
                "completion-trigger: only in Insert mode".to_string(),
            );
            return;
        }
        let (ids, explicit) = match trigger {
            Trigger::Explicit => (self.config.completion_sources.buffer_sources(), true),
            Trigger::Sources(ids) => (ids, false),
        };
        if ids.is_empty() {
            if explicit {
                self.report(
                    Severity::Info,
                    "no completion sources registered".to_string(),
                );
            }
            return;
        }
        let pid = self.focus.id();
        let Some(bid) = view.panes.get(pid).map(|p| p.buffer_id) else {
            return;
        };
        let Some(head) = self
            .focused_buffer_state(bid)
            .map(|pbs| pbs.view(self.buffers.get(bid).text()).primary().head())
        else {
            return;
        };
        let mut session = match self.take_buffer_completion(view) {
            Some(open) => open,
            None => BufferSession::open(
                bid,
                pid,
                self.buffers.get(bid).text().version(),
                head.offset(),
            ),
        };
        if explicit {
            session.mark_explicit_trigger();
        }
        let calls = invoke_buffer_sources(
            &self.config.completion_sources,
            &self.buffers,
            &self.settings,
            &mut session,
            &ids,
            bid,
            head.offset(),
        );
        self.queue_steel_calls(calls);
        session.rank(
            &self.config.completion_sources,
            Some(LiveDoc {
                text: self.buffers.get(bid).text(),
                head: head.offset(),
            }),
        );
        if session.is_spent() {
            self.report(Severity::Info, "no completions".to_string());
            return;
        }
        self.push_layer(view, BufferCompletionLayer { session });
    }

    /// Brings the open `Buffer` session in line with the buffer's current
    /// text and the primary cursor, whatever changed them: dismisses it when
    /// its pane or buffer is gone or its text was replaced, drops the
    /// answers whose token the cursor has left, calls again every source
    /// that flagged its last answer `isIncomplete` (or is still pending
    /// against an older document) once the text changed, and re-ranks.
    /// Idempotent, so every path that needs a current session calls it
    /// without coordinating with the others: the Insert keystroke handler,
    /// each settle pass, and taking the session out of the input stack.
    pub(in crate::editor) fn reconcile_completion(&mut self, view: &EngineView) {
        let Some(session) = self.input.buffer_completion() else {
            return;
        };
        let (bid, pid) = (session.bid(), session.pane_id());
        let shown = self.focus.id() == pid && view.panes.get(pid).map(|p| p.buffer_id) == Some(bid);
        let Some(buf) = self.buffers.try_get(bid).filter(|_| shown) else {
            self.dismiss_completion(view);
            return;
        };
        let Some(head) = self
            .focused_buffer_state(bid)
            .map(|pbs| pbs.view(buf.text()).primary().head().offset())
        else {
            self.dismiss_completion(view);
            return;
        };
        let word_chars = buf.overrides.word_chars(&self.settings);
        let session = self
            .input
            .buffer_completion_mut()
            .expect("found above and not removed since");
        let outcome = session.reconcile(
            &self.config.completion_sources,
            buf.text(),
            head,
            word_chars,
        );
        match outcome {
            Reconciled::Dismiss => self.dismiss_completion(view),
            Reconciled::Unchanged => {}
            Reconciled::Changed { text_changed } => {
                if text_changed {
                    let to_reinvoke = session.sources_to_reinvoke();
                    let calls = invoke_buffer_sources(
                        &self.config.completion_sources,
                        &self.buffers,
                        &self.settings,
                        session,
                        &to_reinvoke,
                        bid,
                        head,
                    );
                    self.queue_steel_calls(calls);
                }
                // Typed out of every token, and nothing on its way: silent,
                // since the user left, nothing "failed".
                self.settle_buffer_completion(view, false);
            }
        }
    }

    // ── Minibuffer target ────────────────────────────────────────────────────

    /// The `:` line's current target command name, if the minibuf is open
    /// on a `:` prompt and the cursor sits past it (in its argument).
    /// `Editor::activate_minibuf_completion_target` reads this before
    /// calling [`Self::trigger_minibuf_completion`], to activate a still-
    /// `TypedBody::Lazy` owner (see that method's own doc) before the
    /// resolve below runs.
    pub(in crate::editor) fn minibuf_target_command(&self) -> Option<String> {
        let mb = self.input.minibuf()?;
        if mb.prompt != ":" {
            return None;
        }
        let (name, _) = target_command_name(&mb.input, mb.cursor)?;
        Some(name.to_owned())
    }

    /// The first Tab on the `:` line: resolves the one source the input
    /// shape names (the command name itself, or the command's declared
    /// argument completer), runs it, and applies the `:` line's own
    /// eager policy ([`Self::settle_minibuf_session`]).
    pub(in crate::editor) fn trigger_minibuf_completion(&mut self, view: &EngineView) {
        let Some(mb) = self.input.minibuf() else {
            return;
        };
        if mb.prompt != ":" {
            return;
        }
        let (input, cursor) = (mb.input.clone(), mb.cursor);
        let Some((name, floor)) = resolve_minibuf_source(&self.config.registry, &input, cursor)
        else {
            return;
        };
        let Some(id) = self.config.completion_sources.minibuf_id_of(&name) else {
            // `TypedCommand.completer` naming no registered source, a stale
            // name after a rename. Silent to the user, same as `:bd`
            // declaring no completer at all; loud enough to find in the log.
            self.report(
                Severity::Trace,
                format!("no completion source named {name:?}"),
            );
            return;
        };
        let mut session = MinibufSession::open(input.clone(), cursor);
        let ctx = CompletionCtx {
            registry: &self.config.registry,
            buffers: &self.buffers,
            cwd: &self.cwd,
            languages: &self.config.languages,
            dirs: &self.dirs,
        };
        let call = invoke_minibuf_source(
            &self.config.completion_sources,
            &ctx,
            &mut session,
            id,
            &input,
            cursor,
            floor,
        );
        if let Some((proc, args)) = call {
            self.queue_steel_call(proc, args);
        }
        let r = self.push_layer(view, MinibufCompletionLayer { session });
        self.settle_minibuf_session(view, r);
    }

    /// The `:` line's eager policy, once every source has answered: nothing
    /// → the popup never shows; one candidate → applied silently, popup
    /// gone; two or more → under `CommonPrefix` the candidates' common
    /// prefix is applied and nothing is picked, under `FirstCandidate` the
    /// first is picked and applied. The popup stays for Tab to cycle. Runs
    /// at open (native sources answer inline) and again when a pending Steel
    /// source's answer lands, including a second answer to a
    /// still-streaming source, so the re-rank (which resets the selection
    /// itself, per `SlotSet::rank_with`'s own contract) always runs first:
    /// the previous selection index has no guaranteed meaning against the
    /// new order, and may point past a narrower list's end entirely.
    fn settle_minibuf_session(&mut self, view: &EngineView, r: LayerRef) {
        let first_candidate = self.settings.command_completion == CommandCompletion::FirstCandidate;
        let Some(layer) = self.input.at_mut::<MinibufCompletionLayer>(r) else {
            return;
        };
        layer.session.rank(&self.config.completion_sources);
        if layer.session.is_pending() {
            return;
        }
        match layer.session.len() {
            0 => self.dismiss_completion(view),
            1 => {
                layer.session.step_selection(true);
                self.apply_minibuf_candidate(r);
                self.dismiss_completion(view);
            }
            _ => {
                if first_candidate {
                    layer.session.step_selection(true);
                }
                self.apply_minibuf_candidate(r);
            }
        }
    }

    /// Splices the session's proposed edit at `r` into the `:` line, over
    /// its own source's token, restoring the input the sources saw first,
    /// so cycling from one candidate to the next never has to know what the
    /// previous one left behind. With no proposal the line is left as typed.
    pub(in crate::editor) fn apply_minibuf_candidate(&mut self, r: LayerRef) {
        let Some(layer) = self.input.at::<MinibufCompletionLayer>(r) else {
            return;
        };
        let Some((span, text)) = layer.session.proposed_apply() else {
            return;
        };
        let (input, text) = (layer.session.input().to_owned(), text.into_owned());
        let Some(mb) = self.input.minibuf_mut() else {
            return;
        };
        mb.input = input;
        mb.splice(span, &text);
    }

    // ── Answers ──────────────────────────────────────────────────────────────

    /// A source's answer to invocation `id`: `completion-emit!`. `false`
    /// when no open session has that invocation as a slot's latest call
    /// (superseded, replaced, or dismissed since: expected-normal for a late
    /// async source). The session is untouched, so it is settled only on
    /// `true`: a dropped answer changed nothing, and settling anyway would
    /// still reset the menu's selection out from under a user who has since
    /// Tabbed to a row an unrelated, superseded call has no bearing on.
    pub(in crate::editor) fn contribute(
        &mut self,
        view: &EngineView,
        id: u64,
        items: Vec<CompletionItem>,
        incomplete: bool,
    ) -> bool {
        if self.input.ref_of::<BufferCompletionLayer>().is_some() {
            let session = self
                .input
                .buffer_completion_mut()
                .expect("ref_of found the layer");
            let landed = session.contribute(&self.config.completion_sources, id, items, incomplete);
            if landed {
                // Silent for a trigger-char session the user never
                // explicitly asked for completion on, same discipline as
                // the `Explicit`-only report at this file's own
                // `trigger_buffer_completion` (`ids.is_empty()`'s `explicit`
                // guard).
                self.settle_buffer_completion(view, true);
            }
            return landed;
        }
        let Some(r) = self.input.ref_of::<MinibufCompletionLayer>() else {
            return false;
        };
        let session = self
            .input
            .minibuf_completion_mut()
            .expect("ref_of found the layer");
        let landed = session.contribute(id, items, incomplete);
        if landed {
            self.settle_minibuf_session(view, r);
        }
        landed
    }

    /// Recovery for a Steel call batch that failed: drops every completion
    /// invocation still `Pending`, since nothing will ever call
    /// `completion-emit!` for it now; see `SlotSet::drop_stalled`'s own
    /// doc for why this is always safe. A no-op with no completion session
    /// open, or when nothing was actually pending.
    pub(in crate::editor) fn settle_completion_after_call_failure(&mut self, view: &EngineView) {
        if let Some(session) = self.input.buffer_completion_mut() {
            if session.drop_stalled_invocations() {
                self.settle_buffer_completion(view, false);
            }
            return;
        }
        let Some(r) = self.input.ref_of::<MinibufCompletionLayer>() else {
            return;
        };
        let session = self
            .input
            .minibuf_completion_mut()
            .expect("ref_of found the layer");
        if session.drop_stalled_invocations() {
            self.settle_minibuf_session(view, r);
        }
    }

    /// Re-ranks the open `Buffer` session and dismisses it if it's now
    /// spent. Every path that lands new information into it (`contribute`,
    /// `reconcile_completion`, a failed call batch) ends here.
    /// `report_empty`: report "no completions" if the session dismisses as
    /// spent and was ever explicitly triggered. Only `contribute` wants
    /// this: a raw edit narrowing to nothing, or a call-batch failure,
    /// isn't the user "asking and getting nothing".
    fn settle_buffer_completion(&mut self, view: &EngineView, report_empty: bool) {
        let explicit = self
            .input
            .buffer_completion()
            .is_some_and(BufferSession::is_explicit);
        let live = self.input.buffer_completion().and_then(|s| {
            let bid = s.bid();
            self.focused_buffer_state(bid)
                .map(|pbs| (bid, pbs.view(self.buffers.get(bid).text()).primary().head()))
        });
        let Some(session) = self.input.buffer_completion_mut() else {
            return;
        };
        let live = live.map(|(bid, head)| LiveDoc {
            text: self.buffers.get(bid).text(),
            head: head.offset(),
        });
        session.rank(&self.config.completion_sources, live);
        if self
            .input
            .buffer_completion()
            .is_some_and(BufferSession::is_spent)
        {
            self.dismiss_completion(view);
            if report_empty && explicit {
                self.report(Severity::Info, "no completions".to_string());
            }
        }
    }

    /// Queues every `(proc, args)` pair `invoke_buffer_sources` returned.
    fn queue_steel_calls(&mut self, calls: Vec<SteelCall>) {
        for (proc, args) in calls {
            self.queue_steel_call(proc, args);
        }
    }
}

/// Mints one invocation per source in `ids` into `session` and returns the
/// Steel calls to queue. Each source's token is the run before the cursor
/// of its own token characters (`BufferSourceEntry::token_chars_over`). Every `Buffer` source is Steel, by design (see
/// `registry.rs`'s module doc). Takes the fields it needs rather than
/// `&mut EditorState` so a caller can hand it a session still borrowed
/// from the input stack.
fn invoke_buffer_sources(
    sources: &SourceRegistry,
    buffers: &BufferStore,
    settings: &EditorSettings,
    session: &mut BufferSession,
    ids: &[BufferSourceId],
    bid: BufferId,
    head: CharOffset,
) -> Vec<SteelCall> {
    let buf = buffers.get(bid);
    let text = buf.text();
    let word_chars = buf.overrides.word_chars(settings);
    let pane = hume_scripting::PaneHandle::with_pane(bid, session.pane_id());
    ids.iter()
        .map(|&id| {
            let entry = sources.buffer_get(id);
            let start = entry.token_start(text, head, word_chars);
            let invocation = Invocation::buffer(text.rope().clone(), start);
            let prefix = invocation.prefix(text, head);
            let invocation_id = session.invoke(id, invocation);
            (
                entry.proc.clone(),
                vec![
                    SteelVal::IntV(invocation_id as isize),
                    hume_scripting::SteelPane::new(pane).into_steel_val(),
                    SteelVal::StringV(prefix.into()),
                ],
            )
        })
        .collect()
}

/// Mints an invocation of `id` into `session` and runs it: a native source
/// answers inline, a Steel one returns the call to queue.
///
/// `NativeDelegated`'s own span comes from calling its function first: the
/// only body variant whose span isn't the generic `'arg` one, since its
/// candidate universe *is* the live input and the two are computed
/// together. Every other body gets the whitespace-delimited argument span
/// computed here, upfront, the same way a `Buffer` source's span is always
/// the word before the cursor (`invoke_buffer_sources`).
/// `floor` is the byte offset the `'arg` span's backward scan must not
/// cross: 0 while still completing the command name itself, or the
/// command name's own end once `resolve_minibuf_source` has resolved past
/// it. Without this, `arg_span`'s "last space before the cursor" rule has
/// nothing to anchor on for a no-space argument (`:b1`, the alias `b`
/// immediately followed by its argument) and falls back to `start = 0`,
/// swallowing the command name itself into the span it hands the resolved
/// completer.
fn invoke_minibuf_source(
    sources: &SourceRegistry,
    ctx: &CompletionCtx<'_>,
    session: &mut MinibufSession,
    id: MinibufSourceId,
    input: &str,
    cursor: usize,
    floor: usize,
) -> Option<SteelCall> {
    let entry = sources.minibuf_get(id);
    let arg = || {
        let span = arg_span(input, cursor, ' ', &[' ']);
        span.start.max(floor)..span.end
    };
    match &entry.body {
        MinibufBody::NativeUniverse(f) => {
            let invocation_id = session.invoke(id, Invocation::minibuf(arg()));
            session.contribute(invocation_id, f(ctx), false);
            None
        }
        MinibufBody::NativeDelegated(f) => {
            let (span, items) = f(input, cursor, ctx);
            let invocation_id = session.invoke(id, Invocation::minibuf(span));
            session.contribute(invocation_id, items, false);
            None
        }
        MinibufBody::Steel(proc) => {
            let invocation_id = session.invoke(id, Invocation::minibuf(arg()));
            Some((
                proc.clone(),
                vec![
                    SteelVal::IntV(invocation_id as isize),
                    SteelVal::StringV(input.into()),
                    SteelVal::IntV(cursor as isize),
                ],
            ))
        }
    }
}

/// The `:` line's own command name and the byte offset one past it (and
/// its optional trailing `!`). `None` if the cursor hasn't moved past the
/// name yet, still typing it. Uses the same name-shape rule
/// `execute_command` itself uses
/// (`input_stack::command::scan_command_name`), so completion never picks
/// a different command than Enter would actually run.
fn target_command_name(input: &str, cursor: usize) -> Option<(&str, usize)> {
    let (cmd, _, cmd_end) = crate::editor::input_stack::command::scan_command_name(input);
    (cursor > cmd_end).then_some((cmd, cmd_end))
}

/// Resolves which registered source applies to the current `(input,
/// cursor)` shape (the command name itself while the cursor is within it
/// (no space yet, or moved left past the space), else the resolved
/// command's declared argument completer) and the byte offset
/// [`invoke_minibuf_source`]'s own argument span must not cross: `0` for
/// the command-name case (the whole prefix typed so far is the token), the
/// name's own end for the argument case.
fn resolve_minibuf_source(
    registry: &CommandRegistry,
    input: &str,
    cursor: usize,
) -> Option<(std::borrow::Cow<'static, str>, usize)> {
    use std::borrow::Cow;
    match target_command_name(input, cursor) {
        None => Some((Cow::Borrowed(super::COMMAND_SOURCE), 0)),
        Some((cmd, cmd_end)) => Some((registry.get_typed(cmd)?.completer.clone()?, cmd_end)),
    }
}
