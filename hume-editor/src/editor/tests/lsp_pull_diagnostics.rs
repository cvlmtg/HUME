// Pull diagnostics: a server that has never pushed is asked for its
// diagnostics with `textDocument/diagnostic`, on attach, after edits and
// saves, and when it asks for a refresh. A server that pushes is never
// pulled.

use super::lsp_rig::{LspRig, RigSpec};
use super::*;
use hume_lsp::test_util::RecordingLspBackend;

const PULL: &str = "textDocument/diagnostic";

/// `initialize` capabilities of a server that offers pull diagnostics.
fn pull_capabilities() -> serde_json::Value {
    serde_json::json!({
        "capabilities": {
            "textDocumentSync": 2,
            "positionEncoding": "utf-8",
            "diagnosticProvider": {
                "interFileDependencies": false,
                "workspaceDiagnostics": false,
            },
        }
    })
}

fn wire_diagnostic(message: &str) -> serde_json::Value {
    serde_json::json!({
        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 4}},
        "severity": 1,
        "message": message,
    })
}

fn full_report(result_id: &str, messages: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "kind": "full",
        "resultId": result_id,
        "items": messages.iter().map(|m| wire_diagnostic(m)).collect::<Vec<_>>(),
    })
}

fn settle(ed: &mut Editor) {
    ed.settle();
    ed.drain_lsp();
    ed.settle();
}

