//! Editor-side LSP state: holds the backend and per-server client state,
//! and drains events at frame cadence. Built incrementally — C4 wires the
//! backend + `AsyncSource` plumbing, C5 adds per-client lifecycle state,
//! C6 (this module) adds request/callback bookkeeping and server->client
//! dispatch; C7–C10 add document sync, diagnostics, registration, and
//! observability commands on top.

mod bridge;
mod diagnostics;
mod registry;
pub(crate) mod sync;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use hume_engine::pipeline::BufferId;
use hume_lsp::backend::{LspBackend, ServerId, ThreadedLspBackend};
use hume_lsp::client::{CallbackToken, ClientAction, LspClient, Outcome, RequestMeta, ServerState};
use hume_lsp::codec::{Message, RequestId, ResponseError};
use hume_lsp::transport::InboundEvent;
#[cfg(test)]
use hume_lsp::inline::InlineLspBackend;

use super::Editor;
use super::async_source::AsyncSource;
use super::message_log::Severity;
use diagnostics::DiagnosticsStore;
use registry::LspServerConfig;

/// How often to poll while any LSP server is running, so idle-time server
/// pushes (e.g. `publishDiagnostics` after the user stops typing) don't sit
/// undrained until the next keypress — `event::read()` cannot be woken
/// externally. See the LSP hub's "Idle wake" decision.
const LSP_HEARTBEAT: Duration = Duration::from_millis(200);

/// A Rust closure run with a completed request's outcome. `hume-lsp` never
/// holds this — only the opaque `CallbackToken` crosses the crate fence.
pub(crate) type LspCallback = Box<dyn FnOnce(&mut Editor, Outcome)>;

struct CallbackEntry {
    callback: LspCallback,
    /// If `Some((bid, text_gen))` and the buffer has moved past `text_gen`
    /// by drain time, the outcome is dropped silently unless the request's
    /// `allow_stale` opts out — the parse-worker staleness discipline.
    stale_check: Option<(BufferId, u64)>,
}

pub(crate) struct LspState {
    backend: Box<dyn LspBackend>,
    clients: HashMap<ServerId, LspClient>,
    callbacks: HashMap<CallbackToken, CallbackEntry>,
    next_token: u64,
    /// Config recorded by `register-lsp-server!`, keyed by language.
    configs: HashMap<String, LspServerConfig>,
    /// Running (or starting) server per (language, resolved root) — the
    /// first buffer under a pair spawns; later buffers with the same pair
    /// attach to the existing entry.
    servers_by_key: HashMap<(String, PathBuf), ServerId>,
    diagnostics: DiagnosticsStore,
    /// Display name (the registered `command`, e.g. `"rust-analyzer"`) per
    /// server — used to prefix stderr/log lines (C10) so `:messages` reads
    /// legibly with multiple servers running.
    server_names: HashMap<ServerId, String>,
}

impl LspState {
    /// Production constructor: one real server process per registration (C8).
    pub(crate) fn new_threaded() -> Self {
        Self {
            backend: Box::new(ThreadedLspBackend::new()),
            clients: HashMap::new(),
            callbacks: HashMap::new(),
            next_token: 0,
            configs: HashMap::new(),
            servers_by_key: HashMap::new(),
            diagnostics: DiagnosticsStore::default(),
            server_names: HashMap::new(),
        }
    }

    /// Test constructor: scripted responses, no process, no threads.
    #[cfg(test)]
    pub(crate) fn new_inline() -> Self {
        Self {
            backend: Box::new(InlineLspBackend::new()),
            clients: HashMap::new(),
            callbacks: HashMap::new(),
            next_token: 0,
            configs: HashMap::new(),
            servers_by_key: HashMap::new(),
            diagnostics: DiagnosticsStore::default(),
            server_names: HashMap::new(),
        }
    }

