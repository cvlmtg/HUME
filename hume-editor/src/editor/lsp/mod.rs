//! Editor-side LSP state: holds the backend and per-server client state,
//! and drains events at frame cadence. Wires the backend + `AsyncSource`
//! plumbing, per-client lifecycle state, request/callback bookkeeping and
//! server->client dispatch (this module), document sync, diagnostics,
//! registration, and observability commands.

mod bridge;
pub(crate) mod completion;
mod diagnostics;
pub(crate) mod edits;
pub(crate) mod introspect;
mod registry;
pub(crate) mod sync;

use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::path::PathBuf;
use std::time::{Duration, Instant};

use hume_engine::pipeline::BufferId;
use hume_lsp::backend::{LspBackend, ServerId, ThreadedLspBackend};
use hume_lsp::client::{ClientAction, LspClient, Outcome, RequestMeta, ServerState};
use hume_lsp::codec::{Message, RequestId, ResponseError};
#[cfg(test)]
use hume_lsp::inline::InlineLspBackend;
use hume_lsp::transport::InboundEvent;

use super::Editor;
use super::async_source::{AsyncSource, PENDING_POLL};
use super::message_log::Severity;
use diagnostics::DiagnosticsStore;
pub(crate) use diagnostics::{DiagSeverity, StoredDiag};
use registry::LspServerConfig;

/// How often to poll while any LSP server is running, so idle-time server
/// pushes (e.g. `publishDiagnostics` after the user stops typing) don't sit
/// undrained until the next keypress — `event::read()` cannot be woken
/// externally.
const LSP_HEARTBEAT: Duration = Duration::from_millis(200);

/// A Rust closure run with a completed request's outcome. `hume-lsp` never
/// holds this — it only ever sees the `(ServerId, RequestId)` pair the
/// editor keys its callback under, which `hume-lsp` already hands back from
/// `send_request`/`take_completed`/`drain_pending`.
pub(crate) type LspCallback = Box<dyn FnOnce(&mut Editor, Outcome)>;

struct CallbackEntry {
    callback: LspCallback,
    /// If `Some((bid, text_gen))` and the buffer has moved past `text_gen`
    /// by drain time, the outcome is dropped silently unless the request's
    /// `allow_stale` opts out — the parse-worker staleness discipline.
    stale_check: Option<(BufferId, u64)>,
}

/// Everything tracked per running (or starting) LSP server, one entry per
/// `ServerId` — single source of truth, no separate (language, root) index:
/// `client.root` already carries the workspace root, so an attach/resolve
/// lookup scans `LspState.servers` (at most a handful of entries running at
/// once) instead of maintaining a second map that could drift out of sync
/// with this one.
struct ServerEntry {
    client: LspClient,
    /// The language this server was registered under
    /// (`register-lsp-server!`'s key) — `None` only for a client inserted
    /// directly by a test without going through `lsp_attach_buffer`.
    language: Option<String>,
    /// Display name (the registered `command`, e.g. `"rust-analyzer"`) —
    /// used to prefix stderr/log lines so `:messages` reads legibly
    /// with multiple servers running.
    name: String,
    /// Decoded `ServerCapabilities`, cached once at handshake completion
    /// (`dispatch_lsp_action`'s `BecameRunning` arm) — the
    /// `(lsp-capabilities …)` builtin reads this rather than reconverting the typed
    /// caps on every call.
    capabilities_json: Option<serde_json::Value>,
}

pub(crate) struct LspState {
    backend: Box<dyn LspBackend>,
    servers: HashMap<ServerId, ServerEntry>,
    /// Keyed by the `(ServerId, RequestId)` pair a callback's own request
    /// was sent under — `drain_lsp` already has both in scope at dispatch
    /// time (the per-server loop, then the response/timeout's own id), so
    /// no separate token needs to be minted or round-tripped.
    callbacks: HashMap<(ServerId, RequestId), CallbackEntry>,
    /// Config recorded by `register-lsp-server!`, keyed by language.
    configs: HashMap<String, LspServerConfig>,
    diagnostics: DiagnosticsStore,
}

impl LspState {
    /// Shared constructor body — every entry point differs only in which
    /// backend it plugs in.
    fn with_backend(backend: Box<dyn LspBackend>) -> Self {
        Self {
            backend,
            servers: HashMap::new(),
            callbacks: HashMap::new(),
            configs: HashMap::new(),
            diagnostics: DiagnosticsStore::default(),
        }
    }

