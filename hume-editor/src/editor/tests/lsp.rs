// Request bookkeeping + server->client dispatch,
// exercised through the editor's LspState glue (drain_lsp). Server
// registration tests live at the bottom of this file.

use std::cell::RefCell;
use std::rc::Rc;

use super::lsp_rig::{LspRig, RUST_ANALYZER, RigSpec};
use super::*;
use hume_lsp::backend::ServerId;
use hume_lsp::client::ServerState;
use hume_lsp::test_util::RecordingLspBackend;

/// A `rust-analyzer` server `Running` on `src/main.rs`, its backend
/// scripted by `script` before the server starts. `Running`, not
/// `Starting`: these tests exercise request/response/staleness bookkeeping,
/// not the handshake queue, and a Starting client queues instead of sending.
fn running(
    tmp: &tempfile::TempDir,
    script: impl FnOnce(&mut RecordingLspBackend),
) -> (Editor, ServerId) {
    let (mut backend, _, _) = RecordingLspBackend::with_default_handshake();
    script(&mut backend);
    let rig = LspRig::drained(tmp.path(), RigSpec::rust("-[w]>ord\n"), backend);
    let sid = rig.sid("rust-analyzer");
    (rig.ed, sid)
}

/// What a request sent through `lsp_request_json` has called back with.
type Answers = Rc<RefCell<Vec<Result<serde_json::Value, String>>>>;

/// Sends `method` to `sid` and records every answer its responder gets.
fn request(ed: &mut Editor, sid: ServerId, method: &str, allow_stale: bool) -> Answers {
    let answers: Answers = Rc::default();
    let sink = answers.clone();
    let bid = ed.focused_buffer_id();
    ed.state.lsp_request_json(
        bid,
        sid,
        method,
        serde_json::Value::Null,
        allow_stale,
        Box::new(move |_state, _view, answer| sink.borrow_mut().push(answer)),
    );
    answers
}

#[test]
fn callback_fires_with_ok_outcome_on_response() {
    let tmp = safe_tempdir();
    let (mut ed, sid) = running(&tmp, |b| {
        b.respond_to("textDocument/hover", serde_json::json!({"contents": "hi"}));
    });

    let answers = request(&mut ed, sid, "textDocument/hover", false);

    ed.drain_lsp();

    assert_eq!(
        *answers.borrow(),
        vec![Ok(serde_json::json!({"contents": "hi"}))]
    );
}

#[test]
fn callback_never_fires_for_a_request_with_no_response() {
    let tmp = safe_tempdir();
    let (mut ed, sid) = running(&tmp, |_| {});

    let answers = request(&mut ed, sid, "textDocument/hover", false);

    ed.drain_lsp();

    assert!(
        answers.borrow().is_empty(),
        "no canned response: callback must not fire"
    );
}

#[test]
fn timed_out_request_dispatches_callback_with_a_timeout_error() {
    // A callback that never fires on timeout has no way to notice. The
    // Steel callbacks are `(err result)`-shaped and need this to map a
    // timeout to `err` rather than hanging silently.
    let tmp = safe_tempdir();
    let (mut ed, sid) = running(&tmp, |_| {});
    ed.state.settings.lsp_request_timeout_ms = 0;

    let answers = request(&mut ed, sid, "textDocument/completion", false);

    ed.drain_lsp();

    assert_eq!(
        *answers.borrow(),
        vec![Err("timed out".to_string())],
        "callback must fire once"
    );
}

#[test]
fn stale_response_is_dropped_when_buffer_moved_past_its_text_version() {
    let tmp = safe_tempdir();
    let (mut ed, sid) = running(&tmp, |b| {
        b.respond_to(
            "textDocument/hover",
            serde_json::json!({"contents": "stale"}),
        );
    });

    let answers = request(&mut ed, sid, "textDocument/hover", false);

    // Move the buffer's text version past the value the request was sent at.
    ed.step(key('d'));

    ed.drain_lsp();

    assert!(
        answers.borrow().is_empty(),
        "the buffer moved past the request's text version: the callback must be dropped"
    );
}

