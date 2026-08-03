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
pub(crate) enum CodecError {
    Io(std::io::Error),
    /// Clean end-of-stream exactly at a frame boundary — no header bytes
    /// were read yet, so nothing was interrupted mid-flight. Distinguishes
    /// a server's voluntary exit (nothing to report as a crash) from a real
    /// truncation error partway through a frame, which stays `Io`.
    Eof,
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
            CodecError::Eof => write!(f, "end of stream"),
            CodecError::MissingLength => write!(f, "missing Content-Length header"),
            CodecError::BadHeader(line) => write!(f, "malformed header: {line:?}"),
            CodecError::Json(e) => write!(f, "json error: {e}"),
            CodecError::Ambiguous => {
                write!(
                    f,
                    "message body is neither request, response, nor notification"
                )
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
/// Deserialize-only — the write path uses [`RawMessageRef`] instead so
/// serializing a message never clones `params`/`result` (a `didOpen`
/// carries the whole document text).
#[derive(Deserialize)]
struct RawMessage {
    id: Option<RequestId>,
    method: Option<String>,
    params: Option<serde_json::Value>,
    /// `None` = field absent; `Some(v)` = field present (`v` may itself be
    /// `serde_json::Value::Null`).
    #[serde(default, deserialize_with = "deserialize_present")]
    result: Option<serde_json::Value>,
    /// Same presence-vs-null distinction as `result`.
    #[serde(default, deserialize_with = "deserialize_present")]
    error: Option<ResponseError>,
}

/// Borrowed twin of [`RawMessage`] for the write path — serialization must
/// not clone `params`/`result` (a `didOpen` carries the whole document
/// text).
#[derive(Serialize)]
struct RawMessageRef<'a> {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a RequestId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<&'a serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<&'a serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a ResponseError>,
}

/// Content-Length above this is never a legitimate LSP body — the largest
/// realistic message (a huge hover or diagnostics burst) is well under
/// this. Bounds the `vec![0u8; len]` allocation against a garbage or
/// hostile `Content-Length` value.
const MAX_CONTENT_LENGTH: usize = 256 * 1024 * 1024; // 256 MiB

/// A single header line above this length cannot be a legitimate
/// `Content-Length`/`Content-Type` header — bounds `read_line`'s growth
/// against a stream that never sends a newline.
const MAX_HEADER_LINE_LEN: usize = 64 * 1024; // 64 KiB

/// Reads one line (through the trailing `\n`, inclusive) via `fill_buf`/
/// `consume`, bounded to `max` bytes total. `BufRead::read_line` has no
/// built-in cap and would otherwise grow without limit against a stream
/// that never sends a newline. `Ok(None)` means a clean EOF with nothing
/// read at all; `Err` means the `max` bound was exceeded with no newline
/// found (a real I/O error is propagated as-is via `?` at the call site).
fn read_bounded_line(r: &mut impl BufRead, max: usize) -> std::io::Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    loop {
        let available = r.fill_buf()?;
        if available.is_empty() {
            return Ok(if buf.is_empty() { None } else { Some(buf) });
        }
        match available.iter().position(|&b| b == b'\n') {
            Some(pos) => {
                buf.extend_from_slice(&available[..=pos]);
                r.consume(pos + 1);
                return Ok(Some(buf));
            }
            None => {
                let n = available.len();
                buf.extend_from_slice(available);
                r.consume(n);
                if buf.len() > max {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("header line exceeds {max} bytes with no newline"),
                    ));
                }
            }
        }
    }
}

/// Reads headers up to the blank `\r\n` line, then exactly `Content-Length`
/// bytes of body. Blocks until one full frame is available; any error
/// (I/O, malformed header, truncated body, bad JSON, ambiguous shape) is
/// fatal for the connection — callers must not attempt to resynchronize.
pub(crate) fn read_message(r: &mut impl BufRead) -> Result<Message, CodecError> {
    let mut content_length: Option<usize> = None;
    let mut first_line = true;
    loop {
        let Some(bytes) = read_bounded_line(r, MAX_HEADER_LINE_LEN)? else {
            // A frame boundary (nothing read yet for this message) is a
            // clean, expected end-of-stream — a server that exited
            // voluntarily. Mid-header-block, it's a genuine truncation.
            if first_line {
                return Err(CodecError::Eof);
            }
            return Err(CodecError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "stream closed while reading headers",
            )));
        };
        first_line = false;
        let line = String::from_utf8_lossy(&bytes);
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
            if len > MAX_CONTENT_LENGTH {
                return Err(CodecError::BadHeader(trimmed.to_string()));
            }
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
pub(crate) fn write_message(w: &mut impl Write, msg: &Message) -> std::io::Result<()> {
    let raw = match msg {
        Message::Request { id, method, params } => RawMessageRef {
            jsonrpc: "2.0",
            id: Some(id),
            method: Some(method),
            params: Some(params),
            result: None,
            error: None,
        },
        Message::Notification { method, params } => RawMessageRef {
            jsonrpc: "2.0",
            id: None,
            method: Some(method),
            params: Some(params),
            result: None,
            error: None,
        },
        Message::Response { id, result: Ok(v) } => RawMessageRef {
            jsonrpc: "2.0",
            id: Some(id),
            method: None,
            params: None,
            result: Some(v),
            error: None,
        },
        Message::Response { id, result: Err(e) } => RawMessageRef {
            jsonrpc: "2.0",
            id: Some(id),
            method: None,
            params: None,
            result: None,
            error: Some(e),
        },
    };
    let body = serde_json::to_vec(&raw)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write!(w, "Content-Length: {}\r\n\r\n", body.len())?;
    w.write_all(&body)?;
    w.flush()
}

/// Allocates monotonically increasing integer request ids.
#[derive(Default)]
pub(crate) struct IdAllocator(i64);

impl IdAllocator {
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> RequestId {
        self.0 += 1;
        RequestId::Int(self.0)
    }
}

#[cfg(test)]
mod tests;
