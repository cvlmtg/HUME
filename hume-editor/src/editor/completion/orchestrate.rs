//! The one driver of a completion session, for both targets: picks the
//! sources a trigger applies to, mints each an [`Invocation`] against the
//! live document, runs it (a native source inline, a Steel one via the
//! deferred-call queue), lands answers, reacts to edits, and decides when a
//! session has nothing left to show. Everything here is `impl EditorState`
//! (not `Editor`), since every input it needs — the input stack, the source
//! registry, the buffers, the Steel call queue — lives there, so it is
//! reachable from `EditorHostImpl` (`completion-emit!`) and from the key
//! handlers alike, and never from inside a Steel eval's own borrow.
//!
//! Steel sources are only ever *queued* (`queue_steel_call`), never called
//! inline — same as the picker's live source and every other Rust→Steel
//! callback. A trigger is user-intent frequency, so the per-keystroke work
//! (`rank`) stays here in Rust and only a source flagged `isIncomplete` is
//! called again as the user types.

use hume_engine::pipeline::{BufferId, EngineView};
use hume_rope::offset::CharOffset;
use hume_scripting::SteelBufferId;
use steel::rvals::SteelVal;

use super::registry::{BufferSourceId, MinibufBody, MinibufSourceId, SourceRegistry};
use super::session::{BufferSession, Invocation, LiveDoc, MinibufSession};
use super::{CompletionCtx, CompletionItem, arg_span};
use crate::editor::buffer::store::BufferStore;
use crate::editor::input_stack::{
    BufferCompletionLayer, InsertLayer, LayerRef, MinibufCompletionLayer,
};
use crate::editor::registry::CommandRegistry;
use crate::editor::settings::EditorSettings;
use crate::editor::{EditorState, Severity};

/// What set an Insert-mode trigger in motion — and so which sources it
/// invokes.
pub(in crate::editor) enum Trigger {
    /// Ctrl-Space / the `completion-trigger` command: every `Buffer` source.
    Explicit,
    /// A char landed in Insert mode: every `Buffer` source registered for
    /// `(ch, language)` via `(completion-set-trigger-chars! …)`
    /// (`SourceRegistry::buffer_sources_for_trigger` is the join) — its own
    /// table, separate from `register-trigger-chars!`'s shared one, which
    /// only ever feeds the generic `on-trigger-char` hook. `language` is
    /// owned, not borrowed from the caller's own `&str` (`LanguageRegistry::
    /// name_of`'s return): the caller passes this straight into a `&mut
    /// EditorState` method, so a borrow tied to that same state would
    /// conflict with the method's own `&mut self`.
    Char { ch: char, language: Option<String> },
}

/// A Steel proc plus its args, minted while a session is borrowed and
/// queued once it isn't — `queue_steel_call` needs the whole `EditorState`.
type SteelCall = (SteelVal, Vec<SteelVal>);

impl EditorState {
    // ── Buffer target ────────────────────────────────────────────────────────