    /// Production constructor: one real server process per registration.
    pub(crate) fn new_threaded() -> Self {
        Self::with_backend(Box::new(ThreadedLspBackend::new()))
    }

    /// Test constructor: scripted responses, no process, no threads.
    #[cfg(test)]
    pub(crate) fn new_inline() -> Self {
        Self::with_backend(Box::new(InlineLspBackend::new()))
    }

    /// Test-only: swap in an already-scripted backend (e.g. one built via
    /// `InlineLspBackend::with_default_handshake` plus extra `respond_to`
    /// calls) — `backend_mut` only exposes the trait object, which can't
    /// reach `InlineLspBackend`'s scripting methods.
    #[cfg(test)]
    pub(crate) fn from_backend_for_test(backend: Box<dyn LspBackend>) -> Self {
        Self::with_backend(backend)
    }

    /// Reach the raw backend directly. Test-only in practice (the scripted
    /// round-trip test): production code goes through `drain_lsp`'s direct
    /// field access instead.
    #[allow(dead_code)]
    pub(crate) fn backend_mut(&mut self) -> &mut dyn LspBackend {
        self.backend.as_mut()
    }

    /// Test-only direct client insertion; the real registration
    /// path (`register-lsp-server!` -> spawn-on-first-open) populates
    /// this map in production. Inserted with no language — tests that need
    /// one call `insert_server_key_for_test` next.
    #[cfg(test)]
    pub(crate) fn insert_client_for_test(&mut self, client: LspClient) -> ServerId {
        let id = client.id;
        self.servers.insert(
            id,
            ServerEntry {
                client,
                language: None,
                name: "lsp".to_string(),
                capabilities_json: None,
            },
        );
        id
    }

    /// `root` must match the client's own `root` (`LspClient::new`'s
    /// second argument) — a real attach never has these disagree, since
    /// both come from the same `resolve_root` call.
    #[cfg(test)]
    pub(crate) fn insert_server_key_for_test(
        &mut self,
        language: String,
        root: PathBuf,
        server_id: ServerId,
    ) {
        let entry = self
            .servers
            .get_mut(&server_id)
            .expect("insert_client_for_test first");
        assert_eq!(
            entry.client.root, root,
            "test key root must match the client's own root"
        );
        entry.language = Some(language);
    }

    #[cfg(test)]
    pub(crate) fn insert_server_name_for_test(&mut self, server_id: ServerId, name: String) {
        if let Some(entry) = self.servers.get_mut(&server_id) {
            entry.name = name;
        }
    }

    #[cfg(test)]
    pub(crate) fn client_for_test(&mut self, server: ServerId) -> Option<&mut LspClient> {
        self.servers.get_mut(&server).map(|e| &mut e.client)
    }

    /// Number of tracked servers — one entry per `backend.start`, so a
    /// second buffer attaching under the same (language, root) key (rather
    /// than spawning) leaves this unchanged.
    #[cfg(test)]
    pub(crate) fn server_count_for_test(&self) -> usize {
        self.servers.len()
    }

    #[cfg(test)]
    pub(crate) fn diagnostic_counts_for_test(&self, bid: BufferId) -> (usize, usize) {
        self.diagnostics.counts(bid)
    }

    /// Number of registered callbacks still awaiting dispatch — a leak
    /// check: every callback must eventually be removed by `dispatch_completed`
    /// (response, timeout, or teardown), never orphaned.
    #[cfg(test)]
    pub(crate) fn callback_count_for_test(&self) -> usize {
        self.callbacks.len()
    }

    /// Diagnostics visible in `range` (buffer-wide char offsets) for `bid`,
    /// at or above `floor` severity — the render write side reads this
    /// directly (no JSON round-trip; that's
    /// `introspect::diagnostics_for_buffer`'s job for Steel).
    pub(crate) fn diagnostics_for_range(
        &self,
        bid: BufferId,
        range: std::ops::Range<usize>,
        floor: DiagSeverity,
    ) -> impl Iterator<Item = &StoredDiag> {
        self.diagnostics.for_range(bid, range, floor)
    }

