//! The generic LSP bridge: sends `(lsp-request …)` / `(lsp-notify …)` calls
//! Steel queued this eval, one at a time as `Editor::apply_script_effects`
//! encounters each `Effect::LspRequest`/`Effect::LspNotify` in emission
//! order, resolving `server` and (for requests) wiring the callback to fire
//! through the queued-Steel-call mechanism once a response, error, or
//! timeout arrives.

use std::time::{Duration, Instant};

use hume_engine::pipeline::BufferId;
use hume_lsp::backend::ServerId;
use hume_lsp::client::{Outcome, RequestMeta};
use hume_scripting::json::json_to_steel;
use hume_scripting::{PendingLspNotify, PendingLspRequest};
use steel::rvals::SteelVal;

use super::Editor;
use crate::editor::message_log::Severity;

impl Editor {
    /// Resolves `server` — a registered language name, or `None` for "the
    /// focused buffer's attached server" — to a running `ServerId`. Shared
    /// with the introspection builtins via `super::introspect::resolve_server`.
    fn resolve_lsp_server(&self, server: Option<&str>) -> Result<ServerId, String> {
        let bid = self.focused_buffer_id();
        super::introspect::resolve_server(&self.state, &self.lsp, bid, server)
    }

    /// If `params` carries a `textDocument.uri` matching an open buffer,
    /// tags the request with that buffer's current `text_gen` — the
    /// staleness check drops the response if the buffer has moved on by the
    /// time it lands (unless the caller passed `#:allow-stale`).
    fn stale_check_for_params(&self, params: &serde_json::Value) -> Option<(BufferId, u64)> {
        let uri_str = params.get("textDocument")?.get("uri")?.as_str()?;
        let uri: lsp_types::Uri = uri_str.parse().ok()?;
        let path = hume_lsp::uri::uri_to_path(&uri).ok()?;
        let canonical = path.canonicalize().ok()?;
        let bid = self.state.buffers.find_by_path(&canonical)?;
        Some((bid, self.state.buffers.get(bid).text_gen))
    }

    /// Sends one queued `(lsp-request …)` call. Called from
    /// `Editor::apply_script_effects` for each `Effect::LspRequest`, in
    /// emission order — after `flush_lsp_pending_changes` so a request
    /// minted against text just edited doesn't reach the wire ahead of the
    /// `didChange` describing that edit.
    pub(in crate::editor) fn send_one_lsp_request(&mut self, req: PendingLspRequest) {
        let server_id = match self.resolve_lsp_server(req.server.as_deref()) {
            Ok(id) => id,
            Err(e) => {
                self.report(Severity::Error, format!("lsp-request: {e}"));
                self.fail_lsp_request_callback(req.callback, &e);
                return;
            }
        };
        let stale_check = self.stale_check_for_params(&req.params);
        let timeout_ms = self.state.settings.lsp_request_timeout_ms as u64;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);

        // `#:supersede`: cancel the caller's own previous still-pending
        // request filed under the same `(server, key)`, if any. Silent —
        // the superseding caller has replaced that request's purpose, so
        // firing its stale callback would deliver a result nobody wants and
        // race the new one; removing the callback (not just cancelling) is
        // what guarantees it never fires even if the response already
        // landed in the client's `completed` queue (in which case `cancel`
        // itself is a no-op — no spurious `$/cancelRequest` follows a
        // response that already arrived).
        if let Some(key) = &req.supersede
            && let Some(old_id) = self.lsp.supersede.remove(&(server_id, key.clone()))
        {
            self.lsp.callbacks.remove(&(server_id, old_id.clone()));
            if let Some((client, backend)) = self.lsp.client_and_backend(server_id) {
                client.cancel(backend, old_id);
            }
        }

        // Cloned (SteelVal is Rc-based, cheap): the send-failure branch
        // below needs its own copy of the callback to fire immediately,
        // since the success-path closure already moved one in.
        let callback_for_send = req.callback.clone();
        let lsp_callback: super::LspCallback = Box::new(move |editor, outcome| {
            let (err, result) = outcome_to_steel(outcome);
            editor
                .state
                .queue_steel_call(callback_for_send, vec![err, result]);
        });
        let meta = RequestMeta {
            method: req.method.clone(),
            allow_stale: req.allow_stale,
            deadline,
        };
        // Send first, register second: `register_callback` keys off the id
        // `send_request` mints, so there is no window where a callback is
        // filed without a matching in-flight request to eventually resolve it.
        let Some(id) = self
            .lsp
            .send_request(server_id, &req.method, req.params, meta)
        else {
            let msg = format!("no client tracked for the server sending '{}'", req.method);
            self.report(Severity::Error, format!("lsp-request: {msg}"));
            self.fail_lsp_request_callback(req.callback, &msg);
            return;
        };
        if let Some(key) = req.supersede {
            self.lsp.supersede.insert((server_id, key), id.clone());
        }
        self.lsp
            .register_callback(server_id, id, stale_check, lsp_callback);
    }

    /// Fires an `(lsp-request …)` callback immediately with an error —
    /// used when resolution or the send itself fails before any
    /// request/response pair could ever exist, so the callback would
    /// otherwise never fire at all. Keeps the documented `(err result)`
    /// contract (exactly one non-`#f`) true even on this early-failure path.
    fn fail_lsp_request_callback(&mut self, callback: SteelVal, message: &str) {
        self.state.queue_steel_call(
            callback,
            vec![
                SteelVal::StringV(message.to_string().into()),
                SteelVal::BoolV(false),
            ],
        );
    }

    /// Sends one queued `(lsp-notify …)` call. Same server resolution as
    /// `send_one_lsp_request`; no callback, so a resolution error is the
    /// only failure mode. Called from `Editor::apply_script_effects` for
    /// each `Effect::LspNotify`.
    pub(in crate::editor) fn send_one_lsp_notify(&mut self, notif: PendingLspNotify) {
        let server_id = match self.resolve_lsp_server(notif.server.as_deref()) {
            Ok(id) => id,
            Err(e) => {
                self.report(Severity::Error, format!("lsp-notify: {e}"));
                return;
            }
        };
        let Some((client, backend)) = self.lsp.client_and_backend(server_id) else {
            return;
        };
        client.send_or_queue(
            backend,
            hume_lsp::codec::Message::Notification {
                method: notif.method,
                params: notif.params,
            },
        );
    }
}

/// `Outcome` → the `(err result)` pair delivered to a Steel callback —
/// exactly one of the two is non-`#f`.
fn outcome_to_steel(outcome: Outcome) -> (SteelVal, SteelVal) {
    match outcome {
        Outcome::Ok(value) => (SteelVal::BoolV(false), json_to_steel(&value)),
        Outcome::Err(e) => {
            let mut map = steel::HashMap::new();
            map.insert(
                SteelVal::StringV("code".into()),
                SteelVal::IntV(e.code as isize),
            );
            map.insert(
                SteelVal::StringV("message".into()),
                SteelVal::StringV(e.message.into()),
            );
            let err = SteelVal::HashMapV(steel::gc::Gc::new(map).into());
            (err, SteelVal::BoolV(false))
        }
        Outcome::TimedOut => (SteelVal::StringV("timeout".into()), SteelVal::BoolV(false)),
    }
}
