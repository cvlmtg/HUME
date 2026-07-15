// Request bookkeeping + server->client dispatch,
// exercised through the editor's LspState glue (drain_lsp). Server
// registration tests live at the bottom of this file.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use super::*;
use crate::editor::lsp::LspState;
use crate::editor::scripting_setup::make_init_host;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::{LspClient, Outcome, RequestMeta, ServerState};
use hume_lsp::inline::InlineLspBackend;
use hume_scripting::ScriptingHost;

/// Wraps a scripted `InlineLspBackend` (with a server already `start`ed) into
/// the editor's `LspState` and tracks a matching `LspClient` for it.
fn wire_client(ed: &mut Editor, backend: InlineLspBackend, sid: ServerId) {
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    // Running, not the `Starting` default: these tests exercise request/
    // response/staleness bookkeeping, not the handshake queue (covered by
    // hume-lsp's own `send_request_while_starting_is_queued_then_flushed_*`
    // test) — a Starting client queues instead of sending, which would
    // leave every `send_request` below stuck unsent.
    let mut client = LspClient::new(sid, PathBuf::from("."));
    client.set_state_for_test(ServerState::Running);
    ed.lsp.insert_client_for_test(client);
}

#[test]
fn callback_fires_with_ok_outcome_on_response() {
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("rust-analyzer", &[], Path::new(".")).unwrap();
    backend.respond_to("textDocument/hover", serde_json::json!({"contents": "hi"}));
    wire_client(&mut ed, backend, sid);

    let result: Rc<RefCell<Option<Outcome>>> = Rc::new(RefCell::new(None));
    let result_in_closure = result.clone();
    let meta = RequestMeta {
        method: "textDocument/hover".to_string(),
        allow_stale: false,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let id = ed
        .lsp
        .send_request(sid, "textDocument/hover", serde_json::Value::Null, meta)
        .expect("client tracked");
    ed.lsp.register_callback(
        sid,
        id,
        None,
        Box::new(move |_ed, outcome| {
            *result_in_closure.borrow_mut() = Some(outcome);
        }),
    );

    ed.drain_lsp();

    match result.borrow_mut().take() {
        Some(Outcome::Ok(v)) => assert_eq!(v, serde_json::json!({"contents": "hi"})),
        other => panic!("expected Ok, got {other:?}"),
    }
}

#[test]
fn callback_never_fires_for_a_request_with_no_response() {
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new(".")).unwrap();
    wire_client(&mut ed, backend, sid);

    let fired = Rc::new(RefCell::new(false));
    let fired_in_closure = fired.clone();
    let meta = RequestMeta {
        method: "textDocument/hover".to_string(),
        allow_stale: false,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let id = ed
        .lsp
        .send_request(sid, "textDocument/hover", serde_json::Value::Null, meta)
        .expect("client tracked");
    ed.lsp.register_callback(
        sid,
        id,
        None,
        Box::new(move |_ed, _outcome| {
            *fired_in_closure.borrow_mut() = true;
        }),
    );

    ed.drain_lsp();

    assert!(
        !*fired.borrow(),
        "no canned response — callback must not fire"
    );
}

#[test]
fn timed_out_request_dispatches_callback_with_timed_out_outcome_and_logs_trace() {
    // Deviates from "timed-out -> log + drop":
    // a callback that never fires on timeout has no way to notice — the
    // Steel callbacks are `(err result)`-shaped and need this to map a
    // timeout to `err` rather than hanging silently.
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new(".")).unwrap();
    wire_client(&mut ed, backend, sid);

    let result: Rc<RefCell<Vec<Outcome>>> = Rc::new(RefCell::new(Vec::new()));
    let result_in_closure = result.clone();
    let meta = RequestMeta {
        method: "textDocument/completion".to_string(),
        allow_stale: false,
        deadline: Instant::now() - Duration::from_millis(1),
    };
    let id = ed
        .lsp
        .send_request(
            sid,
            "textDocument/completion",
            serde_json::Value::Null,
            meta,
        )
        .expect("client tracked");
    ed.lsp.register_callback(
        sid,
        id,
        None,
        Box::new(move |_ed, outcome| {
            result_in_closure.borrow_mut().push(outcome);
        }),
    );

    ed.drain_lsp();

    let outcomes = result.borrow_mut();
    assert_eq!(outcomes.len(), 1, "callback must fire exactly once");
    assert!(
        matches!(outcomes[0], Outcome::TimedOut),
        "expected TimedOut, got {:?}",
        outcomes[0]
    );
}

