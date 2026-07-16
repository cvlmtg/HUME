//! Per-server client state machine: `initialize` handshake with capability
//! and position-encoding negotiation, graceful shutdown, crash detection.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use hume_editing::PositionEncoding;
use lsp_types::{
    ClientCapabilities, ClientInfo, CodeActionClientCapabilities, CodeActionKind,
    CodeActionKindLiteralSupport, CodeActionLiteralSupport, CompletionClientCapabilities,
    CompletionItemCapability, FailureHandlingKind, GeneralClientCapabilities, GotoCapability,
    HoverClientCapabilities, InitializeParams, InitializeResult, InitializedParams, MarkupKind,
    PositionEncodingKind, PublishDiagnosticsClientCapabilities, RenameClientCapabilities,
    ResourceOperationKind, ServerCapabilities, TextDocumentClientCapabilities,
    TextDocumentSyncClientCapabilities, WindowClientCapabilities, WorkspaceClientCapabilities,
    WorkspaceEditClientCapabilities, WorkspaceFolder,
};

use crate::backend::{LspBackend, ServerId};
use crate::codec::{IdAllocator, Message, RequestId, ResponseError};
use crate::transport::InboundEvent;
use crate::uri;

use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerState {
    Starting,
    Running,
    Crashed,
    Dead,
}

/// One action for the editor glue to take in response to a lifecycle event.
#[derive(Debug)]
pub enum ClientAction {
    /// Handshake completed: send these messages in order (`initialized`
    /// first, then anything queued while `Starting`), then fire
    /// `on-lsp-attach` for buffers already attached to this server.
    BecameRunning { send: Vec<Message> },
    /// The connection died — report once; restart stays manual.
    Crashed { error: Option<String> },
    /// The server sent a request; every one must get exactly one response
    /// (a hung server request can stall its whole pipeline). The dispatch
    /// table lives in the editor glue: answered in Rust,
    /// never surfaced to Steel.
    ServerRequest {
        id: RequestId,
        method: String,
        params: serde_json::Value,
    },
    /// `textDocument/publishDiagnostics`, classified and parsed.
    Diagnostics(lsp_types::PublishDiagnosticsParams),
    /// `$/progress`, classified and parsed.
    Progress(lsp_types::ProgressParams),
    /// `window/logMessage`, classified and parsed.
    LogMessage(lsp_types::LogMessageParams),
    /// `window/showMessage`, classified and parsed.
    ShowMessage(lsp_types::ShowMessageParams),
    /// A notification this module doesn't own the meaning of: an
    /// unclassified method, or a known method whose params fail both the
    /// strict parse and `classify_notification`'s lenient recovery
    /// (surfaced here rather than dropped, so Steel's `on-lsp-notification`
    /// can still observe it).
    ServerNotification {
        method: String,
        params: serde_json::Value,
    },
    /// One line of stderr output, forwarded for logging.
    Stderr(String),
}

/// Classifies a well-known notification method into a typed `ClientAction`.
/// Tries a strict deserialize first; on failure, a narrow lenient recovery
/// patches in a default for the one field each shape is known to omit in
/// the wild (see the `recover_*` helpers below) and retries. Falls back to
/// `ServerNotification` for an unknown method, or params neither pass
/// parses — deserializes by reference so a malformed payload leaves
/// `params` intact for that fallback (`from_value` would consume it).
fn classify_notification(method: String, params: serde_json::Value) -> ClientAction {
    use lsp_types::notification::{LogMessage, Progress, PublishDiagnostics, ShowMessage};
    use serde::Deserialize as _;

    match method.as_str() {
        PublishDiagnostics::METHOD => {
            if let Ok(p) = lsp_types::PublishDiagnosticsParams::deserialize(&params) {
                return ClientAction::Diagnostics(p);
            }
        }
        Progress::METHOD => {
            if let Ok(p) = lsp_types::ProgressParams::deserialize(&params) {
                return ClientAction::Progress(p);
            }
            if let Some(p) = recover_progress(&params) {
                return ClientAction::Progress(p);
            }
        }
        LogMessage::METHOD => {
            if let Ok(p) = lsp_types::LogMessageParams::deserialize(&params) {
                return ClientAction::LogMessage(p);
            }
            if let Some(p) = recover_message(&params, lsp_types::MessageType::LOG) {
                return ClientAction::LogMessage(p);
            }
        }
        ShowMessage::METHOD => {
            if let Ok(p) = lsp_types::ShowMessageParams::deserialize(&params) {
                return ClientAction::ShowMessage(p);
            }
            if let Some(p) = recover_message(&params, lsp_types::MessageType::INFO) {
                return ClientAction::ShowMessage(p);
            }
        }
        _ => {}
    }
    ClientAction::ServerNotification { method, params }
}

/// Recovers a `$/progress` whose `WorkDoneProgress::Begin` omits the
/// lsp_types-required `title` — observed from servers that treat it as
/// optional in practice. Patches a placeholder title into a clone and
/// re-runs the strict parse, so any *other* off-spec shape (an unkeyable
/// `token`, an unknown `kind`) still yields `None` and falls through to
/// `ServerNotification` — unchanged from before this recovery existed.
fn recover_progress(params: &serde_json::Value) -> Option<lsp_types::ProgressParams> {
    use serde::Deserialize as _;

    let mut patched = params.clone();
    let value = patched.get_mut("value")?;
    if value.get("kind").and_then(|k| k.as_str()) == Some("begin") && value.get("title").is_none()
    {
        value
            .as_object_mut()?
            .insert("title".into(), serde_json::json!("progress"));
    }
    lsp_types::ProgressParams::deserialize(&patched).ok()
}

/// Recovers a `window/logMessage`/`window/showMessage` whose `type` is
/// missing or not an integer — `MessageType` is a transparent `i32` newtype,
/// so any integer parses; only the type tag itself is ever the problem.
/// `message` staying missing/non-string is unrecoverable (it's the payload)
/// and still falls through.
fn recover_message<T>(params: &serde_json::Value, default_type: lsp_types::MessageType) -> Option<T>
where
    T: for<'de> serde::Deserialize<'de>,
{
    let mut patched = params.clone();
    let obj = patched.as_object_mut()?;
    let type_ok = obj.get("type").is_some_and(|t| t.is_i64() || t.is_u64());
    if !type_ok {
        obj.insert("type".into(), serde_json::to_value(default_type).ok()?);
    }
    T::deserialize(&patched).ok()
}

/// Everything but the outcome needed to route (or discard) a completed
/// request. `hume-lsp` never holds editor closures (crate fence) — the
/// editor keys its own callback under the `(ServerId, RequestId)` pair
/// this crate already hands back from `send_request`/`take_completed`/
/// `drain_pending`, so no separate token needs to round-trip through here.
#[derive(Debug, Clone)]
pub struct RequestMeta {
    pub method: String,
    pub allow_stale: bool,
    pub deadline: Instant,
}

#[derive(Debug)]
pub enum Outcome {
    Ok(serde_json::Value),
    Err(ResponseError),
    TimedOut,
}

/// Builds the `$/cancelRequest` notification params for `id`. Used by
/// `send_cancel_notification`, this module's single production caller
/// (from both the test-only `cancel` and the timeout sweep in
/// `take_completed`).
fn cancel_request_params(id: &RequestId) -> serde_json::Value {
    let id_value = match id {
        RequestId::Int(n) => serde_json::Value::from(*n),
        RequestId::Str(s) => serde_json::Value::String(s.clone()),
    };
    serde_json::json!({ "id": id_value })
}

/// How long `initialize` may go unanswered before the client gives up and
/// transitions to `Crashed` — deliberately independent of
/// `lsp.request-timeout-ms` (a per-request setting): a cold server's
/// handshake legitimately outlasts the timeout an ordinary request would
/// get.
const INITIALIZE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// How long `shutdown` may go unanswered before its pending entry is swept
/// as timed out. Never observed in production — `:lsp-stop` drains pending
/// requests immediately and quit tears the transport down regardless — but
/// every `pending` entry needs a deadline.
const SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Per-server lifecycle state: handshake, negotiated encoding, and the
/// queue of messages a caller tried to send before the handshake finished.
pub struct LspClient {
    id: ServerId,
    state: ServerState,
    caps: Option<ServerCapabilities>,
    /// Negotiated position encoding; UTF-16 until `initialize` proves UTF-8.
    /// A decode-once cache of `caps.position_encoding` — `handle_initialize_
    /// response` is the only writer of either field, an invariant privacy
    /// now enforces — kept separate so callers don't re-derive it from the
    /// raw capability on every position conversion, not an independent fact
    /// that could drift on its own.
    encoding: PositionEncoding,
    root: PathBuf,
    /// `initializationOptions` for the `initialize` request — set via
    /// `set_init_options` before `start_handshake` to take effect; `None`
    /// omits the field entirely (never sent as `null`).
    init_options: Option<serde_json::Value>,
    /// Messages (e.g. `didOpen`) that arrived while `Starting` — sent, in
    /// order, right after `initialized` once the handshake completes.
    queued: Vec<Message>,
    /// Discriminates the `initialize` response in `on_event` — kept
    /// separate from a method-string check so a Steel-issued `(lsp-request
    /// "initialize" ...)` through the generic bridge (which mints its own
    /// ordinary `pending` entry) can never be mistaken for the handshake.
    initialize_id: Option<RequestId>,
    ids: IdAllocator,
    /// Requests awaiting a response, keyed by the id we sent.
    pending: HashMap<RequestId, RequestMeta>,
    /// Responses matched against `pending` by `on_event`, waiting to be
    /// pulled by `take_completed` — never delivered inline (same
    /// drain-boundary discipline as the `InlineLspBackend` double).
    completed: Vec<(RequestId, RequestMeta, Outcome)>,
}

