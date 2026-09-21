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

use super::registry::{
    BufferToken, MinibufToken, SourceBody, SourceId, SourceRegistry, SourceTarget,
};
use super::session::{CompletionSession, Invocation, LiveDoc};
use super::{CompletionCtx, CompletionItem, arg_prefix, token_end_at};
use crate::editor::buffer::store::BufferStore;
use crate::editor::input_stack::{CompletionLayer, InsertLayer, LayerRef};
use crate::editor::registry::CommandRegistry;
use crate::editor::settings::EditorSettings;
use crate::editor::{EditorState, Severity};

/// What set an Insert-mode trigger in motion — and so which sources it
/// invokes.
pub(in crate::editor) enum Trigger<'a> {
    /// Ctrl-Space / the `completion-trigger` command: every `Buffer` source.
    Explicit,
    /// A registered trigger char landed: the sources registered for it
    /// (`EditorState::trigger_sources_for`'s names, matched against the
    /// registry by name — `register-trigger-chars!` is the join).
    Char { sources: &'a [String] },
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
        trigger: Trigger<'_>,
    ) {
        if !self.input.is::<InsertLayer>(self.input.mode_layer()) {
            self.report(
                Severity::Info,
                "completion-trigger: only in Insert mode".to_string(),
            );
            return;
        }
        let sources = &self.config.completion_sources;
        let ids: Vec<SourceId> = match trigger {
            Trigger::Explicit => sources.buffer_sources(),
            Trigger::Char { sources: names } => names
                .iter()
                .filter_map(|name| sources.id_of(name))
                .filter(|&id| matches!(sources.get(id).target(), SourceTarget::Buffer(_)))
                .collect(),
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
        let mut session = match self.take_completion_session(view) {
            Some(open) if open.buffer().is_some_and(|bt| bt.still_valid(self, view)) => open,
            _ => {
                let buf = self.buffers.get(bid);
                CompletionSession::open_buffer(bid, pid, buf.text_gen, buf.text().len_chars())
            }
        };
        let calls = invoke_buffer_sources(
            &self.config.completion_sources,
            &self.buffers,
            &self.settings,
            &mut session,
            &ids,
            bid,
            head,
        );
        for (proc, args) in calls {
            self.queue_steel_call(proc, args);
        }
        session.rank(
            &self.config.completion_sources,
            Some(LiveDoc {
                text: self.buffers.get(bid).text(),
                head,
            }),
        );
        if !session.is_pending() && !session.has_live_sources() {
            self.report(Severity::Info, "no completions".to_string());
            return;
        }
        self.push_layer(view, CompletionLayer { session, ui: None });
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
        let Some(session) = self.input.completion_mut() else {
            return;
        };
        if session.buffer().is_none_or(|bt| bt.bid() != bid) {
            return;
        }
        if !session.observe_edit(cs, text_gen, head) {
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
        for (proc, args) in calls {
            self.queue_steel_call(proc, args);
        }
        self.rerank_open_session();
        // Typed out of every token, and nothing on its way: silent — the
        // user left, nothing "failed".
        if self.open_session_is_spent() {
            self.dismiss_completion(view);
        }
    }

    // ── Minibuffer target ────────────────────────────────────────────────────

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
        let Some(id) = self.config.completion_sources.id_of(name) else {
            // `TypedCommand.completer` naming no registered source — a stale
            // name after a rename. Silent to the user, same as `:bd`
            // declaring no completer at all; loud enough to find in the log.
            self.report(
                Severity::Trace,
                format!("no completion source named {name:?}"),
            );
            return;
        };
        let mut session = CompletionSession::open_minibuf(input.clone(), cursor);
        let ctx = CompletionCtx {
            registry: &self.config.registry,
            buffers: &self.buffers,
            cwd: &self.cwd,
            languages: &self.config.languages,
        };
        let call = match invoke_minibuf_source(
            &self.config.completion_sources,
            &ctx,
            &mut session,
            id,
            &input,
            cursor,
        ) {
            Ok(call) => call,
            Err(msg) => {
                self.report(Severity::Trace, msg);
                return;
            }
        };
        if let Some((proc, args)) = call {
            self.queue_steel_call(proc, args);
        }
        let r = self.push_layer(view, CompletionLayer { session, ui: None });
        self.settle_minibuf_session(view, r);
    }

    /// The `:` line's eager policy, once every source has answered: nothing
    /// → the popup never shows; one candidate → applied silently, popup
    /// gone; two or more → the first is applied and the popup stays for
    /// Tab to cycle. Runs at open (native sources answer inline) and again
    /// when a pending Steel source's answer lands.
    fn settle_minibuf_session(&mut self, view: &EngineView, r: LayerRef) {
        let Some(layer) = self.input.at_mut::<CompletionLayer>(r) else {
            return;
        };
        layer.session.rank(&self.config.completion_sources, None);
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
        let Some(layer) = self.input.at::<CompletionLayer>(r) else {
            return;
        };
        let selected = layer.ui.as_ref().map_or(0, |ui| ui.selected);
        let Some(input) = layer.session.minibuf_input() else {
            return;
        };
        let Some((span, text)) = layer.session.minibuf_apply(selected) else {
            return;
        };
        let (input, text) = (input.to_owned(), text.to_owned());
        let Some(mb) = self.input.minibuf_mut() else {
            return;
        };
        mb.input = input;
        mb.splice(span, &text);
    }

    // ── Answers ──────────────────────────────────────────────────────────────

    /// A source's answer to invocation `id` — `completion-emit!`. `Ok(false)`
    /// when no open session has that invocation as a slot's latest call
    /// (superseded, replaced, or dismissed since: expected-normal for a late
    /// async source). Whatever the answer did, the session is settled
    /// afterwards: re-ranked, and closed with "no completions" once every
    /// source has answered and none had anything.
    pub(in crate::editor) fn contribute(
        &mut self,
        view: &EngineView,
        id: u64,
        items: Vec<CompletionItem>,
        incomplete: bool,
        span: Option<(usize, usize)>,
    ) -> Result<bool, String> {
        let Some(r) = self.input.ref_of::<CompletionLayer>() else {
            return Ok(false);
        };
        let session = self.input.completion_mut().expect("ref_of found the layer");
        let result = session.contribute(id, items, incomplete, span);
        if session.buffer().is_some() {
            self.rerank_open_session();
            if self.open_session_is_spent() {
                self.dismiss_completion(view);
                self.report(Severity::Info, "no completions".to_string());
            }
        } else {
            self.settle_minibuf_session(view, r);
        }
        result
    }

    /// Re-ranks the open session against the live document and resets the
    /// menu selection to row 0 — every path that changes `filtered` must,
    /// since the previous selection index has no guaranteed meaning against
    /// the new order.
    fn rerank_open_session(&mut self) {
        let live = self
            .input
            .completion()
            .and_then(|s| s.buffer())
            .map(|bt| bt.bid())
            .and_then(|bid| {
                self.focused_buffer_state(bid)
                    .map(|pbs| (bid, pbs.selections().primary().head()))
            });
        let Some(session) = self.input.completion_mut() else {
            return;
        };
        let live = live.map(|(bid, head)| LiveDoc {
            text: self.buffers.get(bid).text(),
            head,
        });
        session.rank(&self.config.completion_sources, live);
        self.reset_completion_selection();
    }

    /// Whether the open session has nothing left to show and nothing on
    /// its way — every source answered empty, or the cursor typed out of
    /// every token.
    fn open_session_is_spent(&self) -> bool {
        self.input
            .completion()
            .is_some_and(|s| !s.is_pending() && !s.has_live_sources())
    }
}