    /// Drops every diagnostic for `bid`, across every server — called from
    /// `close_buffer`. A pure memory-leak fix: `bid` is a versioned slotmap
    /// key, so a future reused slot can never alias with the closed
    /// buffer's stale entries, but nothing else ever frees them.
    pub(crate) fn remove_buffer_diagnostics(&mut self, bid: BufferId) {
        self.diagnostics.remove_buffer(bid);
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
            servers, backend, ..
        } = self;
        let client = &mut servers.get_mut(&server)?.client;
        Some((client, backend.as_mut()))
    }

    /// Files `callback` under an already-sent request's `(server, id)` —
    /// `drain_lsp`'s per-server loop already has both in scope at dispatch
    /// time, so no separate token needs to be minted. Production caller:
    /// `bridge::send_one_lsp_request`, called after `send_request`
    /// returns the id.
    pub(crate) fn register_callback(
        &mut self,
        server: ServerId,
        id: RequestId,
        stale_check: Option<(BufferId, u64)>,
        callback: LspCallback,
    ) {
        self.callbacks.insert(
            (server, id),
            CallbackEntry {
                callback,
                stale_check,
            },
        );
    }

    /// Sends a request through `server`'s client, if one is registered.
    /// `None` if `server` has no tracked client (can't happen with the real
    /// registration path; still must not panic).
    pub(crate) fn send_request(
        &mut self,
        server: ServerId,
        method: &str,
        params: serde_json::Value,
        meta: RequestMeta,
    ) -> Option<RequestId> {
        let client = &mut self.servers.get_mut(&server)?.client;
        Some(client.send_request(self.backend.as_mut(), method, params, meta))
    }
}

impl AsyncSource for LspState {
    fn next_wake(&self, now: Instant) -> Option<Instant> {
        // Mid-handshake, the initialize response could land any moment —
        // and after that, anything queued while `Starting` must flush
        // promptly. A request in flight gets the same short cadence, not
        // the coarser Running-idle heartbeat below.
        let pending = self.backend.has_pending()
            || self
                .servers
                .values()
                .any(|e| e.client.state == ServerState::Starting || e.client.pending_count() > 0);
        if pending {
            return Some(now + PENDING_POLL);
        }

        self.servers
            .values()
            .any(|e| e.client.state == ServerState::Running)
            .then(|| now + LSP_HEARTBEAT)
    }
}

