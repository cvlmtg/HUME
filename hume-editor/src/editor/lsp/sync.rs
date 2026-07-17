//! Document sync: mirrors buffer text to attached LSP servers via
//! `textDocument/didOpen` / `didChange` / `didSave` / `didClose`. Pure
//! protocol — zero Steel involvement. Version = `Buffer.text_gen`, no
//! second counter.

use hume_editing::changeset::ChangeSet;
use hume_engine::pipeline::BufferId;
use hume_lsp::codec::Message;
use hume_lsp::sync::{changeset_to_content_changes, wire_version};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument,
    Notification as _,
};
use ropey::Rope;

use super::LspState;
use crate::editor::Editor;
use crate::editor::EditorState;
use crate::editor::buffer::Buffer;

/// One text mutation queued for `didChange` conversion. `before` is the
/// pre-edit rope (an O(1) clone via ropey's structural sharing — the same
/// discipline `doc_ops.rs` already uses for `propagate_cs_to_panes`);
/// `version` is the buffer's `text_gen` *after* this edit, the version the
/// eventual `didChange` notification claims.
pub(crate) struct LspPendingChange {
    pub(crate) cs: ChangeSet,
    pub(crate) before: Rope,
    pub(crate) version: u64,
}

impl Editor {
    /// Shared preamble for every per-buffer document-sync notification:
    /// resolves the buffer's attached server and URI, builds `params` from
    /// the buffer, then sends through the client's Starting-queue
    /// discipline. No-op when the buffer has no attached server, no path,
    /// or an unconvertible path.
    fn send_doc_notification(
        &mut self,
        bid: BufferId,
        method: &str,
        build_params: impl FnOnce(&Buffer, &lsp_types::Uri) -> serde_json::Value,
    ) {
        let buf = self.state.buffers.get(bid);
        let Some(server_id) = buf.lsp_server else {
            return;
        };
        let Some(path) = buf.path() else {
            return;
        };
        let Ok(uri) = hume_lsp::uri::path_to_uri(path) else {
            return;
        };
        let params = build_params(buf, &uri);
        let Some((client, backend)) = self.lsp.client_and_backend(server_id) else {
            return;
        };
        client.send_or_queue(
            backend,
            Message::Notification {
                method: method.to_string(),
                params,
            },
        );
    }

    /// Sends the buffer's full text as `textDocument/didOpen`. Called once,
    /// right after `lsp_attach_buffer` sets `Buffer.lsp_server` — the buffer
    /// is guaranteed to have a path at that point (unnamed buffers never
    /// attach). Queued instead of sent if the handshake hasn't completed —
    /// the spec forbids anything but `initialize` before `initialized`.
    pub(super) fn lsp_did_open(&mut self, bid: BufferId) {
        let language_id = self
            .state
            .buffers
            .get(bid)
            .language
            .map(|id| self.state.languages.name_of(id).to_owned())
            .expect("attached buffer always has a language");
        self.send_doc_notification(bid, DidOpenTextDocument::METHOD, move |buf, uri| {
            serde_json::json!({
                "textDocument": {
                    "uri": uri.as_str(),
                    "languageId": language_id,
                    "version": wire_version(buf.text_gen),
                    "text": buf.text().to_string(),
                }
            })
        });
    }

    /// `textDocument/didSave` — never includes text (`didSave.includeText`
    /// is never advertised in the handshake). Queued while `Starting`,
    /// same as every other send site here.
    pub(in crate::editor) fn lsp_did_save(&mut self, bid: BufferId) {
        self.send_doc_notification(
            bid,
            DidSaveTextDocument::METHOD,
            |_buf, uri| serde_json::json!({ "textDocument": { "uri": uri.as_str() } }),
        );
    }

    /// `textDocument/didClose`. Must run *before* the buffer slot is freed
    /// (`Editor::close_buffer` calls this first) — it needs the buffer's
    /// path and `lsp_server` to build the notification. Queued while
    /// `Starting`, same as every other send site here — a queued didClose
    /// flushes after a queued didOpen, in order, so the pair stays coherent
    /// even if a buffer opens and closes before the handshake completes.
    pub(in crate::editor) fn lsp_did_close(&mut self, bid: BufferId) {
        self.send_doc_notification(
            bid,
            DidCloseTextDocument::METHOD,
            |_buf, uri| serde_json::json!({ "textDocument": { "uri": uri.as_str() } }),
        );
    }

