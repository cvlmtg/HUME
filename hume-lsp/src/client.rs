//! Per-server client state machine: `initialize` handshake with capability
//! and position-encoding negotiation, graceful shutdown, crash detection.

use std::path::PathBuf;

use hume_editing::PositionEncoding;
use lsp_types::{
    ClientCapabilities, ClientInfo, CodeActionClientCapabilities, CompletionClientCapabilities,
    CompletionItemCapability, GeneralClientCapabilities, GotoCapability, HoverClientCapabilities,
    InitializeParams, InitializeResult, InitializedParams, MarkupKind,
    PositionEncodingKind, PublishDiagnosticsClientCapabilities, RenameClientCapabilities,
    ServerCapabilities, TextDocumentClientCapabilities, TextDocumentSyncClientCapabilities,
    WorkspaceClientCapabilities, WorkspaceFolder,
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
        }
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
                self.state = ServerState::Crashed;
                vec![ClientAction::Crashed { error }]
            }
            InboundEvent::Message(Message::Response { id, result })
                if self.initialize_id.as_ref() == Some(&id) =>
            {
                self.initialize_id = None;
                self.handle_initialize_response(result)
            }
            InboundEvent::Message(_) | InboundEvent::Stderr(_) => Vec::new(),
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
            code_action: Some(CodeActionClientCapabilities::default()),
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
}
