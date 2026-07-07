//! B2's generic LSP bridge: flushes `(lsp-request …)` / `(lsp-notify …)`
//! calls Steel queued this eval, resolving `server` and (for requests)
//! wiring the callback to fire through the queued-Steel-call mechanism once
//! a response, error, or timeout arrives.

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
    /// Sends every `(lsp-request …)` / `(lsp-notify …)` call an eval just
    /// queued. Shared tail for `call_steel_cmd`'s call site, `drain_hooks`,
    /// and `drain_pending_steel_calls` — the three places a `SteelCmdResult`
    /// or `HookResult` comes back with these two fields to dispatch.
    pub(in crate::editor) fn flush_pending_lsp_calls(
        &mut self,
        requests: Vec<PendingLspRequest>,
        notifies: Vec<PendingLspNotify>,
    ) {
        self.flush_pending_lsp_requests(requests);
        self.flush_pending_lsp_notifies(notifies);
    }

    /// Resolves `server` — a registered language name, or `None` for "the
    /// focused buffer's attached server" — to a running `ServerId`.
    ///
    /// A bare language name is ambiguous when multiple workspace roots for
    /// that language are running at once (the store is keyed by (language,
    /// root), not language alone): this prefers the focused buffer's own
    /// server if it matches, and otherwise errors rather than guessing.
    fn resolve_lsp_server(&self, server: Option<&str>) -> Result<ServerId, String> {
        let focused_server = || {
            let bid = self.focused_buffer_id();
            self.state.buffers.get(bid).lsp_server
        };
        match server {
            None => focused_server()
                .ok_or_else(|| "no LSP server attached to the current buffer".to_string()),
            Some(name) => {
                let matches: Vec<ServerId> = self
                    .lsp
                    .servers_by_key
                    .iter()
                    .filter(|((lang, _), _)| lang == name)
                    .map(|(_, &sid)| sid)
                    .collect();
                match matches.as_slice() {
                    [] => Err(format!("no running LSP server for language '{name}'")),
                    [sid] => Ok(*sid),
                    _ => focused_server().filter(|sid| matches.contains(sid)).ok_or_else(|| {
                        format!(
                            "multiple '{name}' servers running — pass #f to use the \
                             current buffer's server"
                        )
                    }),
                }
            }
        }
    }

    /// If `params` carries a `textDocument.uri` matching an open buffer,
    /// tags the request with that buffer's current `text_gen` — C6's
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

    /// Sends every queued `(lsp-request …)` call. Errors (unknown/ambiguous
    /// server) are reported and drop that one request — one bad call must
    /// not lose the rest of the batch.
    fn flush_pending_lsp_requests(&mut self, requests: Vec<PendingLspRequest>) {
        for req in requests {
            self.send_one_lsp_request(req);
        }
    }

    fn send_one_lsp_request(&mut self, req: PendingLspRequest) {
        let server_id = match self.resolve_lsp_server(req.server.as_deref()) {
            Ok(id) => id,
            Err(e) => {
                self.report(Severity::Error, format!("lsp-request: {e}"));
                return;
            }
        };
        let stale_check = self.stale_check_for_params(&req.params);
        let timeout_ms = self.state.settings.lsp_request_timeout_ms as u64;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);

        let callback = req.callback;
        let lsp_callback: super::LspCallback = Box::new(move |editor, outcome| {
            let (err, result) = outcome_to_steel(outcome);
            editor.queue_steel_call(callback, vec![err, result]);
        });
        let token = self.lsp.register_callback(stale_check, lsp_callback);
        let meta = RequestMeta {
            method: req.method.clone(),
            allow_stale: req.allow_stale,
            deadline,
            token,
        };
        self.lsp.send_request(server_id, &req.method, req.params, meta);
    }

    /// Sends every queued `(lsp-notify …)` call. Same server resolution as
    /// requests; no callback, so a resolution error is the only failure mode.
    fn flush_pending_lsp_notifies(&mut self, notifies: Vec<PendingLspNotify>) {
        for notif in notifies {
            let server_id = match self.resolve_lsp_server(notif.server.as_deref()) {
                Ok(id) => id,
                Err(e) => {
                    self.report(Severity::Error, format!("lsp-notify: {e}"));
                    continue;
                }
            };
            let Some((client, backend)) = self.lsp.client_and_backend(server_id) else {
                continue;
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
        Outcome::TimedOut => (
            SteelVal::StringV("timeout".into()),
            SteelVal::BoolV(false),
        ),
    }
}
