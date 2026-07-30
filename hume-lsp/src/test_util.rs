//! Test-only doubles exposed to consumer crates via the `test-util` feature
//! — mirrors the `hume-treesitter`/`hume-scripting` precedent.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use crate::backend::{LspBackend, ServerId};
use crate::codec::{Message, RequestId, ResponseError};
use crate::inline::InlineLspBackend;
use crate::transport::InboundEvent;

/// Shared log of `(method, params)` for every notification a
/// `RecordingLspBackend` sends.
pub type NotificationLog = Rc<RefCell<Vec<(String, serde_json::Value)>>>;

/// Shared log of `(server, method, params)` for every *request* a
/// `RecordingLspBackend` sends — separate from `NotificationLog` so
/// existing exact-count assertions over the notification stream (e.g. the
/// didOpen/didChange sequence) aren't disturbed by requests (e.g.
/// `initialize`) also flowing through `send`.
pub type RequestLog = Rc<RefCell<Vec<(ServerId, String, serde_json::Value)>>>;

/// Shared log of `(server, id, result)` for every *response* a
/// `RecordingLspBackend` sends — i.e. what the editor's dispatch table
/// answered a server-initiated request with. Kept separate from the other
/// two logs for the same reason: a test asserting on one stream shouldn't
/// have to account for the others.
pub type ResponseLog = Rc<
    RefCell<
        Vec<(
            ServerId,
            RequestId,
            Result<serde_json::Value, ResponseError>,
        )>,
    >,
>;

/// Wraps `InlineLspBackend`, additionally recording every outgoing
/// notification's `(method, params)` into a shared log. Once a backend is
/// boxed into `Box<dyn LspBackend>` (as `LspState` does), the trait object
/// erases access to `InlineLspBackend::sent` — this lets a test recover the
/// wire stream anyway, for invariants that replay it against an oracle.
pub struct RecordingLspBackend {
    inner: InlineLspBackend,
    log: NotificationLog,
    request_log: RequestLog,
    response_log: ResponseLog,
}

impl RecordingLspBackend {
    /// Returns the backend plus shared handles to its notification and
    /// request logs — keep the handles; the backend itself is typically
    /// moved into a `Box<dyn LspBackend>` immediately. Callers that only
    /// need the notification log bind the request log to `_`.
    pub fn new() -> (Self, NotificationLog, RequestLog) {
        let (backend, log, request_log, _) = Self::from_inline(InlineLspBackend::new());
        (backend, log, request_log)
    }

    /// Same as `new`, but pre-scripted with a canned `initialize` success
    /// response — for tests that need the client to reach `Running` (and
    /// therefore flush anything it queued while `Starting`) via a plain
    /// `drain_lsp()` call.
    pub fn with_default_handshake() -> (Self, NotificationLog, RequestLog) {
        let (backend, log, request_log, _) =
            Self::from_inline(InlineLspBackend::with_default_handshake());
        (backend, log, request_log)
    }

    /// Same as `new`, but returns the response log instead — for tests
    /// asserting what the editor answered a server-initiated request with
    /// (e.g. `workspace/configuration`), rather than what the client itself
    /// sent.
    pub fn with_response_log() -> (Self, ResponseLog) {
        let (backend, _, _, response_log) = Self::from_inline(InlineLspBackend::new());
        (backend, response_log)
    }

    fn from_inline(inner: InlineLspBackend) -> (Self, NotificationLog, RequestLog, ResponseLog) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let request_log = Rc::new(RefCell::new(Vec::new()));
        let response_log = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                inner,
                log: log.clone(),
                request_log: request_log.clone(),
                response_log: response_log.clone(),
            },
            log,
            request_log,
            response_log,
        )
    }

    /// Pass-throughs to the wrapped `InlineLspBackend` — call before boxing
    /// into `Box<dyn LspBackend>`, same as `InlineLspBackend` itself.
    pub fn respond_to(&mut self, method: &str, result: serde_json::Value) {
        self.inner.respond_to(method, result);
    }

    pub fn fail_with(&mut self, method: &str, code: i64, msg: &str) {
        self.inner.fail_with(method, code, msg);
    }

    pub fn push_from_server(&mut self, server: ServerId, msg: Message) {
        self.inner.push_from_server(server, msg);
    }
}

impl LspBackend for RecordingLspBackend {
    fn start(
        &mut self,
        cmd: &str,
        args: &[String],
        root: &Path,
        env: &[(String, String)],
    ) -> std::io::Result<ServerId> {
        self.inner.start(cmd, args, root, env)
    }

    fn send(&mut self, server: ServerId, msg: Message) {
        match &msg {
            Message::Notification { method, params } => {
                self.log.borrow_mut().push((method.clone(), params.clone()));
            }
            Message::Request { method, params, .. } => {
                self.request_log
                    .borrow_mut()
                    .push((server, method.clone(), params.clone()));
            }
            Message::Response { id, result } => {
                self.response_log
                    .borrow_mut()
                    .push((server, id.clone(), result.clone()));
            }
        }
        self.inner.send(server, msg);
    }

    fn drain(&mut self) -> Vec<(ServerId, InboundEvent)> {
        self.inner.drain()
    }

    fn shutdown(&mut self, server: ServerId) {
        self.inner.shutdown(server);
    }
}
