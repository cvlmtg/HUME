// Observability + lifecycle commands:
// :lsp-status, :lsp-stop, :lsp-restart, and stderr/log-message routing.

use super::lsp_rig::{LspRig, RigSpec};
use super::*;
use hume_lsp::client::{ClientAction, ServerState};
use hume_lsp::test_util::RecordingLspBackend;
use hume_scripting::{LspServerTarget, PendingLspServerOp, ServerName};

fn rust_rig(tmp: &tempfile::TempDir) -> LspRig {
    LspRig::rust(
        tmp.path(),
        "-[f]>n main() {}\n",
        serde_json::json!({ "capabilities": {} }),
    )
}

fn stop(ed: &mut Editor, target: LspServerTarget) {
    ed.apply_lsp_server_op(PendingLspServerOp::Stop { target });
}

fn restart(ed: &mut Editor, target: LspServerTarget) {
    ed.apply_lsp_server_op(PendingLspServerOp::Restart { target });
}

fn name(s: &str) -> ServerName {
    ServerName::parse(s).unwrap()
}

#[test]
fn status_text_lists_a_running_server_with_root_and_pending_count() {
    let tmp = safe_tempdir();
    let rig = rust_rig(&tmp);

    let text = rig.ed.lsp_status_text();
    assert!(
        text.contains("rust-analyzer [rust] @"),
        "must show the name and languages: {text:?}"
    );
    assert!(
        text.contains(&rig.root.display().to_string()),
        "must show the root: {text:?}"
    );
    assert!(
        text.contains("running"),
        "must show the lifecycle state, spelled the same as lsp-server-status's 'running: {text:?}"
    );
    assert!(
        text.contains("0 in flight"),
        "must show the pending count: {text:?}"
    );
    assert!(
        text.contains("main.rs [rust-analyzer]: 0 error(s), 0 warning(s)"),
        "must list the attached buffer with its servers: {text:?}"
    );
}

#[test]
fn status_text_reports_no_servers_when_none_are_running() {
    let ed = editor_from("-[w]>ord\n");
    assert_eq!(ed.lsp_status_text(), "No LSP servers running.");
}

#[test]
fn status_text_lists_a_stopped_server_until_it_is_restarted() {
    let tmp = safe_tempdir();
    let mut rig = rust_rig(&tmp);
    assert!(
        !rig.ed.lsp_status_text().contains("Stopped"),
        "a running server is not listed as stopped"
    );

    stop(&mut rig.ed, LspServerTarget::Name(name("rust-analyzer")));

    let text = rig.ed.lsp_status_text();
    assert!(text.contains("No LSP servers running."), "{text:?}");
    assert!(text.contains("Stopped"), "{text:?}");
    assert!(
        text.contains(&format!("  rust-analyzer @ {}", rig.root.display())),
        "must name the stopped server and its root: {text:?}"
    );

    restart(&mut rig.ed, LspServerTarget::Name(name("rust-analyzer")));

    let text = rig.ed.lsp_status_text();
    assert!(!text.contains("Stopped"), "{text:?}");
}

#[test]
fn lsp_stop_by_name_detaches_the_buffer_and_stops_the_server() {
    let tmp = safe_tempdir();
    let mut rig = rust_rig(&tmp);
    let sid = rig.sid("rust-analyzer");

    stop(&mut rig.ed, LspServerTarget::Name(name("rust-analyzer")));

    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 0);
    assert!(rig.attached().is_empty(), "the buffer must be detached");
    assert_eq!(rig.sent(sid, "textDocument/didClose").len(), 1);
    assert_eq!(
        rig.ed.state.status_msg.as_deref(),
        Some("lsp: stopped 1 server(s)")
    );
}

/// `(lsp-stop! pane)` targets its buffer's servers regardless of which
/// buffer is focused: `LspServerTarget` names the buffer explicitly.
#[test]
fn lsp_stop_targets_the_named_buffer_regardless_of_focus() {
    let tmp = safe_tempdir();
    let mut rig = rust_rig(&tmp);
    let other = rig
        .ed
        .open_buffer(Buffer::at_start(BufferText::from("other\n")));
    let fp = FocusedPane::current(&rig.ed.state);
    rig.ed.switch_to_buffer_without_jump(fp, other);
    assert_ne!(rig.ed.focused_buffer_id(), rig.bid);

    stop(&mut rig.ed, LspServerTarget::Buffer(rig.bid));

    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 0);
    assert!(rig.attached().is_empty());
}

#[test]
fn lsp_stop_with_no_matching_server_stops_nothing() {
    let tmp = safe_tempdir();
    let mut rig = rust_rig(&tmp);
    let scratch = rig
        .ed
        .open_buffer(Buffer::at_start(BufferText::from("x\n")));

    stop(&mut rig.ed, LspServerTarget::Buffer(scratch));

    assert_eq!(
        rig.ed.state.status_msg.as_deref(),
        Some("lsp: no matching server to stop")
    );
    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 1);
}