#[test]
fn allow_stale_delivers_despite_buffer_moving_past_its_text_version() {
    let tmp = safe_tempdir();
    let (mut ed, sid) = running(&tmp, |b| {
        b.respond_to("textDocument/hover", serde_json::json!({"contents": "ok"}));
    });

    let answers = request(&mut ed, sid, "textDocument/hover", true);

    ed.step(key('d'));
    ed.drain_lsp();

    assert_eq!(
        answers.borrow().len(),
        1,
        "allow_stale opts out of the staleness drop: the callback must still fire"
    );
}

#[test]
fn crashed_action_is_reported_to_the_message_log() {
    // `on_event(Eof)` producing exactly one `Crashed` action (never twice)
    // is covered in hume-lsp's own client tests; this covers the editor
    // glue's side: dispatching that action actually reaches the log.
    let tmp = safe_tempdir();
    let (mut ed, sid) = running(&tmp, |_| {});

    ed.dispatch_lsp_action(
        sid,
        hume_lsp::client::ClientAction::Crashed {
            error: Some("boom".to_string()),
        },
    );

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("lsp: rust-analyzer crashed: boom (:lsp-restart rust-analyzer)"),
        "the crash names its server and how to restart it: {log}"
    );
}

#[test]
fn crash_fails_in_flight_requests_immediately_instead_of_waiting_for_their_deadline() {
    let tmp = safe_tempdir();
    // No response scripted: this request would otherwise sit pending
    // until its (far-future) deadline.
    let (mut ed, sid) = running(&tmp, |_| {});

    let answers = request(&mut ed, sid, "textDocument/hover", false);

    ed.dispatch_lsp_action(
        sid,
        hume_lsp::client::ClientAction::Crashed {
            error: Some("boom".to_string()),
        },
    );

    assert_eq!(
        *answers.borrow(),
        vec![Err("server stopped before answering".to_string())],
        "the pending request must fail immediately on crash"
    );
}

#[test]
fn initialize_timeout_reports_a_crash_through_drain_lsp() {
    // `take_completed`'s sweep producing the `Crashed` action for an expired
    // `initialize` is covered in hume-lsp's own client tests; this covers
    // the editor glue's side: `drain_lsp` actually dispatches that action.
    let tmp = safe_tempdir();
    // No scripted `initialize` response: the server stays `Starting`.
    let rig = LspRig::open(
        tmp.path(),
        RigSpec::rust("-[w]>ord\n"),
        RecordingLspBackend::new().0,
    );
    let sid = rig.sid("rust-analyzer");
    let mut ed = rig.ed;

    ed.state
        .lsp
        .client_for_test(sid)
        .unwrap()
        .expire_pending_deadlines_for_test();

    ed.drain_lsp();

    assert_eq!(
        ed.state.lsp.client_for_test(sid).unwrap().state(),
        ServerState::Crashed
    );
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("crashed") && log.contains("initialize timed out"),
        "expected a crash+timeout log line, got: {log}"
    );
}

#[test]
fn shutdown_error_response_is_logged_at_trace() {
    let tmp = safe_tempdir();
    let (mut ed, sid) = running(&tmp, |b| {
        b.fail_with("shutdown", -32603, "internal error");
    });

    let (client, backend) = ed.state.lsp.client_and_backend(sid).unwrap();
    client.begin_shutdown(backend);

    ed.drain_lsp();

    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("shutdown failed") && log.contains("internal error"),
        "expected a shutdown-failure trace line, got: {log}"
    );
}

#[test]
fn server_request_action_gets_exactly_one_response() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[w]>ord\n",
        serde_json::json!({ "capabilities": {} }),
    );
    let sid = rig.sid("rust-analyzer");

    rig.push(sid, configuration_request(1, &[""]));

    // The dispatch table itself (every method, including MethodNotFound) is
    // unit-tested in `hume_lsp::client::tests` against the pure
    // `server_request_response`; this layer owes one answer.
    let responses = rig.responses.borrow();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].0, sid);
    assert_eq!(responses[0].1, hume_lsp::codec::RequestId::Int(1));
}

