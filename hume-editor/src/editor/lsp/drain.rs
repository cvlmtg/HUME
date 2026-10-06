//! Per-frame drain of the LSP backend: routes transport events through each
//! client's `on_event`, dispatches the resulting `ClientAction`s, and pulls
//! completed requests (responses + timeouts) via `take_completed`.

use std::time::{Duration, Instant};

use rustc_hash::FxHashSet;

use hume_engine::pipeline::BufferId;
use hume_lsp::backend::ServerId;
use hume_lsp::client::{ClientAction, Outcome, RequestMeta, ServerState, server_request_response};
use hume_lsp::codec::{Message, RequestId};
use lsp_types::request::Request as _;

use super::LspState;
use super::diagnostics::{CANONICALIZE_PER_DRAIN, Ingest};
use crate::editor::commands::FocusedPane;
use crate::editor::{Editor, EditorState, Severity};
use hume_engine::pipeline::EngineView;
use hume_scripting::PaneHandle;

impl Editor {
    /// Per-frame drain: routes every backend event through its client's
    /// `on_event`, dispatches the resulting `ClientAction`s, then pulls
    /// each client's completed requests (responses + timeouts) via
    /// `take_completed` and dispatches those too.
    pub(in crate::editor) fn drain_lsp(&mut self) {
        self.state.lsp_flush_pending();

        let events = self.state.lsp.backend.drain();
        for (server_id, ev) in events {
            let actions = match self.state.lsp.instances.get_mut(server_id) {
                Some(instance) => instance.client.on_event(ev),
                None => continue,
            };
            for action in actions {
                self.dispatch_lsp_action(server_id, action);
            }
        }
        self.ingest_published();

        let now = Instant::now();

        // Advance the statusline loading spinner while any server is mid-
        // handshake or reporting `$/progress`, idle otherwise, so the
        // frame counter doesn't drift while there's nothing to animate.
        if self.state.lsp.has_animating_server() {
            self.state.lsp.spinner.maybe_advance(now);
        }

        for server_id in self.state.lsp.instances.ids() {
            let LspState {
                instances, backend, ..
            } = &mut self.state.lsp;
            let (completed, actions) = match instances.get_mut(server_id) {
                Some(instance) => instance.client.take_completed(backend.as_mut(), now),
                None => continue,
            };
            for action in actions {
                self.dispatch_lsp_action(server_id, action);
            }
            for (id, meta, outcome) in completed {
                self.state
                    .lsp_dispatch_completed(&self.view, server_id, id, meta, outcome);
            }
        }
    }

    /// Ingests the queued `publishDiagnostics`, within one `canonicalize()`
    /// budget, and defers what the budget leaves. `OnDiagnosticsChanged`
    /// fires once per buffer this batch actually touched: a `FxHashSet`
    /// dedupes two (server, uri) entries that both resolved to the same
    /// buffer.
    pub(in crate::editor) fn ingest_published(&mut self) {
        let mut touched: FxHashSet<BufferId> = FxHashSet::default();
        let mut canonicalize_budget = CANONICALIZE_PER_DRAIN;
        for (server_id, published) in self.state.lsp.publishes.take_all() {
            match self.ingest_publish_diagnostics(server_id, published, &mut canonicalize_budget) {
                Ingest::Done(Some(bid)) => {
                    touched.insert(bid);
                }
                Ingest::Done(None) => {}
                Ingest::Deferred(published) => {
                    self.state.lsp.publishes.defer(server_id, published);
                }
            }
        }
        for bid in touched {
            self.queue_diagnostics_changed(bid);
        }
    }

    /// [`lsp_shutdown_all`](Self::lsp_shutdown_all)'s production grace
    /// window: the value `hume_editor::run` and `run_keys`' post-loop
    /// teardown actually use; tests pass their own to exercise the zero- and
    /// long-window edges. `hume_platform::QUIT_GRACE` is sized against this
    /// constant; keep the two in step.
    pub(crate) const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

