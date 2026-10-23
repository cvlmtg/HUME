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
//! Steel sources are only ever *queued* (`queue_source_calls`), never called
//! inline, same as the picker's live source and every other Rust→Steel
//! callback. A trigger is user-intent frequency, so the per-keystroke work
//! (`rank`) stays here in Rust and only a source flagged `isIncomplete` is
//! called again as the user types.

use hume_engine::pipeline::{BufferId, EngineView, PaneId};
use hume_rope::offset::CharOffset;
use hume_scripting::SteelCall;
use hume_scripting::host::{AnswerEnd, EmittedWord};
use steel::rvals::SteelVal;

use super::registry::{BufferSourceId, MinibufBody, MinibufSourceId, SourceRegistry};
use super::session::{
    BufferSession, EndingSession, Invocation, LiveDoc, MinibufSession, Reconciled,
};
use super::{CompletionCtx, CompletionItem, arg_span};
use crate::editor::buffer::store::BufferStore;
use crate::editor::input_stack::{BufferCompletionLayer, LayerRef, MinibufCompletionLayer};
use crate::editor::lsp::{RustDelivery, RustResponder};
use crate::editor::minibuf::PromptCompletion;
use crate::editor::registry::CommandRegistry;
use crate::editor::replay::Interactive;
use crate::editor::settings::{CommandCompletion, EditorSettings};
use crate::editor::{EditorState, Severity};

/// The supersede key of the selected candidate's documentation request.
const COMPLETION_DOC_SUPERSEDE: &str = "completion-doc";

/// What set an Insert-mode trigger in motion, and so which sources it
/// invokes.
pub(in crate::editor) enum Trigger {
    /// Ctrl-Space / the `completion-trigger` command: every `Buffer` source.
    /// An interactive input, so it carries the proof it may open now.
    Explicit(Interactive),
    /// A char landed in Insert mode: these `Buffer` sources
    /// `(set-completion-triggers! …)` registered it for
    /// (`EditorState::triggered_by`). Separate from `set-hook-triggers!`'s
    /// sets, which only feed the generic `on-trigger-char` hook.
    Sources(Vec<BufferSourceId>),
}

impl EditorState {
    /// Retires whichever completion layer is open, wherever it is on the
    /// stack: a completion layer can sit under a `Popup`, and a
    /// pop-if-top rule would leave a stale session behind one. At most one
    /// of the two is ever actually open; `retire` no-ops on the other. A
    /// no-op when no session is open at all.
    pub(in crate::editor) fn dismiss_completion(&mut self, view: &EngineView) {
        self.retire::<crate::editor::input_stack::BufferCompletionLayer>(view);
        self.retire::<crate::editor::input_stack::MinibufCompletionLayer>(view);
    }

    /// [`Self::dismiss_completion`]'s variant for the two accept paths and
    /// the trigger that reuses an open session, which need the session *by
    /// value* rather than merely retired. The session is reconciled with
    /// the buffer first, so a session that no longer applies is dismissed
    /// (`None`) instead of handed out. A session held by value is not
    /// carried through text changes: the caller edits with it in hand only
    /// to drop it, or puts it back before the next edit.
    /// [`Self::take_layer`] handles the truncate-and-pull-out and never runs
    /// the layer's own `tear_down` (only whatever was pushed above it), so
    /// this closes the session's views itself. `Buffer`-target only: both
    /// accept paths only ever act on one. `None` when no `Buffer` session is
    /// open.
    pub(in crate::editor) fn take_buffer_completion(
        &mut self,
        view: &EngineView,
    ) -> Option<super::BufferSession> {
        self.reconcile_completion(view);
        let r = self
            .input
            .ref_of::<crate::editor::input_stack::BufferCompletionLayer>()?;
        let completion =
            self.take_layer::<crate::editor::input_stack::BufferCompletionLayer>(view, r);
        self.close_buffer_completion_views();
        Some(completion.session)
    }