/// The messages stored for the rig's buffer, in buffer order.
fn stored_messages(rig: &LspRig) -> Vec<String> {
    crate::editor::lsp::introspect::diagnostics_for_buffer(&rig.ed.state, rig.bid, None, None)
        .unwrap()
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn server_that_never_pushed_is_pulled_on_attach() {
    let tmp = safe_tempdir();
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to("initialize", pull_capabilities());
    backend.respond_to(PULL, full_report("r1", &["from pull"]));
    let mut rig = LspRig::drained(tmp.path(), RigSpec::rust("-[w]>ord\n"), backend);
    settle(&mut rig.ed);
    let sid = rig.sid("rust-analyzer");

    assert_eq!(
        rig.requests_to(sid, PULL),
        [serde_json::json!({"textDocument": {"uri": rig.uri()}})]
    );
    assert_eq!(stored_messages(&rig), ["from pull"]);
}

fn type_insert(ed: &mut Editor, text: &str) {
    ed.feed_key(key('i'));
    type_chars(ed, text);
    ed.feed_key(key_esc());
}

#[test]
fn an_edit_pulls_again_with_the_previous_result_id() {
    let tmp = safe_tempdir();
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to("initialize", pull_capabilities());
    backend.respond_to(PULL, full_report("r1", &["first"]));
    backend.respond_to(PULL, full_report("r2", &["second"]));
    let mut rig = LspRig::drained(tmp.path(), RigSpec::rust("-[w]>ord\n"), backend);
    settle(&mut rig.ed);
    let sid = rig.sid("rust-analyzer");
    rig.ed.state.settings.lsp_diagnostics_pull_debounce_ms = 0;

    type_insert(&mut rig.ed, "x");
    settle(&mut rig.ed);

    let uri = rig.uri();
    assert_eq!(
        rig.requests_to(sid, PULL),
        [
            serde_json::json!({"textDocument": {"uri": uri}}),
            serde_json::json!({"textDocument": {"uri": uri}, "previousResultId": "r1"}),
        ]
    );
    assert_eq!(stored_messages(&rig), ["second"]);
}

#[test]
fn a_save_pulls_without_waiting_for_the_edit_debounce() {
    let tmp = safe_tempdir();
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to("initialize", pull_capabilities());
    backend.respond_to(PULL, full_report("r1", &["first"]));
    backend.respond_to(PULL, full_report("r2", &["second"]));
    let mut rig = LspRig::drained(tmp.path(), RigSpec::rust("-[w]>ord\n"), backend);
    settle(&mut rig.ed);
    let sid = rig.sid("rust-analyzer");
    rig.ed.state.settings.lsp_diagnostics_pull_debounce_ms = 3_600_000;

    type_insert(&mut rig.ed, "x");
    rig.ed.execute_typed("w", None).unwrap();
    settle(&mut rig.ed);

    assert_eq!(rig.requests_to(sid, PULL).len(), 2);
    assert_eq!(stored_messages(&rig), ["second"]);
}

#[test]
fn a_refresh_request_is_answered_and_pulls_again() {
    let tmp = safe_tempdir();
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to("initialize", pull_capabilities());
    backend.respond_to(PULL, full_report("r1", &["first"]));
    backend.respond_to(PULL, full_report("r2", &["second"]));
    let mut rig = LspRig::drained(tmp.path(), RigSpec::rust("-[w]>ord\n"), backend);
    settle(&mut rig.ed);
    let sid = rig.sid("rust-analyzer");

    rig.push(
        sid,
        hume_lsp::codec::Message::Request {
            id: hume_lsp::codec::RequestId::Int(9),
            method: "workspace/diagnostic/refresh".to_string(),
            params: serde_json::Value::Null,
        },
    );
    settle(&mut rig.ed);

    assert_eq!(
        rig.responses.borrow().as_slice(),
        [(
            sid,
            hume_lsp::codec::RequestId::Int(9),
            Ok(serde_json::Value::Null)
        )]
    );
    assert_eq!(
        rig.requests_to(sid, PULL)[1]["previousResultId"],
        serde_json::json!("r1")
    );
    assert_eq!(stored_messages(&rig), ["second"]);
}

fn excluding(feature: &str) -> String {
    format!(
        r#"(%define-language! "rust" '("rs") '() '() #f '("Cargo.toml"))
(register-lsp-server! "rust-analyzer" #:command "rust-analyzer")
(set-language-servers! "rust" (list (hash 'name "rust-analyzer" 'except-features '({feature}))))"#
    )
}

/// The rig's buffer, attached to a server that answers `initialize` with
/// `capabilities` and its first pull with a full report of `from pull`.
fn attached_to(tmp: &tempfile::TempDir, init: &str, capabilities: serde_json::Value) -> LspRig {
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to("initialize", capabilities);
    backend.respond_to(PULL, full_report("r1", &["from pull"]));
    let mut rig = LspRig::drained(
        tmp.path(),
        RigSpec::rust("-[w]>ord\n").with_init(init),
        backend,
    );
    settle(&mut rig.ed);
    rig
}

#[test]
fn a_server_without_a_diagnostic_provider_is_not_pulled() {
    let tmp = safe_tempdir();
    let rig = attached_to(
        &tmp,
        super::lsp_rig::RUST_ANALYZER,
        serde_json::json!({"capabilities": {"textDocumentSync": 2}}),
    );

    assert!(rig.requests_to(rig.sid("rust-analyzer"), PULL).is_empty());
}

#[test]
fn excluding_pull_diagnostics_sends_no_pull() {
    let tmp = safe_tempdir();
    let rig = attached_to(&tmp, &excluding("pull-diagnostics"), pull_capabilities());

    assert!(rig.requests_to(rig.sid("rust-analyzer"), PULL).is_empty());
}

#[test]
fn excluding_diagnostics_sends_no_pull() {
    let tmp = safe_tempdir();
    let rig = attached_to(&tmp, &excluding("diagnostics"), pull_capabilities());

    assert!(rig.requests_to(rig.sid("rust-analyzer"), PULL).is_empty());
    assert!(stored_messages(&rig).is_empty());
}

#[test]
fn restoring_a_pull_only_servers_diagnostics_pulls_again() {
    let tmp = safe_tempdir();
    let mut rig = attached_to(&tmp, &excluding("diagnostics"), pull_capabilities());
    let sid = rig.sid("rust-analyzer");

    rig.eval(r#"(set-language-servers! "rust" '("rust-analyzer"))"#);
    settle(&mut rig.ed);

    assert_eq!(rig.requests_to(sid, PULL).len(), 1);
    assert_eq!(stored_messages(&rig), ["from pull"]);
}

#[test]
fn a_pull_after_a_filter_change_carries_no_previous_result_id() {
    let tmp = safe_tempdir();
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to("initialize", pull_capabilities());
    backend.respond_to(PULL, full_report("r1", &["first"]));
    backend.respond_to(PULL, full_report("r2", &["first"]));
    let mut rig = LspRig::drained(tmp.path(), RigSpec::rust("-[w]>ord\n"), backend);
    settle(&mut rig.ed);
    let sid = rig.sid("rust-analyzer");

    rig.eval(&excluding("diagnostics"));
    settle(&mut rig.ed);
    assert!(stored_messages(&rig).is_empty());
    rig.eval(r#"(set-language-servers! "rust" '("rust-analyzer"))"#);
    settle(&mut rig.ed);

    let pulls = rig.requests_to(sid, PULL);
    assert_eq!(pulls.len(), 2);
    assert_eq!(pulls[1].get("previousResultId"), None);
    assert_eq!(stored_messages(&rig), ["first"]);
}

#[test]
fn a_save_right_after_typing_pulls_once() {
    let tmp = safe_tempdir();
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to("initialize", pull_capabilities());
    backend.respond_to(PULL, full_report("r1", &["first"]));
    backend.respond_to(PULL, full_report("r2", &["second"]));
    backend.respond_to(PULL, full_report("r3", &["third"]));
    let mut rig = LspRig::drained(tmp.path(), RigSpec::rust("-[w]>ord\n"), backend);
    settle(&mut rig.ed);
    let sid = rig.sid("rust-analyzer");
    rig.ed.state.settings.lsp_diagnostics_pull_debounce_ms = 0;

    type_insert(&mut rig.ed, "x");
    rig.ed.execute_typed("w", None).unwrap();
    settle(&mut rig.ed);

    assert_eq!(rig.requests_to(sid, PULL).len(), 2);
}

#[test]
fn a_save_with_no_edit_since_the_last_pull_pulls_again() {
    let tmp = safe_tempdir();
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to("initialize", pull_capabilities());
    backend.respond_to(PULL, full_report("r1", &["first"]));
    backend.respond_to(PULL, full_report("r2", &["second"]));
    let mut rig = LspRig::drained(tmp.path(), RigSpec::rust("-[w]>ord\n"), backend);
    settle(&mut rig.ed);
    let sid = rig.sid("rust-analyzer");

    rig.ed.execute_typed("w", None).unwrap();
    settle(&mut rig.ed);

    assert_eq!(rig.requests_to(sid, PULL).len(), 2);
    assert_eq!(stored_messages(&rig), ["second"]);
}

#[test]
fn a_buffer_whose_servers_all_pushed_arms_no_pull_timer() {
    let tmp = safe_tempdir();
    let mut rig = attached_to(&tmp, super::lsp_rig::RUST_ANALYZER, pull_capabilities());
    let sid = rig.sid("rust-analyzer");
    rig.publish(sid, serde_json::json!([wire_diagnostic("pushed")]));
    rig.ed.state.settings.lsp_diagnostics_pull_debounce_ms = 3_600_000;

    type_insert(&mut rig.ed, "x");
    settle(&mut rig.ed);

    assert!(rig.ed.diagnostic_pull_debounce.is_empty());
}

#[test]
fn a_server_that_pushed_is_not_pulled_again() {
    let tmp = safe_tempdir();
    let mut rig = attached_to(&tmp, super::lsp_rig::RUST_ANALYZER, pull_capabilities());
    let sid = rig.sid("rust-analyzer");
    rig.publish(sid, serde_json::json!([wire_diagnostic("pushed")]));
    rig.ed.state.settings.lsp_diagnostics_pull_debounce_ms = 0;

    type_insert(&mut rig.ed, "x");
    rig.ed.execute_typed("w", None).unwrap();
    settle(&mut rig.ed);

    assert_eq!(rig.requests_to(sid, PULL).len(), 1);
}

#[test]
fn a_pull_answer_that_lands_after_a_push_is_dropped() {
    let tmp = safe_tempdir();
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to("initialize", pull_capabilities());
    backend.respond_to(PULL, full_report("r1", &["first"]));
    backend.respond_to(PULL, full_report("r2", &["late pull"]));
    let mut rig = LspRig::drained(tmp.path(), RigSpec::rust("-[w]>ord\n"), backend);
    settle(&mut rig.ed);
    let sid = rig.sid("rust-analyzer");

    rig.push(
        sid,
        hume_lsp::codec::Message::Request {
            id: hume_lsp::codec::RequestId::Int(9),
            method: "workspace/diagnostic/refresh".to_string(),
            params: serde_json::Value::Null,
        },
    );
    rig.publish(sid, serde_json::json!([wire_diagnostic("pushed")]));
    settle(&mut rig.ed);

    assert_eq!(rig.requests_to(sid, PULL).len(), 2);
    assert_eq!(stored_messages(&rig), ["pushed"]);
}

#[test]
fn an_unchanged_report_keeps_what_is_stored_and_moves_the_result_id() {
    let tmp = safe_tempdir();
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to("initialize", pull_capabilities());
    backend.respond_to(PULL, full_report("r1", &["first"]));
    backend.respond_to(
        PULL,
        serde_json::json!({"kind": "unchanged", "resultId": "r2"}),
    );
    backend.respond_to(PULL, full_report("r3", &["third"]));
    let mut rig = LspRig::drained(tmp.path(), RigSpec::rust("-[w]>ord\n"), backend);
    settle(&mut rig.ed);
    let sid = rig.sid("rust-analyzer");
    rig.ed.state.settings.lsp_diagnostics_pull_debounce_ms = 0;

    type_insert(&mut rig.ed, "x");
    settle(&mut rig.ed);
    assert_eq!(stored_messages(&rig), ["first"]);

    type_insert(&mut rig.ed, "y");
    settle(&mut rig.ed);
    assert_eq!(
        rig.requests_to(sid, PULL)[2]["previousResultId"],
        serde_json::json!("r2")
    );
    assert_eq!(stored_messages(&rig), ["third"]);
}