#[test]
fn stopping_an_unknown_name_reports_an_error() {
    let tmp = safe_tempdir();
    let mut rig = rust_rig(&tmp);

    stop(&mut rig.ed, LspServerTarget::Name(name("no-such-server")));

    let log = rig.ed.state.message_log.format_for_display();
    assert!(
        log.contains("[error] lsp: no server named 'no-such-server'"),
        "{log:?}"
    );
    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 1);
}

/// An edit queued before the stop goes out before `didClose`, so nothing
/// is left queued for a server that attaches later: the fresh server
/// starts from `didOpen`'s text alone.
#[test]
fn lsp_stop_leaves_no_queued_change_for_a_later_attach() {
    let tmp = safe_tempdir();
    let (mut backend, _, _) = RecordingLspBackend::with_default_handshake();
    backend.respond_to(
        "initialize",
        serde_json::json!({ "capabilities": { "textDocumentSync": 2 } }),
    );
    let mut rig = LspRig::drained(tmp.path(), RigSpec::rust("-[f]>n main() {}\n"), backend);
    let old = rig.sid("rust-analyzer");
    type_chars(&mut rig.ed, "iX");
    rig.ed.feed_key(key_esc());

    stop(&mut rig.ed, LspServerTarget::Name(name("rust-analyzer")));
    assert_eq!(rig.sent(old, "textDocument/didChange").len(), 1);
    assert!(!rig.ed.state.buffer_positions.lsp.has_doc(rig.bid));

    restart(&mut rig.ed, LspServerTarget::Name(name("rust-analyzer")));
    rig.ed.drain_lsp();
    let new = rig.sid("rust-analyzer");
    assert_ne!(old, new);
    assert!(rig.sent(new, "textDocument/didChange").is_empty());
    let opened = rig.sent(new, "textDocument/didOpen");
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0]["textDocument"]["text"], "Xfn main() {}\n");
}

#[test]
fn lsp_restart_spawns_a_fresh_server_id_and_reattaches_the_buffer() {
    let tmp = safe_tempdir();
    let mut rig = rust_rig(&tmp);
    let old_sid = rig.sid("rust-analyzer");

    restart(&mut rig.ed, LspServerTarget::Name(name("rust-analyzer")));

    let new_sid = rig.sid("rust-analyzer");
    assert_ne!(
        old_sid, new_sid,
        "restart must yield a fresh ServerId, not reuse the old one"
    );
    assert_eq!(rig.attached(), vec![new_sid]);
    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 1);
    assert_eq!(
        rig.ed.state.status_msg.as_deref(),
        Some("lsp: restarted 1 server(s)")
    );
}

/// A restarted server's fresh `ServerId` must not coexist with the old
/// server's diagnostics for the same buffer: the detach dropped them.
#[test]
fn lsp_restart_does_not_duplicate_diagnostics_after_a_republish() {
    let tmp = safe_tempdir();
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to("initialize", serde_json::json!({ "capabilities": {} }));
    backend.respond_to("initialize", serde_json::json!({ "capabilities": {} }));
    let mut rig = LspRig::drained(tmp.path(), RigSpec::rust("-[f]>n main() {}\n"), backend);
    let old_sid = rig.sid("rust-analyzer");
    let diagnostic = serde_json::json!([{
        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 2}},
        "severity": 1,
        "message": "boom",
    }]);
    rig.publish(old_sid, diagnostic.clone());
    assert_eq!(rig.ed.diagnostic_counts(rig.bid), (1, 0));

    restart(&mut rig.ed, LspServerTarget::Name(name("rust-analyzer")));
    rig.ed.drain_lsp();
    let new_sid = rig.sid("rust-analyzer");
    assert_ne!(old_sid, new_sid, "restart must yield a fresh ServerId");
    rig.publish(new_sid, diagnostic);

    assert_eq!(
        rig.ed.diagnostic_counts(rig.bid),
        (1, 0),
        "the old server's diagnostics must not coexist with the new server's republish"
    );
}

#[test]
fn stderr_action_is_logged_at_trace_with_the_server_name_prefix() {
    let tmp = safe_tempdir();
    let rig = rust_rig(&tmp);
    let sid = rig.sid("rust-analyzer");
    let mut ed = rig.ed;

    ed.dispatch_lsp_action(sid, ClientAction::Stderr("panic: oh no".to_string()));

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("[trace] rust-analyzer: panic: oh no"),
        "expected a Trace-prefixed stderr line, got: {log:?}"
    );
}

#[test]
fn log_message_error_type_is_reported_at_error_severity() {
    let tmp = safe_tempdir();
    let rig = rust_rig(&tmp);
    let sid = rig.sid("rust-analyzer");
    let mut ed = rig.ed;

    ed.dispatch_lsp_action(
        sid,
        ClientAction::LogMessage(
            serde_json::from_value(serde_json::json!({"type": 1, "message": "something broke"}))
                .unwrap(),
        ),
    );

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("[error] rust-analyzer: something broke"),
        "type=1 (Error) must be reported at Error severity, got: {log:?}"
    );
}