    /// Opens (or re-invokes into) the Insert-mode completion session on the
    /// focused buffer. A session already open on this buffer survives —
    /// its sources are called again, superseding their earlier answers
    /// once the new ones land, so the menu never blinks empty — but is
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
        let sources = &self.config.completion_sources;
        let ids: Vec<BufferSourceId> = match trigger {
            Trigger::Explicit => sources.buffer_sources(),
            Trigger::Char { ch, ref language } => {
                sources.buffer_sources_for_trigger(ch, language.as_deref())
            }
        };
        if ids.is_empty() {
            if let Trigger::Explicit = trigger {
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
            .map(|pbs| pbs.selections().primary().head())
        else {
            return;
        };
        let mut session = match self.take_buffer_completion(view) {
            Some(open) if open.still_valid(self, view) => open,
            _ => {
                let buf = self.buffers.get(bid);
                BufferSession::open(bid, pid, buf.text_gen, buf.text().len_chars())
            }
        };
        if let Trigger::Explicit = trigger {
            session.mark_explicit_trigger();
        }
        let calls = invoke_buffer_sources(
            &self.config.completion_sources,
            &self.buffers,
            &self.settings,
            &mut session,
            &ids,
            bid,
            head,
        );
        self.queue_steel_calls(calls);
        session.rank(
            &self.config.completion_sources,
            Some(LiveDoc {
                text: self.buffers.get(bid).text(),
                head,
            }),
        );
        if session.is_spent() {
            self.report(Severity::Info, "no completions".to_string());
            return;
        }
        self.push_layer(view, BufferCompletionLayer { session });
    }

    /// Records an Insert-mode edit that landed on `bid` — called from the
    /// one chokepoint every keystroke edit goes through
    /// (`Editor::apply_insert_edit`). Re-ranks against the tokens' new
    /// text, dismisses a session the cursor has typed out of, and calls
    /// again every source that flagged its last answer `isIncomplete` (or
    /// is still pending against the document before this edit).
    pub(in crate::editor) fn completion_observe_edit(
        &mut self,
        view: &EngineView,
        bid: BufferId,
        cs: &hume_editing::changeset::ChangeSet,
        text_gen: u64,
    ) {
        let Some(head) = self
            .focused_buffer_state(bid)
            .map(|pbs| pbs.selections().primary().head())
        else {
            return;
        };
        // Computed before `session` borrows `self.input` mutably — needed
        // only to classify a newly-included end-of-token slice
        // (`Invocation::observe`'s own doc).
        let buf = self.buffers.get(bid);
        let chars = crate::editor::commands::effective_word_chars(buf, &self.settings);
        let text = buf.text();
        let Some(session) = self.input.buffer_completion_mut() else {
            return;
        };
        if session.bid() != bid {
            return;
        }
        if !session.observe_edit(
            &self.config.completion_sources,
            cs,
            text_gen,
            head,
            text,
            chars,
        ) {
            self.dismiss_completion(view);
            return;
        }
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
        // Typed out of every token, and nothing on its way: silent — the
        // user left, nothing "failed".
        self.settle_buffer_completion(view, false);
    }

    // ── Minibuffer target ────────────────────────────────────────────────────

    /// The `:` line's current target command name, if the minibuf is open
    /// on a `:` prompt and the cursor sits past it (in its argument) —
    /// `Editor::activate_minibuf_completion_target` reads this before
    /// calling [`Self::trigger_minibuf_completion`], to activate a still-
    /// `TypedBody::Lazy` owner (see that method's own doc) before the
    /// resolve below runs.
    pub(in crate::editor) fn minibuf_target_command(&self) -> Option<String> {
        let mb = self.input.minibuf()?;
        if mb.prompt != ":" {
            return None;
        }
        Some(target_command_name(&mb.input, mb.cursor)?.to_owned())
    }

    /// The first Tab on the `:` line: resolves the one source the input
    /// shape names (the command name itself, or the command's declared
    /// argument completer), runs it, and applies the `:` line's own
    /// eager policy — a sole candidate lands silently with no popup, two or
    /// more open the popup with the first already applied.
    pub(in crate::editor) fn trigger_minibuf_completion(&mut self, view: &EngineView) {
        let Some(mb) = self.input.minibuf() else {
            return;
        };
        if mb.prompt != ":" {
            return;
        }
        let (input, cursor) = (mb.input.clone(), mb.cursor);
        let Some(name) = resolve_minibuf_source(&self.config.registry, &input, cursor) else {
            return;
        };
        let Some(id) = self.config.completion_sources.minibuf_id_of(&name) else {
            // `TypedCommand.completer` naming no registered source — a stale
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
        };
        let call = invoke_minibuf_source(
            &self.config.completion_sources,
            &ctx,
            &mut session,
            id,
            &input,
            cursor,
        );
        if let Some((proc, args)) = call {
            self.queue_steel_call(proc, args);
        }
        let r = self.push_layer(view, MinibufCompletionLayer { session });
        self.settle_minibuf_session(view, r);
    }

    /// The `:` line's eager policy, once every source has answered: nothing
    /// → the popup never shows; one candidate → applied silently, popup
    /// gone; two or more → the first is applied and the popup stays for
    /// Tab to cycle. Runs at open (native sources answer inline) and again
    /// when a pending Steel source's answer lands — including a second
    /// answer to a still-streaming source, so the re-rank (which resets the
    /// selection itself — `SlotSet::rank_with`'s own contract) always runs
    /// first: the previous selection index has no guaranteed meaning
    /// against the new order, and may point past a narrower list's end
    /// entirely.
    fn settle_minibuf_session(&mut self, view: &EngineView, r: LayerRef) {
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
                self.apply_minibuf_candidate(r);
                self.dismiss_completion(view);
            }
            _ => self.apply_minibuf_candidate(r),
        }
    }

    /// Splices the selected candidate of the `Minibuf` session at `r` into
    /// the `:` line, over its own source's token — restoring the input the
    /// sources saw first, so cycling from one candidate to the next never
    /// has to know what the previous one left behind.
    pub(in crate::editor) fn apply_minibuf_candidate(&mut self, r: LayerRef) {
        let Some(layer) = self.input.at::<MinibufCompletionLayer>(r) else {
            return;
        };
        let selected = layer.session.selected();
        let Some((span, text)) = layer.session.selected_apply(selected) else {
            return;
        };
        let (input, text) = (layer.session.input().to_owned(), text.to_owned());
        let Some(mb) = self.input.minibuf_mut() else {
            return;
        };
        mb.input = input;
        mb.splice(span, &text);
    }

    // ── Answers ──────────────────────────────────────────────────────────────

    /// A source's answer to invocation `id` — `completion-emit!`. `false`
    /// when no open session has that invocation as a slot's latest call
    /// (superseded, replaced, or dismissed since: expected-normal for a late
    /// async source) — the session is untouched, so it is settled only on
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
                // explicitly asked for completion on — same discipline as
                // the `Explicit`-only report at this file's own
                // `trigger_buffer_completion` (`ids.is_empty()`'s `if let
                // Trigger::Explicit = trigger` guard).
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

    /// Recovery for a Steel call batch that failed (see
    /// `Editor::run_call_batch`'s own call site): drops every completion
    /// invocation still `Pending`, since nothing will ever call
    /// `completion-emit!` for it now — see `SlotSet::drop_stalled`'s own
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
    /// spent — every path that lands new information into it (`contribute`,
    /// `completion_observe_edit`, a failed call batch) ends here.
    /// `report_empty`: report "no completions" if the session dismisses as
    /// spent and was ever explicitly triggered. Only `contribute` wants
    /// this — a raw edit narrowing to nothing, or a call-batch failure,
    /// isn't the user "asking and getting nothing".
    fn settle_buffer_completion(&mut self, view: &EngineView, report_empty: bool) {
        let explicit = self
            .input
            .buffer_completion()
            .is_some_and(BufferSession::is_explicit);
        let live = self.input.buffer_completion().and_then(|s| {
            let bid = s.bid();
            self.focused_buffer_state(bid)
                .map(|pbs| (bid, pbs.selections().primary().head()))
        });
        let Some(session) = self.input.buffer_completion_mut() else {
            return;
        };
        let live = live.map(|(bid, head)| LiveDoc {
            text: self.buffers.get(bid).text(),
            head,
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

    /// Queues every `(proc, args)` pair `invoke_buffer_sources` returned —
    /// the drain repeated at both its call sites (a fresh/reused trigger,
    /// and a post-edit re-invocation).
    fn queue_steel_calls(&mut self, calls: Vec<SteelCall>) {
        for (proc, args) in calls {
            self.queue_steel_call(proc, args);
        }
    }
}

/// Mints one invocation per source in `ids` into `session` and returns the
/// Steel calls to queue — every `Buffer` source is Steel, by design (see
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
    // Every source shares one token: the word before the cursor. Computed
    // once here rather than per source in the loop below — neither
    // `effective_word_chars` nor the `word_start_before` scan depends on
    // which source is being invoked.
    let chars = crate::editor::commands::effective_word_chars(buf, settings);
    let live = hume_ops::edit::word_start_before(text, head, chars)..head;
    ids.iter()
        .map(|&id| {
            let entry = sources.buffer_get(id);
            let invocation = Invocation::buffer(text.rope().clone(), live.clone());
            let prefix = invocation.prefix(text);
            let invocation_id = session.invoke(id, invocation);
            (
                entry.proc.clone(),
                vec![
                    SteelVal::IntV(invocation_id as isize),
                    SteelBufferId::new(bid).into_steel_val(),
                    SteelVal::StringV(prefix.into()),
                ],
            )
        })
        .collect()
}

/// Mints an invocation of `id` into `session` and runs it: a native source
/// answers inline, a Steel one returns the call to queue.
///
/// `NativeDelegated`'s own span comes from calling its function first — the
/// only body variant whose span isn't the generic `'arg` one, since its
/// candidate universe *is* the live input and the two are computed
/// together. Every other body gets the whitespace-delimited argument span
/// computed here, upfront, the same way a `Buffer` source's span is always
/// the word before the cursor (`invoke_buffer_sources`).
fn invoke_minibuf_source(
    sources: &SourceRegistry,
    ctx: &CompletionCtx<'_>,
    session: &mut MinibufSession,
    id: MinibufSourceId,
    input: &str,
    cursor: usize,
) -> Option<SteelCall> {
    let entry = sources.minibuf_get(id);
    match &entry.body {
        MinibufBody::NativeUniverse(f) => {
            let span = arg_span(input, cursor, ' ', &[' ']);
            let invocation_id = session.invoke(id, Invocation::minibuf(span));
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
            let span = arg_span(input, cursor, ' ', &[' ']);
            let invocation_id = session.invoke(id, Invocation::minibuf(span));
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

/// The `:` line's own command name, stripped of a trailing `!` (alias →
/// command), if the cursor sits past it — in its argument, not still typing
/// the name itself. Shared by [`resolve_minibuf_source`] and
/// [`EditorState::minibuf_target_command`], the one place both need to
/// agree on what "past the command name" means.
fn target_command_name(input: &str, cursor: usize) -> Option<&str> {
    let (cmd_raw, _) = input.split_once(' ')?;
    if cursor <= cmd_raw.len() {
        return None;
    }
    Some(cmd_raw.strip_suffix('!').unwrap_or(cmd_raw))
}

/// Resolves which registered source applies to the current `(input,
/// cursor)` shape: the command name itself while the cursor is within it
/// (no space yet, or moved left past the space), else the resolved
/// command's declared argument completer.
fn resolve_minibuf_source(
    registry: &CommandRegistry,
    input: &str,
    cursor: usize,
) -> Option<std::borrow::Cow<'static, str>> {
    use std::borrow::Cow;
    match target_command_name(input, cursor) {
        None => Some(Cow::Borrowed(super::COMMAND_SOURCE)),
        Some(cmd) => registry.get_typed(cmd)?.completer.clone(),
    }
}