#[test]
fn stale_response_is_dropped_when_buffer_moved_past_text_gen() {
    let mut ed = editor_from("-[w]>ord\n");
    let bid = ed.focused_buffer_id();
    let sent_gen = ed.state.buffers.get(bid).text_gen;

    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new(".")).unwrap();
    backend.respond_to(
        "textDocument/hover",
        serde_json::json!({"contents": "stale"}),
    );
    wire_client(&mut ed, backend, sid);

    let fired = Rc::new(RefCell::new(false));
    let fired_in_closure = fired.clone();
    let meta = RequestMeta {
        method: "textDocument/hover".to_string(),
        allow_stale: false,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let id = ed
        .lsp
        .send_request(sid, "textDocument/hover", serde_json::Value::Null, meta)
        .expect("client tracked");
    ed.lsp.register_callback(
        sid,
        id,
        Some((bid, sent_gen)),
        Box::new(move |_ed, _outcome| {
            *fired_in_closure.borrow_mut() = true;
        }),
    );

    // Move the buffer's text_gen past the value the request was sent at.
    ed.step(key('d'));

    ed.drain_lsp();

    assert!(
        !*fired.borrow(),
        "the buffer moved past the request's text_gen — the callback must be dropped"
    );
}

#[test]
fn allow_stale_delivers_despite_buffer_moving_past_text_gen() {
    let mut ed = editor_from("-[w]>ord\n");
    let bid = ed.focused_buffer_id();
    let sent_gen = ed.state.buffers.get(bid).text_gen;

    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new(".")).unwrap();
    backend.respond_to("textDocument/hover", serde_json::json!({"contents": "ok"}));
    wire_client(&mut ed, backend, sid);

    let fired = Rc::new(RefCell::new(false));
    let fired_in_closure = fired.clone();
    let meta = RequestMeta {
        method: "textDocument/hover".to_string(),
        allow_stale: true,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let id = ed
        .lsp
        .send_request(sid, "textDocument/hover", serde_json::Value::Null, meta)
        .expect("client tracked");
    ed.lsp.register_callback(
        sid,
        id,
        Some((bid, sent_gen)),
        Box::new(move |_ed, _outcome| {
            *fired_in_closure.borrow_mut() = true;
        }),
    );

    ed.step(key('d'));
    ed.drain_lsp();

    assert!(
        *fired.borrow(),
        "allow_stale opts out of the staleness drop — the callback must still fire"
    );
}

#[test]
fn crashed_action_is_reported_to_the_message_log() {
    // `on_event(Eof)` producing exactly one `Crashed` action (never twice)
    // is covered in hume-lsp's own client tests; this covers the editor
    // glue's side: dispatching that action actually reaches the log.
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new(".")).unwrap();
    wire_client(&mut ed, backend, sid);

    ed.dispatch_lsp_action(
        sid,
        hume_lsp::client::ClientAction::Crashed {
            error: Some("boom".to_string()),
        },
    );

    let log = ed.state.message_log.format_for_display();
    // `wire_client` registers under the "lsp" test-only placeholder name
    // (`insert_client_for_test`) — a real registration's server name would
    // appear here instead, so multiple crashed servers are distinguishable.
    assert!(log.contains("lsp crashed") && log.contains("boom"));
}

