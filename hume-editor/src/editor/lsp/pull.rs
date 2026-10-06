//! Pull diagnostics: `textDocument/diagnostic` to a server that does not
//! push. A server is pulled only until its first `publishDiagnostics`. The
//! client does not declare pull support, because a server that offers both
//! then stops pushing; whether a server pushes is observed, not read from
//! its capabilities.

use hume_engine::pipeline::BufferId;
use hume_lsp::backend::ServerId;
use hume_lsp::client::parse_wire_diagnostics;
use hume_scripting::LspFeature;

use super::bridge::RustResponder;
use super::features::provider;
use crate::editor::event::EditorEvent;
use crate::editor::{EditorState, Severity};

const PULL: &str = "textDocument/diagnostic";

impl EditorState {
    /// Asks each server attached to `bid` that has not pushed (only `only`,
    /// when given) for its diagnostics. A server that is not running, or
    /// whose list entry excludes `pull-diagnostics`, or that advertises no
    /// `diagnosticProvider`, is not asked.
    pub(in crate::editor) fn lsp_pull_diagnostics(
        &mut self,
        bid: BufferId,
        only: Option<ServerId>,
    ) {
        let servers: Vec<ServerId> = self
            .buffer_positions
            .lsp
            .servers(bid)
            .filter(|&sid| only.is_none_or(|only| only == sid))
            .collect();
        for sid in servers {
            self.lsp_pull_diagnostics_from(bid, sid);
        }
    }

    fn lsp_pull_diagnostics_from(&mut self, bid: BufferId, sid: ServerId) {
        if self.lsp.instances.has_pushed_diagnostics(sid) {
            return;
        }
        let Some(opened) = self.buffer_positions.lsp.opened_as(bid) else {
            return;
        };
        let mut params = serde_json::json!({ "textDocument": { "uri": opened.uri } });
        let identifier = self
            .lsp
            .instances
            .get(sid)
            .and_then(|instance| instance.client.capabilities_json())
            .and_then(|caps| provider(caps, Some(LspFeature::PullDiagnostics), None))
            .and_then(|options| options.get("identifier"))
            .cloned();
        if let Some(identifier) = identifier {
            params["identifier"] = identifier;
        }
        if let Some(previous) = self.buffer_positions.lsp.pull_result_id(bid, sid) {
            params["previousResultId"] = previous.into();
        }
        let responder: RustResponder = Box::new(move |state, _view, answer| {
            state.lsp_apply_pull_report(bid, sid, answer);
        });
        self.lsp_request_json(bid, sid, PULL, params, false, responder);
    }

    /// Stores a diagnostics report: a full one replaces what `sid` reported
    /// for `bid`, an unchanged one only moves the `resultId` on. A report
    /// that arrives after `sid` has pushed is dropped, and a failed or
    /// malformed one leaves what is stored as it is.
    fn lsp_apply_pull_report(
        &mut self,
        bid: BufferId,
        sid: ServerId,
        answer: Result<serde_json::Value, String>,
    ) {
        if self.lsp.instances.has_pushed_diagnostics(sid) {
            return;
        }
        let name = self.lsp_server_name(sid);
        let mut report = match answer {
            Ok(report) => report,
            Err(error) => {
                self.report(
                    Severity::Trace,
                    format!("lsp: '{name}' failed a diagnostics pull: {error}"),
                );
                return;
            }
        };
        let result_id = report
            .get("resultId")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        match report.get("kind").and_then(serde_json::Value::as_str) {
            Some("unchanged") => self
                .buffer_positions
                .lsp
                .set_pull_result_id(bid, sid, result_id),
            Some("full") => {
                let Some(items) = report
                    .get_mut("items")
                    .and_then(serde_json::Value::as_array_mut)
                else {
                    self.report(
                        Severity::Trace,
                        format!("lsp: '{name}' sent a diagnostics report without items"),
                    );
                    return;
                };
                let Some((diagnostics, skipped)) = parse_wire_diagnostics(items) else {
                    self.report(
                        Severity::Trace,
                        format!(
                            "lsp: '{name}' sent a diagnostics report none of whose items parse"
                        ),
                    );
                    return;
                };
                self.note_skipped_diagnostics(sid, &skipped);
                if self.store_server_diagnostics(sid, bid, diagnostics) {
                    self.buffer_positions
                        .lsp
                        .set_pull_result_id(bid, sid, result_id);
                    self.queue_event(EditorEvent::OnDiagnosticsChanged { buffer: bid });
                }
            }
            _ => self.report(
                Severity::Trace,
                format!("lsp: '{name}' sent a diagnostics report of an unknown kind"),
            ),
        }
    }
}