/// Mints one invocation per source in `ids` into `session` and returns the
/// Steel calls to queue (a native `Buffer` source doesn't exist yet, so
/// every entry here is a Steel one). Takes the fields it needs rather than
/// `&mut EditorState` so a caller can hand it a session still borrowed
/// from the input stack.
fn invoke_buffer_sources(
    sources: &SourceRegistry,
    buffers: &BufferStore,
    settings: &EditorSettings,
    session: &mut CompletionSession,
    ids: &[SourceId],
    bid: BufferId,
    head: CharOffset,
) -> Vec<SteelCall> {
    let buf = buffers.get(bid);
    let text = buf.text();
    ids.iter()
        .filter_map(|&id| {
            let SourceBody::Steel {
                proc,
                target: SourceTarget::Buffer(token),
            } = &sources.get(id).body
            else {
                return None;
            };
            let live = match token {
                BufferToken::Word => {
                    let chars = crate::editor::commands::effective_word_chars(buf, settings);
                    Some(hume_ops::edit::word_start_before(text, head, chars)..head)
                }
                BufferToken::Cursor => Some(head..head),
                BufferToken::Custom => None,
            };
            let invocation = Invocation::buffer(text.rope().clone(), head, live);
            let prefix = invocation.prefix(text);
            let invocation_id = session.invoke(id, invocation);
            Some((
                proc.clone(),
                vec![
                    SteelVal::IntV(invocation_id as isize),
                    SteelBufferId::new(bid).into_steel_val(),
                    SteelVal::StringV(prefix.into()),
                ],
            ))
        })
        .collect()
}

