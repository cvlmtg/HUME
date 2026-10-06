//! Document sync: mirrors buffer text to every attached LSP server via
//! `textDocument/didOpen` / `didChange` / `didSave` / `didClose`. Pure
//! protocol, zero Steel involvement. Version = the buffer text's generation,
//! no second counter.
//!
//! A buffer's queued changes (`LspDocuments`) are taken once per flush and
//! sent to each attached server in the form that server negotiated: ranged
//! events in its position encoding for `INCREMENTAL`, one whole-document
//! event at the current version for `FULL` (which ignores `range` and reads
//! every event's `text` as the whole document), nothing for `NONE`. Before
//! its handshake completes a server is sent `FULL`, so nothing queued while
//! it is `Starting` depends on an encoding not yet negotiated.

use hume_engine::pipeline::BufferId;
use hume_lsp::backend::ServerId;
use hume_lsp::codec::Message;
use hume_lsp::sync::{changeset_to_content_changes, wire_version};
use hume_rope::position_encoding::PositionEncoding;
use lsp_types::TextDocumentSyncKind;
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument,
    Notification as _,
};

use super::document::{OpenedAs, PendingChange};
use crate::editor::EditorState;

impl EditorState {
    /// Sends every buffer's queued changes. Called from the LSP per-frame
    /// drain, before each queued request or notification is sent, and
    /// before `didSave`, so no message a server receives refers to text it
    /// has not been told about.
    pub(in crate::editor) fn lsp_flush_pending(&mut self) {
        for bid in self.buffer_positions.lsp.with_pending() {
            self.lsp_flush_buffer(bid);
        }
    }

    /// Sends `bid`'s queued changes to each of its attached servers.
    pub(in crate::editor) fn lsp_flush_buffer(&mut self, bid: BufferId) {
        let pending = self.buffer_positions.lsp.take_pending(bid);
        if pending.is_empty() {
            return;
        }
        let Some(uri) = self.lsp_opened_uri(bid) else {
            return;
        };
        let servers: Vec<ServerId> = self.buffer_positions.lsp.servers(bid).collect();
        let mut whole_document: Option<serde_json::Value> = None;
        let mut incremental: Vec<(PositionEncoding, Vec<serde_json::Value>)> = Vec::new();
        for sid in servers {
            let Some((client, backend)) = self.lsp.client_and_backend(sid) else {
                continue;
            };
            match client.change_sync() {
                None => {}
                Some(TextDocumentSyncKind::FULL) => {
                    let params = whole_document
                        .get_or_insert_with(|| {
                            let text = self.buffers.get(bid).text();
                            serde_json::json!({
                                "textDocument": {
                                    "uri": uri,
                                    "version": wire_version(text.generation()),
                                },
                                "contentChanges": [{ "text": text.to_string() }],
                            })
                        })
                        .clone();
                    client.send_or_queue(
                        backend,
                        notification(DidChangeTextDocument::METHOD, params),
                    );
                }
                Some(_) => {
                    let encoding = client.encoding();
                    let index = match incremental.iter().position(|(e, _)| *e == encoding) {
                        Some(index) => index,
                        None => {
                            incremental
                                .push((encoding, incremental_changes(&pending, &uri, encoding)));
                            incremental.len() - 1
                        }
                    };
                    for params in &incremental[index].1 {
                        client.send_or_queue(
                            backend,
                            notification(DidChangeTextDocument::METHOD, params.clone()),
                        );
                    }
                }
            }
        }
    }

    /// `textDocument/didSave` to every server attached to `bid`. Never
    /// includes text: `didSave.includeText` is never advertised.
    pub(in crate::editor) fn lsp_did_save(&mut self, bid: BufferId) {
        let Some(uri) = self.lsp_opened_uri(bid) else {
            return;
        };
        let servers: Vec<ServerId> = self.buffer_positions.lsp.servers(bid).collect();
        for sid in servers {
            self.lsp_send_doc_notification(
                sid,
                DidSaveTextDocument::METHOD,
                serde_json::json!({ "textDocument": { "uri": uri } }),
            );
        }
        self.lsp_pull_diagnostics(bid, None);
    }

    /// `textDocument/didOpen` with the full text, to `sid` alone, as
    /// `opened`. Queued while `sid` is `Starting`: the spec forbids
    /// anything but `initialize` before `initialized`.
    pub(in crate::editor::lsp) fn lsp_did_open_to(
        &mut self,
        bid: BufferId,
        sid: ServerId,
        opened: &OpenedAs,
    ) {
        let text = self.buffers.get(bid).text();
        let params = serde_json::json!({
            "textDocument": {
                "uri": opened.uri,
                "languageId": opened.language_id,
                "version": wire_version(text.generation()),
                "text": text.to_string(),
            }
        });
        self.lsp_send_doc_notification(sid, DidOpenTextDocument::METHOD, params);
    }

    /// `textDocument/didClose` to `sid` alone. A crashed or dead server's
    /// client drops it.
    pub(in crate::editor::lsp) fn lsp_did_close_to(&mut self, bid: BufferId, sid: ServerId) {
        let Some(uri) = self.lsp_opened_uri(bid) else {
            return;
        };
        self.lsp_send_doc_notification(
            sid,
            DidCloseTextDocument::METHOD,
            serde_json::json!({ "textDocument": { "uri": uri } }),
        );
    }

    pub(in crate::editor::lsp) fn lsp_send_doc_notification(
        &mut self,
        sid: ServerId,
        method: &str,
        params: serde_json::Value,
    ) {
        if let Some((client, backend)) = self.lsp.client_and_backend(sid) {
            client.send_or_queue(backend, notification(method, params));
        }
    }

    /// The URI `bid`'s servers opened it under, or `None` when it has no
    /// attachment.
    fn lsp_opened_uri(&self, bid: BufferId) -> Option<String> {
        self.buffer_positions
            .lsp
            .opened_as(bid)
            .map(|opened| opened.uri.clone())
    }

    /// `bid`'s document URI from its current path, or `None` for a buffer
    /// with no path or one whose path does not convert.
    pub(in crate::editor::lsp) fn lsp_doc_uri(&self, bid: BufferId) -> Option<lsp_types::Uri> {
        hume_lsp::uri::path_to_uri(self.buffers.try_get(bid)?.path()?).ok()
    }
}

/// One `didChange` params value per change in `pending` that moves text,
/// its ranges in `encoding`.
fn incremental_changes(
    pending: &[PendingChange],
    uri: &str,
    encoding: PositionEncoding,
) -> Vec<serde_json::Value> {
    pending
        .iter()
        .filter_map(|change| {
            let events = changeset_to_content_changes(&change.before, &change.cs, encoding);
            (!events.is_empty()).then(|| {
                serde_json::json!({
                    "textDocument": {
                        "uri": uri,
                        "version": wire_version(change.version),
                    },
                    "contentChanges": events,
                })
            })
        })
        .collect()
}

fn notification(method: &str, params: serde_json::Value) -> Message {
    Message::Notification {
        method: method.to_string(),
        params,
    }
}