fn configuration_request(id: i64, sections: &[&str]) -> hume_lsp::codec::Message {
    let items: Vec<serde_json::Value> = sections
        .iter()
        .map(|section| serde_json::json!({ "section": section }))
        .collect();
    hume_lsp::codec::Message::Request {
        id: hume_lsp::codec::RequestId::Int(id),
        method: "workspace/configuration".to_string(),
        params: serde_json::json!({ "items": items }),
    }
}

#[test]
fn workspace_configuration_resolves_the_attached_servers_registered_settings() {
    // The dispatch-table shape (section resolution, null-per-item) is
    // covered exhaustively in `hume_lsp::client::tests`; what's specific to
    // this layer is the glue: dispatch answers from the *requesting*
    // server's own settings, not some other server's, or none at all.
    let tmp = safe_tempdir();
    let (backend, _, _) = RecordingLspBackend::with_default_handshake();
    let mut rig = LspRig::drained(
        tmp.path(),
        RigSpec::rust("-[w]>ord\n").with_init(
            r#"(register-lsp-server! "rust-analyzer" #:command "rust-analyzer"
                 #:settings (hash "rust-analyzer" (hash "cargo" (hash "features" "all"))))
               (set-language-servers! "rust" '("rust-analyzer"))"#,
        ),
        backend,
    );
    let sid = rig.sid("rust-analyzer");

    rig.push(
        sid,
        configuration_request(7, &["rust-analyzer.cargo.features", "nope"]),
    );

    let responses = rig.responses.borrow();
    assert_eq!(responses.len(), 1, "expected exactly one response sent");
    let (resp_sid, id, result) = &responses[0];
    assert_eq!(*resp_sid, sid);
    assert_eq!(*id, hume_lsp::codec::RequestId::Int(7));
    assert_eq!(result.as_ref().unwrap(), &serde_json::json!(["all", null]));
}

#[test]
fn workspace_configuration_answers_null_when_requesting_server_has_no_registered_settings() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[w]>ord\n",
        serde_json::json!({ "capabilities": {} }),
    );
    let sid = rig.sid("rust-analyzer");

    rig.push(sid, configuration_request(9, &["rust-analyzer"]));

    let responses = rig.responses.borrow();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].2.as_ref().unwrap(), &serde_json::json!([null]));
}

