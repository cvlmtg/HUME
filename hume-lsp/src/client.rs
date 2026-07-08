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
    TextDocumentSyncClientCapabilities, WorkspaceClientCapabilities, WorkspaceEditClientCapabilities,
    WorkspaceFolder,
};

use crate::backend::{LspBackend, ServerId};
use crate::codec::{IdAllocator, Message, RequestId, ResponseError};
use crate::transport::InboundEvent;
use crate::uri;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerState {
    Starting,
    Running,
    ShuttingDown,
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
    /// The connection died — report once; restart stays manual (hub OQ default).
    Crashed { error: Option<String> },
    /// The server sent a request; every one must get exactly one response
    /// (a hung server request can stall its whole pipeline). The dispatch
    /// table lives in the editor glue (hub decision: answered in Rust,
    /// never surfaced to Steel).
    ServerRequest {
        id: RequestId,
        method: String,
        params: serde_json::Value,
    },
    /// A notification this module doesn't own the meaning of (window
    /// messages, progress, publishDiagnostics, custom methods, ...).
    ServerNotification {
        method: String,
        params: serde_json::Value,
    },
    /// One line of stderr output, forwarded for logging.
    Stderr(String),
}

/// Opaque handle the editor maps to a real callback; `hume-lsp` never holds
/// editor closures (crate fence) — it only carries this token round-trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CallbackToken(pub u64);

/// Everything but the outcome needed to route (or discard) a completed
/// request. `token` is minted and interpreted entirely by the editor.
#[derive(Debug, Clone)]
pub struct RequestMeta {
    pub method: String,
    pub allow_stale: bool,
    pub deadline: Instant,
    pub token: CallbackToken,
}

#[derive(Debug)]
pub enum Outcome {
    Ok(serde_json::Value),
    Err(ResponseError),
    TimedOut,
}

/// Builds the `$/cancelRequest` notification params for `id`. Shared by
/// `LspClient::cancel` and the editor glue's timeout path.
pub fn cancel_request_params(id: &RequestId) -> serde_json::Value {
    let id_value = match id {
        RequestId::Int(n) => serde_json::Value::from(*n),
        RequestId::Str(s) => serde_json::Value::String(s.clone()),
    };
    serde_json::json!({ "id": id_value })
}