#[test]
fn crash_fails_in_flight_requests_immediately_instead_of_waiting_for_their_deadline() {
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new(".")).unwrap();
    // No response scripted — this request would otherwise sit pending
    // until its (deliberately far-future) deadline.
    wire_client(&mut ed, backend, sid);

    let result: Rc<RefCell<Option<Outcome>>> = Rc::new(RefCell::new(None));
    let result_in_closure = result.clone();
    let meta = RequestMeta {
        method: "textDocument/hover".to_string(),
        allow_stale: false,
        deadline: Instant::now() + Duration::from_secs(3600),
    };
    let id = ed
        .lsp
        .send_request(sid, "textDocument/hover", serde_json::Value::Null, meta)
        .expect("client tracked");
    ed.lsp.register_callback(
        sid,
        id,
        None,
        Box::new(move |_ed, outcome| {
            *result_in_closure.borrow_mut() = Some(outcome);
        }),
    );

    ed.dispatch_lsp_action(
        sid,
        hume_lsp::client::ClientAction::Crashed {
            error: Some("boom".to_string()),
        },
    );

    match result.borrow_mut().take() {
        Some(Outcome::TimedOut) => {}
        other => panic!("expected the pending request to fail immediately on crash, got {other:?}"),
    }
}

#[test]
fn initialize_timeout_reports_a_crash_through_drain_lsp() {
    // `take_completed`'s sweep producing the `Crashed` action for an expired
    // `initialize` is covered in hume-lsp's own client tests; this covers
    // the editor glue's side: `drain_lsp` actually dispatches that action.
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new(); // no scripted `initialize` response
    let sid = backend.start("x", &[], Path::new(".")).unwrap();
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let mut client = LspClient::new(sid, PathBuf::from("."));
    client.start_handshake(ed.lsp.backend_mut());
    ed.lsp.insert_client_for_test(client);

    ed.lsp
        .client_for_test(sid)
        .unwrap()
        .expire_pending_deadlines_for_test();

    ed.drain_lsp();

    assert_eq!(
        ed.lsp.client_for_test(sid).unwrap().state(),
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
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    backend.fail_with("shutdown", -32603, "internal error");
    let sid = backend.start("x", &[], Path::new(".")).unwrap();
    wire_client(&mut ed, backend, sid);

    let (client, backend) = ed.lsp.client_and_backend(sid).unwrap();
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
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new(".")).unwrap();
    backend.push_from_server(
        sid,
        hume_lsp::codec::Message::Request {
            id: hume_lsp::codec::RequestId::Int(1),
            method: "workspace/configuration".to_string(),
            params: serde_json::json!({"items": [{}]}),
        },
    );
    wire_client(&mut ed, backend, sid);

    ed.drain_lsp();

    // The response went out through the (now-boxed) backend; the only
    // externally observable proof at this layer is that dispatch didn't
    // panic and drained cleanly. The dispatch table itself (every method,
    // including MethodNotFound) is exhaustively unit-tested directly in
    // `hume_lsp::client::tests` against the pure `server_request_response`
    // function — that's the right altitude for table-shape assertions.
    assert!(ed.lsp.backend_mut().drain().is_empty());
}

#[test]
fn lsp_stop_dispatches_timed_out_for_in_flight_callbacks_instead_of_orphaning_them() {
    // Without draining a removed client's `pending` map, `:lsp-stop` dropped
    // the `LspClient` (and its pending requests) outright — a registered
    // callback for a request still in flight never fired, and its
    // `CallbackEntry` leaked in `LspState.callbacks` forever.
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new(".")).unwrap();
    wire_client(&mut ed, backend, sid);
    ed.lsp
        .insert_server_key_for_test("rust".to_string(), PathBuf::from("."), sid);

    let result: Rc<RefCell<Vec<Outcome>>> = Rc::new(RefCell::new(Vec::new()));
    let result_in_closure = result.clone();
    let meta = RequestMeta {
        method: "textDocument/hover".to_string(),
        allow_stale: false,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let id = ed
        .lsp
        .send_request(sid, "textDocument/hover", serde_json::Value::Null, meta)
        .expect("client tracked");
    ed.lsp.register_callback(
        sid,
        id,
        None,
        Box::new(move |_ed, outcome| {
            result_in_closure.borrow_mut().push(outcome);
        }),
    );

    ed.lsp_stop(Some("rust"));

    {
        let outcomes = result.borrow();
        assert_eq!(outcomes.len(), 1, "callback must fire exactly once on stop");
        assert!(
            matches!(outcomes[0], Outcome::TimedOut),
            "expected TimedOut, got {:?}",
            outcomes[0]
        );
    }
    assert_eq!(
        ed.lsp.callback_count_for_test(),
        0,
        "the callback entry must not leak after being dispatched"
    );
}