impl LspClient {
    pub fn new(id: ServerId, root: PathBuf) -> Self {
        Self {
            id,
            state: ServerState::Starting,
            caps: None,
            encoding: PositionEncoding::Utf16,
            root,
            init_options: None,
            queued: Vec::new(),
            initialize_id: None,
            ids: IdAllocator::new(),
            pending: HashMap::new(),
            completed: Vec::new(),
        }
    }

    pub fn id(&self) -> ServerId {
        self.id
    }

    pub fn state(&self) -> ServerState {
        self.state
    }

    pub fn capabilities(&self) -> Option<&ServerCapabilities> {
        self.caps.as_ref()
    }

    /// Sets `initializationOptions` for the upcoming `initialize` request —
    /// must be called before `start_handshake` to take effect.
    pub fn set_init_options(&mut self, init_options: Option<serde_json::Value>) {
        self.init_options = init_options;
    }

    pub fn encoding(&self) -> PositionEncoding {
        self.encoding
    }

    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// Test-only, cross-crate: `hume-editor`'s tests need to force a
    /// client's lifecycle state (e.g. simulate a crash, or skip straight to
    /// `Running`) without a live handshake round-trip.
    #[cfg(any(test, feature = "test-util"))]
    pub fn set_state_for_test(&mut self, state: ServerState) {
        self.state = state;
    }

    /// Sends a request and remembers `meta` for correlation. Requests are
    /// position-independent from this layer's perspective — staleness by
    /// buffer generation is tracked editor-side (the crate fence: `hume-lsp`
    /// has no `BufferId`).
    ///
    /// Goes through `send_or_queue`'s Starting-queue discipline. `pending`
    /// gets the entry regardless of whether the send actually reached the
    /// wire or was queued (or dropped, for a dead/crashed connection), so
    /// `take_completed`'s deadline/timeout handling covers a request that's
    /// still queued exactly like one already on the wire.
    ///
    /// A Crashed/Dead connection has its `meta.deadline` clamped to now: the
    /// send is silently dropped (see `send_or_queue`) and nothing will ever
    /// answer it, so the caller's whole requested timeout — routinely tens
    /// of seconds — must not stand between it and the `TimedOut` outcome
    /// `take_completed`'s sweep already delivers for a request that expires.
    pub fn send_request(
        &mut self,
        backend: &mut dyn LspBackend,
        method: &str,
        params: serde_json::Value,
        mut meta: RequestMeta,
    ) -> RequestId {
        let id = self.ids.next();
        if matches!(self.state, ServerState::Crashed | ServerState::Dead) {
            meta.deadline = Instant::now();
        }
        self.pending.insert(id.clone(), meta);
        let msg = Message::Request {
            id: id.clone(),
            method: method.to_string(),
            params,
        };
        self.send_or_queue(backend, msg);
        id
    }

    /// Requests currently awaiting a response — the "N in flight" count for
    /// `:lsp-status` and `lsp-server-status`. Includes the in-flight
    /// `initialize`/`shutdown` handshake requests, since those are now
    /// ordinary `pending` entries too.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Earliest deadline among pending requests (`initialize`/`shutdown`
    /// included — they are ordinary `pending` entries too). Feeds the
    /// editor's wake predicate: with completion-driven wakes replacing the
    /// old poll cadence, this deadline is what keeps the timeout sweep in
    /// `take_completed` firing promptly even on a server that never
    /// responds.
    pub fn earliest_deadline(&self) -> Option<Instant> {
        self.pending.values().map(|m| m.deadline).min()
    }

    /// Best-effort cancellation: drops the pending entry (if still present),
    /// strips a still-queued Starting-phase send, and — only once the
    /// handshake has completed — sends `$/cancelRequest`. A no-op if the
    /// request already completed. Production caller: the editor bridge's
    /// `#:supersede` path (a new request cancels the caller's previous
    /// still-pending one filed under the same key); also exercised directly
    /// by the unit tests below.
    pub fn cancel(&mut self, backend: &mut dyn LspBackend, id: RequestId) {
        if self.pending.remove(&id).is_some() {
            self.drop_from_queue(&id);
            self.send_cancel_notification(backend, &id);
        }
    }

    /// Removes and returns every still-pending request — for teardown paths
    /// that drop the client (e.g. `:lsp-stop`) so the caller can dispatch
    /// each as timed out rather than silently orphaning a registered
    /// callback along with the client.
    pub fn drain_pending(&mut self) -> Vec<(RequestId, RequestMeta)> {
        self.pending.drain().collect()
    }

    /// Forces every pending deadline into the past, so the next
    /// `take_completed` sweep expires them without waiting out real time —
    /// e.g. to drive an `initialize` timeout in an editor-level test without
    /// an injectable clock.
    #[cfg(any(test, feature = "test-util"))]
    pub fn expire_pending_deadlines_for_test(&mut self) {
        let past = Instant::now() - std::time::Duration::from_secs(1);
        for meta in self.pending.values_mut() {
            meta.deadline = past;
        }
    }

    /// Drops `id`'s `Message::Request` from the Starting-queue, if it's
    /// still sitting there unsent. A cancelled or timed-out request whose
    /// `pending` entry is gone must not still be flushed to the server by
    /// `handle_initialize_response` once the handshake completes — nothing
    /// would be left to correlate the eventual response (or notice it
    /// arrived at all), and no `$/cancelRequest` would ever follow it.
    fn drop_from_queue(&mut self, id: &RequestId) {
        self.queued
            .retain(|msg| !matches!(msg, Message::Request { id: qid, .. } if qid == id));
    }

    /// Best-effort `$/cancelRequest` — only legal once the handshake has
    /// completed, same reasoning as `send_or_queue`'s Starting-queue.
    fn send_cancel_notification(&self, backend: &mut dyn LspBackend, id: &RequestId) {
        if self.state == ServerState::Running {
            backend.send(
                self.id,
                Message::Notification {
                    method: lsp_types::notification::Cancel::METHOD.to_string(),
                    params: cancel_request_params(id),
                },
            );
        }
    }