    /// Test-only: swap in an already-scripted backend (e.g. one built via
    /// `InlineLspBackend::with_default_handshake` plus extra `respond_to`
    /// calls) — `backend_mut` only exposes the trait object, which can't
    /// reach `InlineLspBackend`'s scripting methods.
    #[cfg(test)]
    pub(crate) fn from_backend_for_test(backend: Box<dyn LspBackend>) -> Self {
        Self {
            backend,
            clients: HashMap::new(),
            callbacks: HashMap::new(),
            next_token: 0,
            configs: HashMap::new(),
            servers_by_key: HashMap::new(),
            diagnostics: DiagnosticsStore::default(),
            server_names: HashMap::new(),
        }
    }

    /// Reach the raw backend directly. Test-only in practice (the C4
    /// round-trip test): production code goes through `drain_lsp`'s direct
    /// field access instead.
    #[allow(dead_code)]
    pub(crate) fn backend_mut(&mut self) -> &mut dyn LspBackend {
        self.backend.as_mut()
    }

    /// Test-only direct client insertion; C8 adds the real registration
    /// path (`register-lsp-server!` -> spawn-on-first-open) that populates
    /// this map in production.
    #[cfg(test)]
    pub(crate) fn insert_client_for_test(&mut self, client: LspClient) -> ServerId {
        let id = client.id;
        self.clients.insert(id, client);
        id
    }

    #[cfg(test)]
    pub(crate) fn insert_server_key_for_test(
        &mut self,
        language: String,
        root: PathBuf,
        server_id: ServerId,
    ) {
        self.servers_by_key.insert((language, root), server_id);
    }

    #[cfg(test)]
    pub(crate) fn insert_server_name_for_test(&mut self, server_id: ServerId, name: String) {
        self.server_names.insert(server_id, name);
    }

    #[cfg(test)]
    pub(crate) fn client_for_test(&mut self, server: ServerId) -> Option<&mut LspClient> {
        self.clients.get_mut(&server)
    }

    /// Number of distinct (language, root) keys currently tracked.
    #[cfg(test)]
    pub(crate) fn server_count_for_test(&self) -> usize {
        self.servers_by_key.len()
    }

    /// Number of `LspClient`s ever inserted — unlike `server_count_for_test`
    /// (keyed by (language, root), so a respawn silently overwrites the same
    /// key), a second `backend.start` always adds a new entry here. The two
    /// counts must stay equal for "attach, don't respawn" to actually hold.
    #[cfg(test)]
    pub(crate) fn client_count_for_test(&self) -> usize {
        self.clients.len()
    }

    #[cfg(test)]
    pub(crate) fn diagnostic_counts_for_test(&self, bid: BufferId) -> (usize, usize) {
        self.diagnostics.counts(bid)
    }

    #[cfg(test)]
    pub(crate) fn diagnostics_for_test(
        &self,
        bid: BufferId,
    ) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.diagnostics
            .for_range(bid, 0..usize::MAX, diagnostics::DiagSeverity::Hint)
            .map(|d| (d.start, d.end))
    }

    /// Disjoint-borrow accessor for callers that need to drive a client and
    /// its backend in the same call (`send_or_queue`, `start_handshake`) —
    /// a plain two-method-call sequence can't do this from outside
    /// `LspState` since `backend_mut`/`client_for_test` each borrow the
    /// whole struct. Production caller: every send site in `sync.rs`, so
    /// document sync respects the Starting-queue instead of writing to the
    /// wire directly.
    pub(crate) fn client_and_backend(
        &mut self,
        server: ServerId,
    ) -> Option<(&mut LspClient, &mut dyn LspBackend)> {
        let LspState {
            clients, backend, ..
        } = self;
        let client = clients.get_mut(&server)?;
        Some((client, backend.as_mut()))
    }

    /// Mints a fresh token and files `callback` under it. Production caller:
    /// `bridge::send_one_lsp_request` (B2).
    pub(crate) fn register_callback(
        &mut self,
        stale_check: Option<(BufferId, u64)>,
        callback: LspCallback,
    ) -> CallbackToken {
        let token = CallbackToken(self.next_token);
        self.next_token += 1;
        self.callbacks.insert(
            token,
            CallbackEntry {
                callback,
                stale_check,
            },
        );
        token
    }

    /// Sends a request through `server`'s client, if one is registered.
    /// `None` if `server` has no tracked client (can't happen once C8
    /// lands; still must not panic).
    pub(crate) fn send_request(
        &mut self,
        server: ServerId,
        method: &str,
        params: serde_json::Value,
        meta: RequestMeta,
    ) -> Option<RequestId> {
        let client = self.clients.get_mut(&server)?;
        Some(client.send_request(self.backend.as_mut(), method, params, meta))
    }
}

