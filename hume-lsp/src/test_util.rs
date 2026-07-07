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

/// Wraps `InlineLspBackend`, additionally recording every outgoing
/// notification's `(method, params)` into a shared log. Once a backend is
/// boxed into `Box<dyn LspBackend>` (as `LspState` does), the trait object
/// erases access to `InlineLspBackend::sent` — this lets a test recover the
/// wire stream anyway, for invariants that replay it against an oracle.
pub struct RecordingLspBackend {
    inner: InlineLspBackend,
    log: NotificationLog,
}

impl RecordingLspBackend {
    /// Returns the backend plus a shared handle to its notification log —
    /// keep the handle; the backend itself is typically moved into a
    /// `Box<dyn LspBackend>` immediately.
    pub fn new() -> (Self, NotificationLog) {
        let log = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                inner: InlineLspBackend::new(),
                log: log.clone(),
            },
            log,
        )
    }
}

impl LspBackend for RecordingLspBackend {
    fn start(&mut self, cmd: &str, args: &[String], root: &Path) -> std::io::Result<ServerId> {
        self.inner.start(cmd, args, root)
    }

    fn send(&mut self, server: ServerId, msg: Message) {
        if let Message::Notification { method, params } = &msg {
            self.log.borrow_mut().push((method.clone(), params.clone()));
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
