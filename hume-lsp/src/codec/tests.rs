use super::*;
use std::io::Cursor;

fn roundtrip(msg: Message) -> Message {
    let mut buf = Vec::new();
    write_message(&mut buf, &msg).expect("write");
    let mut cursor = Cursor::new(buf);
    read_message(&mut cursor).expect("read")
}

#[test]
fn roundtrip_request_int_id() {
    let msg = Message::Request {
        id: RequestId::Int(7),
        method: "textDocument/hover".to_string(),
        params: serde_json::json!({"line": 1}),
    };
    match roundtrip(msg) {
        Message::Request { id, method, params } => {
            assert_eq!(id, RequestId::Int(7));
            assert_eq!(method, "textDocument/hover");
            assert_eq!(params, serde_json::json!({"line": 1}));
        }
        other => panic!("expected Request, got {other:?}"),
    }
}

#[test]
fn roundtrip_request_string_id() {
    let msg = Message::Request {
        id: RequestId::Str("abc-123".to_string()),
        method: "initialize".to_string(),
        params: serde_json::Value::Null,
    };
    match roundtrip(msg) {
        Message::Request { id, .. } => assert_eq!(id, RequestId::Str("abc-123".to_string())),
        other => panic!("expected Request, got {other:?}"),
    }
}

#[test]
fn roundtrip_notification() {
    let msg = Message::Notification {
        method: "textDocument/didOpen".to_string(),
        params: serde_json::json!({"uri": "file:///a"}),
    };
    match roundtrip(msg) {
        Message::Notification { method, params } => {
            assert_eq!(method, "textDocument/didOpen");
            assert_eq!(params, serde_json::json!({"uri": "file:///a"}));
        }
        other => panic!("expected Notification, got {other:?}"),
    }
}

#[test]
fn roundtrip_response_ok() {
    let msg = Message::Response {
        id: RequestId::Int(3),
        result: Ok(serde_json::json!({"capabilities": {}})),
    };
    match roundtrip(msg) {
        Message::Response { id, result } => {
            assert_eq!(id, RequestId::Int(3));
            assert_eq!(result.unwrap(), serde_json::json!({"capabilities": {}}));
        }
        other => panic!("expected Response, got {other:?}"),
    }
}

#[test]
fn roundtrip_response_ok_with_a_null_result() {
    // Many LSP methods legitimately succeed with a null result
    // (workspace/executeCommand, rename-with-nothing-to-change, …).
    // `{"id":5,"result":null}` must classify as a successful Response,
    // not Ambiguous — serde's default `Option<Value>` deserialization
    // collapses "field absent" and "field present as null" to the same
    // `None`, which this codec must not do for `result`/`error`.
    let msg = Message::Response {
        id: RequestId::Int(5),
        result: Ok(serde_json::Value::Null),
    };
    match roundtrip(msg) {
        Message::Response { id, result } => {
            assert_eq!(id, RequestId::Int(5));
            assert_eq!(result.unwrap(), serde_json::Value::Null);
        }
        other => panic!("expected Response, got {other:?}"),
    }
}

#[test]
fn roundtrip_response_error() {
    let msg = Message::Response {
        id: RequestId::Int(4),
        result: Err(ResponseError {
            code: -32601,
            message: "method not found".to_string(),
            data: None,
        }),
    };
    match roundtrip(msg) {
        Message::Response { id, result } => {
            assert_eq!(id, RequestId::Int(4));
            let err = result.unwrap_err();
            assert_eq!(err.code, -32601);
            assert_eq!(err.message, "method not found");
        }
        other => panic!("expected Response, got {other:?}"),
    }
}