#[test]
fn log_message_info_type_is_reported_at_trace_not_shown_as_status() {
    let tmp = safe_tempdir();
    let rig = rust_rig(&tmp);
    let sid = rig.sid("rust-analyzer");
    let mut ed = rig.ed;

    ed.dispatch_lsp_action(
        sid,
        ClientAction::LogMessage(
            serde_json::from_value(serde_json::json!({"type": 3, "message": "indexing"})).unwrap(),
        ),
    );

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("[trace] rust-analyzer: indexing"),
        "type=3 (Info) must be demoted to Trace, not shown, got: {log:?}"
    );
}

#[test]
fn show_message_is_reported_at_info_severity() {
    let tmp = safe_tempdir();
    let rig = rust_rig(&tmp);
    let sid = rig.sid("rust-analyzer");
    let mut ed = rig.ed;

    ed.dispatch_lsp_action(
        sid,
        ClientAction::ShowMessage(
            serde_json::from_value(serde_json::json!({"type": 3, "message": "ready"})).unwrap(),
        ),
    );

    // Info severity is never pushed to the persistent log (see
    // Editor::report); it is only shown as the transient status message.
    assert_eq!(ed.state.status_msg.as_deref(), Some("rust-analyzer: ready"));
}

#[test]
fn progress_report_events_are_dropped_without_any_log_line() {
    let tmp = safe_tempdir();
    let rig = rust_rig(&tmp);
    let sid = rig.sid("rust-analyzer");
    let mut ed = rig.ed;
    let before = ed.state.message_log.entries().count();

    ed.dispatch_lsp_action(
        sid,
        ClientAction::Progress(
            serde_json::from_value(
                serde_json::json!({"token": "1", "value": {"kind": "begin", "title": "indexing"}}),
            )
            .unwrap(),
        ),
    );
    ed.dispatch_lsp_action(
        sid,
        ClientAction::Progress(
            serde_json::from_value(
                serde_json::json!({"token": "1", "value": {"kind": "report", "percentage": 50}}),
            )
            .unwrap(),
        ),
    );
    ed.dispatch_lsp_action(
        sid,
        ClientAction::Progress(
            serde_json::from_value(serde_json::json!({"token": "1", "value": {"kind": "end"}}))
                .unwrap(),
        ),
    );

    let entries: Vec<_> = ed.state.message_log.entries().skip(before).collect();
    assert_eq!(
        entries.len(),
        2,
        "begin and end must log once each; report must never log (got: {entries:?})"
    );
}

// ── Graceful shutdown on quit ──────────────────────────────────────────────

#[test]
fn lsp_shutdown_all_transitions_every_running_client_to_dead() {
    let tmp = safe_tempdir();
    let rig = rust_rig(&tmp);
    let sid = rig.sid("rust-analyzer");
    let mut ed = rig.ed;

    // Duration::ZERO means the grace-window loop's `Instant::now() < deadline`
    // is false on first check: no sleep, no waiting for a real process.
    ed.lsp_shutdown_all(std::time::Duration::ZERO);

    let client =
        ed.state.lsp.client_for_test(sid).expect(
            "the client stays tracked (only its state changes): a stop is what deregisters",
        );
    assert_eq!(client.state(), ServerState::Dead);
}

#[test]
fn lsp_shutdown_all_on_a_starting_client_skips_the_protocol_but_still_tears_down() {
    // A client that never completed its handshake must not receive
    // shutdown/exit (nothing but `initialize` is legal before `initialized`),
    // but it must still not be left dangling forever; the transport-level
    // `backend.shutdown` call covers it regardless of protocol state.
    let tmp = safe_tempdir();
    let rig = LspRig::open(
        tmp.path(),
        RigSpec::rust("-[f]>n main() {}\n"),
        RecordingLspBackend::new().0,
    );
    let sid = rig.sid("rust-analyzer");
    let mut ed = rig.ed;

    ed.lsp_shutdown_all(std::time::Duration::ZERO);

    let client = ed
        .state
        .lsp
        .client_for_test(sid)
        .expect("still tracked, but untouched by begin_shutdown");
    assert_eq!(
        client.state(),
        ServerState::Starting,
        "a Starting client's state must not change: it never got the shutdown/exit messages"
    );
}

#[test]
fn lsp_shutdown_all_with_no_clients_returns_immediately() {
    let mut ed = editor_from("-[w]>ord\n");
    let start = std::time::Instant::now();
    ed.lsp_shutdown_all(std::time::Duration::from_secs(5));
    assert!(
        start.elapsed() < std::time::Duration::from_millis(50),
        "no clients means nothing to wait for: must not block on the grace window"
    );
}
