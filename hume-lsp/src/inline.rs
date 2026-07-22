//! Deterministic scripted double for [`LspBackend`]: no process, no threads.
//! The workhorse for every editor/Steel test — the LSP analog
//! of `hume-treesitter`'s `InlineParseBackend`.

use std::collections::VecDeque;
use std::path::Path;

use rustc_hash::FxHashMap;

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
    responses: FxHashMap<String, VecDeque<Result<serde_json::Value, ResponseError>>>,
    /// Everything the editor sent, for assertions.
    pub sent: Vec<(ServerId, Message)>,
    queue: VecDeque<(ServerId, InboundEvent)>,
    next: u32,
}

impl InlineLspBackend {
    pub fn new() -> Self {
        Self {
            responses: FxHashMap::default(),
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

    /// Any undrained event? Test-only introspection — not part of
    /// `LspBackend` (production has no cheap way to peek an `mpsc::Receiver`
    /// without consuming it; wake-up in production is arrival-driven via
    /// `WakeCallback`, not this kind of poll).
    pub fn has_pending(&self) -> bool {
        !self.queue.is_empty()
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
mod tests;