impl Editor {
    /// `(errors, warnings)` for `bid` from the diagnostics store — the
    /// statusline's `Diagnostics` element reads this directly (never through
    /// Steel; `self.lsp` is private to `editor` and its descendants, so
    /// callers outside it, like `ui::statusline`, go through this).
    pub(crate) fn diagnostic_counts(&self, bid: BufferId) -> (usize, usize) {
        introspect::diagnostic_counts(&self.lsp, bid)
    }

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
            let actions = match self.lsp.servers.get_mut(&server_id) {
                Some(entry) => entry.client.on_event(ev),
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
        // OnDiagnosticsChanged fires once per buffer this batch actually
        // touched — a HashSet dedupes two (server, uri) entries that both
        // resolved to the same buffer (multiple roots, same file; not a v1
        // scenario, but cheap to get right).
        let mut touched: HashSet<BufferId> = HashSet::new();
        for ((server_id, _uri), params) in diag_batch {
            if let Some(bid) = self.ingest_publish_diagnostics(server_id, params) {
                touched.insert(bid);
            }
        }
        for bid in touched {
            self.fire_hook_diagnostics_changed(bid);
        }

        let now = Instant::now();
        let server_ids: Vec<ServerId> = self.lsp.servers.keys().copied().collect();
        for server_id in server_ids {
            let LspState {
                servers, backend, ..
            } = &mut self.lsp;
            let completed = match servers.get_mut(&server_id) {
                Some(entry) => entry.client.take_completed(backend.as_mut(), now),
                None => continue,
            };
            for (id, meta, outcome) in completed {
                self.dispatch_completed(server_id, id, meta, outcome);
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
        if self.lsp.servers.is_empty() {
            return;
        }

        let server_ids: Vec<ServerId> = self.lsp.servers.keys().copied().collect();
        let mut awaiting_eof: HashSet<ServerId> = HashSet::new();
        for &server_id in &server_ids {
            let LspState {
                servers, backend, ..
            } = &mut self.lsp;
            if let Some(entry) = servers.get_mut(&server_id)
                && entry.client.state == ServerState::Running
            {
                entry.client.begin_shutdown(backend.as_mut());
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
                // Decode once here rather than per `(lsp-capabilities …)`
                // call — conversion is per-server-startup, not per-call.
                let json = self
                    .lsp
                    .servers
                    .get(&server_id)
                    .and_then(|e| e.client.caps.as_ref())
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
                        self.fire_hook_lsp_attach(bid, &lang);
                    }
                }
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
                // `workspace/applyEdit` needs `&mut Editor` (the edit engine) —
                // every other request answers from the pure lookup table.
                let result = if method == "workspace/applyEdit" {
                    self.apply_edit_request_response(&params)
                } else {
                    server_request_response(&method, &params)
                };
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

    /// Answers a server-initiated `workspace/applyEdit` request by actually
    /// applying it. Per spec this never fails at the JSON-RPC level: a rejected or
    /// malformed edit still gets a 200 response, just with `applied: false`.
    pub(crate) fn apply_edit_request_response(
        &mut self,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, ResponseError> {
        let Some(edit_json) = params.get("edit").cloned() else {
            return Ok(serde_json::json!({
                "applied": false,
                "failureReason": "missing edit",
            }));
        };
        let we: lsp_types::WorkspaceEdit = match serde_json::from_value(edit_json) {
            Ok(we) => we,
            Err(e) => {
                return Ok(serde_json::json!({
                    "applied": false,
                    "failureReason": format!("malformed edit: {e}"),
                }));
            }
        };
        match edits::apply_workspace_edit(&mut self.state, &mut self.view, &self.lsp, we) {
            Ok(_summary) => Ok(serde_json::json!({ "applied": true })),
            Err(e) => Ok(serde_json::json!({
                "applied": false,
                "failureReason": e,
            })),
        }
    }

    /// Name used to prefix this server's log lines — the registered
    /// `command` string, or `"lsp"` if the server was never registered
    /// through the normal path (shouldn't happen outside tests).
    fn lsp_server_name(&self, server_id: ServerId) -> String {
        self.lsp
            .servers
            .get(&server_id)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| "lsp".to_string())
    }

    /// The registered language for `server_id` — the "server name" the
    /// Steel surface deals in, since that's what `register-lsp-server!` and
    /// `lsp-request`'s `server` argument both use.
    fn lsp_server_language(&self, server_id: ServerId) -> Option<String> {
        introspect::server_language(&self.lsp, server_id)
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
                match params
                    .get("value")
                    .and_then(|v| v.get("kind"))
                    .and_then(|v| v.as_str())
                {
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

    fn dispatch_completed(
        &mut self,
        server_id: ServerId,
        id: RequestId,
        meta: RequestMeta,
        outcome: Outcome,
    ) {
        let Some(entry) = self.lsp.callbacks.remove(&(server_id, id)) else {
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

    /// `:lsp-status` text: one line per registered server (language, root,
    /// lifecycle state, in-flight request count, negotiated encoding),
    /// followed by one line per attached buffer with its diagnostic counts.
    pub(in crate::editor) fn lsp_status_text(&self) -> String {
        let mut servers: Vec<(&str, &LspClient)> = self
            .lsp
            .servers
            .values()
            .filter_map(|e| e.language.as_deref().map(|lang| (lang, &e.client)))
            .collect();
        servers.sort_by(|a, b| a.0.cmp(b.0).then_with(|| a.1.root.cmp(&b.1.root)));

        let mut lines = Vec::new();
        if servers.is_empty() {
            lines.push("No LSP servers registered.".to_string());
        }
        for (language, client) in servers {
            lines.push(format!(
                "{language} @ {} — {:?}, {} in flight, encoding: {:?}",
                client.root.display(),
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

/// Answers a server-initiated request. Exhaustive by design (answered in
/// Rust, never surfaced to Steel) — every request gets exactly
/// one response, even the ones this v1 doesn't otherwise support.
fn server_request_response(
    method: &str,
    params: &serde_json::Value,
) -> Result<serde_json::Value, ResponseError> {
    match method {
        // No settings blob exists — every item answers `null`,
        // same shape a server sees from a client with no matching config.
        "workspace/configuration" => Ok(workspace_configuration_response(params)),
        // Answered separately by `apply_edit_request_response` (needs
        // `&mut Editor`, unlike every other request this lookup answers).
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
        let params =
            serde_json::json!({"items": [{"section": "rust-analyzer"}, {"section": "editor"}]});
        let result = server_request_response("workspace/configuration", &params).unwrap();
        assert_eq!(result, serde_json::json!([null, null]));
    }

    #[test]
    fn workspace_configuration_with_no_items_answers_empty_array() {
        let params = serde_json::json!({"items": []});
        let result = server_request_response("workspace/configuration", &params).unwrap();
        assert_eq!(result, serde_json::json!([]));
    }

    // `workspace/applyEdit` moved to `apply_edit_request_response` (needs
    // `&mut Editor`) — see `editor::tests::lsp_edits` for its coverage.

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
