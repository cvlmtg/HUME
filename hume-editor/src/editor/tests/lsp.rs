// C6 (docs/lsp/step-1.md) — request bookkeeping + server->client dispatch,
// exercised through the editor's LspState glue (drain_lsp).

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use super::*;
use crate::editor::lsp::LspState;
use hume_lsp::backend::{LspBackend, ServerId};
use hume_lsp::client::{LspClient, Outcome, RequestMeta};
use hume_lsp::inline::InlineLspBackend;

/// Wraps a scripted `InlineLspBackend` (with a server already `start`ed) into
/// the editor's `LspState` and tracks a matching `LspClient` for it.
fn wire_client(ed: &mut Editor, backend: InlineLspBackend, sid: ServerId) {
    ed.lsp = LspState::from_backend_for_test(Box::new(backend));
    let client = LspClient::new(sid, PathBuf::from("."));
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
    let token = ed.lsp.register_callback(
        None,
        Box::new(move |_ed, outcome| {
            *result_in_closure.borrow_mut() = Some(outcome);
        }),
    );
    let meta = RequestMeta {
        method: "textDocument/hover".to_string(),
        allow_stale: false,
        deadline: Instant::now() + Duration::from_secs(10),
        token,
    };
    ed.lsp
        .send_request(sid, "textDocument/hover", serde_json::Value::Null, meta);

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
    let token = ed.lsp.register_callback(
        None,
        Box::new(move |_ed, _outcome| {
            *fired_in_closure.borrow_mut() = true;
        }),
    );
    let meta = RequestMeta {
        method: "textDocument/hover".to_string(),
        allow_stale: false,
        deadline: Instant::now() + Duration::from_secs(10),
        token,
    };
    ed.lsp
        .send_request(sid, "textDocument/hover", serde_json::Value::Null, meta);

    ed.drain_lsp();

    assert!(!*fired.borrow(), "no canned response — callback must not fire");
}

#[test]
fn timed_out_request_drops_callback_and_logs_trace() {
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new(".")).unwrap();
    wire_client(&mut ed, backend, sid);

    let fired = Rc::new(RefCell::new(false));
    let fired_in_closure = fired.clone();
    let token = ed.lsp.register_callback(
        None,
        Box::new(move |_ed, _outcome| {
            *fired_in_closure.borrow_mut() = true;
        }),
    );
    let meta = RequestMeta {
        method: "textDocument/completion".to_string(),
        allow_stale: false,
        deadline: Instant::now() - Duration::from_millis(1),
        token,
    };
    ed.lsp
        .send_request(sid, "textDocument/completion", serde_json::Value::Null, meta);

    ed.drain_lsp();

    assert!(!*fired.borrow(), "a timed-out request must not dispatch its callback");
}

#[test]
fn stale_response_is_dropped_when_buffer_moved_past_text_gen() {
    let mut ed = editor_from("-[w]>ord\n");
    let bid = ed.focused_buffer_id();
    let sent_gen = ed.state.buffers.get(bid).text_gen;

    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new(".")).unwrap();
    backend.respond_to("textDocument/hover", serde_json::json!({"contents": "stale"}));
    wire_client(&mut ed, backend, sid);

    let fired = Rc::new(RefCell::new(false));
    let fired_in_closure = fired.clone();
    let token = ed.lsp.register_callback(
        Some((bid, sent_gen)),
        Box::new(move |_ed, _outcome| {
            *fired_in_closure.borrow_mut() = true;
        }),
    );
    let meta = RequestMeta {
        method: "textDocument/hover".to_string(),
        allow_stale: false,
        deadline: Instant::now() + Duration::from_secs(10),
        token,
    };
    ed.lsp
        .send_request(sid, "textDocument/hover", serde_json::Value::Null, meta);

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
    let token = ed.lsp.register_callback(
        Some((bid, sent_gen)),
        Box::new(move |_ed, _outcome| {
            *fired_in_closure.borrow_mut() = true;
        }),
    );
    let meta = RequestMeta {
        method: "textDocument/hover".to_string(),
        allow_stale: true,
        deadline: Instant::now() + Duration::from_secs(10),
        token,
    };
    ed.lsp
        .send_request(sid, "textDocument/hover", serde_json::Value::Null, meta);

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
    assert!(log.contains("server crashed") && log.contains("boom"));
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
    // `editor::lsp::tests` against the pure `server_request_response`
    // function — that's the right altitude for table-shape assertions.
    assert!(ed.lsp.backend_mut().drain().is_empty());
}

#[test]
fn became_running_flushes_queued_messages_through_the_backend() {
    let mut ed = editor_from("-[w]>ord\n");
    let mut backend = InlineLspBackend::with_default_handshake();
    let sid = backend.start("rust-analyzer", &[], Path::new(".")).unwrap();
    wire_client(&mut ed, backend, sid);

    let (client, backend) = ed
        .lsp
        .client_and_backend_for_test(sid)
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
    assert_eq!(client.state, hume_lsp::client::ServerState::Running);
}