#[test]
fn became_running_flushes_queued_messages_through_the_backend() {
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::with_default_handshake();
    let sid = backend.start("rust-analyzer", &[], Path::new(".")).unwrap();
    wire_client(&mut ed, backend, sid);

    let (client, backend) = ed
        .lsp
        .client_and_backend(sid)
        .expect("client inserted above");
    client.start_handshake(backend);
    client.send_or_queue(
        backend,
        hume_lsp::codec::Message::Notification {
            method: "textDocument/didOpen".to_string(),
            params: serde_json::json!({"uri": "file:///a"}),
        },
    );

    ed.drain_lsp();

    let client = ed
        .lsp
        .client_for_test(sid)
        .expect("client must still be tracked after drain");
    assert_eq!(client.state(), hume_lsp::client::ServerState::Running);
}

// ── Server registration ────────────────────────────────────────────────

fn eval_register(ed: &mut Editor, host: &mut ScriptingHost, source: &str, tmp: &std::path::Path) {
    let init_path = tmp.join("init.scm");
    std::fs::write(&init_path, source).unwrap();
    {
        let mut ih = make_init_host(&mut ed.state, &mut ed.view);
        host.eval_init(&init_path, 10_000, &mut ih, Default::default())
    }
    .expect("eval_init");
    ed.flush_pending_lsp_server_ops(host);
}

#[test]
fn second_registration_replaces_first() {
    // Last-wins: a second register-lsp-server! for an already-registered
    // language replaces the config rather than being rejected — matching
    // define-language!'s semantics. No error is logged.
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[w]>ord\n");
    ed.lsp = LspState::new_inline();
    let mut host = ScriptingHost::new();

    eval_register(
        &mut ed,
        &mut host,
        r#"(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))"#,
        tmp.path(),
    );
    eval_register(
        &mut ed,
        &mut host,
        r#"(register-lsp-server! "rust" #:command "rust-analyzer-2" #:root-markers '())"#,
        tmp.path(),
    );

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.contains("already registered") && !log.contains("Error"),
        "second registration must not be rejected as a duplicate, got log: {log}"
    );
    assert_eq!(
        ed.lsp.config_command_for_test("rust").as_deref(),
        Some("rust-analyzer-2"),
        "second registration must win"
    );
}