    /// [`Self::take_buffer_completion`] for an accept, which ends the
    /// session: each request it still awaits gets its source's `#:cancel`.
    pub(in crate::editor) fn take_buffer_completion_to_accept(
        &mut self,
        view: &EngineView,
    ) -> Option<super::BufferSession> {
        let mut session = self.take_buffer_completion(view)?;
        self.end_completion_session(&mut session);
        Some(session)
    }

    /// Queues the `#:cancel` calls of `session`, which is ending: for every
    /// request it still owes an answer to, and those that ended before.
    /// Every path that ends a session calls this: each completion layer's
    /// `tear_down`, and an accept.
    pub(in crate::editor) fn end_completion_session(&mut self, session: &mut impl EndingSession) {
        let cancels = session.end_requests(&self.config.completion_sources);
        self.queue_steel_calls(cancels);
    }

    /// Queues the `#:cancel` calls for every request the open session has
    /// stopped waiting on since the last call: superseded by a newer call,
    /// left by the cursor, or ended by a failed source call. A session leaving the input
    /// stack queues its own, in its layer's `tear_down`.
    pub(in crate::editor) fn queue_completion_cancels(&mut self) {
        let sources = &self.config.completion_sources;
        let mut cancels = Vec::new();
        if let Some(session) = self.input.buffer_completion_mut() {
            cancels.extend(session.take_cancels(sources));
        }
        if let Some(session) = self.input.minibuf_completion_mut() {
            cancels.extend(session.take_cancels(sources));
        }
        self.queue_steel_calls(cancels);
    }

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
        let Some(insert) = self.input.insert_session() else {
            self.report(
                Severity::Info,
                "completion-trigger: only in Insert mode".to_string(),
            );
            return;
        };
        let (pid, bid) = (insert.pane(), insert.buffer());
        let (ids, explicit) = match trigger {
            Trigger::Explicit(_) => (self.config.completion_sources.buffer_sources(), true),
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
        let head = self
            .panes
            .entries
            .seeded(pid, bid)
            .view(self.buffers.get(bid).text())
            .primary()
            .head();
        let mut session = match self.take_buffer_completion(view) {
            Some(open) => open,
            None => BufferSession::open(self.buffers.get(bid).text().version(), head.offset()),
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
            (pid, bid),
            head.offset(),
        );
        self.queue_source_calls(calls);
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
        self.push_layer(view, BufferCompletionLayer::new(session));
    }

