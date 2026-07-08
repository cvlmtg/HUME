//! Test-only doubles exposed to consumer crates via the `test-util` feature
//! — mirrors the `hume-treesitter`/`hume-scripting` precedent.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use crate::backend::{LspBackend, ServerId};
use crate::codec::Message;
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

/// Wraps `InlineLspBackend`, additionally recording every outgoing
/// notification's `(method, params)` into a shared log. Once a backend is
/// boxed into `Box<dyn LspBackend>` (as `LspState` does), the trait object
/// erases access to `InlineLspBackend::sent` — this lets a test recover the
/// wire stream anyway, for invariants that replay it against an oracle.
pub struct RecordingLspBackend {
    inner: InlineLspBackend,
    log: NotificationLog,
    request_log: RequestLog,
}

impl RecordingLspBackend {
    /// Returns the backend plus a shared handle to its notification log —
    /// keep the handle; the backend itself is typically moved into a
    /// `Box<dyn LspBackend>` immediately.
    pub fn new() -> (Self, NotificationLog) {
        let (backend, log, _requests) = Self::from_inline(InlineLspBackend::new());
        (backend, log)
    }

    /// Same as `new`, but pre-scripted with a canned `initialize` success
    /// response — for tests that need the client to reach `Running` (and
    /// therefore flush anything it queued while `Starting`) via a plain
    /// `drain_lsp()` call.
    pub fn with_default_handshake() -> (Self, NotificationLog) {
        let (backend, log, _requests) = Self::from_inline(InlineLspBackend::with_default_handshake());
        (backend, log)
    }

    /// Same as `with_default_handshake`, but also returns a shared handle
    /// to the request log — for invariants that need to inspect what a
    /// command actually sent (e.g. a request's `params`), not just
    /// whether a scripted response landed.
    pub fn with_default_handshake_and_requests() -> (Self, NotificationLog, RequestLog) {
        Self::from_inline(InlineLspBackend::with_default_handshake())
    }

    fn from_inline(inner: InlineLspBackend) -> (Self, NotificationLog, RequestLog) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let request_log = Rc::new(RefCell::new(Vec::new()));
        (Self { inner, log: log.clone(), request_log: request_log.clone() }, log, request_log)
    }

    /// Pass-throughs to the wrapped `InlineLspBackend` — call before boxing
    /// into `Box<dyn LspBackend>`, same as `InlineLspBackend` itself.
    pub fn respond_to(&mut self, method: &str, result: serde_json::Value) {
        self.inner.respond_to(method, result);
    }

    pub fn push_from_server(&mut self, server: ServerId, msg: Message) {
        self.inner.push_from_server(server, msg);
    }
}

impl LspBackend for RecordingLspBackend {
    fn start(&mut self, cmd: &str, args: &[String], root: &Path) -> std::io::Result<ServerId> {
        self.inner.start(cmd, args, root)
    }

    fn send(&mut self, server: ServerId, msg: Message) {
        match &msg {
            Message::Notification { method, params } => {
                self.log.borrow_mut().push((method.clone(), params.clone()));
            }
            Message::Request { method, params, .. } => {
                self.request_log.borrow_mut().push((server, method.clone(), params.clone()));
            }
            Message::Response { .. } => {}
        }
        self.inner.send(server, msg);
    }

    fn drain(&mut self) -> Vec<(ServerId, InboundEvent)> {
        self.inner.drain()
    }

    fn has_pending(&self) -> bool {
        self.inner.has_pending()
    }

    fn shutdown(&mut self, server: ServerId) {
        self.inner.shutdown(server);
    }
}
