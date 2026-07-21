//! The seam the editor holds: `Box<dyn LspBackend>`, mirroring
//! `parse_worker: Box<dyn ParseBackend>` in `hume-treesitter`.
//!
//! Transport-flavored only — no capabilities, no `text_gen`, no buffer
//! knowledge. That client-level state lives above this trait.

use std::path::Path;
use std::sync::Arc;

use crate::codec::Message;
pub use crate::transport::WakeCallback;
use crate::transport::{InboundEvent, ServerHandle};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ServerId(pub u32);

pub trait LspBackend {
    /// Spawn (threaded) or register (inline) a server. Handshake is the
    /// client layer's job — this is transport-level only.
    fn start(&mut self, cmd: &str, args: &[String], root: &Path) -> std::io::Result<ServerId>;
    fn send(&mut self, server: ServerId, msg: Message);
    /// All events that arrived since the last drain. Arrival order is
    /// preserved per server; cross-server interleaving is unspecified.
    fn drain(&mut self) -> Vec<(ServerId, InboundEvent)>;
    fn shutdown(&mut self, server: ServerId);
}

/// Production backend: one real server process per registration.
pub struct ThreadedLspBackend {
    servers: std::collections::HashMap<ServerId, ServerHandle>,
    next: u32,
    wake: WakeCallback,
}

impl ThreadedLspBackend {
    pub fn new() -> Self {
        Self::with_waker(Arc::new(|| {}))
    }

    /// Like [`Self::new`], but `wake` is passed to every spawned server's
    /// reader/stderr threads, so the editor's main loop wakes instead of
    /// polling for completion.
    pub fn with_waker(wake: WakeCallback) -> Self {
        Self {
            servers: std::collections::HashMap::new(),
            next: 0,
            wake,
        }
    }
}

impl Default for ThreadedLspBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl LspBackend for ThreadedLspBackend {
    fn start(&mut self, cmd: &str, args: &[String], root: &Path) -> std::io::Result<ServerId> {
        let handle = ServerHandle::spawn(cmd, args, root, Arc::clone(&self.wake))?;
        let id = ServerId(self.next);
        self.next += 1;
        self.servers.insert(id, handle);
        Ok(id)
    }

    fn send(&mut self, server: ServerId, msg: Message) {
        if let Some(handle) = self.servers.get(&server) {
            handle.send(msg);
        }
    }

    fn drain(&mut self) -> Vec<(ServerId, InboundEvent)> {
        let mut out = Vec::new();
        for (&id, handle) in self.servers.iter_mut() {
            for ev in handle.try_recv_all() {
                out.push((id, ev));
            }
        }
        out
    }

    fn shutdown(&mut self, server: ServerId) {
        // Removing the handle drops it: Drop runs kill -> wait -> join.
        self.servers.remove(&server);
    }
}

#[cfg(test)]
mod tests;