/// Two servers on one buffer, two settings blobs: each answers from its
/// own registration.
#[test]
fn workspace_configuration_resolves_each_servers_own_settings_for_one_language() {
    let tmp = safe_tempdir();
    let (mut backend, _, _) = RecordingLspBackend::new();
    backend.respond_to("initialize", serde_json::json!({ "capabilities": {} }));
    backend.respond_to("initialize", serde_json::json!({ "capabilities": {} }));
    let mut rig = LspRig::drained(
        tmp.path(),
        RigSpec::rust("-[w]>ord\n").with_init(
            r#"(register-lsp-server! "rust-analyzer" #:command "rust-analyzer"
                 #:settings (hash "a" "from-rust-analyzer"))
               (register-lsp-server! "ra-lint" #:command "ra-lint"
                 #:settings (hash "a" "from-ra-lint"))
               (set-language-servers! "rust" '("rust-analyzer" "ra-lint"))"#,
        ),
        backend,
    );
    let lint = rig.sid("ra-lint");

    rig.push(lint, configuration_request(1, &["a"]));

    let responses = rig.responses.borrow();
    assert_eq!(responses.len(), 1, "only ra-lint made a request");
    let (resp_sid, _id, result) = &responses[0];
    assert_eq!(*resp_sid, lint);
    assert_eq!(
        result.as_ref().unwrap(),
        &serde_json::json!(["from-ra-lint"]),
        "must resolve ra-lint's own settings, never rust-analyzer's"
    );
}

#[test]
fn lsp_stop_fails_in_flight_requests_as_stopped_instead_of_orphaning_them() {
    // A stopped server's pending requests complete as stopped rather than
    // leaving their callbacks registered forever.
    let tmp = safe_tempdir();
    let (mut ed, sid) = running(&tmp, |_| {});

    let answers = request(&mut ed, sid, "textDocument/hover", false);

    ed.apply_lsp_server_op(hume_scripting::PendingLspServerOp::Stop {
        target: hume_scripting::LspServerTarget::Name(
            hume_scripting::ServerName::parse("rust-analyzer").unwrap(),
        ),
    });

    assert_eq!(
        *answers.borrow(),
        vec![Err("server stopped before answering".to_string())],
        "callback must fire once on stop"
    );
    assert_eq!(
        ed.state.lsp.callback_count_for_test(),
        0,
        "the callback entry must not leak after being dispatched"
    );
}

#[test]
fn became_running_flushes_queued_messages_through_the_backend() {
    let tmp = safe_tempdir();
    let (mut backend, _, _) = RecordingLspBackend::with_default_handshake();
    backend.respond_to("textDocument/hover", serde_json::json!(null));
    let mut rig = LspRig::open(tmp.path(), RigSpec::rust("-[w]>ord\n"), backend);
    let sid = rig.sid("rust-analyzer");
    assert!(
        rig.sent(sid, "textDocument/didOpen").is_empty(),
        "a Starting server's didOpen is queued, not sent"
    );

    rig.ed.drain_lsp();

    assert_eq!(rig.sent(sid, "textDocument/didOpen").len(), 1);
    assert_eq!(
        rig.ed.state.lsp.client_for_test(sid).unwrap().state(),
        ServerState::Running
    );
}

// ── Server registration ────────────────────────────────────────────────

fn rig_without_servers(tmp: &tempfile::TempDir) -> LspRig {
    let (backend, _, _) = RecordingLspBackend::with_default_handshake();
    LspRig::drained(
        tmp.path(),
        RigSpec::rust("-[f]>n main() {}\n").with_init(""),
        backend,
    )
}

#[test]
fn second_registration_replaces_first() {
    // Last-wins per name: a second register-lsp-server! for an
    // already-registered name replaces the config rather than being
    // rejected, matching define-language!'s semantics. No error is logged.
    let tmp = safe_tempdir();
    let mut rig = rig_without_servers(&tmp);

    rig.eval(RUST_ANALYZER);
    rig.eval(r#"(register-lsp-server! "rust-analyzer" #:command "rust-analyzer-2")"#);

    let log = rig.ed.state.message_log.format_for_display();
    assert!(
        !log.contains("already registered") && !log.contains("[error]"),
        "second registration must not be rejected as a duplicate, got log: {log}"
    );
    assert_eq!(
        rig.ed
            .state
            .lsp
            .registered_command_for_test("rust-analyzer")
            .as_deref(),
        Some("rust-analyzer-2"),
        "second registration must win"
    );
}

/// `#:env` decodes from a list of `("KEY" . "VALUE")` dotted pairs all the
/// way into the registration, the wire shape `steel-server/plugin.scm`
/// uses for `STEEL_LSP_HOME`. Complements the Steel-boundary decode test in
/// `hume-scripting`'s `builtins::lsp::tests::decodes_env_dotted_pairs`
/// (which stops at `PendingLspServerReg`) and the real-process delivery
/// test in `hume-lsp`'s `transport::tests::unix` (which stops at
/// `ServerHandle::spawn`). This one is the middle link, the editor-level
/// apply path.
#[test]
fn env_round_trips_into_lsp_server_config() {
    let tmp = safe_tempdir();
    let mut rig = rig_without_servers(&tmp);

    rig.eval(
        r#"(register-lsp-server! "rust-analyzer" #:command "rust-analyzer"
             #:env (list (cons "FOO" "bar") (cons "STEEL_LSP_HOME" "/tmp/lsp-home")))"#,
    );

    assert_eq!(
        rig.ed.state.lsp.registered_env_for_test("rust-analyzer"),
        Some(vec![
            ("FOO".to_string(), "bar".to_string()),
            ("STEEL_LSP_HOME".to_string(), "/tmp/lsp-home".to_string()),
        ])
    );
}

#[test]
fn runtime_registration_attaches_already_open_buffer() {
    // A buffer opened before its language has any registered server gets
    // its language set (via detection) but stays unattached. Registering
    // the server afterward must attach it, with no separate attach step.
    let tmp = safe_tempdir();
    let mut rig = rig_without_servers(&tmp);
    assert!(
        rig.attached().is_empty(),
        "buffer must be unattached before any server is registered"
    );

    rig.eval(RUST_ANALYZER);

    assert_eq!(rig.attached(), vec![rig.sid("rust-analyzer")]);
    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 1);
}

#[test]
fn unregister_stops_running_client_and_clears_config() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[f]>n main() {}\n",
        serde_json::json!({ "capabilities": {} }),
    );
    let sid = rig.sid("rust-analyzer");

    rig.eval(r#"(unregister-lsp-server! "rust-analyzer")"#);

    assert_eq!(
        rig.ed.state.lsp.instance_count_for_test(),
        0,
        "unregister must shut down the running client"
    );
    assert!(
        rig.ed
            .state
            .lsp
            .registered_command_for_test("rust-analyzer")
            .is_none(),
        "unregister must clear the registration"
    );
    assert!(rig.attached().is_empty(), "the buffer must be detached");
    assert_eq!(rig.sent(sid, "textDocument/didClose").len(), 1);
}

#[test]
fn unregister_of_never_registered_name_is_silent_success() {
    let tmp = safe_tempdir();
    let mut rig = rig_without_servers(&tmp);

    rig.eval(r#"(unregister-lsp-server! "nonexistent-server")"#);

    let log = rig.ed.state.message_log.format_for_display();
    assert!(
        !log.to_lowercase().contains("error"),
        "unregistering a never-registered name must not log an error, got: {log}"
    );
}

#[test]
fn replace_while_running_leaves_old_client_untouched() {
    // Replacing an already-registered name does NOT shut down its running
    // instance; only an explicit unregister does (the reinstall path). The
    // instance keeps running on the old config until its next spawn.
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[f]>n main() {}\n",
        serde_json::json!({ "capabilities": {} }),
    );
    let original = rig.sid("rust-analyzer");

    rig.eval(r#"(register-lsp-server! "rust-analyzer" #:command "rust-analyzer-2")"#);

    assert_eq!(
        rig.ed.state.lsp.instance_count_for_test(),
        1,
        "replacing a registration must not shut down the running server"
    );
    assert_eq!(
        rig.attached(),
        vec![original],
        "the already-attached buffer must stay on its original instance"
    );
    assert_eq!(
        rig.ed
            .state
            .lsp
            .registered_command_for_test("rust-analyzer")
            .as_deref(),
        Some("rust-analyzer-2"),
        "the registration itself must still reflect the replacement"
    );
    assert_eq!(
        drift_warnings(&rig),
        1,
        "the running server keeps its command, which must be said"
    );
}

const RA_WITH_COMMAND: &str =
    r#"(register-lsp-server! "rust-analyzer" #:command "rust-analyzer-2")"#;

fn drift_warnings(rig: &LspRig) -> usize {
    rig.ed
        .state
        .message_log
        .format_for_display()
        .matches("keeps what it started with")
        .count()
}

#[test]
fn a_running_server_whose_registration_changed_is_reported_once() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[f]>n main() {}\n",
        serde_json::json!({ "capabilities": {} }),
    );
    assert_eq!(drift_warnings(&rig), 0);

    rig.eval(RA_WITH_COMMAND);
    assert_eq!(drift_warnings(&rig), 1);
    let log = rig.ed.state.message_log.format_for_display();
    assert!(
        log.contains(":lsp-restart rust-analyzer"),
        "the warning names the command that applies the change: {log}"
    );

    rig.eval(RA_WITH_COMMAND);
    assert_eq!(
        drift_warnings(&rig),
        1,
        "the same drift is not reported twice"
    );
}