/// Per-server lifecycle state: handshake, negotiated encoding, and the
/// queue of messages a caller tried to send before the handshake finished.
pub struct LspClient {
    pub id: ServerId,
    pub state: ServerState,
    pub caps: Option<ServerCapabilities>,
    /// Negotiated position encoding; UTF-16 until `initialize` proves UTF-8.
    pub encoding: PositionEncoding,
    pub root: PathBuf,
    /// Messages (e.g. `didOpen`) that arrived while `Starting` — sent, in
    /// order, right after `initialized` once the handshake completes.
    queued: Vec<Message>,
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
            queued: Vec::new(),
            initialize_id: None,
            ids: IdAllocator::new(),
            pending: HashMap::new(),
            completed: Vec::new(),
        }
    }

    /// Sends a request and remembers `meta` for correlation. Requests are
    /// position-independent from this layer's perspective — staleness by
    /// buffer generation is tracked editor-side (the crate fence: `hume-lsp`
    /// has no `BufferId`).
    ///
    /// Respects the same `Starting`-queue discipline as `send_or_queue`:
    /// nothing but `initialize` is legal on the wire before `initialized`
    /// goes out, so a request minted while still handshaking is queued and
    /// flushed by `handle_initialize_response` alongside queued document
    /// sync — never sent directly. `pending` gets the entry either way, so
    /// `take_completed`'s deadline/timeout handling covers a request that's
    /// still queued exactly like one already on the wire.
    pub fn send_request(
        &mut self,
        backend: &mut dyn LspBackend,
        method: &str,
        params: serde_json::Value,
        meta: RequestMeta,
    ) -> RequestId {
        let id = self.ids.next();
        self.pending.insert(id.clone(), meta);
        let msg = Message::Request {
            id: id.clone(),
            method: method.to_string(),
            params,
        };
        match self.state {
            ServerState::Running => backend.send(self.id, msg),
            ServerState::Starting => self.queued.push(msg),
            // Connection is gone or going — nothing to send; the request
            // sits in `pending` until its deadline, then surfaces as
            // `Outcome::TimedOut` rather than hanging silently.
            ServerState::ShuttingDown | ServerState::Crashed | ServerState::Dead => {}
        }
        id
    }

    /// Requests currently awaiting a response — the "N in flight" count for
    /// `:lsp-status` (C10) and B3's `lsp-server-status`.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Best-effort cancellation: drops the pending entry (if still present)
    /// and notifies the server. A no-op if the request already completed.
    pub fn cancel(&mut self, backend: &mut dyn LspBackend, id: RequestId) {
        if self.pending.remove(&id).is_some() {
            backend.send(
                self.id,
                Message::Notification {
                    method: "$/cancelRequest".to_string(),
                    params: cancel_request_params(&id),
                },
            );
        }
    }

    /// Pulls every request that finished (correlated response) or expired
    /// (deadline reached) since the last call. Called at drain, alongside
    /// `on_event` — deadline checks piggyback on the same cadence, no
    /// separate timer thread. A timed-out entry gets a best-effort
    /// `$/cancelRequest` sent here (colocated with the detection, so it's
    /// testable without an editor in the loop).
    pub fn take_completed(
        &mut self,
        backend: &mut dyn LspBackend,
        now: Instant,
    ) -> Vec<(RequestId, RequestMeta, Outcome)> {
        let mut out = std::mem::take(&mut self.completed);
        let timed_out: Vec<RequestId> = self
            .pending
            .iter()
            .filter(|(_, meta)| meta.deadline <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in timed_out {
            if let Some(meta) = self.pending.remove(&id) {
                backend.send(
                    self.id,
                    Message::Notification {
                        method: "$/cancelRequest".to_string(),
                        params: cancel_request_params(&id),
                    },
                );
                out.push((id, meta, Outcome::TimedOut));
            }
        }
        out
    }

    /// Builds `InitializeParams` and sends the request.
    pub fn start_handshake(&mut self, backend: &mut dyn LspBackend) {
        let id = self.ids.next();
        self.initialize_id = Some(id.clone());

        let params = serde_json::to_value(build_initialize_params(&self.root))
            .unwrap_or(serde_json::Value::Null);

        backend.send(
            self.id,
            Message::Request {
                id,
                method: "initialize".to_string(),
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
            ServerState::ShuttingDown | ServerState::Crashed | ServerState::Dead => {}
        }
    }

    /// Feed one inbound event; returns actions for the glue to act on.
    pub fn on_event(&mut self, ev: InboundEvent) -> Vec<ClientAction> {
        match ev {
            InboundEvent::Eof { error } => {
                // Guard against reporting twice if more events trickle in
                // after the connection is already known dead.
                if self.state == ServerState::Crashed {
                    return Vec::new();
                }
                self.state = ServerState::Crashed;
                vec![ClientAction::Crashed { error }]
            }
            InboundEvent::Message(Message::Response { id, result })
                if self.initialize_id.as_ref() == Some(&id) =>
            {
                self.initialize_id = None;
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
                vec![ClientAction::ServerNotification { method, params }]
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
            method: "initialized".to_string(),
            params: serde_json::to_value(InitializedParams {}).unwrap_or(serde_json::Value::Null),
        }];
        send.append(&mut self.queued);
        vec![ClientAction::BecameRunning { send }]
    }

    /// `shutdown` request, then `exit` notification. The transport-level
    /// teardown (`ServerHandle::drop`: kill -> wait -> join) reaps the
    /// process regardless, so this is a best-effort courtesy, not a
    /// synchronous protocol round-trip.
    pub fn begin_shutdown(&mut self, backend: &mut dyn LspBackend) {
        self.state = ServerState::ShuttingDown;
        backend.send(
            self.id,
            Message::Request {
                id: self.ids.next(),
                method: "shutdown".to_string(),
                params: serde_json::Value::Null,
            },
        );
        backend.send(
            self.id,
            Message::Notification {
                method: "exit".to_string(),
                params: serde_json::Value::Null,
            },
        );
        self.state = ServerState::Dead;
    }
}

#[allow(deprecated)] // root_uri/root_path are deprecated in favor of workspace_folders,
// but rootUri compatibility is a deliberate hub decision (older servers still read it).
fn build_initialize_params(root: &std::path::Path) -> InitializeParams {
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
        ..Default::default()
    }
}