    /// Pulls every request that finished (correlated response) or expired
    /// (deadline reached) since the last call. Called at drain, alongside
    /// `on_event` — deadline checks piggyback on the same cadence, no
    /// separate timer thread. A timed-out entry gets a best-effort
    /// `$/cancelRequest` sent here (colocated with the detection, so it's
    /// testable without an editor in the loop) — only once `Running`, same
    /// handshake-ordering reasoning as `cancel`.
    ///
    /// `initialize` piggybacks on this same sweep instead of a separate
    /// timeout check: on expiry it is never pushed into the completed Vec
    /// (no callback is ever registered for it — the handshake response is
    /// handled synchronously in `on_event`) and instead surfaces as a
    /// `Crashed` action, guarded so it reports exactly once even if the
    /// deadline keeps getting swept after the state has already moved on
    /// (e.g. an `Eof` raced it to `Crashed` first).
    pub fn take_completed(
        &mut self,
        backend: &mut dyn LspBackend,
        now: Instant,
    ) -> (Vec<(RequestId, RequestMeta, Outcome)>, Vec<ClientAction>) {
        let mut out = std::mem::take(&mut self.completed);
        let mut actions = Vec::new();
        let timed_out: Vec<RequestId> = self
            .pending
            .iter()
            .filter(|(_, meta)| meta.deadline <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in timed_out {
            if self.initialize_id.as_ref() == Some(&id) {
                self.pending.remove(&id);
                self.initialize_id = None;
                if self.state == ServerState::Starting {
                    self.state = ServerState::Crashed;
                    actions.push(ClientAction::Crashed {
                        error: Some(format!(
                            "initialize timed out after {}s",
                            INITIALIZE_TIMEOUT.as_secs()
                        )),
                    });
                }
                continue;
            }
            if let Some(meta) = self.pending.remove(&id) {
                self.drop_from_queue(&id);
                self.send_cancel_notification(backend, &id);
                out.push((id, meta, Outcome::TimedOut));
            }
        }
        (out, actions)
    }

    /// Builds `InitializeParams` and sends the request.
    pub fn start_handshake(&mut self, backend: &mut dyn LspBackend) {
        let id = self.ids.next();
        self.initialize_id = Some(id.clone());
        self.pending.insert(
            id.clone(),
            RequestMeta {
                method: lsp_types::request::Initialize::METHOD.to_string(),
                allow_stale: false,
                deadline: Instant::now() + INITIALIZE_TIMEOUT,
            },
        );

        let params = serde_json::to_value(build_initialize_params(
            &self.root,
            self.init_options.clone(),
        ))
        .expect("InitializeParams always serializes");

        // Sent directly, never via `send_or_queue` — `initialize` is the one
        // request legal on the wire before `initialized`; routing it through
        // the Starting-queue would deadlock the handshake against itself.
        backend.send(
            self.id,
            Message::Request {
                id,
                method: lsp_types::request::Initialize::METHOD.to_string(),
                params,
            },
        );
    }

    /// Send `msg` now if the handshake has completed, otherwise queue it for
    /// delivery, in order, once `initialized` goes out. A dead or crashed
    /// connection silently drops the send — the crash is already reported
    /// via the `Crashed` state, matching the transport's own
    /// send-after-death discipline.
    pub fn send_or_queue(&mut self, backend: &mut dyn LspBackend, msg: Message) {
        match self.state {
            ServerState::Running => backend.send(self.id, msg),
            ServerState::Starting => self.queued.push(msg),
            ServerState::Crashed | ServerState::Dead => {}
        }
    }

    /// Feed one inbound event; returns actions for the glue to act on.
    pub fn on_event(&mut self, ev: InboundEvent) -> Vec<ClientAction> {
        match ev {
            InboundEvent::Eof { error } => {
                // Guard against reporting twice if more events trickle in
                // after the connection is already known dead — `Dead` covers
                // a graceful `begin_shutdown` teardown racing a trailing
                // `Eof` from the exiting process just as validly as
                // `Crashed` covers an actual crash; either way this must not
                // report a spurious "server crashed" on top of an orderly
                // exit.
                if matches!(self.state, ServerState::Crashed | ServerState::Dead) {
                    return Vec::new();
                }
                self.state = ServerState::Crashed;
                vec![ClientAction::Crashed { error }]
            }
            InboundEvent::Message(Message::Response { id, result })
                if self.initialize_id.as_ref() == Some(&id) =>
            {
                self.initialize_id = None;
                self.pending.remove(&id);
                self.handle_initialize_response(result)
            }
            InboundEvent::Message(Message::Response { id, result }) => {
                if let Some(meta) = self.pending.remove(&id) {
                    let outcome = match result {
                        Ok(v) => Outcome::Ok(v),
                        Err(e) => Outcome::Err(e),
                    };
                    self.completed.push((id, meta, outcome));
                }
                Vec::new()
            }
            InboundEvent::Message(Message::Request { id, method, params }) => {
                vec![ClientAction::ServerRequest { id, method, params }]
            }
            InboundEvent::Message(Message::Notification { method, params }) => {
                vec![classify_notification(method, params)]
            }
            InboundEvent::Stderr(line) => vec![ClientAction::Stderr(line)],
        }
    }

    fn handle_initialize_response(
        &mut self,
        result: Result<serde_json::Value, ResponseError>,
    ) -> Vec<ClientAction> {
        let value = match result {
            Ok(v) => v,
            Err(e) => {
                self.state = ServerState::Crashed;
                return vec![ClientAction::Crashed {
                    error: Some(format!("initialize failed: {} ({})", e.message, e.code)),
                }];
            }
        };
        let parsed: InitializeResult = match serde_json::from_value(value) {
            Ok(r) => r,
            Err(e) => {
                self.state = ServerState::Crashed;
                return vec![ClientAction::Crashed {
                    error: Some(format!("malformed initialize result: {e}")),
                }];
            }
        };

        // Absent `positionEncoding` means UTF-16 per spec — never default to
        // UTF-8 just because the server omitted the field.
        self.encoding = if parsed.capabilities.position_encoding == Some(PositionEncodingKind::UTF8)
        {
            PositionEncoding::Utf8
        } else {
            PositionEncoding::Utf16
        };
        self.caps = Some(parsed.capabilities);
        self.state = ServerState::Running;

        let mut send = vec![Message::Notification {
            method: lsp_types::notification::Initialized::METHOD.to_string(),
            params: serde_json::to_value(InitializedParams {})
                .expect("InitializedParams always serializes"),
        }];
        send.append(&mut self.queued);
        vec![ClientAction::BecameRunning { send }]
    }

    /// `shutdown` request, then `exit` notification — only while `Running`;
    /// nothing but `initialize` is legal on the wire before `initialized`,
    /// so a Starting (or already Crashed/Dead) client sends nothing here.
    /// Every caller still gets a definite `Dead` transition regardless of
    /// prior state, so the transport-level teardown (`ServerHandle::drop`:
    /// kill -> wait -> join, which reaps the process unconditionally) is
    /// always what actually ends a non-Running client — this is a
    /// best-effort protocol courtesy on top of that, never a substitute
    /// for it, and never a synchronous round-trip. The `shutdown` response
    /// now correlates through `pending`/`take_completed` like any other
    /// request; a caller that drops the client immediately (e.g.
    /// `:lsp-stop`) instead dispatches it as timed out via `drain_pending`.
    pub fn begin_shutdown(&mut self, backend: &mut dyn LspBackend) {
        if self.state == ServerState::Running {
            let id = self.ids.next();
            self.pending.insert(
                id.clone(),
                RequestMeta {
                    method: lsp_types::request::Shutdown::METHOD.to_string(),
                    allow_stale: false,
                    deadline: Instant::now() + SHUTDOWN_TIMEOUT,
                },
            );
            backend.send(
                self.id,
                Message::Request {
                    id,
                    method: lsp_types::request::Shutdown::METHOD.to_string(),
                    params: serde_json::Value::Null,
                },
            );
            backend.send(
                self.id,
                Message::Notification {
                    method: lsp_types::notification::Exit::METHOD.to_string(),
                    params: serde_json::Value::Null,
                },
            );
        }
        self.state = ServerState::Dead;
    }
}

#[allow(deprecated)] // root_uri/root_path are deprecated in favor of workspace_folders,
// but rootUri compatibility is deliberate (older servers still read it).
fn build_initialize_params(
    root: &std::path::Path,
    init_options: Option<serde_json::Value>,
) -> InitializeParams {
    let root_uri = uri::path_to_uri(root).ok();
    let workspace_folders = root_uri.clone().map(|u| {
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.to_string_lossy().into_owned());
        vec![WorkspaceFolder { uri: u, name }]
    });

    InitializeParams {
        process_id: Some(std::process::id()),
        root_uri,
        workspace_folders,
        capabilities: build_client_capabilities(),
        client_info: Some(ClientInfo {
            name: "hume".to_string(),
            version: None,
        }),
        initialization_options: init_options,
        ..Default::default()
    }
}