/// Mints an invocation of `id` into `session` and runs it: a native source
/// answers inline, a Steel one returns the call to queue. `Err` names a
/// source that can't serve the `:` line at all (a `Buffer`-target entry a
/// `TypedCommand.completer` mistakenly names).
fn invoke_minibuf_source(
    sources: &SourceRegistry,
    ctx: &CompletionCtx<'_>,
    session: &mut CompletionSession,
    id: SourceId,
    input: &str,
    cursor: usize,
) -> Result<Option<SteelCall>, String> {
    let entry = sources.get(id);
    let SourceTarget::Minibuf(token) = entry.target() else {
        return Err(format!(
            "completion source {:?} serves the buffer, not the command line",
            entry.name
        ));
    };
    let bytes = match token {
        MinibufToken::Arg => {
            let (start, _) = arg_prefix(input, cursor);
            Some(start..token_end_at(input, cursor, &[' ']))
        }
        MinibufToken::Custom => None,
    };
    let invocation_id = session.invoke(id, Invocation::minibuf(bytes));
    let call = match &entry.body {
        SourceBody::NativeUniverse(f) => {
            let items = f(ctx);
            session
                .contribute(invocation_id, items, false, None)
                .expect("an Arg-token invocation takes no span");
            None
        }
        SourceBody::NativeDelegated(f) => {
            let (span, items) = f(input, cursor, ctx);
            session
                .contribute(invocation_id, items, false, Some((span.start, span.end)))
                .expect("a native source's own span is within its own input");
            None
        }
        SourceBody::Steel { proc, .. } => Some((
            proc.clone(),
            vec![
                SteelVal::IntV(invocation_id as isize),
                SteelVal::StringV(input.into()),
                SteelVal::IntV(cursor as isize),
            ],
        )),
    };
    Ok(call)
}

/// Resolves which registered source applies to the current `(input,
/// cursor)` shape: the command name itself while the cursor is within it
/// (no space yet, or moved left past the space), else the resolved
/// command's declared argument completer.
fn resolve_minibuf_source(
    registry: &CommandRegistry,
    input: &str,
    cursor: usize,
) -> Option<&'static str> {
    match input.split_once(' ') {
        None => Some(super::COMMAND_SOURCE),
        Some((cmd_raw, _)) if cursor <= cmd_raw.len() => Some(super::COMMAND_SOURCE),
        Some((cmd_raw, _)) => {
            // Resolve alias → command, and its declared argument completer.
            let cmd = cmd_raw.strip_suffix('!').unwrap_or(cmd_raw);
            registry.get_typed(cmd)?.completer
        }
    }
}
