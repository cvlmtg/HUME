//! JSON-RPC 2.0 framing over `Content-Length`-delimited messages.
//!
//! Wire format: `Content-Length: N\r\n\r\n` followed by exactly `N` bytes of
//! a JSON-RPC body. Any other header before the blank line is tolerated and
//! ignored. Any parse failure is treated as connection-fatal by the caller —
//! this module never tries to resynchronize a corrupted stream.

use std::io::{BufRead, Write};

use serde::{Deserialize, Serialize};

/// JSON-RPC request/response id. HUME allocates monotonically increasing
/// integers for its own requests; servers may echo string ids for theirs.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Int(i64),
    Str(String),
}

#[derive(Debug)]
pub enum Message {
    Request {
        id: RequestId,
        method: String,
        params: serde_json::Value,
    },
    Response {
        id: RequestId,
        result: Result<serde_json::Value, ResponseError>,
    },
    Notification {
        method: String,
        params: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ResponseError {
    pub code: i64,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

#[derive(Debug)]
pub enum CodecError {
    Io(std::io::Error),
    MissingLength,
    BadHeader(String),
    Json(serde_json::Error),
    /// Body has a shape that isn't a well-formed Request, Response, or
    /// Notification (e.g. both `method` and `result` present).
    Ambiguous,
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodecError::Io(e) => write!(f, "io error: {e}"),
            CodecError::MissingLength => write!(f, "missing Content-Length header"),
            CodecError::BadHeader(line) => write!(f, "malformed header: {line:?}"),
            CodecError::Json(e) => write!(f, "json error: {e}"),
            CodecError::Ambiguous => {
                write!(f, "message body is neither request, response, nor notification")
            }
        }
    }
}

impl std::error::Error for CodecError {}

impl From<std::io::Error> for CodecError {
    fn from(e: std::io::Error) -> Self {
        CodecError::Io(e)
    }
}

impl From<serde_json::Error> for CodecError {
    fn from(e: serde_json::Error) -> Self {
        CodecError::Json(e)
    }
}

/// `serde`'s default `Option<T>` deserialization treats a JSON `null` the
/// same as the field being absent — collapsing both to `None`. `result`
/// and `error` need to tell those apart: a response's `result` is
/// routinely a legitimate `null` (many LSP methods succeed with no
/// payload — `workspace/executeCommand`, `textDocument/rename` when
/// nothing changed, …), and that must still classify as a `Response`, not
/// `Ambiguous`. Wrapping the field in an extra `Option` (via this
/// deserializer, invoked only when the field is present at all) keeps
/// "absent" (`#[serde(default)]`, never invoked) and "present, value is
/// null" (`Some(Value::Null)`) distinguishable.
fn deserialize_present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// Untyped wire shape: every field optional, classified after parsing.
#[derive(Deserialize, Serialize)]
struct RawMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<RequestId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Value>,
    /// `None` = field absent; `Some(v)` = field present (`v` may itself be
    /// `serde_json::Value::Null`).
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "deserialize_present")]
    result: Option<serde_json::Value>,
    /// Same presence-vs-null distinction as `result`.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "deserialize_present")]
    error: Option<ResponseError>,
}

/// Reads headers up to the blank `\r\n` line, then exactly `Content-Length`
/// bytes of body. Blocks until one full frame is available; any error
/// (I/O, malformed header, truncated body, bad JSON, ambiguous shape) is
/// fatal for the connection — callers must not attempt to resynchronize.
pub fn read_message(r: &mut impl BufRead) -> Result<Message, CodecError> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = r.read_line(&mut line)?;
        if n == 0 {
            return Err(CodecError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "stream closed while reading headers",
            )));
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        let (name, value) = trimmed
            .split_once(':')
            .ok_or_else(|| CodecError::BadHeader(trimmed.to_string()))?;
        if name.eq_ignore_ascii_case("Content-Length") {
            let len: usize = value
                .trim()
                .parse()
                .map_err(|_| CodecError::BadHeader(trimmed.to_string()))?;
            content_length = Some(len);
        }
        // Other headers (e.g. Content-Type) are tolerated and ignored.
    }

    let len = content_length.ok_or(CodecError::MissingLength)?;
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;

    let raw: RawMessage = serde_json::from_slice(&body)?;
    classify(raw)
}

fn classify(raw: RawMessage) -> Result<Message, CodecError> {
    let RawMessage {
        id,
        method,
        params,
        result,
        error,
    } = raw;

    match (id, method, result, error) {
        (Some(id), Some(method), None, None) => Ok(Message::Request {
            id,
            method,
            params: params.unwrap_or(serde_json::Value::Null),
        }),
        (None, Some(method), None, None) => Ok(Message::Notification {
            method,
            params: params.unwrap_or(serde_json::Value::Null),
        }),
        (Some(id), None, Some(result), None) => Ok(Message::Response {
            id,
            result: Ok(result),
        }),
        (Some(id), None, None, Some(error)) => Ok(Message::Response {
            id,
            result: Err(error),
        }),
        _ => Err(CodecError::Ambiguous),
    }
}

/// Serializes `msg` and writes it with the exact `Content-Length: N\r\n\r\n`
/// framing — some servers reject a bare `\n\n` terminator.
pub fn write_message(w: &mut impl Write, msg: &Message) -> std::io::Result<()> {
    let raw = match msg {
        Message::Request { id, method, params } => RawMessage {
            id: Some(id.clone()),
            method: Some(method.clone()),
            params: Some(params.clone()),
            result: None,
            error: None,
        },
        Message::Notification { method, params } => RawMessage {
            id: None,
            method: Some(method.clone()),
            params: Some(params.clone()),
            result: None,
            error: None,
        },
        Message::Response { id, result: Ok(v) } => RawMessage {
            id: Some(id.clone()),
            method: None,
            params: None,
            result: Some(v.clone()),
            error: None,
        },
        Message::Response {
            id,
            result: Err(e),
        } => RawMessage {
            id: Some(id.clone()),
            method: None,
            params: None,
            result: None,
            error: Some(e.clone()),
        },
    };
    let body = serde_json::to_vec(&raw)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write!(w, "Content-Length: {}\r\n\r\n", body.len())?;
    w.write_all(&body)?;
    w.flush()
}

/// Allocates monotonically increasing integer request ids.
pub struct IdAllocator(i64);

impl Default for IdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl IdAllocator {
    pub fn new() -> Self {
        IdAllocator(0)
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> RequestId {
        self.0 += 1;
        RequestId::Int(self.0)
    }
}

#[cfg(test)]
mod tests {
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
    fn id_allocator_increments() {
        let mut alloc = IdAllocator::new();
        assert_eq!(alloc.next(), RequestId::Int(1));
        assert_eq!(alloc.next(), RequestId::Int(2));
        assert_eq!(alloc.next(), RequestId::Int(3));
    }
}