fn build_client_capabilities() -> ClientCapabilities {
    ClientCapabilities {
        workspace: Some(WorkspaceClientCapabilities {
            apply_edit: Some(true),
            configuration: Some(true),
            workspace_folders: Some(true),
            // Every rename result is a WorkspaceEdit — some servers
            // (rust-analyzer) refuse textDocument/rename outright without
            // this declared, since they can't otherwise confirm the client
            // can apply one (found via manual smoke testing).
            //
            // `resource_operations` must be present (non-empty) or
            // rust-analyzer refuses *every* rename outright — confirmed
            // live: omitting it reproduces the original blanket rejection,
            // not just the file-rename-adjacent case below. `edits::
            // collect_edit_entries` has no HUME equivalent for an actual
            // `DocumentChangeOperation::Op` and rejects the whole edit if
            // one ever arrives (a rename whose target shares its name with
            // its containing module/file, which rust-analyzer folds a file
            // rename into) — a real, occasional, safe-by-design rejection,
            // not the common case, and the alternative (never declaring
            // resource_operations) breaks every rename instead.
            workspace_edit: Some(WorkspaceEditClientCapabilities {
                document_changes: Some(true),
                resource_operations: Some(vec![
                    ResourceOperationKind::Create,
                    ResourceOperationKind::Rename,
                    ResourceOperationKind::Delete,
                ]),
                failure_handling: Some(FailureHandlingKind::Abort),
                ..Default::default()
            }),
            ..Default::default()
        }),
        text_document: Some(TextDocumentClientCapabilities {
            synchronization: Some(TextDocumentSyncClientCapabilities {
                did_save: Some(true),
                ..Default::default()
            }),
            publish_diagnostics: Some(PublishDiagnosticsClientCapabilities::default()),
            hover: Some(HoverClientCapabilities {
                content_format: Some(vec![MarkupKind::PlainText, MarkupKind::Markdown]),
                ..Default::default()
            }),
            completion: Some(CompletionClientCapabilities {
                completion_item: Some(CompletionItemCapability {
                    // v1 strips snippet placeholders to plain text.
                    snippet_support: Some(false),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            signature_help: Some(Default::default()),
            rename: Some(RenameClientCapabilities::default()),
            references: Some(Default::default()),
            definition: Some(GotoCapability::default()),
            declaration: Some(GotoCapability::default()),
            type_definition: Some(GotoCapability::default()),
            implementation: Some(GotoCapability::default()),
            formatting: Some(Default::default()),
            range_formatting: Some(Default::default()),
            // Manual smoke testing found rust-analyzer withholds
            // diagnostic-derived quickfixes entirely without
            // code_action_literal_support declared — the flag saying the
            // client understands CodeAction objects, not just legacy
            // Command[]. A byte-perfect request (correct diagnostic
            // round-tripped verbatim, correct overlapping range) still
            // came back empty until this was added.
            code_action: Some(CodeActionClientCapabilities {
                code_action_literal_support: Some(CodeActionLiteralSupport {
                    code_action_kind: CodeActionKindLiteralSupport {
                        value_set: vec![
                            CodeActionKind::QUICKFIX.as_str().to_string(),
                            CodeActionKind::REFACTOR.as_str().to_string(),
                            CodeActionKind::REFACTOR_EXTRACT.as_str().to_string(),
                            CodeActionKind::REFACTOR_INLINE.as_str().to_string(),
                            CodeActionKind::REFACTOR_REWRITE.as_str().to_string(),
                            CodeActionKind::SOURCE.as_str().to_string(),
                            CodeActionKind::SOURCE_ORGANIZE_IMPORTS.as_str().to_string(),
                        ],
                    },
                }),
                is_preferred_support: Some(true),
                disabled_support: Some(true),
                ..Default::default()
            }),
            inlay_hint: Some(Default::default()),
            ..Default::default()
        }),
        general: Some(GeneralClientCapabilities {
            position_encodings: Some(vec![
                PositionEncodingKind::UTF8,
                PositionEncodingKind::UTF16,
            ]),
            ..Default::default()
        }),
        // rust-analyzer (and others) gate server-initiated `$/progress`
        // (indexing/loading status) on this flag — without it, no progress
        // notifications are sent at all, so the editor has no way to show
        // load status beyond the sub-second `initialize` handshake.
        window: Some(WindowClientCapabilities {
            work_done_progress: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Answers a server-initiated request. Exhaustive by design — every request
/// gets exactly one response, even the ones this v1 doesn't otherwise
/// support. `workspace/applyEdit` is deliberately absent: it's the one
/// server request that needs `&mut Editor` (the edit engine), so the editor
/// glue answers it separately (`apply_edit_request_response`) rather than
/// through this pure lookup table.
pub fn server_request_response(
    method: &str,
    params: &serde_json::Value,
) -> Result<serde_json::Value, ResponseError> {
    use lsp_types::request::{
        RegisterCapability, UnregisterCapability, WorkDoneProgressCreate, WorkspaceConfiguration,
    };
    match method {
        // No settings blob exists — every item answers `null`,
        // same shape a server sees from a client with no matching config.
        WorkspaceConfiguration::METHOD => Ok(workspace_configuration_response(params)),
        // Acknowledged, no-op: these need no editor state to answer, unlike
        // `workspace/applyEdit` (the one request this lookup can't handle —
        // see `apply_edit_request_response`).
        RegisterCapability::METHOD
        | UnregisterCapability::METHOD
        | WorkDoneProgressCreate::METHOD => Ok(serde_json::Value::Null),
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
    use crate::inline::InlineLspBackend;
    use std::path::PathBuf;

    fn canned_result(encoding: Option<PositionEncodingKind>) -> serde_json::Value {
        let caps = ServerCapabilities {
            position_encoding: encoding,
            ..Default::default()
        };
        serde_json::to_value(InitializeResult {
            capabilities: caps,
            ..Default::default()
        })
        .unwrap()
    }

    // Golden-field check on the load-bearing capability list:
    // capabilities are load-bearing config — assert the exact advertised
    // set rather than just "it builds".
    #[test]
    #[allow(deprecated)] // asserting on the deliberately-still-populated compat field
    fn initialize_params_advertise_the_v1_capability_set() {
        #[cfg(windows)]
        let root = PathBuf::from(r"C:\tmp\proj");
        #[cfg(not(windows))]
        let root = PathBuf::from("/tmp/proj");
        let params = build_initialize_params(&root, None);

        assert_eq!(params.process_id, Some(std::process::id()));
        assert!(params.root_uri.is_some());
        let folders = params.workspace_folders.expect("workspace_folders set");
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].name, "proj");

        let caps = params.capabilities;
        assert_eq!(
            caps.general.unwrap().position_encodings,
            Some(vec![
                PositionEncodingKind::UTF8,
                PositionEncodingKind::UTF16
            ])
        );
        let td = caps.text_document.unwrap();
        assert_eq!(
            td.completion
                .unwrap()
                .completion_item
                .unwrap()
                .snippet_support,
            Some(false)
        );
        assert_eq!(
            td.hover.unwrap().content_format,
            Some(vec![MarkupKind::PlainText, MarkupKind::Markdown])
        );
        assert!(td.publish_diagnostics.is_some());
        assert!(td.rename.is_some());
        assert!(td.inlay_hint.is_some());
        let ws = caps.workspace.unwrap();
        assert_eq!(ws.apply_edit, Some(true));
        assert_eq!(ws.configuration, Some(true));
        // Manual smoke testing found rust-analyzer refuses
        // textDocument/rename outright without this declared — every
        // rename result is a WorkspaceEdit, and some servers won't attempt
        // one unless the client has confirmed it can apply it.
        let we = ws
            .workspace_edit
            .expect("workspace_edit capability must be declared");
        assert_eq!(we.document_changes, Some(true));
        // Must be present or rust-analyzer refuses every rename outright
        // (confirmed live) — HUME still can't actually apply a resource
        // op if one arrives (edits::collect_edit_entries rejects it by
        // design), but the alternative breaks the common case.
        assert_eq!(
            we.resource_operations,
            Some(vec![
                ResourceOperationKind::Create,
                ResourceOperationKind::Rename,
                ResourceOperationKind::Delete,
            ])
        );
        assert_eq!(we.failure_handling, Some(FailureHandlingKind::Abort));
        // Manual smoke testing found rust-analyzer withholds
        // diagnostic-derived quickfixes entirely without this declared —
        // a byte-perfect codeAction request still came back empty.
        let ca = td
            .code_action
            .expect("code_action capability must be declared");
        let literal = ca
            .code_action_literal_support
            .expect("code_action_literal_support must be declared");
        assert!(
            literal
                .code_action_kind
                .value_set
                .contains(&CodeActionKind::QUICKFIX.as_str().to_string())
        );
        assert_eq!(ca.is_preferred_support, Some(true));
        assert_eq!(ca.disabled_support, Some(true));
    }

    #[test]
    fn handshake_round_trip_transitions_to_running() {
        let mut backend = InlineLspBackend::with_default_handshake();
        let sid = backend
            .start("rust-analyzer", &[], std::path::Path::new("."))
            .unwrap();
        let mut client = LspClient::new(sid, PathBuf::from("."));

        client.start_handshake(&mut backend);
        assert_eq!(client.state, ServerState::Starting);

        let events = backend.drain();
        assert_eq!(events.len(), 1);
        let (_id, ev) = events.into_iter().next().unwrap();
        let actions = client.on_event(ev);

        assert_eq!(client.state, ServerState::Running);
        assert!(client.caps.is_some());
        match &actions[..] {
            [ClientAction::BecameRunning { send }] => {
                assert_eq!(send.len(), 1);
                match &send[0] {
                    Message::Notification { method, .. } => assert_eq!(method, "initialized"),
                    other => panic!("expected initialized notification, got {other:?}"),
                }
            }
            other => panic!("expected one BecameRunning action, got {other:?}"),
        }
    }

    #[test]
    fn initialize_request_carries_initialization_options_when_set() {
        let mut backend = InlineLspBackend::new();
        let sid = backend.start("x", &[], std::path::Path::new(".")).unwrap();
        let mut client = LspClient::new(sid, PathBuf::from("."));

        client.set_init_options(Some(serde_json::json!({"check": {"command": "clippy"}})));
        client.start_handshake(&mut backend);

        match &backend.sent[0] {
            (_, Message::Request { params, .. }) => {
                assert_eq!(
                    params["initializationOptions"]["check"]["command"],
                    "clippy"
                );
            }
            other => panic!("expected the initialize request, got {other:?}"),
        }
    }

    #[test]
    fn initialize_request_omits_initialization_options_when_unset() {
        let mut backend = InlineLspBackend::new();
        let sid = backend.start("x", &[], std::path::Path::new(".")).unwrap();
        let mut client = LspClient::new(sid, PathBuf::from("."));

        client.start_handshake(&mut backend);

        match &backend.sent[0] {
            (_, Message::Request { params, .. }) => {
                assert!(
                    params.get("initializationOptions").is_none(),
                    "expected the key to be absent, not null: {params:?}"
                );
            }
            other => panic!("expected the initialize request, got {other:?}"),
        }
    }

    #[test]
    fn handshake_failure_response_crashes() {
        let mut backend = InlineLspBackend::new();
        backend.fail_with("initialize", -32603, "boom");
        let sid = backend.start("x", &[], std::path::Path::new(".")).unwrap();
        let mut client = LspClient::new(sid, PathBuf::from("."));

        client.start_handshake(&mut backend);
        let (_id, ev) = backend.drain().into_iter().next().unwrap();
        let actions = client.on_event(ev);

        match &actions[..] {
            [ClientAction::Crashed { error }] => {
                assert!(error.as_ref().unwrap().contains("initialize failed"));
            }
            other => panic!("expected one Crashed action, got {other:?}"),
        }
        assert_eq!(client.state, ServerState::Crashed);
        assert_eq!(
            client.pending_count(),
            0,
            "the error path must also consume the initialize pending entry"
        );
    }

    /// Pins that the `initialize` response is discriminated by the id stashed
    /// in `initialize_id`, not by matching on the method string — a Steel
    /// plugin issuing `(lsp-request "initialize" ...)` through the generic
    /// bridge must get an ordinary correlated response, never be mistaken
    /// for the handshake and hijack the client into `BecameRunning`/`Crashed`.
    #[test]
    fn generic_initialize_request_is_not_hijacked_by_the_handshake_discriminator() {
        let (mut backend, mut client) = make_running_client();
        backend.respond_to("initialize", serde_json::json!({"ok": true}));

        let meta = RequestMeta {
            method: "initialize".to_string(),
            allow_stale: false,
            deadline: Instant::now() + std::time::Duration::from_secs(10),
        };
        let sent_id =
            client.send_request(&mut backend, "initialize", serde_json::Value::Null, meta);

        let (_sid, ev) = backend.drain().into_iter().next().unwrap();
        let actions = client.on_event(ev);
        assert!(
            actions.is_empty(),
            "a Steel-issued initialize response must correlate normally, not surface as a lifecycle action"
        );
        assert_eq!(client.state, ServerState::Running);

        let (completed, actions) = client.take_completed(&mut backend, Instant::now());
        assert!(actions.is_empty());
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].0, sent_id);
    }

    // ── initialize timeout via take_completed's sweep ────────────────────────

    #[test]
    fn initialize_sweep_is_quiet_before_the_deadline() {
        let mut backend = InlineLspBackend::new();
        let sid = backend.start("x", &[], std::path::Path::new(".")).unwrap();
        let mut client = LspClient::new(sid, PathBuf::from("."));
        client.start_handshake(&mut backend);

        let (completed, actions) = client.take_completed(&mut backend, Instant::now());
        assert!(completed.is_empty());
        assert!(actions.is_empty());
        assert_eq!(client.state, ServerState::Starting);
    }

    #[test]
    fn initialize_timeout_crashes_via_the_sweep() {
        let mut backend = InlineLspBackend::new();
        let sid = backend.start("x", &[], std::path::Path::new(".")).unwrap();
        let mut client = LspClient::new(sid, PathBuf::from("."));
        client.start_handshake(&mut backend);

        let past_deadline = Instant::now() + std::time::Duration::from_secs(31);
        let (completed, actions) = client.take_completed(&mut backend, past_deadline);
        assert!(
            completed.is_empty(),
            "the internal initialize entry must never appear as a completed request"
        );
        match &actions[..] {
            [ClientAction::Crashed { error }] => {
                assert!(error.as_ref().unwrap().contains("initialize timed out"));
            }
            other => panic!("expected one Crashed action, got {other:?}"),
        }
        assert_eq!(client.state, ServerState::Crashed);

        // A second sweep after already Crashed must not report again.
        let (completed2, actions2) = client.take_completed(&mut backend, past_deadline);
        assert!(completed2.is_empty());
        assert!(actions2.is_empty());
    }

    #[test]
    fn initialize_never_times_out_once_running() {
        let (mut backend, mut client) = make_running_client();
        let far_future = Instant::now() + std::time::Duration::from_secs(1000);
        let (completed, actions) = client.take_completed(&mut backend, far_future);
        assert!(completed.is_empty());
        assert!(actions.is_empty());
    }

    // ── earliest_deadline ─────────────────────────────────────────────────────

    #[test]
    fn earliest_deadline_none_when_no_pending() {
        let mut backend = InlineLspBackend::new();
        let sid = backend.start("x", &[], std::path::Path::new(".")).unwrap();
        let client = LspClient::new(sid, PathBuf::from("."));
        assert_eq!(client.earliest_deadline(), None);
    }

    #[test]
    fn earliest_deadline_is_min() {
        let (mut backend, mut client) = make_running_client();
        let now = Instant::now();

        client.send_request(
            &mut backend,
            "foo",
            serde_json::Value::Null,
            RequestMeta {
                method: "foo".to_string(),
                allow_stale: false,
                deadline: now + std::time::Duration::from_secs(5),
            },
        );
        client.send_request(
            &mut backend,
            "bar",
            serde_json::Value::Null,
            RequestMeta {
                method: "bar".to_string(),
                allow_stale: false,
                deadline: now + std::time::Duration::from_secs(1),
            },
        );

        let earliest = client.earliest_deadline().expect("two pending requests");
        assert!(
            earliest < now + std::time::Duration::from_secs(5),
            "must be the nearer (1s) deadline, not the farther (5s) one"
        );
    }

    #[test]
    fn messages_sent_while_starting_are_queued_then_flushed_in_order() {
        let mut backend = InlineLspBackend::with_default_handshake();
        let sid = backend.start("x", &[], std::path::Path::new(".")).unwrap();
        let mut client = LspClient::new(sid, PathBuf::from("."));

        client.start_handshake(&mut backend);
        client.send_or_queue(
            &mut backend,
            Message::Notification {
                method: "textDocument/didOpen".to_string(),
                params: serde_json::json!({"uri": "file:///a"}),
            },
        );
        // Not sent yet — must not appear in the backend's sent log as a
        // didOpen (only the initialize request should be there).
        assert!(
            backend
                .sent
                .iter()
                .all(|(_, m)| !matches!(m, Message::Notification { method, .. } if method == "textDocument/didOpen"))
        );

        let (_id, ev) = backend.drain().into_iter().next().unwrap();
        let actions = client.on_event(ev);
        match &actions[..] {
            [ClientAction::BecameRunning { send }] => {
                assert_eq!(send.len(), 2);
                match &send[0] {
                    Message::Notification { method, .. } => assert_eq!(method, "initialized"),
                    other => panic!("expected initialized first, got {other:?}"),
                }
                match &send[1] {
                    Message::Notification { method, .. } => {
                        assert_eq!(method, "textDocument/didOpen")
                    }
                    other => panic!("expected the queued didOpen second, got {other:?}"),
                }
            }
            other => panic!("expected one BecameRunning action, got {other:?}"),
        }
    }

    #[test]
    fn send_request_while_starting_is_queued_then_flushed_and_still_correlates() {
        let mut backend = InlineLspBackend::with_default_handshake();
        backend.respond_to("textDocument/hover", serde_json::json!({"contents": "hi"}));
        let sid = backend.start("x", &[], std::path::Path::new(".")).unwrap();
        let mut client = LspClient::new(sid, PathBuf::from("."));

        client.start_handshake(&mut backend);
        let meta = RequestMeta {
            method: "textDocument/hover".to_string(),
            allow_stale: false,
            deadline: Instant::now() + std::time::Duration::from_secs(10),
        };
        let sent_id = client.send_request(
            &mut backend,
            "textDocument/hover",
            serde_json::Value::Null,
            meta,
        );

        // Nothing but the initialize request should be on the wire yet.
        assert!(
            backend
                .sent
                .iter()
                .all(|(_, m)| !matches!(m, Message::Request { method, .. } if method == "textDocument/hover")),
            "request must be queued, not sent, while Starting"
        );
        assert_eq!(
            client.pending_count(),
            2,
            "pending entry recorded even though queued (plus the in-flight initialize)"
        );

        // Handshake completes: BecameRunning flushes the queued hover request.
        let (_id, ev) = backend.drain().into_iter().next().unwrap();
        let mut actions = client.on_event(ev).into_iter();
        match actions.next() {
            Some(ClientAction::BecameRunning { send }) => {
                assert_eq!(send.len(), 2);
                match &send[1] {
                    Message::Request { id, method, .. } => {
                        assert_eq!(*id, sent_id);
                        assert_eq!(method, "textDocument/hover");
                    }
                    other => panic!("expected the queued hover request second, got {other:?}"),
                }
                // The real editor glue's `BecameRunning` dispatch does this
                // exact send loop (`dispatch_lsp_action`) — replicate it so
                // the flushed hover request actually reaches the backend.
                for msg in send {
                    backend.send(sid, msg);
                }
            }
            other => panic!("expected one BecameRunning action, got {other:?}"),
        }
        assert!(actions.next().is_none());

        // The response still correlates normally once actually sent.
        let (_sid, ev) = backend
            .drain()
            .into_iter()
            .find(|(_, ev)| matches!(ev, InboundEvent::Message(Message::Response { .. })))
            .expect("hover response");
        client.on_event(ev);
        let (completed, actions) = client.take_completed(&mut backend, Instant::now());
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].0, sent_id);
        assert!(actions.is_empty());
    }

    #[test]
    fn utf8_negotiated_when_offered() {
        let mut backend = InlineLspBackend::new();
        backend.respond_to(
            "initialize",
            canned_result(Some(PositionEncodingKind::UTF8)),
        );
        let sid = backend.start("x", &[], std::path::Path::new(".")).unwrap();
        let mut client = LspClient::new(sid, PathBuf::from("."));

        client.start_handshake(&mut backend);
        let (_id, ev) = backend.drain().into_iter().next().unwrap();
        client.on_event(ev);

        assert_eq!(client.encoding, PositionEncoding::Utf8);
    }

    #[test]
    fn utf16_is_the_default_when_server_omits_the_field() {
        let mut backend = InlineLspBackend::new();
        backend.respond_to("initialize", canned_result(None));
        let sid = backend.start("x", &[], std::path::Path::new(".")).unwrap();
        let mut client = LspClient::new(sid, PathBuf::from("."));

        client.start_handshake(&mut backend);
        let (_id, ev) = backend.drain().into_iter().next().unwrap();
        client.on_event(ev);

        assert_eq!(client.encoding, PositionEncoding::Utf16);
    }

    #[test]
    fn eof_transitions_to_crashed_and_further_sends_do_not_panic() {
        let mut backend = InlineLspBackend::new();
        let sid = backend.start("x", &[], std::path::Path::new(".")).unwrap();
        let mut client = LspClient::new(sid, PathBuf::from("."));

        let actions = client.on_event(InboundEvent::Eof {
            error: Some("server exited".to_string()),
        });
        assert_eq!(client.state, ServerState::Crashed);
        match &actions[..] {
            [ClientAction::Crashed { error }] => {
                assert_eq!(error.as_deref(), Some("server exited"));
            }
            other => panic!("expected one Crashed action, got {other:?}"),
        }

        // A send after Crashed must not panic — silently dropped, matching
        // the transport's own send-after-death discipline.
        client.send_or_queue(
            &mut backend,
            Message::Notification {
                method: "textDocument/didOpen".to_string(),
                params: serde_json::Value::Null,
            },
        );
    }

    /// A request filed against an already-Crashed client is silently dropped
    /// on the wire (see `send_or_queue`) and nothing will ever answer it —
    /// its `meta.deadline` must be clamped to now so `take_completed`'s sweep
    /// resolves it as `TimedOut` on the very next tick, instead of leaving
    /// the caller waiting out the deadline it asked for (routinely tens of
    /// seconds) for a request that was doomed the moment it was sent.
    #[test]
    fn send_request_after_crashed_times_out_immediately_via_the_sweep() {
        let mut backend = InlineLspBackend::new();
        let sid = backend.start("x", &[], std::path::Path::new(".")).unwrap();
        let mut client = LspClient::new(sid, PathBuf::from("."));

        client.on_event(InboundEvent::Eof {
            error: Some("server exited".to_string()),
        });
        assert_eq!(client.state, ServerState::Crashed);

        let far_future = Instant::now() + std::time::Duration::from_secs(30);
        let meta = RequestMeta {
            method: "textDocument/hover".to_string(),
            allow_stale: false,
            deadline: far_future,
        };
        let id = client.send_request(
            &mut backend,
            "textDocument/hover",
            serde_json::Value::Null,
            meta,
        );

        assert!(
            client.earliest_deadline().expect("still pending") <= Instant::now(),
            "deadline must be clamped to now, not left at the caller's far-future value"
        );

        let (completed, actions) = client.take_completed(&mut backend, Instant::now());
        assert!(actions.is_empty());
        assert_eq!(completed.len(), 1);
        let (returned_id, _meta, outcome) = &completed[0];
        assert_eq!(*returned_id, id);
        assert!(
            matches!(outcome, Outcome::TimedOut),
            "expected TimedOut, got {outcome:?}"
        );
    }

    #[test]
    fn shutdown_sends_shutdown_request_then_exit_notification_in_order() {
        let (mut backend, mut client) = make_running_client();
        // `make_running_client` already sent `initialize` — this test only
        // asserts on what `begin_shutdown` adds after it.
        let before = backend.sent.len();

        client.begin_shutdown(&mut backend);

        assert_eq!(client.state, ServerState::Dead);
        assert_eq!(backend.sent.len(), before + 2);
        match &backend.sent[before] {
            (_, Message::Request { method, .. }) => assert_eq!(method, "shutdown"),
            other => panic!("expected the shutdown request first, got {other:?}"),
        }
        match &backend.sent[before + 1] {
            (_, Message::Notification { method, .. }) => assert_eq!(method, "exit"),
            other => panic!("expected the exit notification second, got {other:?}"),
        }
    }

    /// Regression: nothing but `initialize` is legal on the wire before
    /// `initialized` — `begin_shutdown` on a still-Starting client must
    /// send neither `shutdown` nor `exit` (it still transitions to `Dead`;
    /// transport-level teardown reaps the process regardless).
    #[test]
    fn begin_shutdown_sends_nothing_while_still_starting() {
        let mut backend = InlineLspBackend::new();
        let sid = backend.start("x", &[], std::path::Path::new(".")).unwrap();
        let mut client = LspClient::new(sid, PathBuf::from("."));
        assert_eq!(client.state, ServerState::Starting);

        client.begin_shutdown(&mut backend);

        assert_eq!(client.state, ServerState::Dead);
        assert!(
            backend.sent.is_empty(),
            "must not send shutdown/exit before the handshake completed: {:?}",
            backend.sent
        );
    }

    #[test]
    fn shutdown_response_surfaces_through_take_completed() {
        let (mut backend, mut client) = make_running_client();
        backend.respond_to("shutdown", serde_json::Value::Null);

        client.begin_shutdown(&mut backend);
        let (_sid, ev) = backend
            .drain()
            .into_iter()
            .find(|(_, ev)| matches!(ev, InboundEvent::Message(Message::Response { .. })))
            .expect("shutdown response");
        let actions = client.on_event(ev);
        assert!(actions.is_empty());

        let (completed, actions) = client.take_completed(&mut backend, Instant::now());
        assert!(actions.is_empty());
        assert_eq!(completed.len(), 1);
        let (_id, meta, outcome) = &completed[0];
        assert_eq!(meta.method, "shutdown");
        match outcome {
            Outcome::Ok(v) => assert_eq!(*v, serde_json::Value::Null),
            other => panic!("expected Ok(null), got {other:?}"),
        }
        assert_eq!(client.pending_count(), 0);
    }

    #[test]
    fn shutdown_error_surfaces_as_err() {
        let (mut backend, mut client) = make_running_client();
        backend.fail_with("shutdown", -32603, "internal error");

        client.begin_shutdown(&mut backend);
        let (_sid, ev) = backend
            .drain()
            .into_iter()
            .find(|(_, ev)| matches!(ev, InboundEvent::Message(Message::Response { .. })))
            .expect("shutdown response");
        client.on_event(ev);

        let (completed, _actions) = client.take_completed(&mut backend, Instant::now());
        assert_eq!(completed.len(), 1);
        let (_id, meta, outcome) = &completed[0];
        assert_eq!(meta.method, "shutdown");
        match outcome {
            Outcome::Err(e) => assert_eq!(e.message, "internal error"),
            other => panic!("expected Err, got {other:?}"),
        }
    }

    fn make_running_client() -> (InlineLspBackend, LspClient) {
        let mut backend = InlineLspBackend::with_default_handshake();
        let sid = backend.start("x", &[], std::path::Path::new(".")).unwrap();
        let mut client = LspClient::new(sid, PathBuf::from("."));
        client.start_handshake(&mut backend);
        let (_id, ev) = backend.drain().into_iter().next().unwrap();
        client.on_event(ev);
        assert_eq!(client.state, ServerState::Running);
        (backend, client)
    }

    #[test]
    fn send_request_delivers_response_via_take_completed() {
        let (mut backend, mut client) = make_running_client();
        backend.respond_to("textDocument/hover", serde_json::json!({"contents": "hi"}));

        let meta = RequestMeta {
            method: "textDocument/hover".to_string(),
            allow_stale: false,
            deadline: Instant::now() + std::time::Duration::from_secs(10),
        };
        let sent_id = client.send_request(
            &mut backend,
            "textDocument/hover",
            serde_json::Value::Null,
            meta,
        );

        let (_sid, ev) = backend.drain().into_iter().next().unwrap();
        let actions = client.on_event(ev);
        assert!(
            actions.is_empty(),
            "a correlated response produces no ClientAction — it's pulled via take_completed"
        );

        let (completed, actions) = client.take_completed(&mut backend, Instant::now());
        assert_eq!(completed.len(), 1);
        assert!(actions.is_empty());
        let (id, meta_out, outcome) = &completed[0];
        assert_eq!(*id, sent_id);
        assert_eq!(meta_out.method, "textDocument/hover");
        match outcome {
            Outcome::Ok(v) => assert_eq!(*v, serde_json::json!({"contents": "hi"})),
            other => panic!("expected Ok, got {other:?}"),
        }

        // Pulled once — a second call finds nothing left.
        let (completed2, actions2) = client.take_completed(&mut backend, Instant::now());
        assert!(completed2.is_empty());
        assert!(actions2.is_empty());
    }

    #[test]
    fn cancel_removes_pending_and_sends_cancel_notification() {
        let (mut backend, mut client) = make_running_client();

        let meta = RequestMeta {
            method: "textDocument/definition".to_string(),
            allow_stale: false,
            deadline: Instant::now() + std::time::Duration::from_secs(10),
        };
        let id = client.send_request(
            &mut backend,
            "textDocument/definition",
            serde_json::Value::Null,
            meta,
        );
        client.cancel(&mut backend, id.clone());

        match backend.sent.last() {
            Some((_, Message::Notification { method, params })) => {
                assert_eq!(method, "$/cancelRequest");
                assert_eq!(params, &cancel_request_params(&id));
            }
            other => panic!("expected a $/cancelRequest notification, got {other:?}"),
        }

        // A late response for the already-cancelled id must not resurrect it.
        backend.push_from_server(
            client.id,
            Message::Response {
                id: id.clone(),
                result: Ok(serde_json::Value::Null),
            },
        );
        let (_sid, ev) = backend.drain().into_iter().next().unwrap();
        client.on_event(ev);
        let (completed, actions) = client.take_completed(&mut backend, Instant::now());
        assert!(completed.is_empty());
        assert!(actions.is_empty());
    }

    /// Minor regression: nothing but `initialize` is legal on the wire
    /// before `initialized` — a request cancelled or timed out while still
    /// `Starting` must not put `$/cancelRequest` on the wire, since its own
    /// send is still sitting in `queued`, unsent, and the server never saw
    /// it in the first place.
    #[test]
    fn cancel_and_timeout_send_no_cancel_request_while_still_starting() {
        let mut backend = InlineLspBackend::new();
        let sid = backend.start("x", &[], std::path::Path::new(".")).unwrap();
        let mut client = LspClient::new(sid, PathBuf::from("."));
        assert_eq!(client.state, ServerState::Starting);

        let meta = RequestMeta {
            method: "textDocument/definition".to_string(),
            allow_stale: false,
            deadline: Instant::now() + std::time::Duration::from_secs(10),
        };
        let id = client.send_request(
            &mut backend,
            "textDocument/definition",
            serde_json::Value::Null,
            meta,
        );
        client.cancel(&mut backend, id);
        assert!(
            backend.sent.is_empty(),
            "cancelling while Starting must not send anything"
        );

        let meta2 = RequestMeta {
            method: "textDocument/hover".to_string(),
            allow_stale: false,
            deadline: Instant::now() - std::time::Duration::from_millis(1),
        };
        client.send_request(
            &mut backend,
            "textDocument/hover",
            serde_json::Value::Null,
            meta2,
        );
        let (completed, actions) = client.take_completed(&mut backend, Instant::now());
        assert_eq!(completed.len(), 1);
        assert!(matches!(completed[0].2, Outcome::TimedOut));
        assert!(actions.is_empty());
        assert!(
            backend.sent.is_empty(),
            "a timeout while still Starting must not send $/cancelRequest either"
        );
    }

    /// Regression: a request cancelled while still `Starting` must not
    /// resurface once the handshake completes — its `Message::Request` sat
    /// unsent in `queued` (removed from `pending` by `cancel`), and without
    /// also stripping it from `queued`, `handle_initialize_response`'s
    /// flush would still deliver it to the server with no pending entry
    /// left to correlate a response (or send `$/cancelRequest` for).
    #[test]
    fn cancelled_request_is_not_flushed_after_handshake_completes() {
        let mut backend = InlineLspBackend::new();
        backend.respond_to("initialize", canned_result(None));
        let sid = backend.start("x", &[], std::path::Path::new(".")).unwrap();
        let mut client = LspClient::new(sid, PathBuf::from("."));
        client.start_handshake(&mut backend);

        let meta = RequestMeta {
            method: "textDocument/definition".to_string(),
            allow_stale: false,
            deadline: Instant::now() + std::time::Duration::from_secs(10),
        };
        let id = client.send_request(
            &mut backend,
            "textDocument/definition",
            serde_json::Value::Null,
            meta,
        );
        client.cancel(&mut backend, id);

        let (_id, ev) = backend
            .drain()
            .into_iter()
            .find(|(_, ev)| matches!(ev, InboundEvent::Message(Message::Response { .. })))
            .expect("initialize response");
        let actions = client.on_event(ev);
        match &actions[..] {
            [ClientAction::BecameRunning { send }] => {
                assert_eq!(
                    send.len(),
                    1,
                    "only 'initialized' should flush — the cancelled request must not reappear: {send:?}"
                );
                match &send[0] {
                    Message::Notification { method, .. } => assert_eq!(method, "initialized"),
                    other => panic!("expected only the initialized notification, got {other:?}"),
                }
            }
            other => panic!("expected one BecameRunning action, got {other:?}"),
        }
    }

    #[test]
    fn take_completed_reports_timeout_and_sends_cancel_request() {
        let (mut backend, mut client) = make_running_client();
        let meta = RequestMeta {
            method: "textDocument/completion".to_string(),
            allow_stale: false,
            deadline: Instant::now() - std::time::Duration::from_millis(1),
        };
        let id = client.send_request(
            &mut backend,
            "textDocument/completion",
            serde_json::Value::Null,
            meta,
        );

        let (completed, actions) = client.take_completed(&mut backend, Instant::now());
        assert_eq!(completed.len(), 1);
        assert!(actions.is_empty());
        let (returned_id, _meta, outcome) = &completed[0];
        assert_eq!(*returned_id, id);
        assert!(matches!(outcome, Outcome::TimedOut));

        match backend.sent.last() {
            Some((_, Message::Notification { method, params })) => {
                assert_eq!(method, "$/cancelRequest");
                assert_eq!(params, &cancel_request_params(&id));
            }
            other => panic!("expected a $/cancelRequest notification, got {other:?}"),
        }

        // Removed from pending — a second call must not report it again.
        let (completed2, actions2) = client.take_completed(&mut backend, Instant::now());
        assert!(completed2.is_empty());
        assert!(actions2.is_empty());
    }

    #[test]
    fn server_initiated_request_becomes_a_client_action() {
        let (mut backend, mut client) = make_running_client();
        backend.push_from_server(
            client.id,
            Message::Request {
                id: RequestId::Int(99),
                method: "workspace/configuration".to_string(),
                params: serde_json::json!({"items": []}),
            },
        );
        let (_sid, ev) = backend.drain().into_iter().next().unwrap();
        let actions = client.on_event(ev);
        match &actions[..] {
            [ClientAction::ServerRequest { id, method, .. }] => {
                assert_eq!(*id, RequestId::Int(99));
                assert_eq!(method, "workspace/configuration");
            }
            other => panic!("expected one ServerRequest action, got {other:?}"),
        }
    }

    #[test]
    fn server_notification_becomes_a_client_action() {
        let (mut backend, mut client) = make_running_client();
        backend.push_from_server(
            client.id,
            Message::Notification {
                method: "custom/thing".to_string(),
                params: serde_json::json!({"anything": true}),
            },
        );
        let (_sid, ev) = backend.drain().into_iter().next().unwrap();
        let actions = client.on_event(ev);
        match &actions[..] {
            [ClientAction::ServerNotification { method, .. }] => {
                assert_eq!(method, "custom/thing");
            }
            other => panic!("expected one ServerNotification action, got {other:?}"),
        }
    }

    #[test]
    fn publish_diagnostics_notification_classifies_as_typed_diagnostics() {
        let (mut backend, mut client) = make_running_client();
        backend.push_from_server(
            client.id,
            Message::Notification {
                method: "textDocument/publishDiagnostics".to_string(),
                params: serde_json::json!({"uri": "file:///a", "diagnostics": []}),
            },
        );
        let (_sid, ev) = backend.drain().into_iter().next().unwrap();
        let actions = client.on_event(ev);
        match &actions[..] {
            [ClientAction::Diagnostics(p)] => {
                assert_eq!(p.uri.as_str(), "file:///a");
                assert!(p.diagnostics.is_empty());
            }
            other => panic!("expected one Diagnostics action, got {other:?}"),
        }
    }

    #[test]
    fn progress_log_and_show_message_classify_as_typed_variants() {
        let (mut backend, mut client) = make_running_client();

        backend.push_from_server(
            client.id,
            Message::Notification {
                method: "$/progress".to_string(),
                params: serde_json::json!({
                    "token": "t1",
                    "value": {"kind": "begin", "title": "Indexing"},
                }),
            },
        );
        let (_sid, ev) = backend.drain().into_iter().next().unwrap();
        match &client.on_event(ev)[..] {
            [ClientAction::Progress(p)] => {
                assert_eq!(p.token, lsp_types::NumberOrString::String("t1".to_string()));
            }
            other => panic!("expected one Progress action, got {other:?}"),
        }

        backend.push_from_server(
            client.id,
            Message::Notification {
                method: "window/logMessage".to_string(),
                params: serde_json::json!({"type": 1, "message": "boom"}),
            },
        );
        let (_sid, ev) = backend.drain().into_iter().next().unwrap();
        match &client.on_event(ev)[..] {
            [ClientAction::LogMessage(p)] => {
                assert_eq!(p.message, "boom");
                assert_eq!(p.typ, lsp_types::MessageType::ERROR);
            }
            other => panic!("expected one LogMessage action, got {other:?}"),
        }

        backend.push_from_server(
            client.id,
            Message::Notification {
                method: "window/showMessage".to_string(),
                params: serde_json::json!({"type": 3, "message": "hi"}),
            },
        );
        let (_sid, ev) = backend.drain().into_iter().next().unwrap();
        match &client.on_event(ev)[..] {
            [ClientAction::ShowMessage(p)] => {
                assert_eq!(p.message, "hi");
                assert_eq!(p.typ, lsp_types::MessageType::INFO);
            }
            other => panic!("expected one ShowMessage action, got {other:?}"),
        }
    }

    #[test]
    fn malformed_known_method_falls_through_as_server_notification() {
        let (mut backend, mut client) = make_running_client();

        backend.push_from_server(
            client.id,
            Message::Notification {
                method: "textDocument/publishDiagnostics".to_string(),
                params: serde_json::json!({"uri": 42}),
            },
        );
        let (_sid, ev) = backend.drain().into_iter().next().unwrap();
        match &client.on_event(ev)[..] {
            [ClientAction::ServerNotification { method, params }] => {
                assert_eq!(method, "textDocument/publishDiagnostics");
                assert_eq!(params, &serde_json::json!({"uri": 42}));
            }
            other => panic!("expected fallthrough ServerNotification, got {other:?}"),
        }

        backend.push_from_server(
            client.id,
            Message::Notification {
                method: "$/progress".to_string(),
                params: serde_json::json!({"token": {}}),
            },
        );
        let (_sid, ev) = backend.drain().into_iter().next().unwrap();
        match &client.on_event(ev)[..] {
            [ClientAction::ServerNotification { method, params }] => {
                assert_eq!(method, "$/progress");
                assert_eq!(params, &serde_json::json!({"token": {}}));
            }
            other => panic!("expected fallthrough ServerNotification, got {other:?}"),
        }
    }

    #[test]
    fn progress_begin_missing_title_recovers_via_lenient_fallback() {
        // A server that treats `title` as optional in practice, even though
        // `WorkDoneProgressBegin::title` is spec-required — the strict parse
        // fails, `recover_progress` patches in a placeholder and retries.
        let (mut backend, mut client) = make_running_client();
        backend.push_from_server(
            client.id,
            Message::Notification {
                method: "$/progress".to_string(),
                params: serde_json::json!({"token": "t1", "value": {"kind": "begin"}}),
            },
        );
        let (_sid, ev) = backend.drain().into_iter().next().unwrap();
        match &client.on_event(ev)[..] {
            [ClientAction::Progress(p)] => {
                assert_eq!(p.token, lsp_types::NumberOrString::String("t1".to_string()));
                match &p.value {
                    lsp_types::ProgressParamsValue::WorkDone(
                        lsp_types::WorkDoneProgress::Begin(begin),
                    ) => assert!(!begin.title.is_empty(), "must recover a non-empty title"),
                    other => panic!("expected a Begin progress value, got {other:?}"),
                }
            }
            other => panic!("expected one recovered Progress action, got {other:?}"),
        }
    }

    #[test]
    fn log_message_missing_type_recovers_via_lenient_fallback() {
        let (mut backend, mut client) = make_running_client();
        backend.push_from_server(
            client.id,
            Message::Notification {
                method: "window/logMessage".to_string(),
                params: serde_json::json!({"message": "boom"}),
            },
        );
        let (_sid, ev) = backend.drain().into_iter().next().unwrap();
        match &client.on_event(ev)[..] {
            [ClientAction::LogMessage(p)] => {
                assert_eq!(p.message, "boom");
                assert_eq!(p.typ, lsp_types::MessageType::LOG);
            }
            other => panic!("expected one recovered LogMessage action, got {other:?}"),
        }
    }

    #[test]
    fn show_message_non_integer_type_recovers_via_lenient_fallback() {
        let (mut backend, mut client) = make_running_client();
        backend.push_from_server(
            client.id,
            Message::Notification {
                method: "window/showMessage".to_string(),
                params: serde_json::json!({"type": "info", "message": "hi"}),
            },
        );
        let (_sid, ev) = backend.drain().into_iter().next().unwrap();
        match &client.on_event(ev)[..] {
            [ClientAction::ShowMessage(p)] => {
                assert_eq!(p.message, "hi");
                assert_eq!(p.typ, lsp_types::MessageType::INFO);
            }
            other => panic!("expected one recovered ShowMessage action, got {other:?}"),
        }
    }

    #[test]
    fn stderr_event_becomes_a_client_action() {
        let (_backend, mut client) = make_running_client();
        let actions = client.on_event(InboundEvent::Stderr("panic: oh no".to_string()));
        match &actions[..] {
            [ClientAction::Stderr(line)] => assert_eq!(line, "panic: oh no"),
            other => panic!("expected one Stderr action, got {other:?}"),
        }
    }

    #[test]
    fn eof_reports_crashed_only_once_even_if_fed_again() {
        let (_backend, mut client) = make_running_client();
        let first = client.on_event(InboundEvent::Eof { error: None });
        assert_eq!(first.len(), 1);
        let second = client.on_event(InboundEvent::Eof { error: None });
        assert!(
            second.is_empty(),
            "a second Eof after already-Crashed must not report again"
        );
    }

    /// Minor regression: a trailing `Eof` racing a graceful `begin_shutdown`
    /// teardown must not report a spurious "server crashed" — `Dead` is as
    /// valid a "connection is already known gone, on purpose" state as
    /// `Crashed`.
    #[test]
    fn eof_after_a_graceful_shutdown_does_not_report_crashed() {
        let (mut backend, mut client) = make_running_client();
        client.begin_shutdown(&mut backend);
        assert_eq!(client.state, ServerState::Dead);

        let actions = client.on_event(InboundEvent::Eof { error: None });
        assert!(
            actions.is_empty(),
            "an Eof after a graceful shutdown must not surface a Crashed action"
        );
        assert_eq!(
            client.state,
            ServerState::Dead,
            "state must stay Dead, not flip to Crashed"
        );
    }

    // ── server_request_response ──────────────────────────────────────────────

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

    // workspace/applyEdit is answered separately (needs `&mut Editor`) — see
    // hume-editor's `editor::tests::lsp_edits` for its coverage.

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
