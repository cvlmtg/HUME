//! Per-frame drain of the LSP backend: routes transport events through each
//! client's `on_event`, dispatches the resulting `ClientAction`s, and pulls
//! completed requests (responses + timeouts) via `take_completed`.

use std::time::{Duration, Instant};

use rustc_hash::{FxHashMap, FxHashSet};

use hume_engine::pipeline::BufferId;
use hume_lsp::backend::ServerId;
use hume_lsp::client::{ClientAction, Outcome, RequestMeta, ServerState, server_request_response};
use hume_lsp::codec::{Message, RequestId};
use lsp_types::request::Request as _;

use super::LspState;
use super::introspect;
use crate::editor::{Editor, Severity};

impl Editor {
    /// Per-frame drain: routes every backend event through its client's
    /// `on_event`, dispatches the resulting `ClientAction`s, then pulls
    /// each client's completed requests (responses + timeouts) via
    /// `take_completed` and dispatches those too.
    pub(in crate::editor) fn drain_lsp(&mut self) {
        self.flush_lsp_pending_changes();

        let events = self.lsp.backend.drain();
        // Coalesce publishDiagnostics within this batch: keep only the last
        // one per (server, uri) — servers burst-publish and only the newest
        // matters. Ingested after the loop so a later action for the same
        // (server, uri) always wins regardless of arrival order within the
        // batch.
        // clippy's `mutable_key_type` flags `lsp_types::Uri` for the `Cell`s
        // inside its underlying `fluent_uri::Uri`'s parse-offset cache — but
        // `Uri`'s `Hash`/`PartialEq`/`Eq` are hand-implemented against
        // `.as_str()` only (lsp-types 0.97.0's uri.rs), which those cells
        // never affect. A false positive for this specific type.
        #[allow(clippy::mutable_key_type)]
        let mut diag_batch: FxHashMap<
            (ServerId, lsp_types::Uri),
            lsp_types::PublishDiagnosticsParams,
        > = FxHashMap::default();
        for (server_id, ev) in events {
            let actions = match self.lsp.servers.get_mut(&server_id) {
                Some(entry) => entry.client.on_event(ev),
                None => continue,
            };
            for action in actions {
                if let ClientAction::Diagnostics(params) = action {
                    diag_batch.insert((server_id, params.uri.clone()), params);
                    continue;
                }
                self.dispatch_lsp_action(server_id, action);
            }
        }
        // OnDiagnosticsChanged fires once per buffer this batch actually
        // touched — a FxHashSet dedupes two (server, uri) entries that both
        // resolved to the same buffer (multiple roots, same file; not a v1
        // scenario, but cheap to get right).
        let mut touched: FxHashSet<BufferId> = FxHashSet::default();
        for ((server_id, _uri), params) in diag_batch {
            if let Some(bid) = self.ingest_publish_diagnostics(server_id, params) {
                touched.insert(bid);
            }
        }
        for bid in touched {
            self.queue_diagnostics_changed(bid);
        }

        let now = Instant::now();

        // Advance the statusline loading spinner while any server is mid-
        // handshake or reporting `$/progress` — idle otherwise, so the
        // frame counter doesn't drift while there's nothing to animate.
        if self.lsp.has_animating_server() {
            self.lsp.spinner.maybe_advance(now);
        }

        let server_ids: Vec<ServerId> = self.lsp.servers.keys().copied().collect();
        for server_id in server_ids {
            let LspState {
                servers, backend, ..
            } = &mut self.lsp;
            let (completed, actions) = match servers.get_mut(&server_id) {
                Some(entry) => entry.client.take_completed(backend.as_mut(), now),
                None => continue,
            };
            for action in actions {
                self.dispatch_lsp_action(server_id, action);
            }
            for (id, meta, outcome) in completed {
                self.dispatch_completed(server_id, id, meta, outcome);
            }
        }
    }

    /// [`lsp_shutdown_all`](Self::lsp_shutdown_all)'s production grace
    /// window — the value `hume_editor::run`'s post-loop teardown actually
    /// uses; tests pass their own to exercise the zero- and long-window
    /// edges. `hume_platform::QUIT_GRACE` is sized against this constant —
    /// keep the two in step.
    pub(crate) const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