#[test]
fn absent_params_becomes_null() {
    let body = br#"{"method":"initialized"}"#;
    let mut framed = Vec::new();
    write!(framed, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
    framed.extend_from_slice(body);
    let mut cursor = Cursor::new(framed);
    match read_message(&mut cursor).expect("read") {
        Message::Notification { method, params } => {
            assert_eq!(method, "initialized");
            assert_eq!(params, serde_json::Value::Null);
        }
        other => panic!("expected Notification, got {other:?}"),
    }
}

#[test]
fn missing_content_length_is_error() {
    let mut cursor = Cursor::new(b"Content-Type: application/json\r\n\r\n{}".to_vec());
    match read_message(&mut cursor) {
        Err(CodecError::MissingLength) => {}
        other => panic!("expected MissingLength, got {other:?}"),
    }
}

#[test]
fn garbage_header_line_is_error() {
    let mut cursor = Cursor::new(b"this is not a header\r\n\r\n{}".to_vec());
    match read_message(&mut cursor) {
        Err(CodecError::BadHeader(_)) => {}
        other => panic!("expected BadHeader, got {other:?}"),
    }
}

#[test]
fn truncated_body_is_io_error() {
    // Declares 100 bytes but only provides a handful.
    let mut cursor = Cursor::new(b"Content-Length: 100\r\n\r\n{}".to_vec());
    match read_message(&mut cursor) {
        Err(CodecError::Io(_)) => {}
        other => panic!("expected Io, got {other:?}"),
    }
}

#[test]
fn content_length_above_cap_is_error() {
    // A garbage/hostile value must be rejected before the `vec![0u8; len]`
    // allocation, not attempted.
    let mut cursor = Cursor::new(b"Content-Length: 99999999999\r\n\r\n{}".to_vec());
    match read_message(&mut cursor) {
        Err(CodecError::BadHeader(line)) => assert!(line.contains("99999999999")),
        other => panic!("expected BadHeader, got {other:?}"),
    }
}

#[test]
fn header_line_with_no_newline_past_cap_is_error() {
    // A stream that never sends a newline must not grow `read_line`'s
    // buffer without bound — it errors once the cap is exceeded.
    let garbage = vec![b'x'; 128 * 1024];
    let mut cursor = Cursor::new(garbage);
    match read_message(&mut cursor) {
        Err(CodecError::Io(_)) => {}
        other => panic!("expected Io (bound exceeded), got {other:?}"),
    }
}

#[test]
fn clean_eof_at_frame_boundary_is_distinct_from_mid_frame_truncation() {
    // Nothing at all read for this frame — a voluntary server exit,
    // not a truncation.
    let mut cursor = Cursor::new(Vec::<u8>::new());
    match read_message(&mut cursor) {
        Err(CodecError::Eof) => {}
        other => panic!("expected Eof, got {other:?}"),
    }

    // A header line was already read for this frame before the stream
    // ended — a genuine mid-frame truncation, not a clean exit.
    let mut cursor = Cursor::new(b"Content-Length: 5\r\n".to_vec());
    match read_message(&mut cursor) {
        Err(CodecError::Io(_)) => {}
        other => panic!("expected Io, got {other:?}"),
    }
}

#[test]
fn two_frames_back_to_back() {
    let mut buf = Vec::new();
    write_message(
        &mut buf,
        &Message::Notification {
            method: "one".to_string(),
            params: serde_json::Value::Null,
        },
    )
    .unwrap();
    write_message(
        &mut buf,
        &Message::Notification {
            method: "two".to_string(),
            params: serde_json::Value::Null,
        },
    )
    .unwrap();
    let mut cursor = Cursor::new(buf);
    match read_message(&mut cursor).unwrap() {
        Message::Notification { method, .. } => assert_eq!(method, "one"),
        other => panic!("expected Notification, got {other:?}"),
    }
    match read_message(&mut cursor).unwrap() {
        Message::Notification { method, .. } => assert_eq!(method, "two"),
        other => panic!("expected Notification, got {other:?}"),
    }
}

#[test]
fn multibyte_utf8_body_length_counts_bytes() {
    // "é" is 2 bytes in UTF-8 but 1 char — Content-Length must count bytes.
    let msg = Message::Notification {
        method: "test".to_string(),
        params: serde_json::json!({"text": "héllo wörld 日本語"}),
    };
    let mut buf = Vec::new();
    write_message(&mut buf, &msg).unwrap();
    let mut cursor = Cursor::new(buf);
    match read_message(&mut cursor).unwrap() {
        Message::Notification { params, .. } => {
            assert_eq!(params, serde_json::json!({"text": "héllo wörld 日本語"}));
        }
        other => panic!("expected Notification, got {other:?}"),
    }
}

#[test]
fn write_emits_exact_crlf_crlf() {
    let msg = Message::Notification {
        method: "x".to_string(),
        params: serde_json::Value::Null,
    };
    let mut buf = Vec::new();
    write_message(&mut buf, &msg).unwrap();
    let s = String::from_utf8_lossy(&buf);
    assert!(s.starts_with("Content-Length: "));
    assert!(s.contains("\r\n\r\n"));
    // Flip check: a bare "\n\n" terminator (no \r) must NOT appear before body.
    let header_end = s.find("\r\n\r\n").unwrap();
    assert!(!s[..header_end].contains("\n\n"));
}

#[test]
fn ambiguous_body_is_error() {
    // Both method and result present — neither request/notification nor response.
    let body = br#"{"id":1,"method":"foo","result":{}}"#;
    let mut framed = Vec::new();
    write!(framed, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
    framed.extend_from_slice(body);
    let mut cursor = Cursor::new(framed);
    match read_message(&mut cursor) {
        Err(CodecError::Ambiguous) => {}
        other => panic!("expected Ambiguous, got {other:?}"),
    }
}

#[test]
fn write_emits_jsonrpc_version_member() {
    // JSON-RPC 2.0 requires "jsonrpc":"2.0" on every request, response,
    // and notification — a strict server-side stack can reject or drop
    // a frame missing it.
    let msg = Message::Notification {
        method: "textDocument/didOpen".to_string(),
        params: serde_json::Value::Null,
    };
    let mut buf = Vec::new();
    write_message(&mut buf, &msg).unwrap();
    let header_end = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("header terminator")
        + 4;
    let body: serde_json::Value = serde_json::from_slice(&buf[header_end..]).unwrap();
    assert_eq!(body.get("jsonrpc"), Some(&serde_json::json!("2.0")));
}

#[test]
fn id_allocator_increments() {
    let mut alloc = IdAllocator::new();
    assert_eq!(alloc.next(), RequestId::Int(1));
    assert_eq!(alloc.next(), RequestId::Int(2));
    assert_eq!(alloc.next(), RequestId::Int(3));
}
