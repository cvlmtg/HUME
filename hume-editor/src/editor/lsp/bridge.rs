//! The generic LSP bridge: sends `(lsp-request …)` / `(lsp-notify …)` calls
//! Steel queued this eval, one at a time as `Editor::apply_script_effects`
//! encounters each `Effect::LspRequest`/`Effect::LspNotify` in emission
//! order, resolving `bid`'s attached server and (for requests) wiring the
//! callback to fire through the queued-Steel-call mechanism once a
//! response, error, or timeout arrives.

use std::time::{Duration, Instant};

use hume_lsp::client::{Outcome, RequestMeta};
use hume_scripting::{PendingLspNotify, PendingLspRequest};
use steel::rvals::SteelVal;

use super::{Editor, ResponseAnchor};
use crate::editor::message_log::Severity;

impl Editor {
    /// Sends one queued `(lsp-request …)` call. Called from
    /// `Editor::apply_script_effects` for each `Effect::LspRequest`, in
    /// emission order, after `flush_lsp_pending_changes` so a request
    /// minted against text just edited doesn't reach the wire ahead of the
    /// `didChange` describing that edit.
    pub(in crate::editor) fn send_one_lsp_request(&mut self, req: PendingLspRequest) {
        let server_id =
            match super::introspect::resolve_server_for_buffer(&self.state, &self.lsp, req.bid) {
                Ok(id) => id,
                Err(e) => {
                    self.report(Severity::Error, format!("lsp-request: {e}"));
                    self.fail_lsp_request_callback(req.callback, &e);
                    return;
                }
            };
        // `resolve_server_for_buffer` above already proved `req.bid` live
        // (its own `try_get` is where a stale bid would have been caught).
        // Nothing between the two calls can close it, so this is a plain
        // read, not a second liveness check.
        let text_gen = self.state.buffers.get(req.bid).text_gen;
        let timeout_ms = self.state.settings.lsp_request_timeout_ms as u64;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);

        // `#:supersede`: cancel the caller's own previous still-pending
        // request filed under the same `(server, key)`, if any. Silent:
        // the superseding caller has replaced that request's purpose, so
        // firing its stale callback would deliver a result nobody wants and
        // race the new one; removing the callback (not just cancelling) is
        // what guarantees it never fires even if the response already
        // landed in the client's `completed` queue (in which case `cancel`
        // itself is a no-op: no spurious `$/cancelRequest` follows a
        // response that already arrived).
        if let Some(key) = &req.supersede
            && let Some(old_id) = self.lsp.supersede.remove(&(server_id, key.clone()))
        {
            self.lsp.callbacks.remove(&(server_id, old_id.clone()));
            if let Some((client, backend)) = self.lsp.client_and_backend(server_id) {
                client.cancel(backend, old_id);
            }
        }

        let anchor = ResponseAnchor {
            bid: req.bid,
            text_gen,
            allow_stale: req.allow_stale,
            require_focus: req.require_focus,
        };
        // Cloned (SteelVal is Rc-based, cheap): the send-failure branch
        // below needs its own copy of the callback to fire immediately,
        // since the success-path closure already moved one in. `anchor` is
        // `Copy`, so the closure gets its own copy to carry into the queued
        // `PendingWork::Call` (re-checked at dequeue time, see that variant's
        // doc), `register_callback` below still gets the original.
        let callback_for_send = req.callback.clone();
        let lsp_callback: super::LspCallback = Box::new(move |editor, server_id, outcome| {
            let (err, result) = outcome_to_steel(editor, server_id, outcome);
            editor
                .state
                .queue_steel_call_anchored(callback_for_send, vec![err, result], anchor);
        });
        let meta = RequestMeta {
            method: req.method.clone(),
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
            .register_callback(server_id, id, anchor, lsp_callback);
    }

    /// Fires an `(lsp-request …)` callback immediately with an error,
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
    /// only failure mode.
    pub(in crate::editor) fn send_one_lsp_notify(&mut self, notif: PendingLspNotify) {
        let server_id =
            match super::introspect::resolve_server_for_buffer(&self.state, &self.lsp, notif.bid) {
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

/// `Outcome` → the `(err result)` pair delivered to a Steel callback:
/// exactly one of the two is non-`#f`. A successful `value` always crosses
/// through `to_steel_handle`'s three-way split: a container becomes an
/// opaque `JsonHandle` (read with
/// `json-ref`/`json-contains?`/`json-list`), a scalar crosses natively, and
/// `null` is `Void`, so every existing `(void? res)` check (a server
/// declining with no completions) stays meaningful. See `JsonHandle`'s own
/// doc (`hume-scripting/src/json.rs`) for the full rationale.
///
/// Tags a successful `value`'s handle with `server_id`'s *current*
/// negotiated encoding, read at dispatch time rather than carried from send
/// time: a `Starting` server (queued, not yet negotiated, when the request
/// went out) has negotiated by the time its response drains. `Err`s if the
/// server is no longer tracked (crashed or stopped between sending and
/// draining): the caller has no client-side generation to check this
/// against, unlike the buffer-side staleness `ResponseAnchor` already
/// covers, so a value whose encoding can no longer be resolved is refused
/// rather than guessed.
fn outcome_to_steel(
    editor: &Editor,
    server_id: hume_lsp::backend::ServerId,
    outcome: Outcome,
) -> (SteelVal, SteelVal) {
    match outcome {
        Outcome::Ok(value) => match super::introspect::server_encoding(&editor.lsp, server_id) {
            Some(encoding) => (
                SteelVal::BoolV(false),
                hume_scripting::json::to_steel_handle(
                    std::sync::Arc::new(value),
                    hume_scripting::json::WireOrigin::Server(encoding),
                ),
            ),
            None => (
                SteelVal::StringV("lsp server no longer tracked".into()),
                SteelVal::BoolV(false),
            ),
        },
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