    /// Graceful shutdown on quit: `begin_shutdown` (shutdown request, then
    /// exit notification) for every Running client, then a bounded grace
    /// window draining for their voluntary EOF, before transport-level
    /// teardown (`backend.shutdown`, which reaps any process still alive)
    /// regardless. Starting clients skip the protocol handshake — nothing
    /// but `initialize` is legal to send before `initialized`, so a plain
    /// transport kill is the only option for them.
    ///
    /// Events drained during the grace window are otherwise discarded — a
    /// lingering response or stderr line has nowhere useful to go while the
    /// editor is tearing down.
    pub(crate) fn lsp_shutdown_all(&mut self, grace: Duration) {
        if self.lsp.servers.is_empty() {
            return;
        }

        let server_ids: Vec<ServerId> = self.lsp.servers.keys().copied().collect();
        let mut awaiting_eof: FxHashSet<ServerId> = FxHashSet::default();
        for &server_id in &server_ids {
            let LspState {
                servers, backend, ..
            } = &mut self.lsp;
            if let Some(entry) = servers.get_mut(&server_id)
                && entry.client.state() == ServerState::Running
            {
                entry.client.begin_shutdown(backend.as_mut());
                awaiting_eof.insert(server_id);
            }
        }

        if !awaiting_eof.is_empty() {
            let deadline = Instant::now() + grace;
            while !awaiting_eof.is_empty() && Instant::now() < deadline {
                for (server_id, ev) in self.lsp.backend.drain() {
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
            self.lsp.backend.shutdown(server_id);
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
                    self.lsp.backend.send(server_id, msg);
                }
                // Decode once here rather than per `(lsp-capabilities …)`
                // call — conversion is per-server-startup, not per-call.
                let json = self
                    .lsp
                    .servers
                    .get(&server_id)
                    .and_then(|e| e.client.capabilities())
                    .and_then(|caps| serde_json::to_value(caps).ok());
                if let Some(json) = json
                    && let Some(entry) = self.lsp.servers.get_mut(&server_id)
                {
                    entry.capabilities_json = Some(json);
                }
                // Fire on-lsp-attach for every buffer already attached to
                // this server — it was Starting until now, so `lsp_attach_buffer`
                // deliberately skipped firing it for them.
                if let Some(lang) = introspect::server_language(&self.lsp, server_id) {
                    let bids: Vec<BufferId> = self
                        .state
                        .buffers
                        .iter()
                        .filter(|(_, buf)| buf.lsp_server == Some(server_id))
                        .map(|(bid, _)| bid)
                        .collect();
                    for bid in bids {
                        self.queue_lsp_attach(bid, &lang);
                    }
                }
            }
            ClientAction::Crashed { error } => {
                let name = self.lsp_server_name(server_id);
                self.report(
                    Severity::Error,
                    format!(
                        "lsp: {name} crashed{}",
                        error.map(|e| format!(": {e}")).unwrap_or_default()
                    ),
                );
                // Fail every in-flight request immediately rather than
                // leaving each to expire on its own deadline — the crash is
                // already known, so there's nothing to wait for. Mirrors
                // `:lsp-stop`'s own teardown (`lsp_stop_one`).
                if let Some(entry) = self.lsp.servers.get_mut(&server_id) {
                    // A crashed server can't finish whatever it was loading —
                    // drop its tracked progress so the statusline spinner
                    // doesn't keep animating for a server that's gone.
                    entry.progress.clear();
                    for (id, meta) in entry.client.drain_pending() {
                        self.dispatch_completed(server_id, id, meta, Outcome::TimedOut);
                    }
                }
            }
            ClientAction::ServerRequest { id, method, params } => {
                // `workspace/applyEdit` needs `&mut Editor` (the edit engine) —
                // every other request answers from the pure lookup table.
                let result = if method == lsp_types::request::ApplyWorkspaceEdit::METHOD {
                    self.apply_edit_request_response(&params)
                } else {
                    let settings = introspect::server_language(&self.lsp, server_id)
                        .and_then(|lang| self.lsp.configs.get(&lang))
                        .and_then(|cfg| cfg.settings.as_ref());
                    server_request_response(&method, &params, settings)
                };
                self.lsp
                    .backend
                    .send(server_id, Message::Response { id, result });
            }
            ClientAction::Diagnostics(params) => {
                // The uncoalesced single-notification path — `drain_lsp`'s
                // batching loop intercepts and coalesces `Diagnostics`
                // before dispatch, so this arm only fires for a test or any
                // future caller that dispatches one directly.
                if let Some(bid) = self.ingest_publish_diagnostics(server_id, params) {
                    self.queue_diagnostics_changed(bid);
                }
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
                // rust-analyzer logs a lot — Trace keeps :messages usable;
                // never promote stderr to a higher severity.
                let name = self.lsp_server_name(server_id);
                self.report(Severity::Trace, format!("{name}: {line}"));
            }
        }
    }