#[test]
fn runtime_registration_attaches_already_open_buffer() {
    // A buffer opened before its language has any registered server gets
    // its language set (via detection) but stays unattached. Registering
    // the server afterward must sweep it in — no separate attach step.
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::write(root.join("Cargo.toml"), b"").unwrap();
    let file = root.join("main.rs");
    std::fs::write(&file, b"fn main() {}\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    ed.lsp = LspState::new_inline();
    ed.state
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .unwrap();
    let mut host = ScriptingHost::new();

    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    assert!(
        ed.state.buffers.get(bid).lsp_server.is_none(),
        "buffer must be unattached before any server is registered"
    );

    eval_register(
        &mut ed,
        &mut host,
        r#"(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))"#,
        tmp.path(),
    );

    assert!(
        ed.state.buffers.get(bid).lsp_server.is_some(),
        "registration must sweep and attach the already-open matching buffer"
    );
    assert_eq!(ed.lsp.server_count_for_test(), 1);
}

#[test]
fn unregister_stops_running_client_and_clears_config() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::write(root.join("Cargo.toml"), b"").unwrap();
    let file = root.join("main.rs");
    std::fs::write(&file, b"fn main() {}\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    ed.lsp = LspState::new_inline();
    ed.state
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .unwrap();
    let mut host = ScriptingHost::new();
    eval_register(
        &mut ed,
        &mut host,
        r#"(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))"#,
        tmp.path(),
    );
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    assert_eq!(ed.lsp.server_count_for_test(), 1);
    assert!(ed.state.buffers.get(bid).lsp_server.is_some());

    eval_register(
        &mut ed,
        &mut host,
        r#"(unregister-lsp-server! "rust")"#,
        tmp.path(),
    );

    assert_eq!(
        ed.lsp.server_count_for_test(),
        0,
        "unregister must shut down the running client"
    );
    assert!(
        ed.lsp.config_command_for_test("rust").is_none(),
        "unregister must clear the registration"
    );
    assert!(
        ed.state.buffers.get(bid).lsp_server.is_none(),
        "the detached buffer's lsp_server must be cleared"
    );
}

#[test]
fn unregister_of_never_registered_language_is_silent_success() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[w]>ord\n");
    ed.lsp = LspState::new_inline();
    let mut host = ScriptingHost::new();

    eval_register(
        &mut ed,
        &mut host,
        r#"(unregister-lsp-server! "nonexistent-language")"#,
        tmp.path(),
    );

    let log = ed.state.message_log.format_for_display();
    assert!(
        !log.to_lowercase().contains("error"),
        "unregistering an orphan/never-registered language must not log an error, got: {log}"
    );
}

#[test]
fn replace_while_running_leaves_old_client_untouched() {
    // Spec: replacing an already-registered language does NOT shut down
    // running clients — that only happens via an explicit unregister
    // (the reinstall path). The old client keeps running on the old config
    // until its next spawn.
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::write(root.join("Cargo.toml"), b"").unwrap();
    let file = root.join("main.rs");
    std::fs::write(&file, b"fn main() {}\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    ed.lsp = LspState::new_inline();
    ed.state
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .unwrap();
    let mut host = ScriptingHost::new();
    eval_register(
        &mut ed,
        &mut host,
        r#"(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))"#,
        tmp.path(),
    );
    ed.execute_typed("e", Some(file.to_str().unwrap())).unwrap();
    let bid = ed.focused_buffer_id();
    let original_server = ed.state.buffers.get(bid).lsp_server;
    assert!(original_server.is_some());

    eval_register(
        &mut ed,
        &mut host,
        r#"(register-lsp-server! "rust" #:command "rust-analyzer-2" #:root-markers '("Cargo.toml"))"#,
        tmp.path(),
    );

    assert_eq!(
        ed.lsp.server_count_for_test(),
        1,
        "replacing a registration must not shut down the running server"
    );
    assert_eq!(
        ed.state.buffers.get(bid).lsp_server,
        original_server,
        "the already-attached buffer must stay on its original client"
    );
    assert_eq!(
        ed.lsp.config_command_for_test("rust").as_deref(),
        Some("rust-analyzer-2"),
        "the config itself must still reflect the replacement"
    );
}

#[test]
fn register_and_open_matching_file_spawns_exactly_one_server_and_second_buffer_attaches() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::write(root.join("Cargo.toml"), b"").unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let file1 = root.join("src/main.rs");
    std::fs::write(&file1, b"fn main() {}\n").unwrap();
    let file2 = root.join("src/lib.rs");
    std::fs::write(&file2, b"// lib\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    ed.lsp = LspState::new_inline();
    ed.state
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .unwrap();
    let mut host = ScriptingHost::new();
    eval_register(
        &mut ed,
        &mut host,
        r#"(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))"#,
        tmp.path(),
    );

    ed.execute_typed("e", Some(file1.to_str().unwrap()))
        .unwrap();
    assert_eq!(
        ed.lsp.server_count_for_test(),
        1,
        "first matching file must spawn exactly one server"
    );

    ed.execute_typed("e", Some(file2.to_str().unwrap()))
        .unwrap();
    assert_eq!(
        ed.lsp.server_count_for_test(),
        1,
        "second file under the same root must attach, not spawn a second server, \
         nor insert a second entry that silently overwrites the first"
    );
}

#[test]
fn opening_a_file_under_a_different_root_spawns_a_second_server() {
    let tmp = tempfile::tempdir().unwrap();
    let root_a = std::fs::canonicalize(tmp.path()).unwrap().join("a");
    let root_b = std::fs::canonicalize(tmp.path()).unwrap().join("b");
    for root in [&root_a, &root_b] {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("Cargo.toml"), b"").unwrap();
        std::fs::write(root.join("src/main.rs"), b"fn main() {}\n").unwrap();
    }

    let mut ed = editor_from("-[w]>ord\n");
    ed.lsp = LspState::new_inline();
    ed.state
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .unwrap();
    let mut host = ScriptingHost::new();
    eval_register(
        &mut ed,
        &mut host,
        r#"(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))"#,
        tmp.path(),
    );

    ed.execute_typed("e", Some(root_a.join("src/main.rs").to_str().unwrap()))
        .unwrap();
    ed.execute_typed("e", Some(root_b.join("src/main.rs").to_str().unwrap()))
        .unwrap();

    assert_eq!(
        ed.lsp.server_count_for_test(),
        2,
        "a different workspace root must spawn a second, independent server"
    );
}