impl AsyncSource for LspState {
    /// True while a client is mid-handshake (the initialize response could
    /// land any moment — and after that, anything queued while `Starting`
    /// must flush promptly) or has a request in flight (the C6 card's 8ms
    /// poll cadence, not the coarser Running-idle heartbeat below).
    fn has_pending(&self) -> bool {
        self.backend.has_pending()
            || self
                .clients
                .values()
                .any(|c| c.state == ServerState::Starting || c.pending_count() > 0)
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.clients
            .values()
            .any(|c| c.state == ServerState::Running)
            .then(|| Instant::now() + LSP_HEARTBEAT)
    }
}

impl Editor {
    /// Per-frame drain: routes every backend event through its client's
    /// `on_event`, dispatches the resulting `ClientAction`s, then pulls
    /// each client's completed requests (responses + timeouts) via
    /// `take_completed` and dispatches those too.
    pub(super) fn drain_lsp(&mut self) {
        self.flush_lsp_pending_changes();

        let events = self.lsp.backend.drain();
        // Coalesce publishDiagnostics within this batch: keep only the last
        // one per (server, uri) — servers burst-publish and only the newest
        // matters. Ingested after the loop so a later action for the same
        // (server, uri) always wins regardless of arrival order within the
        // batch.
        let mut diag_batch: HashMap<(ServerId, String), serde_json::Value> = HashMap::new();
        for (server_id, ev) in events {
            let actions = match self.lsp.clients.get_mut(&server_id) {
                Some(client) => client.on_event(ev),
                None => continue,
            };
            for action in actions {
                if let ClientAction::ServerNotification { method, params } = &action
                    && method == "textDocument/publishDiagnostics"
                {
                    if let Some(uri) = params.get("uri").and_then(|v| v.as_str()) {
                        diag_batch.insert((server_id, uri.to_string()), params.clone());
                    }
                    continue;
                }
                self.dispatch_lsp_action(server_id, action);
            }
        }
        for ((server_id, _uri), params) in diag_batch {
            self.ingest_publish_diagnostics(server_id, params);
        }

        let now = Instant::now();
        let server_ids: Vec<ServerId> = self.lsp.clients.keys().copied().collect();
        for server_id in server_ids {
            let LspState {
                clients, backend, ..
            } = &mut self.lsp;
            let completed = match clients.get_mut(&server_id) {
                Some(client) => client.take_completed(backend.as_mut(), now),
                None => continue,
            };
            for (id, meta, outcome) in completed {
                self.dispatch_completed(id, meta, outcome);
            }
        }
    }

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
    pub(in crate::editor) fn lsp_shutdown_all(&mut self, grace: Duration) {
        if self.lsp.clients.is_empty() {
            return;
        }

        let server_ids: Vec<ServerId> = self.lsp.clients.keys().copied().collect();
        let mut awaiting_eof: HashSet<ServerId> = HashSet::new();
        for &server_id in &server_ids {
            let LspState {
                clients, backend, ..
            } = &mut self.lsp;
            if let Some(client) = clients.get_mut(&server_id)
                && client.state == ServerState::Running
            {
                client.begin_shutdown(backend.as_mut());
                awaiting_eof.insert(server_id);
            }
        }

        if !awaiting_eof.is_empty() {
            let deadline = Instant::now() + grace;
            while !awaiting_eof.is_empty() && Instant::now() < deadline {
                for (server_id, ev) in self.lsp.backend.drain() {
                    if matches!(ev, InboundEvent::Eof { .. }) {
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

    pub(super) fn dispatch_lsp_action(&mut self, server_id: ServerId, action: ClientAction) {
        match action {
            ClientAction::BecameRunning { send } => {
                for msg in send {
                    self.lsp.backend.send(server_id, msg);
                }
                // on-lsp-attach hook: B7 (Step 2).
            }
            ClientAction::Crashed { error } => {
                self.report(
                    Severity::Error,
                    format!(
                        "lsp: server crashed{}",
                        error.map(|e| format!(": {e}")).unwrap_or_default()
                    ),
                );
            }
            ClientAction::ServerRequest { id, method, params } => {
                let result = server_request_response(&method, &params);
                self.lsp
                    .backend
                    .send(server_id, Message::Response { id, result });
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
    fn lsp_server_name(&self, server_id: ServerId) -> String {
        self.lsp
            .server_names
            .get(&server_id)
            .cloned()
            .unwrap_or_else(|| "lsp".to_string())
    }

    /// The registered language for `server_id` (the `servers_by_key` key,
    /// not the display `command` string) — the "server name" B2/B3's Steel
    /// surface deals in, since that's what `register-lsp-server!` and
    /// `lsp-request`'s `server` argument both use.
    fn lsp_server_language(&self, server_id: ServerId) -> Option<String> {
        self.lsp
            .servers_by_key
            .iter()
            .find(|&(_, &id)| id == server_id)
            .map(|((lang, _), _)| lang.clone())
    }

    /// `textDocument/publishDiagnostics` never reaches here — `drain_lsp`
    /// intercepts and coalesces it before dispatch (see the batching loop).
    fn dispatch_server_notification(
        &mut self,
        server_id: ServerId,
        method: &str,
        params: serde_json::Value,
    ) {
        let name = self.lsp_server_name(server_id);
        match method {
            "window/logMessage" => {
                let text = params
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                // MessageType: 1=Error, 2=Warning, 3=Info, 4=Log.
                let severity = match params.get("type").and_then(|v| v.as_i64()) {
                    Some(1) => Severity::Error,
                    Some(2) => Severity::Warning,
                    _ => Severity::Trace, // Info/Log/malformed
                };
                self.report(severity, format!("{name}: {text}"));
            }
            "window/showMessage" => {
                let text = params
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                self.report(Severity::Info, format!("{name}: {text}"));
            }
            "$/progress" => {
                // begin/end at Trace; per-report messages dropped entirely
                // (OQ default) — a real progress bar is Future work.
                match params.get("value").and_then(|v| v.get("kind")).and_then(|v| v.as_str()) {
                    Some("begin") => {
                        let title = params
                            .get("value")
                            .and_then(|v| v.get("title"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("progress");
                        self.report(Severity::Trace, format!("{name}: {title} started"));
                    }
                    Some("end") => {
                        self.report(Severity::Trace, format!("{name}: progress finished"));
                    }
                    _ => {} // "report", or a malformed payload — never spam
                }
            }
            other => {
                let handlers = self
                    .scripting
                    .as_ref()
                    .map(|h| h.lsp_notification_handlers_for(other))
                    .unwrap_or_default();
                if handlers.is_empty() {
                    self.report(
                        Severity::Trace,
                        format!("{name}: unhandled notification {other}"),
                    );
                    return;
                }
                let server_val = match self.lsp_server_language(server_id) {
                    Some(lang) => steel::rvals::SteelVal::StringV(lang.into()),
                    None => steel::rvals::SteelVal::BoolV(false),
                };
                let params_val = hume_scripting::json::json_to_steel(&params);
                for handler in handlers {
                    self.queue_steel_call(handler, vec![server_val.clone(), params_val.clone()]);
                }
            }
        }
    }

    fn dispatch_completed(&mut self, _id: RequestId, meta: RequestMeta, outcome: Outcome) {
        let Some(entry) = self.lsp.callbacks.remove(&meta.token) else {
            return;
        };

        if matches!(outcome, Outcome::TimedOut) {
            self.report(Severity::Trace, format!("lsp: {} timed out", meta.method));
            // Dispatched (not dropped, deviating from the hub C6 card's
            // "timed-out -> log + drop"): a callback that never fires on
            // timeout means a caller (e.g. B2's Steel err-mapped callback)
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

    /// `:lsp-status` text: one line per registered server (language, root,
    /// lifecycle state, in-flight request count, negotiated encoding),
    /// followed by one line per attached buffer with its diagnostic counts.
    pub(in crate::editor) fn lsp_status_text(&self) -> String {
        let mut servers: Vec<(&(String, PathBuf), &ServerId)> =
            self.lsp.servers_by_key.iter().collect();
        servers.sort_by(|a, b| a.0.cmp(b.0));

        let mut lines = Vec::new();
        if servers.is_empty() {
            lines.push("No LSP servers registered.".to_string());
        }
        for ((language, root), &server_id) in servers {
            let Some(client) = self.lsp.clients.get(&server_id) else {
                continue;
            };
            lines.push(format!(
                "{language} @ {} — {:?}, {} in flight, encoding: {:?}",
                root.display(),
                client.state,
                client.pending_count(),
                client.encoding,
            ));
        }

        let mut buffer_lines: Vec<String> = self
            .state
            .buffers
            .iter()
            .filter_map(|(bid, buf)| {
                buf.lsp_server.map(|_| {
                    let (errors, warnings) = self.lsp.diagnostics.counts(bid);
                    format!(
                        "  {} — {errors} error(s), {warnings} warning(s)",
                        buf.display_name()
                    )
                })
            })
            .collect();
        lines.append(&mut buffer_lines);

        lines.join("\n")
    }
}

/// Answers a server-initiated request. Exhaustive by design (hub decision:
/// answered in Rust, never surfaced to Steel) — every request gets exactly
/// one response, even the ones this v1 doesn't otherwise support.
fn server_request_response(
    method: &str,
    params: &serde_json::Value,
) -> Result<serde_json::Value, ResponseError> {
    match method {
        // No settings blob exists until C8 — every item answers `null`,
        // same shape a server sees from a client with no matching config.
        "workspace/configuration" => Ok(workspace_configuration_response(params)),
        "workspace/applyEdit" => Ok(serde_json::json!({
            "applied": false,
            "failureReason": "workspace edits not supported yet",
        })),
        "client/registerCapability"
        | "client/unregisterCapability"
        | "window/workDoneProgress/create" => Ok(serde_json::Value::Null),
        other => Err(ResponseError {
            code: -32601,
            message: format!("method not found: {other}"),
            data: None,
        }),
    }
}

/// One `null` per requested item — same length as `params.items`, per spec
/// (the result array must line up positionally with the request).
fn workspace_configuration_response(params: &serde_json::Value) -> serde_json::Value {
    let item_count = params
        .get("items")
        .and_then(|v| v.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);
    serde_json::Value::Array(vec![serde_json::Value::Null; item_count])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_configuration_answers_null_per_item() {
        let params = serde_json::json!({"items": [{"section": "rust-analyzer"}, {"section": "editor"}]});
        let result = server_request_response("workspace/configuration", &params).unwrap();
        assert_eq!(result, serde_json::json!([null, null]));
    }

    #[test]
    fn workspace_configuration_with_no_items_answers_empty_array() {
        let params = serde_json::json!({"items": []});
        let result = server_request_response("workspace/configuration", &params).unwrap();
        assert_eq!(result, serde_json::json!([]));
    }

    #[test]
    fn workspace_apply_edit_answers_not_supported() {
        let result =
            server_request_response("workspace/applyEdit", &serde_json::Value::Null).unwrap();
        assert_eq!(result["applied"], serde_json::json!(false));
        assert!(result["failureReason"].is_string());
    }

    #[test]
    fn register_and_unregister_capability_and_progress_create_answer_null() {
        for method in [
            "client/registerCapability",
            "client/unregisterCapability",
            "window/workDoneProgress/create",
        ] {
            let result = server_request_response(method, &serde_json::Value::Null).unwrap();
            assert_eq!(result, serde_json::Value::Null, "method {method}");
        }
    }

    #[test]
    fn unknown_server_request_is_method_not_found() {
        let err =
            server_request_response("some/madeUpMethod", &serde_json::Value::Null).unwrap_err();
        assert_eq!(err.code, -32601);
        assert!(err.message.contains("some/madeUpMethod"));
    }
}