    /// Name used to prefix this server's log lines — the registered
    /// `command` string, or `"lsp"` if the server was never registered
    /// through the normal path (shouldn't happen outside tests).
    pub(super) fn lsp_server_name(&self, server_id: ServerId) -> String {
        self.lsp
            .servers
            .get(&server_id)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| "lsp".to_string())
    }

    /// `textDocument/publishDiagnostics`, `$/progress`, `window/logMessage`,
    /// and `window/showMessage` never reach here — `hume-lsp` classifies
    /// them into typed `ClientAction` variants, handled directly in
    /// `dispatch_lsp_action`. Only an unclassified method, or a known
    /// method whose params fail both the strict parse and `hume-lsp`'s
    /// lenient recovery, arrives here — either goes to a registered Steel
    /// `on-lsp-notification` handler, or an "unhandled notification" Trace
    /// line if none is registered.
    fn dispatch_server_notification(
        &mut self,
        server_id: ServerId,
        method: &str,
        params: serde_json::Value,
    ) {
        let name = self.lsp_server_name(server_id);
        let handlers = self
            .scripting
            .as_ref()
            .map(|h| h.lsp_notification_handlers_for(method))
            .unwrap_or_default();
        if handlers.is_empty() {
            self.report(
                Severity::Trace,
                format!("{name}: unhandled notification {method}"),
            );
            return;
        }
        // The registered language is the "server name" the Steel surface deals
        // in, since that's what `register-lsp-server!` and `lsp-request`'s
        // `server` argument both use.
        let server_val = match introspect::server_language(&self.lsp, server_id) {
            Some(lang) => steel::rvals::SteelVal::StringV(lang.into()),
            None => steel::rvals::SteelVal::BoolV(false),
        };
        let params_val = hume_scripting::json::json_to_steel(&params);
        for handler in handlers {
            self.state
                .queue_steel_call(handler, vec![server_val.clone(), params_val.clone()]);
        }
    }

    pub(super) fn dispatch_completed(
        &mut self,
        server_id: ServerId,
        id: RequestId,
        meta: RequestMeta,
        outcome: Outcome,
    ) {
        // A tracked `#:supersede` entry for this id is finished with —
        // response, timeout, crash-drain, and `:lsp-stop`-drain all arrive
        // here, so this is the one chokepoint that can't miss any of them.
        self.lsp
            .supersede
            .retain(|(sid, _), rid| !(*sid == server_id && *rid == id));

        let Some(entry) = self.lsp.callbacks.remove(&(server_id, id)) else {
            // No callback is ever registered for the internal `shutdown`
            // request (it's fire-and-forget from `begin_shutdown`) — a
            // server-side error on it would otherwise vanish silently.
            if meta.method == lsp_types::request::Shutdown::METHOD
                && let Outcome::Err(e) = &outcome
            {
                self.report(
                    Severity::Trace,
                    format!("lsp: shutdown failed: {} ({})", e.message, e.code),
                );
            }
            return;
        };

        if matches!(outcome, Outcome::TimedOut) {
            self.report(Severity::Trace, format!("lsp: {} timed out", meta.method));
            // Dispatched (not dropped): a callback that never fires on
            // timeout means a caller (e.g. a Steel err-mapped callback)
            // has no way to notice and would hang silently. TimedOut still
            // goes through the staleness check below like any other outcome.
        }

        if let Some((bid, text_gen)) = entry.stale_check {
            let current = self.state.buffers.try_get(bid).map(|b| b.text_gen);
            if current != Some(text_gen) && !meta.allow_stale {
                return; // dropped silently — parse-worker staleness discipline
            }
        }

        (entry.callback)(self, outcome);
    }
}