#[test]
fn crashed_server_is_not_silently_reattached_to() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::write(root.join("Cargo.toml"), b"").unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let file1 = root.join("src/main.rs");
    std::fs::write(&file1, b"fn main() {}\n").unwrap();
    let file2 = root.join("src/lib.rs");
    std::fs::write(&file2, b"// lib\n").unwrap();

    let mut ed = editor_from("-[w]>ord\n");
    ed.lsp = LspState::new_inline();
    ed.state
        .languages
        .register_identity("rust", &["rs"], &[], &[])
        .unwrap();
    let mut host = ScriptingHost::new();
    eval_register(
        &mut ed,
        &mut host,
        r#"(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))"#,
        tmp.path(),
    );

    ed.execute_typed("e", Some(file1.to_str().unwrap()))
        .unwrap();
    let bid1 = ed.focused_buffer_id();
    let server_id = ed
        .state
        .buffers
        .get(bid1)
        .lsp_server
        .expect("first buffer attached");
    assert_eq!(ed.lsp.server_count_for_test(), 1);

    // Simulate a crash: the same state `on_event(Eof)`/an expired
    // `initialize` (via `take_completed`'s sweep) would transition to.
    // Nothing removes a Crashed entry from `LspState.servers` on its own
    // (only `:lsp-stop`/`:lsp-restart` do), so the corpse stays put for the
    // next attach attempt to find.
    ed.lsp
        .client_for_test(server_id)
        .unwrap()
        .set_state_for_test(hume_lsp::client::ServerState::Crashed);

    ed.execute_typed("e", Some(file2.to_str().unwrap()))
        .unwrap();
    let bid2 = ed.focused_buffer_id();

    assert!(
        ed.state.buffers.get(bid2).lsp_server.is_none(),
        "must not silently attach to a Crashed server"
    );
    assert_eq!(
        ed.lsp.server_count_for_test(),
        1,
        "must not spawn a second server either — the corpse blocks reattachment until :lsp-restart"
    );
    let log = ed.state.message_log.format_for_display();
    assert!(
        log.contains("crashed") && log.contains(":lsp-restart"),
        "must hint at :lsp-restart: {log}"
    );
}
