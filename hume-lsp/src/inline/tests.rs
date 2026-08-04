use super::*;
use crate::codec::RequestId;

#[test]
fn respond_to_delivers_on_next_drain_not_inline() {
    let mut backend = InlineLspBackend::new();
    backend.respond_to("textDocument/hover", serde_json::json!({"contents": "hi"}));
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();

    backend.send(
        sid,
        Message::Request {
            id: RequestId::Int(1),
            method: "textDocument/hover".to_string(),
            params: serde_json::Value::Null,
        },
    );
    // `send` never returns the response directly — it only becomes
    // observable through a later `drain()` call, matching the discipline
    // callers depend on (they never get answers synchronously).
    assert!(backend.has_pending());
    let events = backend.drain();
    assert_eq!(events.len(), 1);
    match &events[0] {
        (id, InboundEvent::Message(Message::Response { id: rid, result })) => {
            assert_eq!(*id, sid);
            assert_eq!(*rid, RequestId::Int(1));
            assert_eq!(
                result.clone().unwrap(),
                serde_json::json!({"contents": "hi"})
            );
        }
        _ => panic!("expected a Response event"),
    }
    // Drained once — second drain is empty.
    assert!(!backend.has_pending());
    assert!(backend.drain().is_empty());
}

#[test]
fn fail_with_delivers_error_response() {
    let mut backend = InlineLspBackend::new();
    backend.fail_with("textDocument/definition", -32601, "not found");
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();
    backend.send(
        sid,
        Message::Request {
            id: RequestId::Int(2),
            method: "textDocument/definition".to_string(),
            params: serde_json::Value::Null,
        },
    );
    let events = backend.drain();
    match &events[0] {
        (_, InboundEvent::Message(Message::Response { result, .. })) => {
            let err = result.clone().unwrap_err();
            assert_eq!(err.code, -32601);
            assert_eq!(err.message, "not found");
        }
        _ => panic!("expected a Response event"),
    }
}

#[test]
fn responses_are_fifo_per_method() {
    let mut backend = InlineLspBackend::new();
    backend.respond_to("m", serde_json::json!(1));
    backend.respond_to("m", serde_json::json!(2));
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();

    for i in 1..=2 {
        backend.send(
            sid,
            Message::Request {
                id: RequestId::Int(i),
                method: "m".to_string(),
                params: serde_json::Value::Null,
            },
        );
    }
    let events = backend.drain();
    assert_eq!(events.len(), 2);
    match &events[0] {
        (_, InboundEvent::Message(Message::Response { result, .. })) => {
            assert_eq!(result.clone().unwrap(), serde_json::json!(1));
        }
        _ => panic!("expected Response"),
    }
    match &events[1] {
        (_, InboundEvent::Message(Message::Response { result, .. })) => {
            assert_eq!(result.clone().unwrap(), serde_json::json!(2));
        }
        _ => panic!("expected Response"),
    }
}

#[test]
fn request_with_no_canned_response_is_recorded_but_produces_no_event() {
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();
    backend.send(
        sid,
        Message::Request {
            id: RequestId::Int(1),
            method: "unscripted".to_string(),
            params: serde_json::Value::Null,
        },
    );
    assert!(backend.drain().is_empty());
    assert_eq!(backend.sent.len(), 1);
}

#[test]
fn push_from_server_surfaces_on_drain() {
    let mut backend = InlineLspBackend::new();
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();
    backend.push_from_server(
        sid,
        Message::Notification {
            method: "textDocument/publishDiagnostics".to_string(),
            params: serde_json::json!({"uri": "file:///a", "diagnostics": []}),
        },
    );
    let events = backend.drain();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, sid);
}

#[test]
fn with_default_handshake_answers_initialize() {
    let mut backend = InlineLspBackend::with_default_handshake();
    let sid = backend.start("x", &[], Path::new("."), &[]).unwrap();
    backend.send(
        sid,
        Message::Request {
            id: RequestId::Int(1),
            method: "initialize".to_string(),
            params: serde_json::Value::Null,
        },
    );
    let events = backend.drain();
    match &events[0] {
        (_, InboundEvent::Message(Message::Response { result, .. })) => {
            let parsed: InitializeResult = serde_json::from_value(result.clone().unwrap()).unwrap();
            assert_eq!(
                parsed.capabilities.position_encoding,
                Some(PositionEncodingKind::UTF8)
            );
        }
        _ => panic!("expected Response"),
    }
}
