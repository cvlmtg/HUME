//! Pull diagnostics: `textDocument/diagnostic` to a server that does not
//! push. A server is pulled only until its first `publishDiagnostics`, only
//! while its list entry admits `diagnostics` and `pull-diagnostics`, and
//! once per text version unless a save or a refresh asks again. The
//! client does not declare pull support, because a server that offers both
//! then stops pushing; whether a server pushes is observed, not read from
//! its capabilities.

use hume_engine::pipeline::BufferId;
use hume_lsp::backend::ServerId;
use hume_lsp::client::parse_wire_diagnostics;
use hume_scripting::{CapabilityQuery, LspFeature};

use super::bridge::RustResponder;
use super::introspect::provider_of;
use crate::editor::event::EditorEvent;
use crate::editor::{EditorState, Severity};

const PULL: &str = "textDocument/diagnostic";

impl EditorState {
    /// Whether `sid` is a server to pull `bid`'s diagnostics from: running,
    /// advertising `diagnosticProvider`, not yet pushed, and its list entry
    /// admits both `diagnostics` and `pull-diagnostics`.
    fn lsp_pull_candidate(&self, bid: BufferId, sid: ServerId) -> bool {
        let docs = &self.buffer_positions.lsp;
        docs.admits(bid, sid, LspFeature::Diagnostics)
            && !self.lsp.instances.has_pushed_diagnostics(sid)
            && self.lsp_handles(bid, sid, LspFeature::PullDiagnostics)
    }

    /// Whether any server attached to `bid` is a pull candidate.
    pub(in crate::editor) fn lsp_buffer_wants_pull(&self, bid: BufferId) -> bool {
        self.buffer_positions
            .lsp
            .servers(bid)
            .any(|sid| self.lsp_pull_candidate(bid, sid))
    }

    /// Pulls diagnostics from each of `bid`'s servers that has not already
    /// been asked about its current text.
    pub(in crate::editor) fn lsp_pull_diagnostics(&mut self, bid: BufferId) {
        let servers: Vec<ServerId> = self.buffer_positions.lsp.servers(bid).collect();
        for sid in servers {
            self.lsp_pull_diagnostics_from(bid, sid);
        }
    }

    /// Pulls from `sid` again even if it was asked about this text: a save
    /// or a server's refresh request can change what it reports.
    pub(in crate::editor) fn lsp_repull_diagnostics(&mut self, bid: BufferId, sid: ServerId) {
        self.buffer_positions.lsp.set_pulled_at(bid, sid, None);
        self.lsp_pull_diagnostics_from(bid, sid);
    }

    /// [`Self::lsp_repull_diagnostics`] for each of `bid`'s servers.
    pub(in crate::editor) fn lsp_repull_diagnostics_all(&mut self, bid: BufferId) {
        let servers: Vec<ServerId> = self.buffer_positions.lsp.servers(bid).collect();
        for sid in servers {
            self.lsp_repull_diagnostics(bid, sid);
        }
    }

    /// Pulls from `sid` unless it is not a candidate or was already asked
    /// about `bid`'s current text.
    pub(in crate::editor) fn lsp_pull_diagnostics_from(&mut self, bid: BufferId, sid: ServerId) {
        if !self.lsp_pull_candidate(bid, sid) {
            return;
        }
        let version = self.buffers.get(bid).text().version();
        if self.buffer_positions.lsp.pulled_at(bid, sid) == Some(version) {
            return;
        }
        self.buffer_positions
            .lsp
            .set_pulled_at(bid, sid, Some(version));
        let Some(opened) = self.buffer_positions.lsp.opened_as(bid) else {
            return;
        };
        let mut params = serde_json::json!({ "textDocument": { "uri": opened.uri } });
        let identifier = provider_of(
            &self.lsp,
            sid,
            CapabilityQuery::Feature(LspFeature::PullDiagnostics),
        )
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
        self.lsp_request_json(bid, sid, PULL, params, responder);
    }

    /// Stores a diagnostics report: a full one replaces what `sid` reported
    /// for `bid`, an unchanged one only moves the `resultId` on. A report
    /// that arrives after `sid` has pushed or lost its `diagnostics` entry is
    /// dropped, and a
    /// failed or malformed one leaves what is stored as it is.
    fn lsp_apply_pull_report(
        &mut self,
        bid: BufferId,
        sid: ServerId,
        answer: Result<serde_json::Value, String>,
    ) {
        if !self.lsp_pull_candidate(bid, sid) {
            return;
        }
        let name = self.lsp_server_name(sid);
        let mut report = match answer {
            Ok(report) => report,
            Err(error) => {
                self.buffer_positions.lsp.set_pulled_at(bid, sid, None);
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
