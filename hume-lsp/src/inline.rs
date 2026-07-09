//! Deterministic scripted double for [`LspBackend`]: no process, no threads.
//! The workhorse for every editor/Steel test — the LSP analog
//! of `hume-treesitter`'s `InlineParseBackend`.

use std::collections::{HashMap, VecDeque};
use std::path::Path;

use lsp_types::{
    CodeActionProviderCapability, CompletionOptions, DeclarationCapability,
    HoverProviderCapability, ImplementationProviderCapability, InitializeResult, OneOf,
    PositionEncodingKind, ServerCapabilities, SignatureHelpOptions, TextDocumentSyncCapability,
    TextDocumentSyncKind, TypeDefinitionProviderCapability,
};

use crate::backend::{LspBackend, ServerId};
use crate::codec::{Message, ResponseError};
use crate::transport::InboundEvent;

/// Deterministic test double: scripted responses, no process, no threads.
pub struct InlineLspBackend {
    /// method -> FIFO of canned results; a request pops one and enqueues
    /// the Response event for the next drain.
    responses: HashMap<String, VecDeque<Result<serde_json::Value, ResponseError>>>,
    /// Everything the editor sent, for assertions.
    pub sent: Vec<(ServerId, Message)>,
    queue: VecDeque<(ServerId, InboundEvent)>,
    next: u32,
}

impl InlineLspBackend {
    pub fn new() -> Self {
        Self {
            responses: HashMap::new(),
            sent: Vec::new(),
            queue: VecDeque::new(),
            next: 0,
        }
    }

    pub fn respond_to(&mut self, method: &str, result: serde_json::Value) {
        self.responses
            .entry(method.to_string())
            .or_default()
            .push_back(Ok(result));
    }

    pub fn fail_with(&mut self, method: &str, code: i64, msg: &str) {
        self.responses
            .entry(method.to_string())
            .or_default()
            .push_back(Err(ResponseError {
                code,
                message: msg.to_string(),
                data: None,
            }));
    }

    /// Server-initiated traffic (publishDiagnostics, server->client requests).
    pub fn push_from_server(&mut self, server: ServerId, msg: Message) {
        self.queue.push_back((server, InboundEvent::Message(msg)));
    }

    /// Canned successful `initialize` result with the standard v1
    /// `ServerCapabilities`; tests override single capabilities as needed by
    /// calling `respond_to("initialize", ...)` again before the request fires.
    pub fn with_default_handshake() -> Self {
        let mut backend = Self::new();
        backend.respond_to("initialize", default_initialize_result());
        backend
    }
}

impl Default for InlineLspBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl LspBackend for InlineLspBackend {
    fn start(&mut self, _cmd: &str, _args: &[String], _root: &Path) -> std::io::Result<ServerId> {
        let id = ServerId(self.next);
        self.next += 1;
        Ok(id)
    }

    fn send(&mut self, server: ServerId, msg: Message) {
        // Delivered on the *next* drain, never inline here — callers depend
        // on the drain boundary, same discipline as InlineParseBackend.
        if let Message::Request { id, method, .. } = &msg
            && let Some(q) = self.responses.get_mut(method)
            && let Some(result) = q.pop_front()
        {
            self.queue.push_back((
                server,
                InboundEvent::Message(Message::Response {
                    id: id.clone(),
                    result,
                }),
            ));
        }
        self.sent.push((server, msg));
    }

    fn drain(&mut self) -> Vec<(ServerId, InboundEvent)> {
        self.queue.drain(..).collect()
    }

    fn has_pending(&self) -> bool {
        !self.queue.is_empty()
    }

    fn shutdown(&mut self, _server: ServerId) {}
}