#[test]
fn a_restart_applies_the_registration_and_ends_the_drift() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[f]>n main() {}\n",
        serde_json::json!({ "capabilities": {} }),
    );
    rig.eval(RA_WITH_COMMAND);
    assert_eq!(drift_warnings(&rig), 1);

    rig.ed
        .apply_lsp_server_op(hume_scripting::PendingLspServerOp::Restart {
            target: hume_scripting::LspServerTarget::Name(
                hume_scripting::ServerName::parse("rust-analyzer").unwrap(),
            ),
        });
    rig.eval(RA_WITH_COMMAND);

    assert_eq!(
        drift_warnings(&rig),
        1,
        "the restarted server runs the registration, so there is nothing to report"
    );
}

#[test]
fn registering_what_the_server_already_runs_with_is_not_reported() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[f]>n main() {}\n",
        serde_json::json!({ "capabilities": {} }),
    );

    rig.eval(r#"(register-lsp-server! "rust-analyzer" #:command "rust-analyzer")"#);

    assert_eq!(drift_warnings(&rig), 0);
}

#[test]
fn register_and_open_matching_file_spawns_exactly_one_server_and_second_buffer_attaches() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[f]>n main() {}\n",
        serde_json::json!({ "capabilities": {} }),
    );
    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 1);
    let lib = rig.root.join("src/lib.rs");
    std::fs::write(&lib, b"// lib\n").unwrap();

    rig.ed
        .execute_typed("e", Some(lib.to_str().unwrap()))
        .unwrap();

    assert_eq!(
        rig.ed.state.lsp.instance_count_for_test(),
        1,
        "second file under the same root must attach, not spawn a second server"
    );
    let lib_bid = rig.ed.focused_buffer_id();
    assert_eq!(
        rig.ed
            .state
            .buffer_positions
            .lsp
            .servers(lib_bid)
            .collect::<Vec<_>>(),
        vec![rig.sid("rust-analyzer")]
    );
}