    /// Brings the open `Buffer` session in line with the buffer's current
    /// text and the primary cursor, whatever changed them: dismisses it when
    /// its text was replaced or its buffer closed, drops the
    /// answers whose token the cursor has left, calls again every source
    /// that flagged its last answer `isIncomplete` (or is still pending
    /// against an older document) once the text changed, and re-ranks.
    /// Idempotent, so every path that needs a current session calls it
    /// without coordinating with the others: the Insert keystroke handler,
    /// each settle pass, and taking the session out of the input stack.
    pub(in crate::editor) fn reconcile_completion(&mut self, view: &EngineView) {
        let Some((pid, bid)) = self.input.completion_owner() else {
            return;
        };
        let buf = self.buffers.get(bid);
        let head = self
            .panes
            .entries
            .seeded(pid, bid)
            .view(buf.text())
            .primary()
            .head()
            .offset();
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
                        (pid, bid),
                        head,
                    );
                    self.queue_source_calls(calls);
                }
                // Typed out of every token, and nothing on its way: silent,
                // since the user left, nothing "failed".
                self.settle_buffer_completion(view, false);
            }
        }
    }

    /// Clears a `Buffer` session's menu and documentation box and cancels
    /// its documentation request, for a session leaving the input stack:
    /// nothing would read the answer.
    pub(in crate::editor) fn close_buffer_completion_views(&mut self) {
        self.views.completion_menu.set(None);
        self.views.completion_doc.set(None);
        self.cancel_superseded(COMPLETION_DOC_SUPERSEDE);
    }

    /// Asks the selected candidate's server for its documentation, through
    /// `completionItem/resolve`, unless it has it already or was asked since it
    /// was selected ([`BufferSession::doc_request`]), or the last frame found
    /// no room for the box. Docs carry no buffer position, so the reply is
    /// delivered even after further typing. Sent from the settle pass only:
    /// accepting or reusing a session takes it out before its reply could
    /// matter.
    pub(in crate::editor) fn request_selected_completion_doc(&mut self) {
        if !self.settings.completion_docs {
            return;
        }
        let Some((_, bid)) = self.input.completion_owner() else {
            return;
        };
        let Some(session) = self
            .input
            .find_mut::<BufferCompletionLayer>()
            .filter(|layer| layer.doc_box_fits())
            .map(|layer| &mut layer.session)
        else {
            return;
        };
        let Some(request) = session.doc_request(&self.config.completion_sources) else {
            return;
        };
        let (key, version) = (request.key, request.version);
        let respond: RustResponder = Box::new(move |state, _view, result| {
            let Some(session) = state.input.buffer_completion_mut() else {
                return;
            };
            match result {
                Ok(reply) => session.finish_doc_request(key, version, reply),
                Err(message) => {
                    state.report(Severity::Trace, format!("lsp completion docs: {message}"));
                }
            }
        });
        self.lsp_request_json(
            bid,
            request.server,
            "completionItem/resolve",
            request.params,
            RustDelivery {
                allow_stale: true,
                supersede: Some(COMPLETION_DOC_SUPERSEDE),
            },
            respond,
        );
    }

    // ── Minibuffer target ────────────────────────────────────────────────────

    /// The `:` line's current target command name, if the open minibuffer
    /// is the `:` line and the cursor sits past it (in its argument).
    /// `Editor::activate_minibuf_completion_target` reads this before
    /// calling [`Self::trigger_minibuf_completion`], to activate a still-
    /// `TypedBody::Lazy` owner (see that method's own doc) before the
    /// resolve below runs.
    pub(in crate::editor) fn minibuf_target_command(&self) -> Option<String> {
        let mb = self.input.minibuf()?;
        if mb.completion != Some(PromptCompletion::CommandLine) {
            return None;
        }
        let (name, _) = target_command_name(&mb.input, mb.cursor)?;
        Some(name.to_owned())
    }

    /// The first Tab on a minibuffer line that completes: resolves the one
    /// source the line and its input shape name, runs it, and applies the
    /// minibuffer's eager policy ([`Self::settle_minibuf_session`]).
    pub(in crate::editor) fn trigger_minibuf_completion(&mut self, view: &EngineView) {
        let Some(mb) = self.input.minibuf() else {
            return;
        };
        let Some(completion) = mb.completion else {
            return;
        };
        let (input, cursor) = (mb.input.clone(), mb.cursor);
        let Some((name, floor)) =
            resolve_minibuf_source(&self.config.registry, completion, &input, cursor)
        else {
            return;
        };
        let Some(id) = self.config.completion_sources.minibuf_id_of(&name) else {
            // On the `:` line this is a command's completer naming no
            // registered source, a stale name after a rename: silent to the
            // user, same as `:bd` declaring no completer at all. A line that
            // names its own source has no other completion, so its Tab
            // failing is the user's to see.
            let severity = match completion {
                PromptCompletion::CommandLine => Severity::Trace,
                PromptCompletion::Source(_) => Severity::Warning,
            };
            self.report(severity, format!("no completion source named {name:?}"));
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
        self.queue_source_calls(call);
        let r = self.push_layer(view, MinibufCompletionLayer { session });
        self.settle_minibuf_session(view, r);
    }

    /// The minibuffer's eager policy, once every source has answered in
    /// full: nothing → the popup never shows; one candidate → applied,
    /// popup gone; two or more → under `CommonPrefix` the
    /// candidates' common prefix is applied and nothing is picked, under
    /// `FirstCandidate` the first is picked and applied. The popup stays for
    /// Tab to cycle. Runs at open (native sources answer inline) and again
    /// when a pending Steel source's answer or part of one lands, so the
    /// re-rank always runs first: a previous selection index has no meaning
    /// against the new order. While a source still owes an answer the popup
    /// shows what has landed. A pick kept across the parts of an answer
    /// skips the policy: the line already holds that candidate.
    fn settle_minibuf_session(&mut self, view: &EngineView, r: LayerRef) {
        let first_candidate = self.settings.command_completion == CommandCompletion::FirstCandidate;
        let Some(layer) = self.input.at_mut::<MinibufCompletionLayer>(r) else {
            return;
        };
        layer.session.rank(&self.config.completion_sources);
        if layer.session.is_pending() || layer.session.selected().is_some() {
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

    /// Splices the session's proposed edit at `r` into the minibuffer, over
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

    /// A source's answer, or part of one (`end`), to invocation `id`:
    /// `completion-emit!`. `false` when no source in an open session still
    /// owes an answer to `id` (superseded, answered in full, replaced, or
    /// dismissed since: expected-normal for a late async source). The session is untouched, so it is settled only on
    /// `true`: a dropped answer changed nothing, and settling anyway would
    /// still reset the menu's selection out from under a user who has since
    /// Tabbed to a row an unrelated, superseded call has no bearing on.
    /// `word` is for a minibuffer session only: `Err` when a buffer session
    /// awaits `id`, or when [`MinibufSession::emit`] refuses it.
    pub(in crate::editor) fn contribute(
        &mut self,
        view: &EngineView,
        id: u64,
        items: Vec<CompletionItem>,
        incomplete: bool,
        end: AnswerEnd,
        word: EmittedWord,
    ) -> Result<bool, String> {
        if self.input.ref_of::<BufferCompletionLayer>().is_some() {
            let session = self
                .input
                .buffer_completion_mut()
                .expect("ref_of found the layer");
            if word != EmittedWord::default() && session.awaits(id) {
                return Err(
                    "completion-emit!: #:span and #:prefix are for a 'minibuf source; a 'buffer \
                     source's token is the word before the cursor"
                        .to_string(),
                );
            }
            let landed =
                session.contribute(&self.config.completion_sources, id, items, incomplete, end);
            if landed {
                // Silent for a trigger-char session the user never
                // explicitly asked for completion on, same discipline as
                // the `Explicit`-only report at this file's own
                // `trigger_buffer_completion` (`ids.is_empty()`'s `explicit`
                // guard).
                self.settle_buffer_completion(view, true);
            }
            return Ok(landed);
        }
        let Some(r) = self.input.ref_of::<MinibufCompletionLayer>() else {
            return Ok(false);
        };
        let session = self
            .input
            .minibuf_completion_mut()
            .expect("ref_of found the layer");
        let landed = session.emit(
            &self.config.completion_sources,
            id,
            items,
            incomplete,
            end,
            word,
        )?;
        if landed {
            self.settle_minibuf_session(view, r);
        }
        Ok(landed)
    }

    /// Ends completion request `id`, whose source call raised or was
    /// dropped unrun (`CallGuard::Invocation`), and settles the session
    /// that owed it, without its "no completions" report: the user asked,
    /// but the source failed. A no-op when no open session owes an answer
    /// to `id`.
    pub(in crate::editor) fn end_completion_request(&mut self, view: &EngineView, id: u64) {
        if let Some(session) = self.input.buffer_completion_mut() {
            if session.end_request(id) {
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
        if session.end_request(id) {
            self.settle_minibuf_session(view, r);
        }
    }

    /// Re-ranks the open `Buffer` session and dismisses it if it's now
    /// spent. Every path that lands new information into it (`contribute`,
    /// `reconcile_completion`, `end_completion_request`) ends here.
    /// `report_empty`: report "no completions" if the session dismisses as
    /// spent and was ever explicitly triggered. Only `contribute` wants
    /// this: a raw edit narrowing to nothing, or a failed source call,
    /// isn't the user "asking and getting nothing".
    fn settle_buffer_completion(&mut self, view: &EngineView, report_empty: bool) {
        let explicit = self
            .input
            .buffer_completion()
            .is_some_and(BufferSession::is_explicit);
        let live = self.input.completion_owner().map(|(pid, bid)| {
            let pbs = self.panes.entries.seeded(pid, bid);
            (bid, pbs.view(self.buffers.get(bid).text()).primary().head())
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

    /// Queues each of `calls`, as [`Self::queue_steel_call`] does one.
    pub(in crate::editor) fn queue_steel_calls(&mut self, calls: Vec<SteelCall>) {
        for call in calls {
            self.queue_steel_call(call);
        }
    }
}

/// Mints one invocation per source in `ids` into `session` and returns the
/// Steel calls to queue, each with its request id. Each source's token is the run before the cursor
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
    (pid, bid): (PaneId, BufferId),
    head: CharOffset,
) -> Vec<(u64, SteelCall)> {
    let buf = buffers.get(bid);
    let text = buf.text();
    let word_chars = buf.overrides.word_chars(settings);
    let pane = hume_scripting::PaneHandle::with_pane(bid, pid);
    ids.iter()
        .map(|&id| {
            let entry = sources.buffer_get(id);
            let start = entry.token_start(text, head, word_chars);
            let invocation = Invocation::buffer(text.rope().clone(), start);
            let prefix = invocation.prefix(text, head);
            let invocation_id = session.invoke(id, invocation);
            let call = SteelCall::new(
                entry.proc.clone(),
                vec![
                    SteelVal::IntV(invocation_id as isize),
                    hume_scripting::SteelPane::new(pane).into_steel_val(),
                    SteelVal::StringV(prefix.into()),
                ],
            );
            (invocation_id, call)
        })
        .collect()
}

/// Mints an invocation of `id` into `session` and runs it: a native source
/// answers inline, a Steel one returns the call to queue with its request
/// id.
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
) -> Option<(u64, SteelCall)> {
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
        MinibufBody::Steel { proc, .. } => {
            let invocation_id = session.invoke(id, Invocation::minibuf(arg()));
            // Steel indexes a string by char.
            let cursor_chars = input[..cursor].chars().count();
            let call = SteelCall::new(
                proc.clone(),
                vec![
                    SteelVal::IntV(invocation_id as isize),
                    SteelVal::StringV(input.into()),
                    SteelVal::IntV(cursor_chars as isize),
                ],
            );
            Some((invocation_id, call))
        }
    }
}

/// The `:` line's own command name and the byte offset one past it (and
/// its optional trailing `!`). `None` if the cursor hasn't moved past the
/// name yet, still typing it. Uses the same name-shape rule
/// `run_command_line` itself uses
/// (`input_stack::command::scan_command_name`), so completion never picks
/// a different command than Enter would actually run.
fn target_command_name(input: &str, cursor: usize) -> Option<(&str, usize)> {
    let (cmd, _, cmd_end) = crate::editor::input_stack::command::scan_command_name(input);
    (cursor > cmd_end).then_some((cmd, cmd_end))
}

/// Resolves which registered source applies to `completion`'s line in its
/// current `(input, cursor)` shape, and the byte offset
/// [`invoke_minibuf_source`]'s own argument span must not cross. On the
/// `:` line that is the command name itself while the cursor is within it
/// (no space yet, or moved left past the space), with floor `0` (the whole
/// prefix typed so far is the token), else the resolved command's declared
/// argument completer, with the name's own end as the floor. A line that
/// names its source gets it with floor `0`.
fn resolve_minibuf_source(
    registry: &CommandRegistry,
    completion: PromptCompletion,
    input: &str,
    cursor: usize,
) -> Option<(std::borrow::Cow<'static, str>, usize)> {
    use std::borrow::Cow;
    match completion {
        PromptCompletion::Source(name) => Some((Cow::Borrowed(name), 0)),
        PromptCompletion::CommandLine => match target_command_name(input, cursor) {
            None => Some((Cow::Borrowed(super::COMMAND_SOURCE), 0)),
            Some((cmd, cmd_end)) => Some((registry.get_typed(cmd)?.completer.clone()?, cmd_end)),
        },
    }
}
