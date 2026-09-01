//! Per-server client state machine: `initialize` handshake with capability
//! and position-encoding negotiation, graceful shutdown, crash detection.

use rustc_hash::FxHashMap;
use std::path::PathBuf;
use std::time::Instant;

use hume_rope::position_encoding::PositionEncoding;
use lsp_types::{
    ClientCapabilities, ClientInfo, CodeActionClientCapabilities, CodeActionKind,
    CodeActionKindLiteralSupport, CodeActionLiteralSupport, CompletionClientCapabilities,
    CompletionItemCapability, DidChangeConfigurationClientCapabilities,
    DidChangeConfigurationParams, FailureHandlingKind, GeneralClientCapabilities, GotoCapability,
    HoverClientCapabilities, InitializeParams, InitializeResult, InitializedParams, MarkupKind,
    ParameterInformationSettings, PositionEncodingKind, PublishDiagnosticsClientCapabilities,
    RenameClientCapabilities, ResourceOperationKind, ServerCapabilities,
    SignatureHelpClientCapabilities, SignatureInformationSettings, TextDocumentClientCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncClientCapabilities, TextDocumentSyncKind,
    TextDocumentSyncOptions, WindowClientCapabilities, WorkspaceClientCapabilities,
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
/// optional in practice. Checks the shape against the borrowed `params`
/// first: any other off-spec shape (an unkeyable `token`, an unknown `kind`)
/// returns `None` without cloning, and falls through to
/// `ServerNotification`. Only the recoverable shape is cloned, patched with
/// a placeholder title, and re-parsed.
fn recover_progress(params: &serde_json::Value) -> Option<lsp_types::ProgressParams> {
    use serde::Deserialize as _;

    let value = params.get("value")?;
    if value.get("kind").and_then(|k| k.as_str()) != Some("begin") || value.get("title").is_some() {
        return None;
    }

    let mut patched = params.clone();
    patched
        .get_mut("value")?
        .as_object_mut()?
        .insert("title".into(), serde_json::json!("progress"));
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
    /// The server's raw `capabilities` object off the wire, kept alongside
    /// the typed `caps` above rather than re-derived from it — `caps` only
    /// round-trips through whatever `lsp_types::ServerCapabilities` models,
    /// silently dropping any capability the pinned crate version doesn't
    /// know about (e.g. LSP 3.18's `documentRangeFormattingProvider.
    /// rangesSupport`). `capabilities_json` below is what `(lsp-capabilities
    /// …)` actually hands to Steel, so it must see the wire value verbatim.
    caps_json: Option<serde_json::Value>,
    /// Negotiated position encoding; UTF-16 until `initialize` proves UTF-8.
    /// A decode-once cache of `caps.position_encoding` — `handle_initialize_
    /// response` is the only writer of either field, an invariant field
    /// privacy enforces — kept separate so callers don't re-derive it from
    /// the raw capability on every position conversion, not an independent
    /// fact that could drift on its own.
    encoding: PositionEncoding,
    root: PathBuf,
    /// `initializationOptions` for the `initialize` request — set via
    /// `set_init_options` before `start_handshake` to take effect; `None`
    /// omits the field entirely (never sent as `null`).
    init_options: Option<serde_json::Value>,
    /// Server configuration — set via `set_settings` before `start_handshake`
    /// to take effect. Pushed once as `workspace/didChangeConfiguration`
    /// right after `initialized`, and answered to `workspace/configuration`
    /// pull requests (resolved per-item by `resolve_config_section`). `None`
    /// sends no push at all, and pull requests fall back to `null` per item.
    settings: Option<serde_json::Value>,
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
    pending: FxHashMap<RequestId, RequestMeta>,
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
            caps_json: None,
            encoding: PositionEncoding::Utf16,
            root,
            init_options: None,
            settings: None,
            queued: Vec::new(),
            initialize_id: None,
            ids: IdAllocator::default(),
            pending: FxHashMap::default(),
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

    /// The server's raw `capabilities` object, verbatim off the wire — see
    /// `caps_json`'s doc comment for why this is what `(lsp-capabilities …)`
    /// must read instead of re-serializing `capabilities()`.
    pub fn capabilities_json(&self) -> Option<&serde_json::Value> {
        self.caps_json.as_ref()
    }

    /// Sets `initializationOptions` for the upcoming `initialize` request —
    /// must be called before `start_handshake` to take effect.
    pub fn set_init_options(&mut self, init_options: Option<serde_json::Value>) {
        self.init_options = init_options;
    }

    /// Sets the server configuration blob — must be called before
    /// `start_handshake` to take effect. Pushed as `workspace/
    /// didChangeConfiguration` right after `initialized`; also consulted to
    /// answer `workspace/configuration` pull requests.
    pub fn set_settings(&mut self, settings: Option<serde_json::Value>) {
        self.settings = settings;
    }

    pub fn encoding(&self) -> PositionEncoding {
        self.encoding
    }

    /// The `didChange` form the server asked for, or `None` when it wants no
    /// change notifications at all (declared `NONE`, or declared nothing —
    /// the spec's default for an absent `textDocumentSync`). Before the
    /// handshake there is no declaration to read, so this answers `FULL`: a
    /// whole-document event is the one form every server accepts, and
    /// dropping edits queued while `Starting` would desync the mirror
    /// permanently. Derived from `caps` rather than cached — read once per
    /// flush, not once per position conversion, so `encoding`'s decode-once
    /// rationale doesn't apply here.
    pub fn change_sync(&self) -> Option<TextDocumentSyncKind> {
        let Some(caps) = self.caps.as_ref() else {
            return Some(TextDocumentSyncKind::FULL); // pre-handshake: nothing declared yet
        };
        let kind = match caps.text_document_sync.as_ref()? {
            TextDocumentSyncCapability::Kind(k) => *k,
            TextDocumentSyncCapability::Options(TextDocumentSyncOptions { change, .. }) => {
                (*change)?
            }
        };
        (kind != TextDocumentSyncKind::NONE).then_some(kind)
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
    /// `initialize`/`shutdown` handshake requests — those are ordinary
    /// `pending` entries too.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Earliest deadline among pending requests (`initialize`/`shutdown`
    /// included — they are ordinary `pending` entries too). Feeds the
    /// editor's completion-driven wake predicate: this deadline is what
    /// keeps the timeout sweep in `take_completed` firing promptly even on a
    /// server that never responds.
    pub fn earliest_deadline(&self) -> Option<Instant> {
        self.pending.values().map(|m| m.deadline).min()
    }

    /// Best-effort cancellation: drops the pending entry (if still present),
    /// strips a still-queued Starting-phase send, and — only once the
    /// handshake has completed — sends `$/cancelRequest`. A no-op if the
    /// request already completed. Production caller: the editor bridge's
    /// `#:supersede` path (a new request cancels the caller's previous
    /// still-pending one filed under the same key).
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
                    params: serde_json::json!({ "id": id }),
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

        let mut params = serde_json::to_value(build_initialize_params(
            &self.root,
            self.init_options.clone(),
        ))
        .expect("InitializeParams always serializes");
        advertise_ranges_support(&mut params);

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
                // `begin_shutdown` on a still-Starting client jumps straight
                // to `Dead` without waiting for (or cancelling) the in-flight
                // `initialize` — a response landing after that must not
                // resurrect the client into `Running` via the handler below,
                // which unconditionally overwrites `state`.
                if self.state == ServerState::Starting {
                    self.handle_initialize_response(result)
                } else {
                    Vec::new()
                }
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
        // Captured before `from_value` below consumes `value` — the typed
        // parse below is lossy (see `caps_json`'s doc comment), so the raw
        // object is the only place `rangesSupport` and any other capability
        // outside the pinned `lsp_types` version survive.
        let raw_caps = value.get("capabilities").cloned();
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
        self.caps_json = raw_caps;
        self.state = ServerState::Running;

        let mut send = vec![Message::Notification {
            method: lsp_types::notification::Initialized::METHOD.to_string(),
            params: serde_json::to_value(InitializedParams {})
                .expect("InitializedParams always serializes"),
        }];
        // Pushed once, right after `initialized` and before any queued
        // `didOpen` — some servers read configuration only from this push
        // and never issue a `workspace/configuration` pull. Omitted
        // entirely when unset, never sent as `settings: null`.
        if let Some(settings) = self.settings.clone() {
            send.push(Message::Notification {
                method: lsp_types::notification::DidChangeConfiguration::METHOD.to_string(),
                params: serde_json::to_value(DidChangeConfigurationParams { settings })
                    .expect("DidChangeConfigurationParams always serializes"),
            });
        }
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
    /// correlates through `pending`/`take_completed` like any other request;
    /// a caller that drops the client immediately (e.g. `:lsp-stop`) instead
    /// dispatches it as timed out via `drain_pending`.
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

/// Sets `capabilities.textDocument.rangeFormatting.rangesSupport = true` on
/// the serialized `initialize` params, advertising LSP 3.18's
/// `textDocument/rangesFormatting`. `lsp_types` 0.97 predates that
/// extension — `DocumentRangeFormattingClientCapabilities` is a bare
/// `DynamicRegistrationClientCapabilities` alias with no `rangesSupport`
/// field — so the flag is unrepresentable in the typed `ClientCapabilities`
/// built by `build_client_capabilities` and must be patched into the
/// already-serialized JSON instead. `range_formatting: Some(Default::
/// default())` there guarantees this pointer resolves to an (empty) object.
fn advertise_ranges_support(params: &mut serde_json::Value) {
    if let Some(range_formatting) = params
        .pointer_mut("/capabilities/textDocument/rangeFormatting")
        .and_then(serde_json::Value::as_object_mut)
    {
        range_formatting.insert("rangesSupport".to_string(), serde_json::Value::Bool(true));
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
            // No dynamic registration path exists — the push happens
            // unconditionally right after `initialized`, so this only
            // needs to be present, not negotiated.
            did_change_configuration: Some(DidChangeConfigurationClientCapabilities {
                dynamic_registration: Some(false),
            }),
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
            // Without `label_offset_support` a server must describe each
            // parameter by repeating its text, which only *names* the
            // parameter — locating it back inside the signature label means
            // substring-searching for it, and a label like
            // `fn f(a: T, b: T)` has no unique match to find. The offset
            // form says where it is outright, which is what marking the
            // active parameter in place will need. The offsets count code
            // units in the negotiated encoding, so they convert host-side
            // (`lsp-label-offsets->text`) — Scheme has no way to know what
            // was negotiated.
            signature_help: Some(SignatureHelpClientCapabilities {
                signature_information: Some(SignatureInformationSettings {
                    parameter_information: Some(ParameterInformationSettings {
                        label_offset_support: Some(true),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            rename: Some(RenameClientCapabilities::default()),
            references: Some(Default::default()),
            definition: Some(GotoCapability::default()),
            declaration: Some(GotoCapability::default()),
            type_definition: Some(GotoCapability::default()),
            implementation: Some(GotoCapability::default()),
            formatting: Some(Default::default()),
            range_formatting: Some(Default::default()),
            // rust-analyzer withholds diagnostic-derived quickfixes
            // entirely without code_action_literal_support declared — the
            // flag saying the client understands CodeAction objects, not
            // just legacy Command[]. Without it, even a byte-perfect
            // request (correct diagnostic round-tripped verbatim, correct
            // overlapping range) comes back empty.
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
///
/// `settings` is the server's configured blob (if any) — the caller resolves
/// it from the registry keyed by server id, since this function has no
/// editor state of its own.
pub fn server_request_response(
    method: &str,
    params: &serde_json::Value,
    settings: Option<&serde_json::Value>,
) -> Result<serde_json::Value, ResponseError> {
    use lsp_types::request::{
        RegisterCapability, UnregisterCapability, WorkDoneProgressCreate, WorkspaceConfiguration,
    };
    match method {
        WorkspaceConfiguration::METHOD => Ok(workspace_configuration_response(params, settings)),
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

/// Resolves one requested item's `section` (e.g. `"a.b"`) against `settings`,
/// treating it as the root config object — VS Code semantics. No section (or
/// an empty one) returns the whole blob; a dotted path walks object keys;
/// any miss (missing key, or a non-object encountered mid-path) is `null`.
pub(crate) fn resolve_config_section(
    settings: &serde_json::Value,
    section: Option<&str>,
) -> serde_json::Value {
    let Some(section) = section.filter(|s| !s.is_empty()) else {
        return settings.clone();
    };
    let mut current = settings;
    for part in section.split('.') {
        match current.get(part) {
            Some(next) => current = next,
            None => return serde_json::Value::Null,
        }
    }
    current.clone()
}

/// One entry per requested item, same length and order as `params.items`,
/// per spec (the result array must line up positionally with the request).
/// With no settings blob, every item answers `null` — same shape a server
/// sees from a client with no matching config.
fn workspace_configuration_response(
    params: &serde_json::Value,
    settings: Option<&serde_json::Value>,
) -> serde_json::Value {
    let items = params.get("items").and_then(|v| v.as_array());
    let Some(settings) = settings else {
        return serde_json::Value::Array(vec![
            serde_json::Value::Null;
            items.map_or(0, |v| v.len())
        ]);
    };
    let values = items
        .into_iter()
        .flatten()
        .map(|item| {
            let section = item.get("section").and_then(|s| s.as_str());
            resolve_config_section(settings, section)
        })
        .collect();
    serde_json::Value::Array(values)
}

#[cfg(test)]
mod tests;
