//! Per-server client state machine: `initialize` handshake with capability
//! and position-encoding negotiation, graceful shutdown, crash detection.

use rustc_hash::FxHashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use hume_core::LspServerConfig;
use hume_rope::position_encoding::PositionEncoding;
use lsp_types::{
    DidChangeConfigurationParams, InitializeResult, InitializedParams, PositionEncodingKind,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions,
};

use crate::backend::{LspBackend, ServerId};
use crate::codec::{IdAllocator, Message, RequestId, ResponseError};
use crate::diagnostic::{PublishedDiagnostics, classify_publish};
use crate::initialize::initialize_params_json;
use crate::transport::InboundEvent;

use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;

hume_core::named_enum! {
    /// A server's lifecycle state, named as scripting sees it
    /// (`lsp-server-status`'s `'state`).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ServerState {
        Starting => "starting",
        Running => "running",
        Crashed => "crashed",
        Dead => "dead",
    }
}

/// One action for the editor glue to take in response to a lifecycle event.
#[derive(Debug)]
pub enum ClientAction {
    /// Handshake completed: send these messages in order (`initialized`
    /// first, then anything queued while `Starting`), then fire
    /// `on-lsp-attach` for buffers already attached to this server.
    BecameRunning { send: Vec<Message> },
    /// The connection died: report once; restart stays manual.
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
    Diagnostics(PublishedDiagnostics),
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
    /// hook can still observe it).
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
/// parses. Deserializes by reference so a malformed payload leaves
/// `params` intact for that fallback (`from_value` would consume it).
fn classify_notification(method: String, mut params: serde_json::Value) -> ClientAction {
    use lsp_types::notification::{LogMessage, Progress, PublishDiagnostics, ShowMessage};
    use serde::Deserialize as _;

    match method.as_str() {
        PublishDiagnostics::METHOD => {
            if let Some(published) = classify_publish(&mut params) {
                return ClientAction::Diagnostics(published);
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
/// lsp_types-required `title`, observed from servers that treat it as
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
/// missing or not an integer: `MessageType` is a transparent `i32` newtype,
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
/// request. `hume-lsp` never holds editor closures (crate fence); the
/// editor keys its own callback under the `(ServerId, RequestId)` pair
/// this crate already hands back from `send_request`/`take_completed`/
/// `drain_pending`, so no separate token needs to round-trip through here.
#[derive(Debug, Clone)]
pub struct RequestMeta {
    pub method: String,
    pub deadline: Instant,
}

#[derive(Debug)]
pub enum Outcome {
    Ok(serde_json::Value),
    Err(ResponseError),
    TimedOut,
    /// The server stopped or crashed before it answered.
    Stopped,
}

/// How long `initialize` may go unanswered before the client gives up and
/// transitions to `Crashed`, independent of
/// `lsp-request-timeout-ms` (a per-request setting): a cold server's
/// handshake legitimately outlasts the timeout an ordinary request would
/// get.
const INITIALIZE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// How long `shutdown` may go unanswered before its pending entry is swept
/// as timed out. Never observed in production (`:lsp-stop` drains pending
/// requests immediately and quit tears the transport down regardless), but
/// every `pending` entry needs a deadline.
const SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Per-server lifecycle state: handshake, negotiated encoding, and the
/// queue of messages a caller tried to send before the handshake finished.
pub struct LspClient {
    id: ServerId,
    state: ServerState,
    /// The server's raw `capabilities` object, verbatim off the wire:
    /// the capabilities source of truth. A typed `ServerCapabilities` decode
    /// would drop any field the pinned `lsp_types` version doesn't model
    /// (e.g. LSP 3.18's `documentRangeFormattingProvider.rangesSupport`),
    /// and `(lsp-capabilities …)` must hand Steel the wire value verbatim.
    /// `Arc`-wrapped so every capability-guarded command's read of it (a
    /// `JsonHandle`, once it crosses to Steel) shares this one allocation
    /// instead of cloning the whole capabilities blob per read.
    caps_json: Option<Arc<serde_json::Value>>,
    /// Negotiated position encoding; UTF-16 until `initialize` proves UTF-8.
    /// A decode-once cache of the capabilities' `positionEncoding`
    /// (`handle_initialize_response` is the only writer of either field, an
    /// invariant field privacy enforces), kept separate so callers don't re-derive it from
    /// the raw capability on every position conversion, not an independent
    /// fact that could drift on its own.
    encoding: PositionEncoding,
    /// The `didChange` form the server asked for, decoded from the
    /// capabilities by `handle_initialize_response`; `FULL` until then.
    change_sync: Option<TextDocumentSyncKind>,
    root: PathBuf,
    /// The registration this server was spawned with. Its `init_options`
    /// are the `initialize` request's `initializationOptions`, omitted (never
    /// `null`) when `None`. Its `settings` are pushed once as
    /// `workspace/didChangeConfiguration` right after `initialized` and
    /// resolved per item for `workspace/configuration` pull requests
    /// (`server_request_response`); `None` sends no push, and each pulled
    /// item answers `null`.
    config: Arc<LspServerConfig>,
    /// Messages (e.g. `didOpen`) that arrived while `Starting`, sent, in
    /// order, right after `initialized` once the handshake completes.
    queued: Vec<Message>,
    /// Discriminates the `initialize` response in `on_event`, kept
    /// separate from a method-string check so a Steel-issued `(lsp-request!
    /// "initialize" ...)` through the generic bridge (which mints its own
    /// ordinary `pending` entry) can never be mistaken for the handshake.
    initialize_id: Option<RequestId>,
    ids: IdAllocator,
    /// Requests awaiting a response, keyed by the id we sent.
    pending: FxHashMap<RequestId, RequestMeta>,
    /// Responses matched against `pending` by `on_event`, waiting to be
    /// pulled by `take_completed`, never delivered inline (same
    /// drain-boundary discipline as the `InlineLspBackend` double).
    completed: Vec<(RequestId, RequestMeta, Outcome)>,
}

/// Whether `new` makes `last` redundant: `new` is a whole-document
/// `didChange` and `last` a `didChange` for the same document.
fn supersedes(last: &Message, new: &Message) -> bool {
    let (
        Message::Notification {
            method: last_method,
            params: last_params,
        },
        Message::Notification {
            method: new_method,
            params: new_params,
        },
    ) = (last, new)
    else {
        return false;
    };
    let did_change = lsp_types::notification::DidChangeTextDocument::METHOD;
    let whole_document = new_params["contentChanges"]
        .as_array()
        .is_some_and(|changes| changes.len() == 1 && changes[0].get("range").is_none());
    last_method == did_change
        && new_method == did_change
        && whole_document
        && new_params["textDocument"]["uri"]
            .as_str()
            .is_some_and(|uri| last_params["textDocument"]["uri"].as_str() == Some(uri))
}

impl LspClient {
    pub fn new(id: ServerId, root: PathBuf, config: Arc<LspServerConfig>) -> Self {
        Self {
            id,
            state: ServerState::Starting,
            caps_json: None,
            encoding: PositionEncoding::Utf16,
            change_sync: Some(TextDocumentSyncKind::FULL),
            root,
            config,
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

    /// See `caps_json`'s doc comment.
    pub fn capabilities_json(&self) -> Option<&Arc<serde_json::Value>> {
        self.caps_json.as_ref()
    }

    /// The registration this server was spawned with: its
    /// `initializationOptions`, and the settings pushed after `initialized`
    /// and resolved for `workspace/configuration`.
    pub fn config(&self) -> &Arc<LspServerConfig> {
        &self.config
    }

    pub fn encoding(&self) -> PositionEncoding {
        self.encoding
    }

    /// The `didChange` form the server asked for, or `None` when it wants no
    /// change notifications at all (declared `NONE`, or declared nothing,
    /// the spec's default for an absent `textDocumentSync`). Before the
    /// handshake there is no declaration to read, so this answers `FULL`: a
    /// whole-document event is the one form every server accepts, and
    /// dropping edits queued while `Starting` would desync the mirror
    /// permanently.
    pub fn change_sync(&self) -> Option<TextDocumentSyncKind> {
        self.change_sync
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
    /// position-independent from this layer's perspective; staleness by
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
    /// answer it, so the caller's whole requested timeout (routinely tens
    /// of seconds) must not stand between it and the `TimedOut` outcome
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

    /// Requests currently awaiting a response: the "N in flight" count for
    /// `:lsp-status` and `lsp-server-status`. Includes the in-flight
    /// `initialize`/`shutdown` handshake requests; those are ordinary
    /// `pending` entries too.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Earliest deadline among pending requests (`initialize`/`shutdown`
    /// included; they are ordinary `pending` entries too). Feeds the
    /// editor's completion-driven wake predicate: this deadline is what
    /// keeps the timeout sweep in `take_completed` firing promptly even on a
    /// server that never responds.
    pub fn earliest_deadline(&self) -> Option<Instant> {
        self.pending.values().map(|m| m.deadline).min()
    }

    /// Best-effort cancellation: drops the pending entry (if still present),
    /// strips a still-queued Starting-phase send, and (only once the
    /// handshake has completed) sends `$/cancelRequest`. A no-op if the
    /// request already completed.
    pub fn cancel(&mut self, backend: &mut dyn LspBackend, id: RequestId) {
        if self.pending.remove(&id).is_some() {
            self.drop_from_queue(&id);
            self.send_cancel_notification(backend, &id);
        }
    }

    /// Removes and returns every still-pending request as
    /// [`Outcome::Stopped`], for teardown paths that drop the client (e.g.
    /// `:lsp-stop`, a crash) so the caller can dispatch each rather than
    /// silently orphaning a registered callback along with the client.
    pub fn drain_pending(&mut self) -> Vec<(RequestId, RequestMeta, Outcome)> {
        self.pending
            .drain()
            .map(|(id, meta)| (id, meta, Outcome::Stopped))
            .collect()
    }

    /// Forces every pending deadline into the past, so the next
    /// `take_completed` sweep expires them without waiting out real time,
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
    /// `handle_initialize_response` once the handshake completes: nothing
    /// would be left to correlate the eventual response (or notice it
    /// arrived at all), and no `$/cancelRequest` would ever follow it.
    fn drop_from_queue(&mut self, id: &RequestId) {
        self.queued
            .retain(|msg| !matches!(msg, Message::Request { id: qid, .. } if qid == id));
    }

    /// Best-effort `$/cancelRequest`, only legal once the handshake has
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
    /// `on_event`: deadline checks piggyback on the same cadence, no
    /// separate timer thread. A timed-out entry gets a best-effort
    /// `$/cancelRequest` sent here (colocated with the detection, so it's
    /// testable without an editor in the loop), only once `Running`, same
    /// handshake-ordering reasoning as `cancel`.
    ///
    /// `initialize` piggybacks on this same sweep instead of a separate
    /// timeout check: on expiry it is never pushed into the completed Vec
    /// (no callback is ever registered for it; the handshake response is
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
                    self.crash();
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
                deadline: Instant::now() + INITIALIZE_TIMEOUT,
            },
        );

        let params = initialize_params_json(&self.root, self.config.init_options.clone());

        // Sent directly, never via `send_or_queue`: `initialize` is the one
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

    /// Moves to `Crashed`. What was queued for the handshake is never sent.
    fn crash(&mut self) {
        self.state = ServerState::Crashed;
        self.queued.clear();
    }

    /// Send `msg` now if the handshake has completed, otherwise queue it for
    /// delivery, in order, once `initialized` goes out. A whole-document
    /// `didChange` replaces a `didChange` for the same document that is the
    /// last message queued, so a burst of edits queues one copy of the text.
    /// A dead or crashed connection silently drops the send; the crash is
    /// already reported via the `Crashed` state, matching the transport's
    /// own send-after-death discipline.
    pub fn send_or_queue(&mut self, backend: &mut dyn LspBackend, msg: Message) {
        match self.state {
            ServerState::Running => backend.send(self.id, msg),
            ServerState::Starting => match self.queued.last_mut() {
                Some(last) if supersedes(last, &msg) => *last = msg,
                _ => self.queued.push(msg),
            },
            ServerState::Crashed | ServerState::Dead => {}
        }
    }

    /// Feed one inbound event; returns actions for the glue to act on.
    pub fn on_event(&mut self, ev: InboundEvent) -> Vec<ClientAction> {
        match ev {
            InboundEvent::Eof { error } => {
                // Guard against reporting twice if more events trickle in
                // after the connection is already known dead. `Dead` covers
                // a graceful `begin_shutdown` teardown racing a trailing
                // `Eof` from the exiting process just as validly as
                // `Crashed` covers an actual crash; either way this must not
                // report a spurious "server crashed" on top of an orderly
                // exit.
                if matches!(self.state, ServerState::Crashed | ServerState::Dead) {
                    return Vec::new();
                }
                self.crash();
                vec![ClientAction::Crashed { error }]
            }
            InboundEvent::Message(Message::Response { id, result })
                if self.initialize_id.as_ref() == Some(&id) =>
            {
                self.initialize_id = None;
                self.pending.remove(&id);
                // `begin_shutdown` on a still-Starting client jumps straight
                // to `Dead` without waiting for (or cancelling) the in-flight
                // `initialize`, and a response landing after that must not
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
                self.crash();
                return vec![ClientAction::Crashed {
                    error: Some(format!("initialize failed: {} ({})", e.message, e.code)),
                }];
            }
        };
        // Captured before `from_value` below consumes `value`. Arc-wrapped
        // here, at the one clone this data ever needs: every later read
        // (`capabilities_json`, and the `JsonHandle` it becomes crossing to
        // Steel) shares this allocation instead of cloning again.
        let raw_caps = value.get("capabilities").cloned().map(Arc::new);
        let parsed: InitializeResult = match serde_json::from_value(value) {
            Ok(r) => r,
            Err(e) => {
                self.crash();
                return vec![ClientAction::Crashed {
                    error: Some(format!("malformed initialize result: {e}")),
                }];
            }
        };

        // Absent `positionEncoding` means UTF-16 per spec. Never default to
        // UTF-8 just because the server omitted the field.
        self.encoding = if parsed.capabilities.position_encoding == Some(PositionEncodingKind::UTF8)
        {
            PositionEncoding::Utf8
        } else {
            PositionEncoding::Utf16
        };
        self.change_sync = match &parsed.capabilities.text_document_sync {
            Some(TextDocumentSyncCapability::Kind(kind)) => Some(*kind),
            Some(TextDocumentSyncCapability::Options(TextDocumentSyncOptions {
                change, ..
            })) => *change,
            None => None,
        }
        .filter(|kind| *kind != TextDocumentSyncKind::NONE);
        self.caps_json = raw_caps;
        self.state = ServerState::Running;

        let mut send = vec![Message::Notification {
            method: lsp_types::notification::Initialized::METHOD.to_string(),
            params: serde_json::to_value(InitializedParams {})
                .expect("InitializedParams always serializes"),
        }];
        // Pushed once, right after `initialized` and before any queued
        // `didOpen`: some servers read configuration only from this push
        // and never issue a `workspace/configuration` pull. Omitted
        // entirely when unset, never sent as `settings: null`.
        if let Some(settings) = self.config.settings.clone() {
            send.push(Message::Notification {
                method: lsp_types::notification::DidChangeConfiguration::METHOD.to_string(),
                params: serde_json::to_value(DidChangeConfigurationParams { settings })
                    .expect("DidChangeConfigurationParams always serializes"),
            });
        }
        send.append(&mut self.queued);
        vec![ClientAction::BecameRunning { send }]
    }

    /// `shutdown` request, then `exit` notification, only while `Running`;
    /// nothing but `initialize` is legal on the wire before `initialized`,
    /// so a Starting (or already Crashed/Dead) client sends nothing here.
    /// Every caller still gets a definite `Dead` transition regardless of
    /// prior state, so the transport-level teardown (`ServerHandle::drop`:
    /// kill -> wait -> join, which reaps the process unconditionally) is
    /// always what actually ends a non-Running client. This is a
    /// best-effort protocol courtesy on top of that, never a substitute
    /// for it, and never a synchronous round-trip. The `shutdown` response
    /// correlates through `pending`/`take_completed` like any other request;
    /// a caller that drops the client immediately (e.g. `:lsp-stop`) instead
    /// dispatches it as stopped via `drain_pending`.
    pub fn begin_shutdown(&mut self, backend: &mut dyn LspBackend) {
        if self.state == ServerState::Running {
            let id = self.ids.next();
            self.pending.insert(
                id.clone(),
                RequestMeta {
                    method: lsp_types::request::Shutdown::METHOD.to_string(),
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

#[cfg(test)]
mod tests;