    /// Graceful shutdown on quit: `begin_shutdown` (shutdown request, then
    /// exit notification) for every Running client, then a bounded grace
    /// window draining for their voluntary EOF, before transport-level
    /// teardown (`backend.shutdown`, which reaps any process still alive)
    /// regardless. Starting clients skip the protocol handshake: nothing
    /// but `initialize` is legal to send before `initialized`, so a plain
    /// transport kill is the only option for them.
    ///
    /// Events drained during the grace window are otherwise discarded: a
    /// lingering response or stderr line has nowhere useful to go while the
    /// editor is tearing down.
    pub(crate) fn lsp_shutdown_all(&mut self, grace: Duration) {
        let server_ids = self.state.lsp.instances.ids();
        if server_ids.is_empty() {
            return;
        }

        let mut awaiting_eof: FxHashSet<ServerId> = FxHashSet::default();
        for &server_id in &server_ids {
            let LspState {
                instances, backend, ..
            } = &mut self.state.lsp;
            if let Some(instance) = instances.get_mut(server_id)
                && instance.client.state() == ServerState::Running
            {
                instance.client.begin_shutdown(backend.as_mut());
                awaiting_eof.insert(server_id);
            }
        }

        if !awaiting_eof.is_empty() {
            let deadline = Instant::now() + grace;
            while !awaiting_eof.is_empty() && Instant::now() < deadline {
                for (server_id, ev) in self.state.lsp.backend.drain() {
                    if matches!(ev, hume_lsp::transport::InboundEvent::Eof { .. }) {
                        awaiting_eof.remove(&server_id);
                    }
                }
                if !awaiting_eof.is_empty() {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }

        for server_id in server_ids {
            self.state.lsp.backend.shutdown(server_id);
        }
    }

    pub(in crate::editor) fn dispatch_lsp_action(
        &mut self,
        server_id: ServerId,
        action: ClientAction,
    ) {
        match action {
            ClientAction::BecameRunning { send } => {
                for msg in send {
                    self.state.lsp.backend.send(server_id, msg);
                }
                // Fire on-lsp-attach for every buffer already attached to
                // this server: it was Starting until now, so attaching
                // skipped firing it for them.
                for bid in self.state.buffer_positions.lsp.buffers_of(server_id) {
                    self.state.queue_lsp_attach(bid, server_id);
                }
            }
            ClientAction::Crashed { error } => {
                let name = self.lsp_server_name(server_id);
                self.report(
                    Severity::Error,
                    format!("lsp: {}", super::crashed_text(&name, error.as_deref())),
                );
                // Fail every in-flight request immediately rather than
                // leaving each to expire on its own deadline. The crash is
                // already known, so there's nothing to wait for. Buffers
                // stay attached: routing skips a crashed server, and
                // `:lsp-restart` reconciles them onto a fresh instance.
                if let Some(instance) = self.state.lsp.instances.get_mut(server_id) {
                    // A crashed server can't finish whatever it was loading:
                    // drop its tracked progress so the statusline spinner
                    // doesn't keep animating for a server that's gone.
                    instance.progress.clear();
                    for (id, meta, outcome) in instance.client.drain_pending() {
                        self.state
                            .lsp_dispatch_completed(&self.view, server_id, id, meta, outcome);
                    }
                }
            }
            ClientAction::ServerRequest { id, method, params } => {
                // `workspace/applyEdit` needs `&mut Editor` (the edit engine);
                // every other request answers from the pure lookup table.
                let result = if method == lsp_types::request::ApplyWorkspaceEdit::METHOD {
                    self.apply_edit_request_response(&params, server_id)
                } else {
                    let settings = self
                        .state
                        .lsp
                        .instances
                        .get(server_id)
                        .and_then(|i| i.client.settings());
                    server_request_response(&method, &params, settings)
                };
                self.state
                    .lsp
                    .backend
                    .send(server_id, Message::Response { id, result });
            }
            ClientAction::Diagnostics(published) => {
                // Queued, the newest per (server, uri): servers
                // burst-publish and only the newest matters. `drain_lsp`
                // ingests the queue after dispatching a whole batch, so a
                // later action for the same (server, uri) wins whatever its
                // arrival order, and a publish an earlier drain deferred is
                // still waiting there.
                self.note_skipped_diagnostics(server_id, &published);
                self.state.lsp.publishes.offer(server_id, published);
            }
            ClientAction::Progress(params) => {
                self.handle_progress(server_id, params);
            }
            ClientAction::LogMessage(params) => {
                let name = self.lsp_server_name(server_id);
                let severity = match params.typ {
                    lsp_types::MessageType::ERROR => Severity::Error,
                    lsp_types::MessageType::WARNING => Severity::Warning,
                    _ => Severity::Trace, // Info/Log
                };
                self.report(severity, format!("{name}: {}", params.message));
            }
            ClientAction::ShowMessage(params) => {
                let name = self.lsp_server_name(server_id);
                self.report(Severity::Info, format!("{name}: {}", params.message));
            }
            ClientAction::ServerNotification { method, params } => {
                self.dispatch_server_notification(server_id, &method, params);
            }
            ClientAction::Stderr(line) => {
                // rust-analyzer logs a lot; Trace keeps :messages usable;
                // never promote stderr to a higher severity.
                let name = self.lsp_server_name(server_id);
                self.report(Severity::Trace, format!("{name}: {line}"));
            }
        }
    }

    /// Name used to prefix this server's log lines: its registration name,
    /// or `"lsp"` for one no longer running.
    pub(super) fn lsp_server_name(&self, server_id: ServerId) -> String {
        self.state
            .lsp
            .instances
            .get(server_id)
            .map(|i| i.name.to_string())
            .unwrap_or_else(|| "lsp".to_string())
    }

    /// `textDocument/publishDiagnostics`, `$/progress`, `window/logMessage`,
    /// and `window/showMessage` never reach here: `hume-lsp` classifies
    /// them into typed `ClientAction` variants, handled directly in
    /// `dispatch_lsp_action`. Only an unclassified method, or a known
    /// method whose params fail both the strict parse and `hume-lsp`'s
    /// lenient recovery, arrives here. Either becomes an
    /// `on-lsp-notification` event. `fire_one_event` traces it as
    /// "unhandled notification" if, once lazy plugins have activated, no
    /// handler takes `method`.
    fn dispatch_server_notification(
        &mut self,
        server_id: ServerId,
        method: &str,
        params: serde_json::Value,
    ) {
        // No untagged fallback: a notification's params may carry wire
        // positions (e.g. a server-defined custom notification echoing a
        // range), so an untracked server (stopped between sending this and
        // it being drained) is dropped rather than tagged with a guessed
        // encoding.
        let Some((server, encoding)) = self.state.lsp.instances.origin(server_id) else {
            self.report(
                Severity::Trace,
                format!("lsp: dropping {method} from an untracked server"),
            );
            return;
        };
        self.state
            .queue_event(crate::editor::event::EditorEvent::OnLspNotification {
                server,
                method: method.to_owned(),
                params: std::sync::Arc::new(params),
                origin: hume_scripting::json::WireOrigin::Server {
                    id: server_id,
                    encoding,
                },
            });
    }
}

impl EditorState {
    /// Hands `server_id`'s answer to request `id` to the delivery slot that
    /// waits for it. Every way a request ends (response, timeout, crash,
    /// stop) arrives here.
    pub(in crate::editor) fn lsp_dispatch_completed(
        &mut self,
        view: &EngineView,
        server_id: ServerId,
        id: RequestId,
        meta: RequestMeta,
        outcome: Outcome,
    ) {
        let Some(outcome) = self.lsp_fill_slot(view, server_id, id, &meta, outcome) else {
            return;
        };
        // No delivery is ever filed for the internal `shutdown` request
        // (it's fire-and-forget from `begin_shutdown`): a server-side error
        // on it would otherwise vanish silently.
        if meta.method == lsp_types::request::Shutdown::METHOD
            && let Outcome::Err(e) = &outcome
        {
            self.report(
                Severity::Trace,
                format!("lsp: shutdown failed: {} ({})", e.message, e.code),
            );
        }
    }

    /// Releases the position a request holds through `#:tracked`, unless its
    /// callback kept it: every place a request's callback has run, or is
    /// dropped without running, calls this.
    pub(in crate::editor) fn release_request_position(
        &mut self,
        tracked: Option<hume_scripting::host::HostToken>,
    ) {
        if let Some(token) = tracked {
            self.panes.tracked.release_unless_kept(token);
        }
    }

    /// Whether a completed request's callback should actually fire, per its
    /// `ResponseAnchor`: the one place both of a callback's drop conditions
    /// are checked, so a caller only has to gather the anchor at send time
    /// rather than repeat either check itself. Text-gen first: `#:allow-stale`
    /// is the more targeted opt-out (a single request's own reason for
    /// tolerating staleness), so it decides before focus is even considered.
    ///
    /// Two call points, not one: `finish_delivery` (`bridge.rs`) checks it at
    /// LSP drain time, the only gate at all for a delivery answered by a
    /// Rust responder (`completionItem/resolve`), and an early drop for a
    /// Steel one, before its `(proc, args)` is even queued.
    /// `Editor::run_pending_batch`
    /// (`scripting_setup.rs`) re-checks the same anchor for a queued Steel
    /// callback right before it actually runs: arbitrary other queued work
    /// (a hook, an earlier callback in the same batch) can execute between
    /// the two checks and change the state the first one saw, so admission
    /// at drain time alone doesn't guarantee admission at run time.
    pub(in crate::editor) fn anchor_admits(
        &mut self,
        view: &EngineView,
        anchor: &super::ResponseAnchor,
    ) -> bool {
        let current_gen = self.buffers.try_get(anchor.bid).map(|b| b.text().version());
        if current_gen != Some(anchor.version) && !anchor.allow_stale {
            return false; // dropped silently, per parse-worker staleness discipline
        }
        if let Some(pid) = anchor.require_focus {
            // Dropped the same way `async_opener_stale` drops a menu/drawer
            // open whose stack has moved on: the response is for UI
            // anchored to `pid`, and the user has since navigated elsewhere
            // (moved focus to another pane, even one still showing
            // `anchor.bid`, or the pane now shows a different buffer), so
            // delivering it would show hover/signature-help/a code-action
            // menu over the wrong pane. Exactly `FocusedPane::resolve`'s own
            // check, against the handle the request was made from.
            let handle = PaneHandle::with_pane(anchor.bid, pid);
            if FocusedPane::resolve(self, view, handle).is_err() {
                self.report(
                    Severity::Trace,
                    "lsp-request!: the focused pane moved before the response could open; ignored"
                        .to_string(),
                );
                return false;
            }
        }
        true
    }
}