fn default_initialize_result() -> serde_json::Value {
    let caps = ServerCapabilities {
        position_encoding: Some(PositionEncodingKind::UTF8),
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::INCREMENTAL,
        )),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        completion_provider: Some(CompletionOptions::default()),
        definition_provider: Some(OneOf::Left(true)),
        declaration_provider: Some(DeclarationCapability::Simple(true)),
        type_definition_provider: Some(TypeDefinitionProviderCapability::Simple(true)),
        implementation_provider: Some(ImplementationProviderCapability::Simple(true)),
        references_provider: Some(OneOf::Left(true)),
        rename_provider: Some(OneOf::Left(true)),
        signature_help_provider: Some(SignatureHelpOptions::default()),
        document_formatting_provider: Some(OneOf::Left(true)),
        document_range_formatting_provider: Some(OneOf::Left(true)),
        code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
        inlay_hint_provider: Some(OneOf::Left(true)),
        ..Default::default()
    };
    let result = InitializeResult {
        capabilities: caps,
        ..Default::default()
    };
    serde_json::to_value(result).expect("ServerCapabilities always serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::RequestId;

    #[test]
    fn respond_to_delivers_on_next_drain_not_inline() {
        let mut backend = InlineLspBackend::new();
        backend.respond_to("textDocument/hover", serde_json::json!({"contents": "hi"}));
        let sid = backend.start("x", &[], Path::new(".")).unwrap();

        backend.send(
            sid,
            Message::Request {
                id: RequestId::Int(1),
                method: "textDocument/hover".to_string(),
                params: serde_json::Value::Null,
            },
        );
        // `send` never returns the response directly — it only becomes
        // observable through a later `drain()` call, matching the discipline
        // callers depend on (they never get answers synchronously).
        assert!(backend.has_pending());

        let events = backend.drain();
        assert_eq!(events.len(), 1);
        match &events[0] {
            (id, InboundEvent::Message(Message::Response { id: rid, result })) => {
                assert_eq!(*id, sid);
                assert_eq!(*rid, RequestId::Int(1));
                assert_eq!(
                    result.clone().unwrap(),
                    serde_json::json!({"contents": "hi"})
                );
            }
            _ => panic!("expected a Response event"),
        }
        // Drained once — second drain is empty.
        assert!(backend.drain().is_empty());
        assert!(!backend.has_pending());
    }

    #[test]
    fn fail_with_delivers_error_response() {
        let mut backend = InlineLspBackend::new();
        backend.fail_with("textDocument/definition", -32601, "not found");
        let sid = backend.start("x", &[], Path::new(".")).unwrap();
        backend.send(
            sid,
            Message::Request {
                id: RequestId::Int(2),
                method: "textDocument/definition".to_string(),
                params: serde_json::Value::Null,
            },
        );
        let events = backend.drain();
        match &events[0] {
            (_, InboundEvent::Message(Message::Response { result, .. })) => {
                let err = result.clone().unwrap_err();
                assert_eq!(err.code, -32601);
                assert_eq!(err.message, "not found");
            }
            _ => panic!("expected a Response event"),
        }
    }

    #[test]
    fn responses_are_fifo_per_method() {
        let mut backend = InlineLspBackend::new();
        backend.respond_to("m", serde_json::json!(1));
        backend.respond_to("m", serde_json::json!(2));
        let sid = backend.start("x", &[], Path::new(".")).unwrap();

        for i in 1..=2 {
            backend.send(
                sid,
                Message::Request {
                    id: RequestId::Int(i),
                    method: "m".to_string(),
                    params: serde_json::Value::Null,
                },
            );
        }
        let events = backend.drain();
        assert_eq!(events.len(), 2);
        match &events[0] {
            (_, InboundEvent::Message(Message::Response { result, .. })) => {
                assert_eq!(result.clone().unwrap(), serde_json::json!(1));
            }
            _ => panic!("expected Response"),
        }
        match &events[1] {
            (_, InboundEvent::Message(Message::Response { result, .. })) => {
                assert_eq!(result.clone().unwrap(), serde_json::json!(2));
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn request_with_no_canned_response_is_recorded_but_produces_no_event() {
        let mut backend = InlineLspBackend::new();
        let sid = backend.start("x", &[], Path::new(".")).unwrap();
        backend.send(
            sid,
            Message::Request {
                id: RequestId::Int(1),
                method: "unscripted".to_string(),
                params: serde_json::Value::Null,
            },
        );
        assert!(backend.drain().is_empty());
        assert_eq!(backend.sent.len(), 1);
    }

    #[test]
    fn push_from_server_surfaces_on_drain() {
        let mut backend = InlineLspBackend::new();
        let sid = backend.start("x", &[], Path::new(".")).unwrap();
        backend.push_from_server(
            sid,
            Message::Notification {
                method: "textDocument/publishDiagnostics".to_string(),
                params: serde_json::json!({"uri": "file:///a", "diagnostics": []}),
            },
        );
        let events = backend.drain();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, sid);
    }

    #[test]
    fn with_default_handshake_answers_initialize() {
        let mut backend = InlineLspBackend::with_default_handshake();
        let sid = backend.start("x", &[], Path::new(".")).unwrap();
        backend.send(
            sid,
            Message::Request {
                id: RequestId::Int(1),
                method: "initialize".to_string(),
                params: serde_json::Value::Null,
            },
        );
        let events = backend.drain();
        match &events[0] {
            (_, InboundEvent::Message(Message::Response { result, .. })) => {
                let parsed: InitializeResult =
                    serde_json::from_value(result.clone().unwrap()).unwrap();
                assert_eq!(
                    parsed.capabilities.position_encoding,
                    Some(PositionEncodingKind::UTF8)
                );
            }
            _ => panic!("expected Response"),
        }
    }
}