#[test]
fn opening_a_file_under_a_different_root_spawns_a_second_server() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[f]>n main() {}\n",
        serde_json::json!({ "capabilities": {} }),
    );
    let other_root = rig.root.join("other");
    std::fs::create_dir_all(other_root.join("src")).unwrap();
    std::fs::write(other_root.join("Cargo.toml"), b"").unwrap();
    let other = other_root.join("src/main.rs");
    std::fs::write(&other, b"fn main() {}\n").unwrap();

    rig.ed
        .execute_typed("e", Some(other.to_str().unwrap()))
        .unwrap();

    assert_eq!(
        rig.ed.state.lsp.instance_count_for_test(),
        2,
        "a different workspace root must spawn a second, independent server"
    );
}

/// A crashed instance keeps its buffers attached until `:lsp-restart`, and
/// a buffer opened under the same root joins it rather than spawning a
/// second process for a server that just crashed. The crash was reported
/// once, when it happened; the later attach reports nothing.
#[test]
fn attach_attempt_beside_a_crashed_server_reports_no_error() {
    let tmp = safe_tempdir();
    let mut rig = LspRig::rust(
        tmp.path(),
        "-[f]>n main() {}\n",
        serde_json::json!({ "capabilities": {} }),
    );
    let sid = rig.sid("rust-analyzer");
    rig.crash(sid);
    let errors_after_crash = rig.ed.state.message_log.totals().0;
    let lib = rig.root.join("src/lib.rs");
    std::fs::write(&lib, b"// lib\n").unwrap();

    rig.ed
        .execute_typed("e", Some(lib.to_str().unwrap()))
        .unwrap();

    let lib_bid = rig.ed.focused_buffer_id();
    assert_eq!(
        rig.ed
            .state
            .buffer_positions
            .lsp
            .servers(lib_bid)
            .collect::<Vec<_>>(),
        vec![sid]
    );
    assert_eq!(rig.ed.state.lsp.instance_count_for_test(), 1);
    assert_eq!(rig.ed.state.message_log.totals().0, errors_after_crash);
}