fn build_client_capabilities() -> ClientCapabilities {
    ClientCapabilities {
        workspace: Some(WorkspaceClientCapabilities {
            apply_edit: Some(true),
            configuration: Some(true),
            workspace_folders: Some(true),
            // Every rename result (F5) is a WorkspaceEdit — some servers
            // (rust-analyzer) refuse textDocument/rename outright without
            // this declared, since they can't otherwise confirm the client
            // can apply one (found via F5's manual smoke test).
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
                    // v1 strips snippet placeholders to plain text (hub OQ default).
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
            // F9's manual smoke test found rust-analyzer withholds
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
            position_encodings: Some(vec![PositionEncodingKind::UTF8, PositionEncodingKind::UTF16]),
            ..Default::default()
        }),
        ..Default::default()
    }
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

    // Golden-field check on the load-bearing capability list (hub's C5 card:
    // "capabilities are load-bearing config" — assert the exact advertised
    // set rather than just "it builds").
    #[test]
    #[allow(deprecated)] // asserting on the deliberately-still-populated compat field
    fn initialize_params_advertise_the_v1_capability_set() {
        let params = build_initialize_params(&PathBuf::from("/tmp/proj"));

        assert_eq!(params.process_id, Some(std::process::id()));
        assert!(params.root_uri.is_some());
        let folders = params.workspace_folders.expect("workspace_folders set");
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].name, "proj");

        let caps = params.capabilities;
        assert_eq!(
            caps.general.unwrap().position_encodings,
            Some(vec![PositionEncodingKind::UTF8, PositionEncodingKind::UTF16])
        );
        let td = caps.text_document.unwrap();
        assert_eq!(
            td.completion.unwrap().completion_item.unwrap().snippet_support,
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
        // F5's manual smoke test found rust-analyzer refuses
        // textDocument/rename outright without this declared — every
        // rename result is a WorkspaceEdit, and some servers won't attempt
        // one unless the client has confirmed it can apply it.
        let we = ws.workspace_edit.expect("workspace_edit capability must be declared");
        assert_eq!(we.document_changes, Some(true));
        // Must be present or rust-analyzer refuses every rename outright
        // (confirmed live) — HUME still can't actually apply a resource
        // op if one arrives (edits::collect_edit_entries rejects it, B6
        // design decision), but the alternative breaks the common case.
        assert_eq!(
            we.resource_operations,
            Some(vec![
                ResourceOperationKind::Create,
                ResourceOperationKind::Rename,
                ResourceOperationKind::Delete,
            ])
        );
        assert_eq!(we.failure_handling, Some(FailureHandlingKind::Abort));
        // F9's manual smoke test found rust-analyzer withholds
        // diagnostic-derived quickfixes entirely without this declared —
        // a byte-perfect codeAction request still came back empty.
        let ca = td.code_action.expect("code_action capability must be declared");
        let literal = ca.code_action_literal_support.expect("code_action_literal_support must be declared");
        assert!(literal.code_action_kind.value_set.contains(&CodeActionKind::QUICKFIX.as_str().to_string()));
        assert_eq!(ca.is_preferred_support, Some(true));
        assert_eq!(ca.disabled_support, Some(true));
    }

    #[test]
    fn handshake_round_trip_transitions_to_running() {
        let mut backend = InlineLspBackend::with_default_handshake();
        let sid = backend.start("rust-analyzer", &[], std::path::Path::new(".")).unwrap();
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
            token: CallbackToken(1),
        };
        let sent_id =
            client.send_request(&mut backend, "textDocument/hover", serde_json::Value::Null, meta);

        // Nothing but the initialize request should be on the wire yet.
        assert!(
            backend
                .sent
                .iter()
                .all(|(_, m)| !matches!(m, Message::Request { method, .. } if method == "textDocument/hover")),
            "request must be queued, not sent, while Starting"
        );
        assert_eq!(client.pending_count(), 1, "pending entry recorded even though queued");

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
        let completed = client.take_completed(&mut backend, Instant::now());
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].0, sent_id);
    }

    #[test]
    fn utf8_negotiated_when_offered() {
        let mut backend = InlineLspBackend::new();
        backend.respond_to("initialize", canned_result(Some(PositionEncodingKind::UTF8)));
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

    #[test]
    fn shutdown_sends_shutdown_request_then_exit_notification_in_order() {
        let mut backend = InlineLspBackend::new();
        let sid = backend.start("x", &[], std::path::Path::new(".")).unwrap();
        let mut client = LspClient::new(sid, PathBuf::from("."));

        client.begin_shutdown(&mut backend);

        assert_eq!(client.state, ServerState::Dead);
        assert_eq!(backend.sent.len(), 2);
        match &backend.sent[0] {
            (_, Message::Request { method, .. }) => assert_eq!(method, "shutdown"),
            other => panic!("expected the shutdown request first, got {other:?}"),
        }
        match &backend.sent[1] {
            (_, Message::Notification { method, .. }) => assert_eq!(method, "exit"),
            other => panic!("expected the exit notification second, got {other:?}"),
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

        let token = CallbackToken(1);
        let meta = RequestMeta {
            method: "textDocument/hover".to_string(),
            allow_stale: false,
            deadline: Instant::now() + std::time::Duration::from_secs(10),
            token,
        };
        let sent_id =
            client.send_request(&mut backend, "textDocument/hover", serde_json::Value::Null, meta);

        let (_sid, ev) = backend.drain().into_iter().next().unwrap();
        let actions = client.on_event(ev);
        assert!(
            actions.is_empty(),
            "a correlated response produces no ClientAction — it's pulled via take_completed"
        );

        let completed = client.take_completed(&mut backend, Instant::now());
        assert_eq!(completed.len(), 1);
        let (id, meta_out, outcome) = &completed[0];
        assert_eq!(*id, sent_id);
        assert_eq!(meta_out.token, token);
        match outcome {
            Outcome::Ok(v) => assert_eq!(*v, serde_json::json!({"contents": "hi"})),
            other => panic!("expected Ok, got {other:?}"),
        }

        // Pulled once — a second call finds nothing left.
        assert!(client.take_completed(&mut backend, Instant::now()).is_empty());
    }

    #[test]
    fn cancel_removes_pending_and_sends_cancel_notification() {
        let (mut backend, mut client) = make_running_client();

        let meta = RequestMeta {
            method: "textDocument/definition".to_string(),
            allow_stale: false,
            deadline: Instant::now() + std::time::Duration::from_secs(10),
            token: CallbackToken(7),
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
        assert!(client.take_completed(&mut backend, Instant::now()).is_empty());
    }

    #[test]
    fn take_completed_reports_timeout_and_sends_cancel_request() {
        let (mut backend, mut client) = make_running_client();
        let meta = RequestMeta {
            method: "textDocument/completion".to_string(),
            allow_stale: false,
            deadline: Instant::now() - std::time::Duration::from_millis(1),
            token: CallbackToken(3),
        };
        let id = client.send_request(
            &mut backend,
            "textDocument/completion",
            serde_json::Value::Null,
            meta,
        );

        let completed = client.take_completed(&mut backend, Instant::now());
        assert_eq!(completed.len(), 1);
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
        assert!(client.take_completed(&mut backend, Instant::now()).is_empty());
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
                method: "textDocument/publishDiagnostics".to_string(),
                params: serde_json::json!({"uri": "file:///a", "diagnostics": []}),
            },
        );
        let (_sid, ev) = backend.drain().into_iter().next().unwrap();
        let actions = client.on_event(ev);
        match &actions[..] {
            [ClientAction::ServerNotification { method, .. }] => {
                assert_eq!(method, "textDocument/publishDiagnostics");
            }
            other => panic!("expected one ServerNotification action, got {other:?}"),
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
}
