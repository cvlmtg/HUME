//! Editor-side LSP state: holds the backend and drains its events at frame
//! cadence. Built incrementally — C4 wires the backend + `AsyncSource`
//! plumbing only; C5 adds per-client lifecycle state, C6 adds callback
//! dispatch, C7–C10 add document sync, diagnostics, registration, and
//! observability commands on top of this module.

use hume_lsp::backend::{LspBackend, ThreadedLspBackend};
#[cfg(test)]
use hume_lsp::inline::InlineLspBackend;

use super::async_source::AsyncSource;

pub(crate) struct LspState {
    backend: Box<dyn LspBackend>,
}

impl LspState {
    /// Production constructor: one real server process per registration (C8).
    pub(crate) fn new_threaded() -> Self {
        Self {
            backend: Box::new(ThreadedLspBackend::new()),
        }
    }

    /// Test constructor: scripted responses, no process, no threads.
    #[cfg(test)]
    pub(crate) fn new_inline() -> Self {
        Self {
            backend: Box::new(InlineLspBackend::new()),
        }
    }

    /// Test-only: swap in an already-scripted backend (e.g. one built via
    /// `InlineLspBackend::with_default_handshake` plus extra `respond_to`
    /// calls) — `backend_mut` only exposes the trait object, which can't
    /// reach `InlineLspBackend`'s scripting methods.
    #[cfg(test)]
    pub(crate) fn from_backend_for_test(backend: Box<dyn LspBackend>) -> Self {
        Self { backend }
    }

    /// Reach the raw backend directly. Used by the C4 round-trip test and by
    /// the per-frame drain below; C6 replaces the latter with real callback
    /// dispatch instead of a bare drain-and-discard.
    pub(crate) fn backend_mut(&mut self) -> &mut dyn LspBackend {
        self.backend.as_mut()
    }
}

impl AsyncSource for LspState {
    fn has_pending(&self) -> bool {
        self.backend.has_pending()
    }
    // next_deadline: no override yet — the heartbeat (poll at ~5Hz while any
    // server is Running, so idle-time server pushes like publishDiagnostics
    // aren't stuck behind the next keypress; see the LSP hub's "Idle wake"
    // decision) activates once C5 adds per-client `ServerState` tracking.
}