    /// Whole-document `didChange` (no `range`, legal per spec) for reload
    /// paths that replace the text outright (`:e!`) rather than applying a
    /// `ChangeSet` — `Buffer::reload_from_text` computes a line-diff CS for
    /// *undo*, but the wire message here is simplest as a full-text sync.
    /// Queued while `Starting`, same as every other send site here.
    pub(in crate::editor) fn lsp_did_change_whole_document(&mut self, bid: BufferId) {
        self.send_doc_notification(bid, DidChangeTextDocument::METHOD, |buf, uri| {
            serde_json::json!({
                "textDocument": { "uri": uri.as_str(), "version": wire_version(buf.text_gen) },
                "contentChanges": [{ "text": buf.text().to_string() }],
            })
        });
    }

    /// Converts and sends every pending change recorded since the last
    /// flush, one `didChange` notification per entry, in order — draining
    /// `Buffer.lsp_pending`. Called from the LSP per-frame drain
    /// (`drain_lsp`), before the diagnostics remap consumes the same entries
    /// for diagnostics (same source, both consumers — the entries aren't
    /// cleared until every consumer of this drain pass has run).
    ///
    /// `lsp_pending` isn't LSP-exclusive: `record_lsp_edits` (`doc_ops.rs`)
    /// also queues entries for a buffer with no attached server but with
    /// char-offset decorations (`set-inlay-hints!`/`set-extra-highlights!`)
    /// that still need to track edits. This drains and remaps *every*
    /// buffer with pending entries; sending a `didChange` is the one part
    /// gated on actually having a server, path, and URI to send it to —
    /// missing any of those skips the send but never skips the remap, so a
    /// decoration-only buffer's positions don't silently drift.
    pub(in crate::editor) fn flush_lsp_pending_changes(&mut self) {
        flush_lsp_pending_changes(&mut self.state, &mut self.lsp);
    }
}

/// Free-function body of [`Editor::flush_lsp_pending_changes`] — operates on
/// disjoint `state`/`lsp` borrows only (no other `Editor` field), so it's
/// also callable from `EditorHostImpl` (completion's accept path needs this
/// same flush, synchronously, before sending a `completionItem/resolve`
/// request — see `completion::CompletionSession::accept`).
pub(crate) fn flush_lsp_pending_changes(state: &mut EditorState, lsp: &mut LspState) {
    let with_pending: Vec<BufferId> = state
        .buffers
        .iter()
        .filter(|(_, buf)| !buf.lsp_pending.is_empty())
        .map(|(id, _)| id)
        .collect();

    for bid in with_pending {
        let buf = state.buffers.get(bid);
        // Resolved *before* taking the queue below, from the buffer's
        // state at this instant — a missing server/path/URI just means
        // there's nothing to send, not that the entries should be
        // dropped unremapped.
        let send_target = buf
            .lsp_server
            .and_then(|sid| Some((sid, hume_lsp::uri::path_to_uri(buf.path()?).ok()?)));

        let buf = state.buffers.get_mut(bid);
        let pending = std::mem::take(&mut buf.lsp_pending);

        // Manual field split: the loop below interleaves a diagnostics
        // remap with a client-routed send, so `servers`/`backend`/
        // `diagnostics` all need to be borrowed independently out of
        // `lsp` rather than through method calls on the whole struct.
        let LspState {
            servers,
            backend,
            diagnostics,
            ..
        } = lsp;

        for change in pending {
            // Same source as the didChange conversion below — remap
            // stored diagnostics through the identical ChangeSet before
            // it's consumed, so both consumers see the exact
            // same edit stream, including undo/redo. The char-offset
            // decoration stores (inlay hints, extra highlights) go
            // through the same chokepoint for the same reason — done
            // unconditionally, whether or not this buffer has anywhere
            // to send a didChange.
            diagnostics.remap_through(bid, &change.cs);
            state.decorations.remap_through(bid, &change.cs);

            let Some((server_id, uri)) = &send_target else {
                continue; // no attached server (or no path/URI yet) — nothing to send
            };
            let Some(client) = servers.get_mut(server_id).map(|e| &mut e.client) else {
                continue; // can't happen once attached, but never send into the void
            };
            let events =
                changeset_to_content_changes(&change.before, &change.cs, client.encoding());
            if events.is_empty() {
                continue;
            }
            let params = serde_json::json!({
                "textDocument": { "uri": uri.as_str(), "version": wire_version(change.version) },
                "contentChanges": events,
            });
            client.send_or_queue(
                backend.as_mut(),
                Message::Notification {
                    method: DidChangeTextDocument::METHOD.to_string(),
                    params,
                },
            );
        }
    }
}
